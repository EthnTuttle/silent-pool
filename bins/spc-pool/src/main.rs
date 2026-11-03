use std::path::PathBuf;
#[cfg(feature = "rpc")]
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "rpc")]
use bitcoincore_rpc::Error as RpcError;
#[cfg(feature = "rpc")]
use bitcoincore_rpc::RpcApi;
use clap::Parser;
use spc_pool::api_backend::StoreBackend;
#[cfg(feature = "rpc")]
use spc_pool::ark::{
    ArkService, ArkTransport, ExternalCommandTransport, OrderVerifier, SeenOrders,
};
use spc_pool::config::{Config, Role};
use spc_pool::front::{Front, FrontConfig, FrontEvent};
use spc_pool::jds_server::JdsServer;
#[cfg(feature = "rpc")]
use spc_pool::payout::RpcChainView;
use spc_pool::payout::{AccountingConfig, ChainView, ScrubConfig};
use spc_pool::sv1_server::Sv1Server;
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template, TemplateSource};
use spc_store::redis::Streams;
use spc_store::Store;
use tokio::sync::mpsc;

/// Hard bound on a startup Postgres connect. Without it, an absent database
/// can leave `PgPoolOptions::connect` retrying forever and the front never
/// reaches `refresh()` (front-only regtest is the common case).
const STORE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

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

/// A chain view that periodically refreshes the tip from an RPC client.
/// Used by the `ark` role to enforce `not_before_height` against a live tip.
#[cfg(feature = "rpc")]
struct RpcTipSource {
    client: bitcoincore_rpc::Client,
    tip: tokio::sync::RwLock<u32>,
}

#[cfg(feature = "rpc")]
impl RpcTipSource {
    fn new(client: bitcoincore_rpc::Client) -> Self {
        Self {
            client,
            tip: tokio::sync::RwLock::new(0),
        }
    }

    async fn refresh_tip(&self) -> Result<u32, RpcError> {
        let count = self.client.get_block_count()?;
        let tip = u32::try_from(count)
            .map_err(|_| RpcError::ReturnedError("tip exceeds u32".to_string()))?;
        *self.tip.write().await = tip;
        Ok(tip)
    }

    #[allow(dead_code)]
    async fn tip(&self) -> u32 {
        *self.tip.read().await
    }

