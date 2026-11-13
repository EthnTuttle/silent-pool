//! Actual native activation service in the existing wallet process. Native wire
//! and signature preimages belong to spc-scan; full source verification belongs
//! to UnixSourceAuditor. Only the wallet owner loop borrows its signing key.
use super::native_audit_io as io;
use super::native_audit_service::{
    self as service, AuditFuture, EndpointLease, KernelPeer, PeerAuthorizer, ProtocolHandler,
};
use super::native_source_audit::{auditor_channel, AuditorFactory, AuditorRequests, SourceBinding};
use super::WalletKeys;
use anyhow::{ensure, Context};
use bitcoin::secp256k1::{Keypair, Secp256k1};
use spc_scan::{
    native_activation::{
        self as wire, ActivationReceipt, CandidateRequest, ProcessIdentity, Response, UnknownCode,
        WalletInstance,
    },
    source_feed::{self, VerifiedSourceEvidence},
};
use std::{
    collections::BTreeMap,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

fn now() -> anyhow::Result<u64> {
    super::ui_inputs::now_ms()
}
fn framed(response: &Response) -> anyhow::Result<Vec<u8>> {
    let body = wire::encode_response(response).map_err(anyhow::Error::msg)?;
    let mut bytes = (u32::try_from(body.len())?).to_le_bytes().to_vec();
    bytes.extend(body);
    Ok(bytes)
}
fn unknown(request: &CandidateRequest, code: UnknownCode) -> anyhow::Result<Vec<u8>> {
    framed(&Response::Unknown {
        schema: wire::SCHEMA.into(),
        request_digest: request.signing_digest().map_err(anyhow::Error::msg)?,
        candidate_epoch: request.candidate_epoch,
        nonce: request.nonce,
        code,
    })
}
async fn request(stream: &mut UnixStream) -> anyhow::Result<CandidateRequest> {
    let n = stream.read_u32_le().await? as usize;
    ensure!(
        n > 0 && n <= wire::MAX_REQUEST_BYTES,
        "native request frame bound"
    );
    let mut bytes = vec![0; n];
    stream.read_exact(&mut bytes).await?;
    wire::decode_request(&bytes).map_err(anyhow::Error::msg)
}

type Connection = (String, String);
struct Latest {
    rank: (u64, u64, u64),
    digest: [u8; 32],
    cancel: watch::Sender<bool>,
    cached: Option<(u64, Vec<u8>)>,
}
#[derive(Default)]
struct Tracker {
    latest: BTreeMap<Connection, Latest>,
    nonces: BTreeMap<[u8; 16], ([u8; 32], u64)>,
}
impl Tracker {
    fn admit(
        &mut self,
        r: &CandidateRequest,
        t: u64,
    ) -> Result<(watch::Receiver<bool>, Option<Vec<u8>>), UnknownCode> {
        let digest = r.signing_digest().map_err(|_| UnknownCode::Rejected)?;
        let key = (r.miner_id.clone(), r.connection_id.clone());
        let rank = (r.peer_generation, r.session_epoch, r.candidate_epoch);
        self.nonces.retain(|_, (_, expiry)| *expiry > t);
        if let Some((old, _)) = self.nonces.get(&r.nonce) {
            if old != &digest {
                return Err(UnknownCode::Rejected);
            }
        }
        if let Some(old) = self.latest.get(&key) {
            if rank < old.rank {
                return Err(UnknownCode::Superseded);
            }
            if rank == old.rank {
                if digest != old.digest {
                    return Err(UnknownCode::Rejected);
                }
                return match &old.cached {
                    Some((expiry, bytes)) if *expiry > t => {
                        Ok((old.cancel.subscribe(), Some(bytes.clone())))
                    }
                    _ => Err(UnknownCode::Unavailable),
                };
            }
        }
        if (!self.latest.contains_key(&key) && self.latest.len() >= 256)
            || self.nonces.len() >= 1024
        {
            return Err(UnknownCode::Unavailable);
        }
        let (cancel, rx) = watch::channel(false);
        if let Some(old) = self.latest.insert(
            key,
            Latest {
                rank,
                digest,
                cancel,
                cached: None,
            },
        ) {
            let _ = old.cancel.send(true);
        }
        self.nonces.insert(r.nonce, (digest, r.expires_at_unix_ms));
        Ok((rx, None))
    }
    fn current(&self, r: &CandidateRequest) -> bool {
        self.latest
            .get(&(r.miner_id.clone(), r.connection_id.clone()))
            .is_some_and(|v| r.signing_digest().is_ok_and(|d| d == v.digest))
    }
    fn cache(&mut self, r: &CandidateRequest, expiry: u64, bytes: Vec<u8>) -> anyhow::Result<()> {
        ensure!(self.current(r), "native candidate superseded");
        self.latest
            .get_mut(&(r.miner_id.clone(), r.connection_id.clone()))
            .context("native current candidate")?
            .cached = Some((expiry, bytes));
        Ok(())
    }
}

struct ContextState {
    instance: WalletInstance,
    root: PathBuf,
    private: PathBuf,
    root_device: u64,
    root_inode: u64,
    keys: wire::VerificationKeys,
    source: SourceBinding,
    published: AtomicBool,
    tracker: Mutex<Tracker>,
}
struct SocketCreation {
    path: PathBuf,
    device: u64,
    inode: u64,
    armed: bool,
}
impl Drop for SocketCreation {
    fn drop(&mut self) {
        if self.armed
            && self.path.symlink_metadata().is_ok_and(|m| {
                m.file_type().is_socket() && m.dev() == self.device && m.ino() == self.inode
            })
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
impl ContextState {
    fn endpoint(&self) -> anyhow::Result<()> {
        io::private_dir(&self.root, self.instance.process.uid)?;
        let root = std::fs::symlink_metadata(&self.root)?;
        ensure!(
            root.dev() == self.root_device && root.ino() == self.root_inode,
            "native root replaced"
        );
        let socket = std::fs::symlink_metadata(&self.instance.socket_path)?;
        ensure!(
            socket.file_type().is_socket()
                && socket.uid() == self.instance.process.uid
                && socket.mode() & 0o777 == 0o600
                && socket.dev() == self.instance.socket_device
                && socket.ino() == self.instance.socket_inode,
            "native socket replaced"
        );
        ensure!(
            io::start_ticks(self.instance.process.pid)? == self.instance.process.start_time_ticks,
            "native wallet lifecycle changed"
        );
        ensure!(
            io::hash(&io::private_read(
                Path::new(&self.instance.config_path),
                self.instance.process.uid
            )?) == self.instance.process.config_sha256,
            "native wallet config changed"
        );
        if self.published.load(Ordering::SeqCst) {
            let current = wire::decode_instance(&io::private_read(
                &self.root.join(wire::INSTANCE_FILENAME),
                self.instance.process.uid,
            )?)
            .map_err(anyhow::Error::msg)?;
            ensure!(
                current == self.instance,
                "native published instance changed"
            );
        }
        Ok(())
    }
    fn authorize(
        &self,
        r: &CandidateRequest,
        kernel: KernelPeer,
    ) -> anyhow::Result<ProcessIdentity> {
        ensure!(
            self.published.load(Ordering::SeqCst),
            "native instance not published"
        );
        self.endpoint()?;
        wire::validate_request_time(r, now()?).map_err(anyhow::Error::msg)?;
        ensure!(
            r.wallet_instance == self.instance.instance_id
                && r.run_id == self.instance.run_id
                && r.identity == self.instance.identity,
            "native request belongs to another wallet lifecycle"
        );
        let registry = wire::decode_registry(&io::private_read(
            &self.root.join(wire::REGISTRY_FILENAME),
            self.instance.process.uid,
        )?)
        .map_err(anyhow::Error::msg)?;
        // The path comes only from the controller's private registry. The PID
        // and UID used to observe it come only from this accepted Unix socket.
        let peer = registry
            .peers
            .iter()
            .find(|p| p.miner_id == r.miner_id && p.connection_id == r.connection_id)
            .context("native peer absent")?;
        ensure!(
            peer.process.pid == kernel.pid.get() && peer.process.uid == kernel.uid,
            "native kernel peer is not registered"
        );
        let (observed, _) = io::process(
            kernel.pid.get(),
            kernel.uid,
            &self.private,
            Some(Path::new(&peer.config_path)),
            &self.instance.process.run_token_sha256,
        )?;
        ensure!(
            observed.run_token_sha256 == self.instance.process.run_token_sha256,
            "native peer run token differs from wallet"
        );
        registry
            .authorize(&self.instance, r, &observed, now()?)
            .map_err(anyhow::Error::msg)?;
        Ok(observed)
    }
    fn current(&self, r: &CandidateRequest) -> anyhow::Result<()> {
        ensure!(
            self.published.load(Ordering::SeqCst)
                && r.wallet_instance == self.instance.instance_id
                && r.identity == self.instance.identity
                && r.run_id == self.instance.run_id,
            "native wallet instance mismatch"
        );
        wire::validate_request_time(r, now()?).map_err(anyhow::Error::msg)?;
        ensure!(
            self.tracker
                .lock()
                .map_err(|_| anyhow::anyhow!("native tracker"))?
                .current(r),
            "native candidate superseded"
        );
        Ok(())
    }
    fn retire(&self) -> anyhow::Result<()> {
        self.endpoint()?;
        let current = wire::decode_instance(&io::private_read(
            &self.root.join(wire::INSTANCE_FILENAME),
            self.instance.process.uid,
        )?)
        .map_err(anyhow::Error::msg)?;
        ensure!(
            current == self.instance,
            "native retirement instance changed"
        );
        // Registry belongs to the controller and remains a tombstone for the
        // previous instance. Only this exact lifecycle's own files are removed.
        std::fs::remove_file(self.root.join(wire::INSTANCE_FILENAME))?;
        self.published.store(false, Ordering::SeqCst);
        std::fs::remove_file(&self.instance.socket_path)?;
        std::fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
impl EndpointLease for ContextState {
    fn owner_uid(&self) -> u32 {
        self.instance.process.uid
    }
    fn validate(&self) -> anyhow::Result<()> {
        self.endpoint()
    }
}
struct Scope {
    request: CandidateRequest,
    peer: KernelPeer,
    observed: ProcessIdentity,
    cancel: watch::Receiver<bool>,
    cached: Option<Vec<u8>>,
}
struct Authorizer {
    context: Arc<ContextState>,
}
impl PeerAuthorizer for Authorizer {
    type Scope = Arc<Scope>;
    fn authorize<'a>(
        &'a self,
        stream: &'a mut UnixStream,
        peer: KernelPeer,
        _: Instant,
    ) -> AuditFuture<'a, Self::Scope> {
        Box::pin(async move {
            let r = request(stream).await?;
            let admitted = (|| {
                let observed = self.context.authorize(&r, peer)?;
                let result = self
                    .context
                    .tracker
                    .lock()
                    .map_err(|_| anyhow::anyhow!("native tracker"))?
                    .admit(&r, now()?);
                Ok::<_, anyhow::Error>((observed, result))
            })();
            let (observed, slot) = match admitted {
                Ok(v) => v,
                Err(_) => {
                    stream
                        .write_all(&unknown(&r, UnknownCode::Rejected)?)
                        .await?;
                    anyhow::bail!("native admission rejected");
                }
            };
            let (cancel, cached) = match slot {
                Ok(v) => v,
                Err(code) => {
                    stream.write_all(&unknown(&r, code)?).await?;
                    anyhow::bail!("native candidate unavailable");
                }
            };
            Ok(Arc::new(Scope {
                request: r,
                peer,
                observed,
                cancel,
                cached,
            }))
        })
    }
    fn revalidate<'a>(&'a self, peer: KernelPeer, scope: &'a Self::Scope) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            ensure!(peer == scope.peer, "native peer changed");
            ensure!(
                self.context.authorize(&scope.request, peer)? == scope.observed,
                "native process/config changed"
            );
            Ok(())
        })
    }
}
enum Pending {
    Encoded(Vec<u8>),
    Evidence(Box<VerifiedSourceEvidence>),
    Unknown(UnknownCode),
}
struct SignRequest {
    scope: Arc<Scope>,
    evidence: VerifiedSourceEvidence,
    deadline: Instant,
    reply: oneshot::Sender<anyhow::Result<Vec<u8>>>,
}
struct Handler {
    context: Arc<ContextState>,
    auditors: AuditorRequests,
    signer: mpsc::Sender<SignRequest>,
}
impl ProtocolHandler<Arc<Scope>> for Handler {
    type Pending = Pending;
    fn prepare<'a>(
        &'a self,
        stream: &'a mut UnixStream,
        scope: &'a Arc<Scope>,
        deadline: Instant,
    ) -> AuditFuture<'a, Pending> {
        Box::pin(async move {
            if let Some(bytes) = &scope.cached {
                return Ok(Pending::Encoded(bytes.clone()));
            }
            let mut cancel = scope.cancel.clone();
            let mut extra = [0u8; 1];
            let audit = async {
                let auditor = self.auditors.request(deadline).await?;
                auditor
                    .verify_received(&scope.request.job, scope.request.nonce, deadline)
                    .await
            };
            tokio::select! {
                biased;
                _=cancel.changed()=>Ok(Pending::Unknown(UnknownCode::Superseded)),
                _=stream.read(&mut extra)=>Ok(Pending::Unknown(UnknownCode::Rejected)),
                result=audit=>Ok(match result {Ok(evidence)=>Pending::Evidence(Box::new(evidence)),Err(_)=>Pending::Unknown(if Instant::now()>=deadline {UnknownCode::Timeout}else {UnknownCode::Unavailable})}),
            }
        })
    }
    fn finish<'a>(
        &'a self,
        pending: Pending,
        scope: &'a Arc<Scope>,
        deadline: Instant,
    ) -> AuditFuture<'a, Vec<u8>> {
        Box::pin(async move {
            match pending {
                Pending::Encoded(bytes) => {
                    self.context.current(&scope.request)?;
                    Ok(bytes)
                }
                Pending::Unknown(code) => unknown(&scope.request, code),
                Pending::Evidence(evidence) => {
                    if self.context.current(&scope.request).is_err() {
                        return unknown(&scope.request, UnknownCode::Superseded);
                    }
                    let (reply, result) = oneshot::channel();
                    if self
                        .signer
                        .try_send(SignRequest {
                            scope: scope.clone(),
                            evidence: *evidence,
                            deadline,
                            reply,
                        })
                        .is_err()
                    {
                        return unknown(&scope.request, UnknownCode::Unavailable);
                    }
                    match tokio::time::timeout_at(deadline, result).await {
                        Ok(Ok(Ok(bytes))) => Ok(bytes),
                        _ => unknown(&scope.request, UnknownCode::Unavailable),
                    }
                }
            }
        })
    }
    fn validate_release<'a>(
        &'a self,
        scope: &'a Arc<Scope>,
        bytes: &'a [u8],
    ) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            ensure!(bytes.len() >= 4, "native response framing");
            let response = wire::decode_response(&bytes[4..]).map_err(anyhow::Error::msg)?;
            if matches!(response, Response::Approved { .. }) {
                self.context.current(&scope.request)?;
                wire::verify_response(&scope.request, &response, &self.context.keys, now()?)
                    .map_err(anyhow::Error::msg)?;
            }
            Ok(())
        })
    }
}

