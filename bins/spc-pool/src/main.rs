use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use spc_pool::api_backend::StoreBackend;
use spc_pool::config::{Config, Role};
use spc_pool::front::{Front, FrontConfig, FrontEvent};
use spc_pool::sv1_server::Sv1Server;
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template, TemplateSource};
use spc_store::redis::Streams;
use spc_store::Store;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "spc-pool", version = spc_pool::VERSION)]
struct Cli {
    /// TOML configuration file (SPEC §6.3).
    #[arg(short, long, env = "SPC_POOL_CONFIG")]
    config: PathBuf,
    /// Override the configured roles (comma-separated).
    #[arg(long, value_delimiter = ',')]
    roles: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,spc_pool=debug".into()),
        )
        .init();

    let config = Config::load(&cli.config)?;
    let roles = if cli.roles.is_empty() {
        config.roles().map_err(|e| anyhow::anyhow!(e))?
    } else {
        cli.roles
            .iter()
            .map(|r| Role::parse(r).ok_or_else(|| anyhow::anyhow!("unknown role '{r}'")))
            .collect::<anyhow::Result<Vec<_>>>()?
    };

    tracing::info!(
        version = spc_pool::VERSION,
        network = %config.pool.network,
        front_id = config.pool.front_id,
        roles = ?roles,
        "spc-pool starting"
    );

    let payout_task = if roles.contains(&Role::Payout) {
        let store = Store::connect(&config.storage.postgres).await?;
        let streams = Streams::connect(&config.storage.redis).await?;
        let svc = spc_pool::payout::PayoutService::new(
            store,
            streams,
            config.accounting.confirmation_depth,
        );
        Some(tokio::spawn(async move {
            if let Err(e) = svc.run().await {
                tracing::error!(error = %e, "payout role stopped");
            }
        }))
    } else {
        None
    };

    let network = config.network().map_err(|e| anyhow::anyhow!(e))?;
    if roles.contains(&Role::Api) {
        let store = Store::connect(&config.storage.postgres).await?;
        let backend = std::sync::Arc::new(StoreBackend::new(
            store,
            serde_json::json!({
                "network": config.pool.network,
                "window_n": config.accounting.window_n,
                "coinbase_threshold_sats": config.accounting.coinbase_threshold_sats,
                "extensions": ["0x0001", "0x0003", "0x0020", "0x5350"],
            }),
            serde_json::json!({
                "budget_wu": config.budget.floor_wu,
                "ceiling_wu": config.budget.ceiling_wu,
                "max_payout_slots": config.accounting.max_payout_slots,
            }),
        ));
        let app = spc_pool::api::router(backend, network);
        let listener = tokio::net::TcpListener::bind(&config.api.listen).await?;
        tracing::info!(addr = %config.api.listen, "api listener ready");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "api server stopped");
            }
        });
    }

    if !roles.contains(&Role::Front) {
        tracing::warn!("no front role selected; waiting on other roles");
        if let Some(t) = payout_task {
            let _ = t.await;
        }
        return Ok(());
    }
    let (tx, mut rx) = mpsc::channel(4096);
    let server = Sv1Server::new(
        network,
        config.sv1.extranonce1_bytes,
        config.sv1.extranonce2_bytes,
        1.0,
        tx.clone(),
    );
    if config.sv1.enabled {
        let (addr, _handle) = server.bind(&config.sv1.listen).await?;
        tracing::info!(%addr, "native SV1 listener ready");
    }
    let sv2 = Sv2Server::new(network, tx);
    let (sv2_addr, _sv2_handle) = sv2.bind(&config.pool.listen).await?;
    tracing::info!(%sv2_addr, "native SV2 listener ready");

    let source: Arc<dyn TemplateSource> = match config.template.source.as_str() {
        "rpc" => {
            #[cfg(feature = "rpc")]
            {
                Arc::new(spc_pool::template_rpc::RpcTemplateSource::new(
                    &config.template.rpc_url,
                    &config.template.rpc_user,
                    &config.template.rpc_pass,
                    config.template.extranonce_bytes as u32 * 4,
                )?)
            }
            #[cfg(not(feature = "rpc"))]
            {
                anyhow::bail!("this spc-pool was built without the `rpc` feature");
            }
        }
        "mock" => Arc::new(MockTemplateSource::new(demo_template())),
        other => anyhow::bail!("template source '{other}' is not implemented in this build"),
    };

    let streams = match Streams::connect(&config.storage.redis).await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "redis unavailable; slice/block events will not be published");
            None
        }
    };

    let mut front = Front::new(
        FrontConfig::from_config(&config).map_err(|e| anyhow::anyhow!(e))?,
        source.clone(),
    );
    let job = front.refresh().await?;
    publish_events(&streams, front.take_events()).await;
    server.publish(job.clone()).await;
    sv2.publish(job).await;

    loop {
        let Some(share) = rx.recv().await else { break };
        match front.on_share(share) {
            Ok(Some(block)) => {
                tracing::info!(merkle = %block.merkle_root.iter().map(|b| format!("{b:02x}")).collect::<String>(), "share found a block; submitting");
                if let Err(e) = source.submit_block(&block.block).await {
                    tracing::error!(error = %e, "block submit failed");
                }
                publish_events(&streams, front.take_events()).await;
                match front.refresh().await {
                    Ok(job) => {
                        publish_events(&streams, front.take_events()).await;
                        server.publish(job.clone()).await;
                        sv2.publish(job).await;
                    }
                    Err(e) => tracing::error!(error = %e, "template refresh failed"),
                }
            }
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, "share rejected"),
        }
    }
    Ok(())
}

fn demo_template() -> Template {
    Template {
        height: 1,
        prev_hash: [0u8; 32],
        version: 0x2000_0000,
        bits: 0x207f_ffff,
        curtime: 1_700_000_000,
        subsidy: 5_000_000_000,
        fees: 0,
        coinbase_value: 5_000_000_000,
        transactions: Vec::new(),
        merkle_branches: Vec::new(),
        coinbase_output_max_additional_size: 50_000,
    }
}

async fn publish_events(streams: &Option<Streams>, events: Vec<FrontEvent>) {
    let Some(streams) = streams else { return };
    for event in events {
        let (stream, payload) = match event {
            FrontEvent::Slice(e) => match e.encode() {
                Ok(p) => ("spc:slices", p),
                Err(err) => {
                    tracing::error!(error = %err, "slice encode failed");
                    continue;
                }
            },
            FrontEvent::Block(e) => match e.encode() {
                Ok(p) => ("spc:blocks", p),
                Err(err) => {
                    tracing::error!(error = %err, "block encode failed");
                    continue;
                }
            },
        };
        if let Err(err) = streams.add(stream, &payload).await {
            tracing::error!(error = %err, stream, "stream publish failed");
        }
    }
}