    /// Spawn a background task to refresh the tip periodically.
    #[allow(dead_code)]
    fn spawn_refresh(self: &Arc<Self>, interval: Duration) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                interval.tick().await;
                if let Err(e) = this.refresh_tip().await {
                    tracing::warn!(error = %e, "ark tip refresh failed");
                }
            }
        });
    }
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

    let network = config.network().map_err(|e| anyhow::anyhow!(e))?;
    let is_dev_network = matches!(config.pool.network.as_str(), "regtest" | "signet");
    let allow_unchecked = config.accounting.allow_unchecked_chain && is_dev_network;

    // ---- Chain view for payout (settlement gate) ----
    let chain: Arc<dyn ChainView> = match config.template.source.as_str() {
        "rpc" => {
            #[cfg(feature = "rpc")]
            {
                Arc::new(RpcChainView::new(
                    &config.template.rpc_url,
                    &config.template.rpc_user,
                    &config.template.rpc_pass,
                )?)
            }
            #[cfg(not(feature = "rpc"))]
            {
                anyhow::bail!("template source 'rpc' requires the `rpc` feature");
            }
        }
        other => {
            if allow_unchecked {
                tracing::warn!(
                    source = other,
                    "DEV MODE: payout has no chain view (allow_unchecked_chain=true); settlement books without an active-chain check"
                );
                Arc::new(spc_pool::payout::AlwaysConfirm)
            } else {
                anyhow::bail!(
                    "template source '{}' has no chain view; settlement requires `template.source = \"rpc\"` (or dev-only `accounting.allow_unchecked_chain = true` on regtest/signet)",
                    other
                );
            }
        }
    };

    let order_key = config.ark.as_ref().and_then(load_order_key);
    let max_order_sats = config
        .ark
        .as_ref()
        .map(|a| {
            if a.max_order_sats == 0 {
                config.accounting.coinbase_threshold_sats
            } else {
                a.max_order_sats
            }
        })
        .unwrap_or(0);

    let payout_task = if roles.contains(&Role::Payout) {
        let store =
            Store::connect_with_timeout(&config.storage.postgres, STORE_CONNECT_TIMEOUT).await?;
        let streams = Streams::connect(&config.storage.redis).await?;
        let svc = spc_pool::payout::PayoutService::new(
            store,
            streams,
            config.accounting.confirmation_depth,
            config.accounting.coinbase_threshold_sats,
            chain,
            order_key,
            max_order_sats,
        )
        .with_accounting(AccountingConfig {
            fee_bps: config.pool.fee_bps,
            window_n: config.accounting.window_n,
            dust_limit_sats: config.accounting.dust_limit_sats,
            max_payout_slots: config.accounting.max_payout_slots,
            budget_wu: config.budget.floor_wu,
            overhead_wu: 0,
        })
        // SPEC §5.2 retention/scrub policy, straight from `[accounting]`.
        .with_scrub(ScrubConfig {
            retain_windows: config.accounting.retain_windows,
            scrub_after_blocks: config.accounting.scrub_after_blocks,
        })
        // Identity rows are written with the configured chain code.
        .with_network(network_code(network));
        Some(tokio::spawn(async move {
            if let Err(e) = svc.run().await {
                tracing::error!(error = %e, "payout role stopped");
            }
        }))
    } else {
        None
    };

    if roles.contains(&Role::Api) {
        let store =
            Store::connect_with_timeout(&config.storage.postgres, STORE_CONNECT_TIMEOUT).await?;
        store.migrate().await?;
        let backend = std::sync::Arc::new(StoreBackend::with_metrics_config(
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
            spc_pool::api_backend::MetricsConfig {
                window_n: config.accounting.window_n,
                coinbase_threshold_sats: config.accounting.coinbase_threshold_sats,
                budget_floor_wu: u64::from(config.budget.floor_wu),
                budget_ceiling_wu: u64::from(config.budget.ceiling_wu),
            },
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

    // ---- Front role ----
    let front_task = if roles.contains(&Role::Front) {
        // Front requires Redis for publishing slice/block events.
        let streams = match Streams::connect(&config.storage.redis).await {
            Ok(s) => Some(s),
            Err(e) => {
                if allow_unchecked {
                    tracing::warn!(error = %e, "DEV MODE: redis unavailable; slice/block events will not be published");
                    None
                } else {
                    anyhow::bail!("front role requires Redis; connection failed: {e}");
                }
            }
        };

        let type32 = std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        ));
        let (tx, mut rx) = mpsc::channel(4096);
        let share_difficulty = config.accounting.share_difficulty;
        // Bounds how long an external new block leaves the JDS distribution and
        // miner jobs stale (SPEC §2.2, §3.5).
        let refresh_interval = Duration::from_millis(config.template.refresh_interval_ms);
        let dust_limit = u32::try_from(config.accounting.dust_limit_sats).unwrap_or(u32::MAX);
        let server = Sv1Server::new(
            network,
            config.sv1.extranonce1_bytes,
            config.sv1.extranonce2_bytes,
            share_difficulty as f64,
            tx.clone(),
        );
        if config.sv1.enabled {
            let (addr, _handle) = server.bind(&config.sv1.listen).await?;
            tracing::info!(%addr, "native SV1 listener ready");
        }
        let sv2 = Sv2Server::new(
            network,
            tx,
            type32.clone(),
            config.sv1.extranonce1_bytes + config.sv1.extranonce2_bytes,
        )
        .with_share_difficulty(share_difficulty);
        let (sv2_addr, _sv2_handle) = sv2.bind(&config.pool.listen).await?;
        tracing::info!(%sv2_addr, "native SV2 listener ready");

        let jds = JdsServer::new(config.pool.front_id);
        let (jds_addr, _jds_handle) = jds.bind(&config.jd.listen).await?;
        tracing::info!(%jds_addr, "JDS listener ready");

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
            "mock" => {
                if allow_unchecked {
                    Arc::new(MockTemplateSource::new(demo_template()))
                } else {
                    anyhow::bail!("template source 'mock' requires dev network and accounting.allow_unchecked_chain = true");
                }
            }
            other => anyhow::bail!("template source '{other}' is not implemented in this build"),
        };

        let mut front = Front::new(
            FrontConfig::from_config(&config).map_err(|e| anyhow::anyhow!(e))?,
            source.clone(),
            type32.clone(),
        );
        front
            .set_share_difficulty(config.accounting.share_difficulty)
            .map_err(|e| anyhow::anyhow!(e))?;
        // The payout role is the source of truth for pending balances and Ark
        // registrations; the front applies the bootstrap snapshot before it
        // builds its first distribution. Postgres/Redis are optional for a dev
        // front, so an unavailable store logs and the front starts from empty
        // RAM state.
        match load_bootstrap(&config).await? {
            Some(snapshot) => {
                front.apply_bootstrap(&snapshot);
                tracing::info!(
                    pending = snapshot.pending_balance.len(),
                    ark = snapshot.ark_delivery.len(),
                    rev = snapshot.rev,
                    "front bootstrap applied"
                );
            }
            None if allow_unchecked => {
                tracing::warn!("DEV MODE: front starts without a bootstrap snapshot");
            }
            None => anyhow::bail!(
                "front requires a bootstrap snapshot (pending/Ark/window); refusing to start empty"
            ),
        }
        // Live `spc:front:events`: a front-specific consumer group so every
        // front instance sees every event. The reader hands the entry id to the
        // loop, which acks only after applying the event.
        let front_events_group = format!("spc-front-{}", config.pool.front_id);
        let (front_events_tx, mut front_events_rx) =
            mpsc::channel::<(String, Arc<spc_store::wire::FrontEvent>)>(1024);
        if let Some(streams) = &streams {
            spawn_front_event_reader(
                streams.clone(),
                front_events_group.clone(),
                front_events_tx.clone(),
            );
        }
        let job = front.refresh().await?;
        publish_events(&streams, front.take_events()).await;
        let mut published = published_key(&front);
        publish_jd(&front, &jds, dust_limit).await;
        server.publish(job.clone()).await;
        sv2.set_job_txids(front.template_txids().to_vec()).await;
        sv2.publish(job).await;

        Some(tokio::spawn(async move {
            // Held for the life of the loop so the event channel never closes.
            let _events_open = front_events_tx;
            // Bounded periodic template refresh: an external new block must not
            // leave the jobs and the JDS distribution stale (SPEC §2.2, §3.5).
            // `Skip` drops missed ticks instead of queueing them up.
            let mut ticker = tokio::time::interval(refresh_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // the interval fires immediately
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        refresh_and_publish(
                            &mut front,
                            &streams,
                            &jds,
                            &server,
                            &sv2,
                            dust_limit,
                            &mut published,
                        )
                        .await;
                    }
                    share = rx.recv() => {
                        let Some(share) = share else { break };
                        match front.on_share(share) {
                            Ok(Some(block)) => {
                                tracing::info!(merkle = %block.merkle_root.iter().map(|b| format!("{b:02x}")).collect::<String>(), "share found a block; submitting");
                                if let Err(e) = source.submit_block(&block.block).await {
                                    tracing::error!(error = %e, "block submit failed");
                                }
                                publish_events(&streams, front.take_events()).await;
                                // A found block always republishes: the height
                                // and therefore the distribution moved.
                                published = None;
                                refresh_and_publish(
                                    &mut front,
                                    &streams,
                                    &jds,
                                    &server,
                                    &sv2,
                                    dust_limit,
                                    &mut published,
                                )
                                .await;
                            }
                            Ok(None) => {}
                            Err(e) => tracing::error!(error = %e, "share rejected"),
                        }
                    }
                    event = front_events_rx.recv() => {
                        if let Some((id, event)) = event {
                            let applied = front.apply_front_event(&event);
                            if applied {
                                tracing::debug!(rev = event.rev(), "front event applied");
                            }
                            // Ack only after the front has applied (or has
                            // determined the event is not newer than the
                            // bootstrapped rev). A crash before this point
                            // leaves the entry pending for replay.
                            if let Some(streams) = &streams {
                                if let Err(e) =
                                    streams.ack(FRONT_EVENTS_STREAM, &front_events_group, &id).await
                                {
                                    tracing::warn!(error = %e, id, "front event ack failed");
                                }
                            }
                        }
                    }
                }
            }
        }))
    } else {
        None
    };

    // ---- Ark role ----
    #[cfg(not(feature = "rpc"))]
    if roles.contains(&Role::Ark) {
        anyhow::bail!("the `ark` role requires the `rpc` feature (live chain tip)");
    }
    #[cfg(feature = "rpc")]
    let ark_task = if roles.contains(&Role::Ark) {
        let ark_cfg = config
            .ark
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ark role requires [ark] config"))?;
        if !ark_cfg.enabled {
            anyhow::bail!("ark role selected but [ark] enabled = false");
        }

        // Parse order pubkey
        let order_pubkey_hex = ark_cfg.order_pubkey.trim();
        if order_pubkey_hex.len() != 64 {
            anyhow::bail!("ark order_pubkey must be 64 hex chars (32 bytes)");
        }
        let order_pubkey_bytes = hex::decode(order_pubkey_hex)
            .map_err(|e| anyhow::anyhow!("ark order_pubkey invalid hex: {e}"))?;
        let order_pubkey = secp256k1::XOnlyPublicKey::from_slice(&order_pubkey_bytes)
            .map_err(|e| anyhow::anyhow!("ark order_pubkey invalid pubkey: {e}"))?;

        // Parse ark_id (8 hex chars = 4 bytes)
        let ark_id_hex = ark_cfg.ark_id.trim();
        if ark_id_hex.len() != 8 {
            anyhow::bail!("ark ark_id must be 8 hex chars (4 bytes)");
        }
        let ark_id_bytes =
            hex::decode(ark_id_hex).map_err(|e| anyhow::anyhow!("ark ark_id invalid hex: {e}"))?;
        let mut ark_id = [0u8; 4];
        ark_id.copy_from_slice(&ark_id_bytes);

        // Verify send_command is configured
        let send_command = ark_cfg
            .send_command
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ark send_command is required"))?;

        // Connect to Redis
        let streams = Streams::connect(&config.storage.redis)
            .await
            .map_err(|e| anyhow::anyhow!("ark role requires Redis: {e}"))?;

        // Build RPC tip source for live chain tip
        let rpc_client = bitcoincore_rpc::Client::new(
            &config.template.rpc_url,
            if config.template.rpc_user.is_empty() {
                bitcoincore_rpc::Auth::None
            } else {
                bitcoincore_rpc::Auth::UserPass(
                    config.template.rpc_user.clone(),
                    config.template.rpc_pass.clone(),
                )
            },
        )?;
        let tip_source = Arc::new(RpcTipSource::new(rpc_client));
        // Initial tip fetch
        let initial_tip = tip_source.refresh_tip().await?;

        // Build ArkService components
        let verifier =
            OrderVerifier::new(order_pubkey, ark_id, max_order_sats, ark_cfg.max_daily_sats);
        let dedupe = SeenOrders::new(streams.clone(), ark_cfg.retry_blocks);
        let transport: Box<dyn ArkTransport> = Box::new(ExternalCommandTransport::new(
            send_command.clone(),
            30, // 30 second timeout for the external command
        ));
        let service = Arc::new(tokio::sync::Mutex::new(ArkService::new(
            verifier,
            dedupe,
            transport,
            streams,
            initial_tip,
        )));

        // Spawn a task to periodically update the service's tip
        let service_tip = service.lock().await.tip_arc();
        let tip_source_clone = tip_source.clone();
        let tip_updater = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Ok(tip) = tip_source_clone.refresh_tip().await {
                    service_tip.store(tip, Ordering::Release);
                }
            }
        });

        Some(tokio::spawn(async move {
            // Run the service
            let mut service = service.lock().await;
            if let Err(e) = service.run().await {
                tracing::error!(error = %e, "ark role stopped");
            }
            tip_updater.abort();
        }))
    } else {
        None
    };
    #[cfg(not(feature = "rpc"))]
    let ark_task: Option<tokio::task::JoinHandle<()>> = None;

    // Wait for all roles
    let mut tasks = Vec::new();
    if let Some(t) = payout_task {
        tasks.push(t);
    }
    if let Some(t) = front_task {
        tasks.push(t);
    }
    if let Some(t) = ark_task {
        tasks.push(t);
    }

    if tasks.is_empty() {
        tracing::warn!("no roles selected; exiting");
        return Ok(());
    }

    for task in tasks {
        if let Err(e) = task.await {
            tracing::error!(error = %e, "role task panicked");
        }
    }

    Ok(())
}

