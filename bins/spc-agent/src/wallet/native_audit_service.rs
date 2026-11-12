//! Bounded executor for the in-process typed native_audit service. Wire and
//! crypto remain in the owner codec/shared auditor. The concrete wallet adapter
//! supplies endpoint/process authorization and final release validation.
use anyhow::{ensure, Context};
use std::future::Future;
use std::num::NonZeroU32;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, Semaphore};
use tokio::task::JoinSet;
use tokio::time::Instant;

pub type AuditFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// Kernel credentials only, never a PID/UID asserted inside a request frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KernelPeer {
    pub uid: u32,
    pub gid: u32,
    pub pid: NonZeroU32,
}
impl KernelPeer {
    fn read(stream: &UnixStream) -> anyhow::Result<Self> {
        let cred = stream.peer_cred()?;
        let pid = cred
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .and_then(NonZeroU32::new)
            .context("native audit peer PID unavailable")?;
        Ok(Self {
            uid: cred.uid(),
            gid: cred.gid(),
            pid,
        })
    }
}

/// Transport resource policy, not public wire constants. Exact framing/limits
/// must come from the native codec in the future typed adapter.
#[derive(Clone, Copy)]
pub struct Limits {
    pub concurrent_sessions: usize,
    pub total_deadline: Duration,
    pub max_encoded_response_bytes: usize,
}
impl Limits {
    fn validate(self) -> anyhow::Result<()> {
        ensure!(
            (1..=4).contains(&self.concurrent_sessions),
            "native audit session bound"
        );
        ensure!(
            !self.total_deadline.is_zero() && self.total_deadline <= Duration::from_secs(10),
            "native audit deadline bound"
        );
        ensure!(
            (1..=16 * 1024 * 1024).contains(&self.max_encoded_response_bytes),
            "native audit response bound"
        );
        Ok(())
    }
}

/// The runtime/owned-wallet endpoint adapter must validate its existing 0700
/// root, socket inode/UID, process/executable lease and current placement here.
/// The server checks it at startup, admission and immediately before release.
pub trait EndpointLease: Send + Sync + 'static {
    fn owner_uid(&self) -> u32;
    fn validate(&self) -> anyhow::Result<()>;
}

/// Implemented by the actual runtime binding adapter. `Scope` must bind the
/// current run, identity, miner/connections, PID/start token, executable and
/// config generation. No serialized peer-registry schema is invented here.
pub trait PeerAuthorizer: Send + Sync + 'static {
    type Scope: Send + Sync + 'static;
    fn authorize<'a>(
        &'a self,
        stream: &'a mut UnixStream,
        peer: KernelPeer,
        deadline: Instant,
    ) -> AuditFuture<'a, Self::Scope>;
    fn revalidate<'a>(&'a self, peer: KernelPeer, scope: &'a Self::Scope) -> AuditFuture<'a, ()>;
}

/// The typed codec/shared-auditor adapter owns bounded request decoding,
/// actual ReceivedJob verification, newest-candidate cancellation, exact nonce
/// replay/session rules and collector COMMITTED evidence. `finish` must check
/// current candidate/expiry before signing ONLY the codec's activation tag.
/// It returns the already encoded canonical response frame; no funds API,
/// wallet store or operation lock is supplied by this execution scaffold.
pub trait ProtocolHandler<S>: Send + Sync + 'static {
    type Pending: Send + 'static;
    fn prepare<'a>(
        &'a self,
        stream: &'a mut UnixStream,
        scope: &'a S,
        deadline: Instant,
    ) -> AuditFuture<'a, Self::Pending>;
    fn finish<'a>(
        &'a self,
        pending: Self::Pending,
        scope: &'a S,
        deadline: Instant,
    ) -> AuditFuture<'a, Vec<u8>>;
    fn validate_release<'a>(&'a self, scope: &'a S, bytes: &'a [u8]) -> AuditFuture<'a, ()>;
}

pub struct Service {
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
}
impl Service {
    /// One dedicated I/O thread INSIDE the existing wallet process. Synchronous
    /// bounded proof verification cannot monopolize its maintenance executor.
    /// The caller supplies an already bound listener from its owned Unix lease.
    pub async fn start<A, H>(
        listener: StdUnixListener,
        lease: Arc<dyn EndpointLease>,
        limits: Limits,
        authorizer: Arc<A>,
        handler: Arc<H>,
    ) -> anyhow::Result<Self>
    where
        A: PeerAuthorizer,
        H: ProtocolHandler<A::Scope>,
    {
        limits.validate()?;
        lease.validate()?;
        listener.set_nonblocking(true)?;
        let (shutdown, stop) = oneshot::channel();
        let (ready, started) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("wallet-native-audit".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async move {
                    let listener = UnixListener::from_std(listener)?;
                    lease.validate()?;
                    let _ = ready.send(());
                    serve(listener, lease, limits, authorizer, handler, stop).await
                })
            })?;
        let mut service = Self {
            shutdown: Some(shutdown),
            thread: Some(thread),
        };
        if started.await.is_err() {
            service.stop_inner().await?;
            anyhow::bail!("native audit executor unavailable");
        }
        Ok(service)
    }

    pub async fn stop(mut self) -> anyhow::Result<()> {
        self.stop_inner().await
    }

    async fn stop_inner(&mut self) -> anyhow::Result<()> {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || {
                thread
                    .join()
                    .map_err(|_| anyhow::anyhow!("native audit executor failed"))?
            })
            .await??;
        }
        Ok(())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
    }
}

async fn serve<A, H>(
    listener: UnixListener,
    lease: Arc<dyn EndpointLease>,
    limits: Limits,
    authorizer: Arc<A>,
    handler: Arc<H>,
    mut stop: oneshot::Receiver<()>,
) -> anyhow::Result<()>
where
    A: PeerAuthorizer,
    H: ProtocolHandler<A::Scope>,
{
    let permits = Arc::new(Semaphore::new(limits.concurrent_sessions));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (mut stream, _) = accepted?;
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                let lease = lease.clone(); let authorizer = authorizer.clone(); let handler = handler.clone();
                let deadline = Instant::now() + limits.total_deadline;
                tasks.spawn(async move {
                    let _permit = permit;
                    // Errors close the connection. No ad-hoc success/error
                    // wire is emitted before the native codec is integrated.
                    let _ = tokio::time::timeout_at(deadline, async {
                        lease.validate()?;
                        let peer = KernelPeer::read(&stream)?;
                        ensure!(peer.uid == lease.owner_uid(), "native audit peer UID differs from owned endpoint");
                        let scope = authorizer.authorize(&mut stream, peer, deadline).await?;
                        let pending = handler.prepare(&mut stream, &scope, deadline).await?;
                        authorizer.revalidate(peer, &scope).await?;
                        lease.validate()?;
                        let bytes = handler.finish(pending, &scope, deadline).await?;
                        ensure!(!bytes.is_empty() && bytes.len() <= limits.max_encoded_response_bytes, "native audit encoded response bound");
                        authorizer.revalidate(peer, &scope).await?;
                        lease.validate()?;
                        handler.validate_release(&scope, &bytes).await?;
                        ensure!(Instant::now() < deadline, "native audit result expired before release");
                        stream.write_all(&bytes).await?;
                        stream.shutdown().await?;
                        Ok::<_, anyhow::Error>(())
                    }).await;
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

#[cfg(test)]
#[path = "native_audit_service_tests.rs"]
mod tests;
