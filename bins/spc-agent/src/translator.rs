//! Translator role: downstream SV1 ASICs, upstream SV2 pool. The pool's own
//! extranonce prefix is kept out of the miner's rolled bytes; the miner rolls
//! only the pool's miner-extranonce slot.

use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::config::AgentConfig;
use crate::sv2_client::{to_sv1_job, PoolShare, PoolShareWriter, Sv2Client};
use spc_pool::sv1_server::Sv1Server;

/// Wallet-local source proof feed. Implementations return independently captured
/// collector/history seals; unavailability is an error, never an owed default.
#[async_trait::async_trait]
pub trait JobAuditor: Send + Sync {
    async fn verify(
        &self,
        job: &crate::sv2_client::PoolJob,
        pool_extranonce: &[u8],
        miner_extranonce_len: usize,
    ) -> anyhow::Result<()>;
}

pub struct SourceProof {
    pub bundle: Vec<u8>,
    pub trusted: spc_scan::source_audit::TrustedSource,
}
#[async_trait::async_trait]
pub trait SourceProofFeed: Send + Sync {
    async fn for_job(&self, job: &crate::sv2_client::PoolJob) -> anyhow::Result<SourceProof>;
}
/// Held inside the wallet/agent process, never the native CPU miner. Caller
/// supplies the existing protected collector feed and provisioned scan material.
pub struct SourceJobAuditor<F: SourceProofFeed> {
    pub feed: F,
    pub scan: bitcoin::secp256k1::SecretKey,
    pub spend_public: bitcoin::secp256k1::PublicKey,
}
#[async_trait::async_trait]
impl<F: SourceProofFeed> JobAuditor for SourceJobAuditor<F> {
    async fn verify(
        &self,
        job: &crate::sv2_client::PoolJob,
        pool_extranonce: &[u8],
        miner_extranonce_len: usize,
    ) -> anyhow::Result<()> {
        let proof = self.feed.for_job(job).await?;
        job.verify_source_bundle(
            &proof.bundle,
            &proof.trusted,
            pool_extranonce,
            miner_extranonce_len,
            &self.scan,
            &self.spend_public,
        )?;
        Ok(())
    }
}

pub async fn run(config: AgentConfig) -> anyhow::Result<()> {
    let (_addr, handle) = bind_and_run(config).await?;
    handle.await??;
    Ok(())
}

/// Bind the downstream listener and start the translation loop. Returns the
/// bound address so tests and supervisors do not have to guess a port.
pub async fn bind_and_run(
    config: AgentConfig,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<anyhow::Result<()>>)> {
    let auditor: Option<std::sync::Arc<dyn JobAuditor>> = config
        .source_audit
        .clone()
        .map(|settings| {
            crate::source_feed::UnixSourceAuditor::from_config(
                settings,
                &config.upstream.sp_address,
                config.network(),
            )
            .map(|a| std::sync::Arc::new(a) as std::sync::Arc<dyn JobAuditor>)
        })
        .transpose()?;
    if config.upstream.require_own_payout == "refuse" && auditor.is_none() {
        anyhow::bail!("refuse-mode translator requires a complete source proof feed");
    }
    bind_and_run_with_auditor(config, auditor).await
}
struct AbortTask<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The runtime entry point for AP-3b's authenticated collector integration.
/// Verification runs before EVERY actual job publication to downstream miners.
pub async fn bind_and_run_with_auditor(
    config: AgentConfig,
    auditor: Option<std::sync::Arc<dyn JobAuditor>>,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<anyhow::Result<()>>)> {
    if config.upstream.require_own_payout == "refuse" && auditor.is_none() {
        anyhow::bail!("source proof feed unavailable");
    }
    let upstream = config.upstream_target()?;
    // Validated at startup; re-checked here because a proxied `.onion` dial
    // must fail, never fall back to direct (SPEC §3.8).
    let egress = config.net.egress().map_err(|e| anyhow::anyhow!(e))?;
    let network = config.network();
    let authority = config.upstream_authority()?;
    let client = Sv2Client::connect(
        &upstream,
        &egress,
        &config.upstream.sp_address,
        authority,
        network,
    )
    .await?;
    let (mut reader, writer, mut share_results) = client.split();
    let extranonce_size = reader.extranonce_size;
    let prefix = reader.extranonce_prefix.clone();
    let (jobs_tx, mut jobs_rx) = mpsc::channel(16);
    // Never cancel a Noise frame read when an audit/share branch wins select.
    let reader_task = AbortTask(tokio::spawn(async move {
        loop {
            let job = reader.next_job().await;
            let failed = job.is_err();
            if jobs_tx.send(job).await.is_err() || failed {
                break;
            }
        }
    }));

    let (share_tx, mut share_rx) = mpsc::channel(1024);
    let sv1 = Sv1Server::new(network, 0, extranonce_size as usize, 1.0, share_tx);
    let (addr, handle) = sv1.bind(&config.downstream.listen).await?;
    tracing::info!(%addr, "translator SV1 listener ready");

    let server = sv1.clone();
    let task = tokio::spawn(async move {
        let _reader = reader_task;
        let _listener = AbortTask(handle);
        let result = supervised_loop(
            &mut jobs_rx,
            (&prefix, extranonce_size as usize),
            writer,
            &server,
            &mut share_rx,
            &mut share_results,
            auditor,
        )
        .await;
        server.clear_current().await;
        result
    });
    Ok((addr, task))
}