pub(super) struct NativeRuntime {
    context: Arc<ContextState>,
    service: Option<service::Service>,
    factory: Option<AuditorFactory>,
    signing: Option<mpsc::Receiver<SignRequest>>,
}
impl NativeRuntime {
    pub(super) async fn start(config: &crate::config::AgentConfig) -> anyhow::Result<Option<Self>> {
        let Some(ark) = &config.ark else {
            return Ok(None);
        };
        let Some(profile) = &ark.native_audit else {
            return Ok(None);
        };
        profile.validate(&config.upstream.sp_address, config.ark_network())?;
        let (config_path, config_hash) = config
            .loaded_from
            .as_ref()
            .context("native audit requires actual loaded config provenance")?;
        let root = ark
            .datadir
            .as_ref()
            .context("native datadir")?
            .canonicalize()?;
        ensure!(
            ark.datadir.as_ref() == Some(&root)
                && root
                    == config_path
                        .parent()
                        .context("native config parent")?
                        .join("state"),
            "native datadir/config layout"
        );
        let private = config_path
            .parent()
            .and_then(Path::parent)
            .context("native run private root")?
            .to_path_buf();
        let uid = io::uid()?;
        io::private_dir(&root, uid)?;
        io::private_dir(&private, uid)?;
        ensure!(
            profile.collector_socket.starts_with(&private),
            "native collector outside owned run"
        );
        io::private_dir(
            profile
                .collector_socket
                .parent()
                .context("native collector parent")?,
            uid,
        )?;
        let (process, path) = io::process(
            std::process::id(),
            uid,
            &private,
            Some(config_path),
            &profile.expected_run_token()?,
        )?;
        ensure!(
            process.config_sha256 == *config_hash,
            "native loaded config changed"
        );
        let socket_path = root.join(wire::SOCKET_FILENAME);
        let instance_path = root.join(wire::INSTANCE_FILENAME);
        if socket_path.symlink_metadata().is_ok() || instance_path.symlink_metadata().is_ok() {
            let old = wire::decode_instance(&io::private_read(&instance_path, uid)?)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                old.run_id == profile.run_id
                    && old.identity_id == profile.identity_id
                    && old.identity == config.upstream.sp_address
                    && old.pool_id == profile.pool_id
                    && old.front_id == profile.front_id
                    && old.process.uid == uid
                    && old.process.run_token_sha256 == process.run_token_sha256
                    && old.process.config_sha256 == *config_hash
                    && Path::new(&old.config_path) == config_path
                    && Path::new(&old.socket_path) == socket_path,
                "native stale instance ownership"
            );
            let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
            ensure!(
                old.process.boot_id != boot.trim()
                    || io::start_ticks(old.process.pid).ok() != Some(old.process.start_time_ticks),
                "native wallet instance still live"
            );
            if let Ok(m) = socket_path.symlink_metadata() {
                ensure!(
                    m.file_type().is_socket()
                        && m.uid() == uid
                        && m.mode() & 0o777 == 0o600
                        && m.dev() == old.socket_device
                        && m.ino() == old.socket_inode,
                    "native stale socket foreign"
                );
                std::fs::remove_file(&socket_path)?;
            }
        }
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        let socket = std::fs::symlink_metadata(&socket_path)?;
        let rm = std::fs::symlink_metadata(&root)?;
        let mut creation = SocketCreation {
            path: socket_path.clone(),
            device: socket.dev(),
            inode: socket.ino(),
            armed: true,
        };
        let instance_id: [u8; 16] = rand::random();
        ensure!(instance_id != [0; 16], "native instance entropy invalid");
        let instance = WalletInstance {
            schema: wire::INSTANCE_SCHEMA.into(),
            instance_id,
            run_id: profile.run_id.clone(),
            identity_id: profile.identity_id.clone(),
            identity: config.upstream.sp_address.clone(),
            pool_id: profile.pool_id.clone(),
            front_id: profile.front_id.clone(),
            process,
            config_path: path.to_str().context("native config UTF8")?.into(),
            socket_path: socket_path.to_str().context("native socket UTF8")?.into(),
            socket_device: socket.dev(),
            socket_inode: socket.ino(),
            collector_pubkey: profile.collector_pubkey.clone(),
            pool_pubkey: profile.pool_pubkey.clone(),
        };
        instance.validate().map_err(anyhow::Error::msg)?;
        let keys = wire::VerificationKeys::for_regtest_identity(
            &instance.identity,
            &instance.collector_pubkey,
            &instance.pool_pubkey,
        )
        .map_err(anyhow::Error::msg)?;
        let context = Arc::new(ContextState {
            source: SourceBinding {
                socket: profile.collector_socket.clone(),
                pool: keys.pool,
                collector: keys.collector,
            },
            instance,
            root,
            private,
            root_device: rm.dev(),
            root_inode: rm.ino(),
            keys,
            published: AtomicBool::new(false),
            tracker: Mutex::new(Tracker::default()),
        });
        let (auditors, factory) = auditor_channel(4)?;
        let (signer, signing) = mpsc::channel(4);
        let service = service::Service::start(
            listener,
            context.clone(),
            service::Limits {
                concurrent_sessions: 4,
                total_deadline: Duration::from_secs(wire::DEADLINE_SECS),
                max_encoded_response_bytes: wire::MAX_RESPONSE_BYTES + 4,
            },
            Arc::new(Authorizer {
                context: context.clone(),
            }),
            Arc::new(Handler {
                context: context.clone(),
                auditors,
                signer,
            }),
        )
        .await?;
        if let Err(error) =
            io::atomic_private(&instance_path, &serde_json::to_vec(&context.instance)?, uid)
        {
            service.stop().await?;
            return Err(error);
        }
        context.published.store(true, Ordering::SeqCst);
        if let Err(error) = context.endpoint() {
            service.stop().await?;
            return Err(error);
        }
        creation.armed = false;
        Ok(Some(Self {
            context,
            service: Some(service),
            factory: Some(factory),
            signing: Some(signing),
        }))
    }

    pub(super) async fn serve_owner(&mut self, keys: &WalletKeys) -> anyhow::Result<()> {
        let factory = self
            .factory
            .take()
            .context("native factory already served")?;
        let mut signing = self
            .signing
            .take()
            .context("native signer already served")?;
        let owner = async {
            while let Some(work) = signing.recv().await {
                if work.reply.is_closed() || Instant::now() >= work.deadline {
                    continue;
                }
                let result = (|| {
                    self.context.current(&work.scope.request)?;
                    let request = &work.scope.request;
                    ensure!(
                        work.evidence.checkpoint.job_id
                            == request.full_job_id().map_err(anyhow::Error::msg)?,
                        "native full job ID mismatch"
                    );
                    source_feed::verify_public_evidence(
                        &work.evidence,
                        request.nonce,
                        request.job.digest(),
                        &self.context.keys.pool,
                        &self.context.keys.collector,
                        &self.context.keys.wallet,
                        now()?,
                    )
                    .map_err(anyhow::Error::msg)?;
                    let mut receipt = ActivationReceipt {
                        request_digest: request.signing_digest().map_err(anyhow::Error::msg)?,
                        candidate_epoch: request.candidate_epoch,
                        nonce: request.nonce,
                        evidence: work.evidence,
                        wallet_signature: String::new(),
                    };
                    let digest =
                        wire::activation_digest(request, &receipt).map_err(anyhow::Error::msg)?;
                    // No raw digest arrives from a caller. This infallible,
                    // synchronous signing step uses only the codec's exact tag.
                    let mut key = Keypair::from_secret_key(&Secp256k1::new(), &keys.b_scan);
                    receipt.wallet_signature = source_feed::sign(digest, &key);
                    key.non_secure_erase();
                    let response = Response::Approved {
                        schema: wire::SCHEMA.into(),
                        capability: wire::CAPABILITY.into(),
                        receipt,
                    };
                    let verified =
                        wire::verify_response(request, &response, &self.context.keys, now()?)
                            .map_err(anyhow::Error::msg)?;
                    self.context.current(request)?;
                    ensure!(Instant::now() < work.deadline, "native signing expired");
                    let bytes = framed(&response)?;
                    self.context
                        .tracker
                        .lock()
                        .map_err(|_| anyhow::anyhow!("native tracker"))?
                        .cache(request, verified.expires_at_unix_ms, bytes.clone())?;
                    Ok::<_, anyhow::Error>(bytes)
                })();
                let _ = work.reply.send(result);
                tokio::task::yield_now().await;
            }
            anyhow::bail!("native activation owner channel stopped")
        };
        tokio::select! {
            _=factory.serve(keys,&self.context.instance.identity,&self.context.source)=>anyhow::bail!("native auditor factory stopped"),
            result=owner=>result,
        }
    }
    pub(super) async fn stop(mut self) -> anyhow::Result<()> {
        self.context.published.store(false, Ordering::SeqCst);
        {
            let tracker = self
                .context
                .tracker
                .lock()
                .map_err(|_| anyhow::anyhow!("native tracker"))?;
            for latest in tracker.latest.values() {
                let _ = latest.cancel.send(true);
            }
        }
        let stopped = if let Some(service) = self.service.take() {
            match tokio::time::timeout(Duration::from_secs(wire::DEADLINE_SECS + 1), service.stop())
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    Err(anyhow::Error::new(error).context("native service stop deadline"))
                }
            }
        } else {
            Ok(())
        };
        let retired = self.context.retire();
        stopped.and(retired)
    }
}

#[cfg(test)]
mod tests;
