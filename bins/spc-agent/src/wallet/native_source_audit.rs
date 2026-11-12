//! One request-owned bridge to the source-bound shared verifier. Native wire
//! and activation signing live in native_audit and the owner's native codec.
use crate::source_feed::UnixSourceAuditor;
use crate::wallet::WalletKeys;
use anyhow::{ensure, Context};
use bitcoin::secp256k1::XOnlyPublicKey;
use spc_scan::source_feed::{ReceivedJob, VerifiedSourceEvidence};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

/// Public, wallet-side collector binding from the declared runtime profile.
/// This never includes a key-file path or secret and is not request input.
#[derive(Clone)]
pub struct SourceBinding {
    pub socket: PathBuf,
    pub pool: XOnlyPublicKey,
    pub collector: XOnlyPublicKey,
}

/// Deliberately neither Clone nor Debug. Construction is synchronous: only
/// the shared helper's Zeroizing scan copy crosses the subsequent await.
/// No wallet reference, Bark key, child seed, store or operation lock is held.
pub struct RequestAudit {
    inner: UnixSourceAuditor,
}

struct Construction {
    deadline: Instant,
    reply: oneshot::Sender<anyhow::Result<RequestAudit>>,
}

/// A bounded in-process request channel, not an IPC protocol. Handles carry no
/// key or caller-selected binding. The authorized audit worker asks the wallet
/// owner to construct one fresh consuming verifier after native admission.
#[derive(Clone)]
pub struct AuditorRequests {
    sender: mpsc::Sender<Construction>,
}

/// Drive concurrently with the existing wallet loop, borrowing its original
/// keys. Construction performs no collector I/O and holds no wallet DB lock.
/// The audit thread receives only a request-owned verifier, never WalletKeys.
pub struct AuditorFactory {
    receiver: mpsc::Receiver<Construction>,
}

pub fn auditor_channel(capacity: usize) -> anyhow::Result<(AuditorRequests, AuditorFactory)> {
    ensure!(
        (1..=4).contains(&capacity),
        "native auditor construction queue bound"
    );
    let (sender, receiver) = mpsc::channel(capacity);
    Ok((AuditorRequests { sender }, AuditorFactory { receiver }))
}

impl AuditorRequests {
    pub async fn request(&self, deadline: Instant) -> anyhow::Result<RequestAudit> {
        ensure!(
            Instant::now() < deadline,
            "native auditor construction expired"
        );
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Construction { deadline, reply })
            .map_err(|_| anyhow::anyhow!("native auditor construction unavailable or full"))?;
        tokio::time::timeout_at(deadline, result)
            .await
            .context("native auditor construction deadline")?
            .context("native auditor construction unavailable")?
    }
}

impl AuditorFactory {
    pub async fn serve(mut self, keys: &WalletKeys, address: &str, binding: &SourceBinding) {
        while let Some(request) = self.receiver.recv().await {
            if request.reply.is_closed() || Instant::now() >= request.deadline {
                continue;
            }
            let auditor = RequestAudit::from_wallet(keys, address, binding);
            // If the worker cancelled during construction, send returns the
            // owned verifier and dropping that result invokes its shared Drop.
            let _ = request.reply.send(auditor);
        }
    }
}
impl RequestAudit {
    pub fn from_wallet(
        keys: &WalletKeys,
        address: &str,
        binding: &SourceBinding,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            inner: UnixSourceAuditor::from_keys(
                binding.socket.clone(),
                binding.pool,
                binding.collector,
                &keys.b_scan,
                keys.b_spend_public,
                address,
                keys.network,
            )?,
        })
    }

    /// Pass the exact native outer attempt unchanged. The shared helper consumes
    /// itself and returns only checked v2 checkpoint/Consumed/Committed evidence.
    /// Outer timeout/cancellation drops that same owned key copy and stream.
    pub async fn verify_received(
        self,
        received: &ReceivedJob,
        attempt: [u8; 16],
        deadline: Instant,
    ) -> anyhow::Result<VerifiedSourceEvidence> {
        ensure!(
            Instant::now() < deadline,
            "native source audit admission expired"
        );
        let evidence =
            tokio::time::timeout_at(deadline, self.inner.verify_received(received, attempt))
                .await
                .context("native source audit total deadline")??;
        ensure!(
            Instant::now() < deadline,
            "native source audit completion expired"
        );
        Ok(evidence)
    }
}

#[cfg(test)]
#[path = "native_source_audit_tests.rs"]
mod tests;