async fn supervised_loop(
    jobs: &mut mpsc::Receiver<Result<crate::sv2_client::PoolJob, crate::sv2_client::ClientError>>,
    context: (&[u8], usize),
    mut writer: PoolShareWriter,
    sv1: &Sv1Server,
    share_rx: &mut mpsc::Receiver<spc_pool::sv1::Share>,
    share_results: &mut mpsc::Receiver<crate::sv2_client::ShareResult>,
    auditor: Option<std::sync::Arc<dyn JobAuditor>>,
) -> anyhow::Result<()> {
    let (prefix, miner_len) = context;
    let mut active_job = None;
    let mut candidate: Option<crate::sv2_client::PoolJob> = None;
    let mut audit: Option<AbortTask<anyhow::Result<()>>> = None;
    loop {
        tokio::select! {
            biased;
            job = jobs.recv() => {
                let job=job.ok_or_else(||anyhow::anyhow!("pool job stream closed"))??;
                drop(audit.take());active_job=None;sv1.clear_current().await;
                if let Some(auditor)=&auditor {
                    let auditor=auditor.clone();let input=job.clone();let prefix=prefix.to_vec();
                    audit=Some(AbortTask(tokio::spawn(async move {auditor.verify(&input,&prefix,miner_len).await})));
                    candidate=Some(job);
                }else {
                    active_job=Some(job.job_id);sv1.publish(to_sv1_job(&job,prefix)).await;
                }
            }
            result=async {match audit.as_mut() {Some(task)=>(&mut task.0).await,None=>std::future::pending().await}}, if audit.is_some() => {
                drop(audit.take());result??;
                let job=candidate.take().ok_or_else(||anyhow::anyhow!("missing audited candidate"))?;
                active_job=Some(job.job_id);sv1.publish(to_sv1_job(&job,prefix)).await;
                tracing::info!(event="source_job_activated",job_id=job.job_id,"audited work activated after committed consumption ACK");
            }
            Some(result) = share_results.recv() => {
                match result {
                    crate::sv2_client::ShareResult::Accepted { sequence_number, credited_work } => {
                        tracing::debug!(sequence_number, credited_work, "front accepted share into slice");
                    }
                    crate::sv2_client::ShareResult::Rejected { sequence_number, error_code } => {
                        tracing::warn!(sequence_number, error_code, "front rejected share");
                    }
                }
            }
            Some(share) = share_rx.recv() => {
                let job_id = u32::from_str_radix(&share.job_id, 16).ok();
                if job_id.is_none() || job_id!=active_job {
                    tracing::info!(event="translator_stale_submission",submitted_job=?job_id,active_job=?active_job,"inactive or superseded work was not forwarded");
                    continue;
                }
                writer.submit(PoolShare {
                    job_id:job_id.expect("checked job ID"),
                    nonce: share.nonce,
                    ntime: share.ntime,
                    version: share.version,
                    extranonce: share.extranonce,
                    sequence_number: share.share_index,
                }).await?;
            }
        }
    }
}