/// Read the front bootstrap snapshot (WIRE-FORMATS §3.5). A missing Postgres
/// is not fatal for a dev front: it logs and the front starts from empty RAM
/// state.
async fn load_bootstrap(
    config: &Config,
) -> anyhow::Result<Option<spc_store::models::BootstrapSnapshot>> {
    let store = match Store::connect_with_timeout(&config.storage.postgres, STORE_CONNECT_TIMEOUT)
        .await
    {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!(error = %e, "postgres unavailable; front starts without a bootstrap snapshot");
            return Ok(None);
        }
    };
    // The 0002 watermark table must exist before the first bootstrap read;
    // the front may start before the payout role has migrated.
    store.migrate().await?;
    match store
        .bootstrap(i64::from(config.accounting.retain_windows))
        .await
    {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(e) => {
            tracing::warn!(error = %e, "front bootstrap failed; starting from empty pending");
            Ok(None)
        }
    }
}

/// Consume `spc:front:events` for one front. The group is per `front_id` so
/// every front instance of that id sees every event.
///
/// The reader never acks: it hands `(entry_id, event)` to the front loop, which
/// applies the event and only then acks it. If the loop is behind, or the
/// process dies before applying, the entry stays in the group's pending list
/// and is replayed on restart instead of being lost (SPEC §5.2, §7.4). Decode
/// failures are logged and acked: such a payload would never decode on retry.
fn spawn_front_event_reader(
    streams: Streams,
    group: String,
    tx: mpsc::Sender<(String, Arc<spc_store::wire::FrontEvent>)>,
) {
    tokio::spawn(async move {
        if let Err(e) = streams.ensure_group(FRONT_EVENTS_STREAM, &group).await {
            tracing::warn!(error = %e, "front event group unavailable; live front events disabled");
            return;
        }
        // Re-deliver anything this consumer was handed but never acked (a
        // crash between apply and ack). The front ignores a rev at or below the
        // bootstrapped rev, so a settled event is never counted twice.
        match streams
            .read_pending(FRONT_EVENTS_STREAM, &group, &group, 256)
            .await
        {
            Ok(pending) if !pending.is_empty() => {
                tracing::info!(entries = pending.len(), "replaying unacked front events");
                for (id, payload) in pending {
                    match spc_store::wire::FrontEvent::decode(&payload) {
                        Ok(event) => {
                            if tx.send((id, Arc::new(event))).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, id, "undecodable front event"),
                    }
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "front pending replay failed"),
        }
        loop {
            match streams
                .read_group(FRONT_EVENTS_STREAM, &group, &group, 64)
                .await
            {
                Ok(entries) => {
                    for (id, payload) in entries {
                        match spc_store::wire::FrontEvent::decode(&payload) {
                            Ok(event) => {
                                if tx.send((id, Arc::new(event))).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, id, "undecodable front event");
                                if let Err(e) = streams.ack(FRONT_EVENTS_STREAM, &group, &id).await
                                {
                                    tracing::warn!(error = %e, "front event ack failed");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "front event read failed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
}

/// The `spc:front:events` stream (WIRE-FORMATS §3.2).
const FRONT_EVENTS_STREAM: &str = "spc:front:events";

/// Identity-row network code (SPEC §5.1): 0 mainnet, 1 testnet4, 2 signet,
/// 3 regtest.
///
/// `spc_crypto::Network` carries no numeric code, so the mapping lives here.
/// It cannot see an unknown variant (the enum is exhaustive) and mainnet is
/// already refused by `Config::network`, so a payout role never writes a
/// mainnet identity by accident.
fn network_code(network: spc_crypto::Network) -> i16 {
    match network {
        spc_crypto::Network::Mainnet => 0,
        spc_crypto::Network::Testnet => 1,
        spc_crypto::Network::Signet => 2,
        spc_crypto::Network::Regtest => 3,
    }
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

/// Identity of the last distribution sent to miners and to the JDS. Used to
/// skip republishing when a periodic refresh found nothing new (same height,
/// same previous hash, same distribution fingerprint).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Published {
    height: u32,
    prev_hash: [u8; 32],
    /// Merkle identity of the non-coinbase tx set: a same-height tx-set change
    /// must republish the job even though height and prev-hash are unchanged,
    /// because the miner's merkle path changed.
    template: [u8; 32],
    fingerprint: [u8; 32],
}

fn published_key(front: &Front) -> Option<Published> {
    Some(Published {
        height: front.job()?.height,
        prev_hash: front.job()?.prev_hash,
        template: front.template_id(),
        fingerprint: front.distribution()?.fingerprint,
    })
}

/// The JD weights distribution for the current template (SPEC §3.5, §4.7):
/// weights from the same routing as the coinbase distribution, and the miner
/// output keys `P_i` from that distribution's entries.
fn jd_distribution(front: &Front, dust_limit: u32) -> Option<spc_ext::SetPayoutDistribution> {
    let weights = front.weights()?;
    let dist = front.distribution()?;
    let pool_script = front.pool_payout_script()?;
    let p_x_by_id: std::collections::BTreeMap<[u8; 32], [u8; 32]> = dist
        .entries
        .iter()
        .filter_map(|entry| entry.p_x.map(|p_x| (entry.sp_id, p_x)))
        .collect();
    match spc_pool::jds::distribution_from_weights(
        weights,
        &p_x_by_id,
        pool_script,
        &dist.a_send_x,
        dust_limit,
    ) {
        Ok(distribution) => Some(distribution),
        Err(e) => {
            tracing::error!(error = %e, "JD distribution build failed");
            None
        }
    }
}

/// Publish the current JD distribution to the JDS.
async fn publish_jd(front: &Front, jds: &JdsServer, dust_limit: u32) {
    let (Some(dist), Some(revenue), Some(jd)) = (
        front.distribution(),
        front.template_revenue(),
        jd_distribution(front, dust_limit),
    ) else {
        return;
    };
    if let Err(e) = jds.publish(dist.height, revenue, jd).await {
        tracing::warn!(error = %e, "JDS publish failed");
    }
}

/// Refresh the template and republish jobs/events when the distribution moved.
/// Returns true when the jobs were republished.
async fn refresh_and_publish(
    front: &mut Front,
    streams: &Option<Streams>,
    jds: &JdsServer,
    sv1: &Sv1Server,
    sv2: &Sv2Server,
    dust_limit: u32,
    published: &mut Option<Published>,
) -> bool {
    let job = match front.refresh().await {
        Ok(job) => job,
        Err(e) => {
            tracing::warn!(error = %e, "template refresh failed");
            return false;
        }
    };
    publish_events(streams, front.take_events()).await;
    let key = published_key(front);
    if key == *published {
        return false;
    }
    publish_jd(front, jds, dust_limit).await;
    sv1.publish(job.clone()).await;
    sv2.set_job_txids(front.template_txids().to_vec()).await;
    sv2.publish(job).await;
    *published = key;
    true
}

fn load_order_key(ark: &spc_pool::config::Ark) -> Option<bitcoin::secp256k1::Keypair> {
    let hex_key = std::fs::read_to_string(&ark.order_key_path).ok()?;
    let bytes = (0..hex_key.trim().len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_key.trim()[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let sk = bitcoin::secp256k1::SecretKey::from_slice(&bytes).ok()?;
    let secp = bitcoin::secp256k1::Secp256k1::new();
    Some(bitcoin::secp256k1::Keypair::from_secret_key(&secp, &sk))
}
