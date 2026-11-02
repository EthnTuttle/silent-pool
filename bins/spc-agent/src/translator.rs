//! Translator role: downstream SV1 ASICs, upstream SV2 pool. The pool's own
//! extranonce prefix is kept out of the miner's rolled bytes; the miner rolls
//! only the pool's miner-extranonce slot.

use std::net::SocketAddr;

use spc_crypto::Network;
use tokio::sync::mpsc;

use crate::config::AgentConfig;
use crate::sv2_client::{to_sv1_job, PoolShare, PoolShareWriter, Sv2Client};
use spc_pool::sv1_server::Sv1Server;

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
    let upstream: SocketAddr = config.upstream.address.parse()?;
    let network = config.network();
    let client = Sv2Client::connect(upstream, &config.upstream.sp_address, network).await?;
    let (mut reader, writer) = client.split();
    let extranonce_size = reader.extranonce_size;

    let (share_tx, mut share_rx) = mpsc::channel(1024);
    let sv1 = Sv1Server::new(network, 0, extranonce_size as usize, 1.0, share_tx);
    let (addr, _handle) = sv1.bind(&config.downstream.listen).await?;
    tracing::info!(%addr, "translator SV1 listener ready");

    let first = reader.next_job().await?;
    sv1.publish(to_sv1_job(&first, &reader.extranonce_prefix))
        .await;
    tracing::info!(job = first.job_id, "translator received first job");

    let server = sv1.clone();
    let task = tokio::spawn(async move {
        supervised_loop(&mut reader, writer, &server, &mut share_rx, network).await
    });
    Ok((addr, task))
}

async fn supervised_loop(
    reader: &mut crate::sv2_client::PoolJobReader,
    mut writer: PoolShareWriter,
    sv1: &Sv1Server,
    share_rx: &mut mpsc::Receiver<spc_pool::sv1::Share>,
    _network: Network,
) -> anyhow::Result<()> {
    let mut pending_job = None;
    loop {
        tokio::select! {
            job = reader.next_job() => {
                let job = job?;
                pending_job = Some(job.job_id);
                sv1.publish(to_sv1_job(&job, &reader.extranonce_prefix)).await;
                tracing::debug!(job = job.job_id, "new pool job");
            }
            Some(share) = share_rx.recv() => {
                let job_id = u32::from_str_radix(&share.job_id, 16).unwrap_or(0);
                if let Some(expected) = pending_job {
                    if job_id != expected {
                        tracing::debug!(job_id, expected, "stale translator share");
                        continue;
                    }
                }
                writer.submit(PoolShare {
                    job_id,
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
