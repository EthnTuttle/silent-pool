//! Actual private Unix collector transport used by the translator runtime.
use crate::{sv2_client::PoolJob, translator::JobAuditor};
use anyhow::{anyhow, ensure, Context};
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey, XOnlyPublicKey},
};
use serde::Deserialize;
use spc_scan::{
    source_audit::{SourceBundle, TrustedSource},
    source_feed::*,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub socket: PathBuf,
    pub collector_pubkey: String,
    pub pool_pubkey: String,
    pub scan_secret_key_file: PathBuf,
}
pub struct UnixSourceAuditor {
    socket: PathBuf,
    collector: XOnlyPublicKey,
    pool: XOnlyPublicKey,
    scan: zeroize::Zeroizing<[u8; 32]>,
    spend: PublicKey,
    #[cfg(test)]
    drop_observer: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}
// SecretKey/Keypair are Copy, not zeroizing ownership types. Wrap each actual
// secp temporary and use its supported best-effort erase surface on all exits.
struct ErasedSecret(SecretKey);
impl Drop for ErasedSecret {
    fn drop(&mut self) {
        self.0.non_secure_erase();
    }
}
struct ErasedKeypair(Keypair);
impl Drop for ErasedKeypair {
    fn drop(&mut self) {
        self.0.non_secure_erase();
    }
}
impl Drop for UnixSourceAuditor {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.scan.zeroize();
        debug_assert!(self.scan.iter().all(|byte| *byte == 0));
        #[cfg(test)]
        if let Some(observer) = &self.drop_observer {
            observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
fn now_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
fn private_file(path: &Path, socket: bool) -> anyhow::Result<std::fs::Metadata> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    ensure!(path.is_absolute(), "source feed path must be absolute");
    let meta = std::fs::symlink_metadata(path)?;
    let parent =
        std::fs::symlink_metadata(path.parent().ok_or_else(|| anyhow!("source feed parent"))?)?;
    ensure!(
        parent.is_dir()
            && parent.permissions().mode() & 0o077 == 0
            && meta.permissions().mode() & 0o077 == 0
            && meta.uid() == parent.uid(),
        "source feed private ownership/permissions"
    );
    ensure!(
        if socket {
            meta.file_type().is_socket()
        } else {
            meta.is_file() && meta.len() <= 128
        },
        "source feed file type/bound"
    );
    Ok(meta)
}
impl UnixSourceAuditor {
    /// Existing wallet-process integration: reuse already loaded scan authority.
    /// This constructor reads no secret file and creates no wallet process.
    pub fn from_keys(
        socket: PathBuf,
        pool: XOnlyPublicKey,
        collector: XOnlyPublicKey,
        scan: &SecretKey,
        spend: PublicKey,
        address: &str,
        network: spc_crypto::Network,
    ) -> anyhow::Result<Self> {
        ensure!(socket.is_absolute(), "source feed path must be absolute");
        ensure!(
            pool != collector,
            "collector key must be independent of pool source key"
        );
        let identity = spc_crypto::parse_sp_address(address, network)
            .map_err(|e| anyhow!("source identity: {e}"))?;
        ensure!(
            identity.b_scan == PublicKey::from_secret_key(&Secp256k1::new(), scan)
                && identity.b_spend == spend,
            "loaded audit keys differ from configured public identity"
        );
        Ok(Self {
            socket,
            pool,
            collector,
            scan: zeroize::Zeroizing::new(scan.secret_bytes()),
            spend,
            #[cfg(test)]
            drop_observer: None,
        })
    }
    pub fn from_config(
        config: Config,
        address: &str,
        network: spc_crypto::Network,
    ) -> anyhow::Result<Self> {
        let collector = config.collector_pubkey.parse()?;
        let pool = config.pool_pubkey.parse()?;
        ensure!(
            collector != pool,
            "collector key must be independent of pool source key"
        );
        private_file(&config.scan_secret_key_file, false)?;
        let text = zeroize::Zeroizing::new(std::fs::read_to_string(&config.scan_secret_key_file)?);
        let scan = ErasedSecret(text.trim().parse()?);
        let identity = spc_crypto::parse_sp_address(address, network)
            .map_err(|e| anyhow!("source identity: {e}"))?;
        ensure!(
            PublicKey::from_secret_key(&Secp256k1::new(), &scan.0) == identity.b_scan,
            "source scan key differs from configured identity"
        );
        Self::from_keys(
            config.socket,
            pool,
            collector,
            &scan.0,
            identity.b_spend,
            address,
            network,
        )
    }
    /// Consumes the request-owned auditor. Return, error, timeout and task
    /// cancellation drop its Zeroizing scan copy; no signer task is spawned.
    pub async fn verify_received(
        self,
        received: &ReceivedJob,
        attempt: [u8; 16],
    ) -> anyhow::Result<VerifiedSourceEvidence> {
        ensure!(attempt != [0; 16], "source attempt must be nonzero");
        let deadline = Instant::now() + Duration::from_secs(DEADLINE_SECS);
        tokio::time::timeout(
            Duration::from_secs(DEADLINE_SECS),
            self.exchange(received, attempt, deadline),
        )
        .await
        .context("source feed deadline; work remains inactive")?
    }
    async fn exchange(
        &self,
        received: &ReceivedJob,
        attempt: [u8; 16],
        deadline: Instant,
    ) -> anyhow::Result<VerifiedSourceEvidence> {
        use std::os::unix::fs::MetadataExt;
        let meta = private_file(&self.socket, true)?;
        let mut stream = UnixStream::connect(&self.socket).await?;
        ensure!(
            stream.peer_cred()?.uid() == meta.uid(),
            "source collector peer ownership"
        );
        let wire_digest = received.digest();
        write_frame(
            &mut stream,
            &Frame::Job {
                schema: SCHEMA.into(),
                attempt,
                job: received.clone(),
            },
        )
        .await?;
        let Frame::Proof {
            schema,
            bundle_hex,
            checkpoint,
        } = read_frame(&mut stream).await?
        else {
            return Err(anyhow!("source proof unavailable"));
        };
        ensure!(
            schema == SCHEMA && bundle_hex.len() <= 2 * MAX_BUNDLE,
            "source frame schema/bound"
        );
        let bundle = hex::decode(bundle_hex)?;
        ensure!(
            checkpoint.attempt == attempt
                && checkpoint.wire_job_digest == wire_digest
                && checkpoint.complete,
            "source checkpoint correlation/completeness"
        );
        let now = now_ms()?;
        ensure!(
            checkpoint.expires_at_unix_ms > now
                && checkpoint.expires_at_unix_ms - now <= MAX_LIFETIME_MS,
            "source checkpoint expired/lifetime"
        );
        verify(
            checkpoint.pool.signing_digest(),
            &checkpoint.pool.signature,
            &self.pool,
        )
        .map_err(anyhow::Error::msg)?;
        verify(
            checkpoint.signing_digest(),
            &checkpoint.signature,
            &self.collector,
        )
        .map_err(anyhow::Error::msg)?;
        let source: SourceBundle = serde_json::from_slice(&bundle)?;
        ensure!(
            checkpoint.job_id == source.job.job_id
                && checkpoint.generation == source.job.generation
                && checkpoint.source_watermark == source.source_watermark
                && checkpoint.pool.seq == source.source_watermark
                && checkpoint.history_watermark == source.prior.watermark
                && checkpoint.history_digest == source.prior.digest
                && checkpoint.bundle_digest == sha256::Hash::hash(&bundle).to_byte_array(),
            "source checkpoint/bundle binding"
        );
        let trusted = TrustedSource {
            bundle_digest: checkpoint.bundle_digest,
            history_digest: checkpoint.history_digest,
            history_watermark: checkpoint.history_watermark,
        };
        let window = {
            let scan = ErasedSecret(SecretKey::from_slice(self.scan.as_ref())?);
            let mut verified = spc_scan::source_audit::verify_received_owned(
                &bundle,
                &trusted,
                received,
                &scan.0,
                &self.spend,
            )?;
            let facts = verified.entitlement.window();
            use zeroize::Zeroize;
            for output in &mut verified.outputs {
                output.tweak.zeroize();
            }
            facts
        };
        ensure!(
            Instant::now() < deadline && checkpoint.expires_at_unix_ms > now_ms()?,
            "source audit deadline reached before ACK signing"
        );
        let mut ack = Consumed {
            attempt,
            job_id: checkpoint.job_id,
            generation: checkpoint.generation,
            bundle_digest: checkpoint.bundle_digest,
            history_digest: checkpoint.history_digest,
            checkpoint_digest: checkpoint.signing_digest(),
            window,
            signature: String::new(),
        };
        let digest = ack.signing_digest();
        let scan_public;
        ack.signature = {
            let scan = ErasedSecret(SecretKey::from_slice(self.scan.as_ref())?);
            let key = ErasedKeypair(Keypair::from_secret_key(&Secp256k1::new(), &scan.0));
            scan_public = key.0.x_only_public_key().0;
            sign(digest, &key.0)
        };
        ensure!(
            Instant::now() < deadline && checkpoint.expires_at_unix_ms > now_ms()?,
            "source audit deadline reached before ACK publication"
        );
        write_frame(
            &mut stream,
            &Frame::Consumed {
                schema: SCHEMA.into(),
                ack: ack.clone(),
            },
        )
        .await?;
        let Frame::Committed {
            schema,
            attempt: reply,
            ack_digest,
            signature,
        } = read_frame(&mut stream).await?
        else {
            return Err(anyhow!("source consumption acknowledgement unavailable"));
        };
        ensure!(
            schema == SCHEMA
                && reply == attempt
                && ack_digest == digest
                && checkpoint.expires_at_unix_ms > now_ms()?,
            "source consumption acknowledgement mismatch/expired"
        );
        verify(
            committed_digest(attempt, digest),
            &signature,
            &self.collector,
        )
        .map_err(anyhow::Error::msg)?;
        tracing::info!(event="source_proof_consumed",job_id=checkpoint.job_id,generation=checkpoint.generation,
            bundle_digest=%hex::encode(checkpoint.bundle_digest),checkpoint_digest=%hex::encode(checkpoint.signing_digest()),
            history_digest=%hex::encode(checkpoint.history_digest),source_watermark=checkpoint.source_watermark,
            history_watermark=checkpoint.history_watermark,ack_digest=%hex::encode(digest),"collector committed proof consumption before activation");
        let evidence = VerifiedSourceEvidence {
            schema: SCHEMA.into(),
            checkpoint,
            consumed: ack,
            committed: CommittedEvidence {
                attempt: reply,
                ack_digest,
                signature,
            },
        };
        verify_public_evidence(
            &evidence,
            attempt,
            wire_digest,
            &self.pool,
            &self.collector,
            &scan_public,
            now_ms()?,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(evidence)
    }
}
#[async_trait::async_trait]
impl JobAuditor for UnixSourceAuditor {
    async fn verify(&self, job: &PoolJob, prefix: &[u8], miner_len: usize) -> anyhow::Result<()> {
        let received = ReceivedJob {
            job_id: job.job_id,
            prev_hash: job.prev_hash,
            version: job.version,
            nbits: job.nbits,
            ntime: job.ntime,
            coinbase_prefix: job.coinbase_prefix.clone(),
            coinbase_suffix: job.coinbase_suffix.clone(),
            merkle_path: job.merkle_path.clone(),
            share_target_be: job.share_target.to_be_bytes(),
            pool_extranonce: prefix.to_vec(),
            miner_extranonce_len: miner_len.try_into()?,
        };
        let request = Self {
            socket: self.socket.clone(),
            collector: self.collector,
            pool: self.pool,
            scan: zeroize::Zeroizing::new(*self.scan),
            spend: self.spend,
            #[cfg(test)]
            drop_observer: None,
        };
        request.verify_received(&received, rand::random()).await?;
        Ok(())
    }
}
pub async fn write_frame(stream: &mut UnixStream, frame: &Frame) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(frame)?;
    ensure!(bytes.len() <= MAX_FRAME, "source frame bound");
    stream.write_u32_le(bytes.len().try_into()?).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}
pub async fn read_frame(stream: &mut UnixStream) -> anyhow::Result<Frame> {
    let len = stream.read_u32_le().await? as usize;
    ensure!(len > 0 && len <= MAX_FRAME, "source frame bound");
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn loaded_wallet_keys_verify_empty_native_context_and_return_only_committed_public_evidence(
    ) {
        use spc_scan::source_audit::{
            history_digest, HistoryCheckpoint, JobProof, WindowEvidence, WindowFacts,
        };
        use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
        let key = |n| {
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
        };
        for invalid_revenue in [false, true] {
            let wallet = key(7);
            let spend = key(8).public_key();
            let pool = key(9);
            let collector = key(10);
            let id = spc_crypto::sp_id(&wallet.public_key(), &spend);
            let dir = tempfile::tempdir().unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let socket = dir.path().join("collector.sock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let send = spc_crypto::SendKey::generate(1).unwrap();
            let mut script = vec![0x51, 0x20];
            script.extend(pool.x_only_public_key().0.serialize());
            let cb = bitcoin::Transaction {
                version: bitcoin::transaction::Version::ONE,
                lock_time: bitcoin::absolute::LockTime::ZERO,
                input: vec![bitcoin::TxIn {
                    previous_output: bitcoin::OutPoint::null(),
                    script_sig: bitcoin::ScriptBuf::from_bytes(spc_crypto::emit_script_sig(
                        1,
                        send.x_only(),
                    )),
                    sequence: bitcoin::Sequence::MAX,
                    witness: bitcoin::Witness::new(),
                }],
                output: vec![bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(5_000_000_000 - u64::from(invalid_revenue)),
                    script_pubkey: bitcoin::ScriptBuf::from_bytes(script),
                }],
            };
            let mut reference = [0xff; 32];
            reference[0] = 0x7f;
            let config = spc_pool::config::Accounting {
                work_unit_version: 2,
                work_reference_target: Some(hex::encode(reference)),
                ..Default::default()
            };
            let fingerprint: [u8; 32] = hex::decode(
                config.profile_json()["work_unit_fingerprint"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap()
            .try_into()
            .unwrap();
            let job = JobProof {
                job_id: (1u64 << 32) | 17,
                generation: 17,
                height: 1,
                prev_hash: [0; 32],
                version: 0x2000_0000,
                nbits: 0x207f_ffff,
                ntime: 1,
                coinbase_prefix: bitcoin::consensus::serialize(&cb),
                coinbase_suffix: vec![],
                extranonce_len: 0,
                merkle_path: vec![],
                share_target_be: reference,
                credited_work: 1,
                transactions: vec![],
                prevout_transactions: vec![],
            };
            let received = ReceivedJob {
                job_id: 17,
                prev_hash: job.prev_hash,
                version: job.version,
                nbits: job.nbits,
                ntime: job.ntime,
                coinbase_prefix: job.coinbase_prefix.clone(),
                coinbase_suffix: vec![],
                merkle_path: vec![],
                share_target_be: reference,
                pool_extranonce: vec![],
                miner_extranonce_len: 0,
            };
            let pending = BTreeMap::from([(hex::encode(id), 0)]);
            let history = history_digest(2, &pending).unwrap();
            let facts = WindowFacts::from_work(2, 0);
            // This unit collector models a separately validated empty-history
            // seal. It is not a live SQL/chain completeness or native-run receipt.
            let bundle = SourceBundle {
                schema: "spc.payout-proof/1".into(),
                source_complete: true,
                source_watermark: 3,
                job: job.clone(),
                jobs: vec![],
                reference_target_be: reference,
                work_unit_fingerprint: fingerprint,
                window_n: 1,
                window_target: "2".into(),
                high_water: BTreeMap::from([(0, 0)]),
                finding_slice_id: 1,
                slices: vec![],
                subsidy: 5_000_000_000,
                fees: 0,
                fee_bps: 100,
                prior: HistoryCheckpoint {
                    watermark: 2,
                    digest: history,
                    pending,
                },
                owed: BTreeMap::from([(hex::encode(id), 0)]),
                window: Some(WindowEvidence {
                    facts,
                    eligible_history_complete: true,
                }),
                witness_reserved: BTreeMap::new(),
            };
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let Frame::Job {
                    schema,
                    attempt,
                    job: received,
                } = read_frame(&mut stream).await.unwrap()
                else {
                    panic!("expected job")
                };
                assert_eq!(schema, SCHEMA);
                assert_eq!(attempt, [4; 16]);
                let bytes = serde_json::to_vec(&bundle).unwrap();
                let mut trace = TraceSeal {
                    session: [1; 16],
                    seq: 3,
                    digest: [2; 32],
                    signature: String::new(),
                };
                trace.signature = sign(trace.signing_digest(), &pool);
                let mut checkpoint = Checkpoint {
                    attempt,
                    wire_job_digest: received.digest(),
                    job_id: job.job_id,
                    generation: 17,
                    source_watermark: 3,
                    history_watermark: 2,
                    bundle_digest: sha256::Hash::hash(&bytes).to_byte_array(),
                    history_digest: history,
                    expires_at_unix_ms: now_ms().unwrap() + 15000,
                    complete: true,
                    pool: trace,
                    signature: String::new(),
                };
                checkpoint.signature = sign(checkpoint.signing_digest(), &collector);
                write_frame(
                    &mut stream,
                    &Frame::Proof {
                        schema: SCHEMA.into(),
                        bundle_hex: hex::encode(bytes),
                        checkpoint,
                    },
                )
                .await
                .unwrap();
                if invalid_revenue {
                    assert!(
                        read_frame(&mut stream).await.is_err(),
                        "invalid source must close without signing an ACK"
                    );
                    return;
                }
                let Frame::Consumed { schema, ack } = read_frame(&mut stream).await.unwrap() else {
                    panic!("expected consumed")
                };
                assert_eq!(schema, SCHEMA);
                assert_eq!(ack.window, facts);
                verify(
                    ack.signing_digest(),
                    &ack.signature,
                    &wallet.x_only_public_key().0,
                )
                .unwrap();
                write_frame(
                    &mut stream,
                    &Frame::Committed {
                        schema: SCHEMA.into(),
                        attempt,
                        ack_digest: ack.signing_digest(),
                        signature: sign(
                            committed_digest(attempt, ack.signing_digest()),
                            &collector,
                        ),
                    },
                )
                .await
                .unwrap();
            });
            let scan = ErasedSecret(wallet.secret_key());
            let address = spc_crypto::address::encode_sp_address(
                spc_crypto::Network::Regtest,
                &wallet.public_key(),
                &spend,
            )
            .unwrap();
            let mut auditor = UnixSourceAuditor::from_keys(
                socket,
                pool.x_only_public_key().0,
                collector.x_only_public_key().0,
                &scan.0,
                spend,
                &address,
                spc_crypto::Network::Regtest,
            )
            .unwrap();
            let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            auditor.drop_observer = Some(dropped.clone());
            let result = auditor.verify_received(&received, [4; 16]).await;
            assert_eq!(
                dropped.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "request-owned scan copy scrubbed on success/error"
            );
            if invalid_revenue {
                assert!(result.is_err());
            } else {
                let evidence = result.unwrap();
                assert_eq!(evidence.consumed.window, facts);
                verify_public_evidence(
                    &evidence,
                    [4; 16],
                    received.digest(),
                    &pool.x_only_public_key().0,
                    &collector.x_only_public_key().0,
                    &wallet.x_only_public_key().0,
                    now_ms().unwrap(),
                )
                .unwrap();
                let public = serde_json::to_value(evidence).unwrap();
                assert!(public.get("bundle_hex").is_none());
                assert!(public.get("scan").is_none());
            }
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn unix_feed_rejects_oversized_or_truncated_frames_before_any_ack() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.write_u32_le((MAX_FRAME + 1) as u32).await.unwrap();
        assert!(read_frame(&mut client).await.is_err());
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.write_u32_le(100).await.unwrap();
        server.write_all(b"{}").await.unwrap();
        server.shutdown().await.unwrap();
        assert!(read_frame(&mut client).await.is_err());
    }
    #[tokio::test]
    async fn cancellation_drops_request_owned_scan_and_stream_without_a_signer_task() {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
        };
        let key = |n| {
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
        };
        let scan = ErasedSecret(key(7).secret_key());
        let spend = key(8).public_key();
        let address = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Regtest,
            &key(7).public_key(),
            &spend,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("collector.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut peer).await.unwrap(),
                Frame::Job { .. }
            ));
            entered.send(()).unwrap();
            assert_eq!(
                peer.read(&mut [0u8; 1]).await.unwrap(),
                0,
                "cancelled exchange must close before any ACK"
            );
        });
        let mut auditor = UnixSourceAuditor::from_keys(
            path,
            key(9).x_only_public_key().0,
            key(10).x_only_public_key().0,
            &scan.0,
            spend,
            &address,
            spc_crypto::Network::Regtest,
        )
        .unwrap();
        let dropped = Arc::new(AtomicUsize::new(0));
        auditor.drop_observer = Some(dropped.clone());
        let received = ReceivedJob {
            job_id: 1,
            prev_hash: [0; 32],
            version: 1,
            nbits: 0x207f_ffff,
            ntime: 1,
            coinbase_prefix: vec![],
            coinbase_suffix: vec![],
            merkle_path: vec![],
            share_target_be: [0; 32],
            pool_extranonce: vec![],
            miner_extranonce_len: 0,
        };
        let task = tokio::spawn(async move { auditor.verify_received(&received, [4; 16]).await });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        server.await.unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            scan.0.secret_bytes(),
            [7; 32],
            "the wallet-owned original remains unchanged"
        );
    }
    #[test]
    fn loaded_constructor_rejects_wrong_public_identity_before_copying_authority() {
        let scan = ErasedSecret(SecretKey::from_slice(&[7; 32]).unwrap());
        let secp = Secp256k1::new();
        let spend = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[8; 32]).unwrap());
        let pool = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[9; 32]).unwrap())
            .x_only_public_key()
            .0;
        let collector = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[10; 32]).unwrap())
            .x_only_public_key()
            .0;
        let wrong = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[11; 32]).unwrap());
        let address =
            spc_crypto::address::encode_sp_address(spc_crypto::Network::Regtest, &wrong, &spend)
                .unwrap();
        assert!(UnixSourceAuditor::from_keys(
            PathBuf::from("/not-read/collector.sock"),
            pool,
            collector,
            &scan.0,
            spend,
            &address,
            spc_crypto::Network::Regtest
        )
        .is_err());
        let correct = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Regtest,
            &PublicKey::from_secret_key(&secp, &scan.0),
            &spend,
        )
        .unwrap();
        assert!(UnixSourceAuditor::from_keys(
            PathBuf::from("/not-read/collector.sock"),
            pool,
            collector,
            &scan.0,
            wrong,
            &correct,
            spc_crypto::Network::Regtest
        )
        .is_err());
        assert!(UnixSourceAuditor::from_keys(
            PathBuf::from("/not-read/collector.sock"),
            pool,
            collector,
            &scan.0,
            spend,
            &correct,
            spc_crypto::Network::Mainnet
        )
        .is_err());
    }
    #[tokio::test]
    async fn unix_feed_unknown_is_an_explicit_frame_and_not_an_activation_ack() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        write_frame(
            &mut server,
            &Frame::Unavailable {
                schema: SCHEMA.into(),
                attempt: [1; 16],
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            Frame::Unavailable { .. }
        ));
    }
}
