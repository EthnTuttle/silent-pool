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
use spc_pool::front::{Front, FrontConfig, FrontError, FrontEvent, V2_RETAINED_SLICES_LIMIT};
use spc_pool::jds_server::JdsServer;
#[cfg(feature = "rpc")]
use spc_pool::payout::RpcChainView;
use spc_pool::payout::{AccountingConfig, ChainView, ScrubConfig};
use spc_pool::sv1_server::Sv1Server;
use spc_pool::sv2_server::{ShareOutcome, Sv2Server};
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
/// spc `Network` -> `bitcoin::Network` for the bark wallet.
#[cfg(feature = "ark")]
fn bitcoin_network(n: spc_crypto::Network) -> bitcoin::Network {
    match n {
        spc_crypto::Network::Mainnet => bitcoin::Network::Bitcoin,
        spc_crypto::Network::Testnet => bitcoin::Network::Testnet,
        spc_crypto::Network::Signet => bitcoin::Network::Signet,
        spc_crypto::Network::Regtest => bitcoin::Network::Regtest,
    }
}

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

    // EXP-OP: the operator listener is only ever spawned inside the api
    // role below; an `[operator] listen` configured while the effective
    // role set (config or CLI override) drops the api role fails closed
    // instead of silently not binding.
    let mut operator_task: Option<tokio::task::JoinHandle<()>> = None;
    if config.operator.listen.is_some() && !roles.contains(&Role::Api) {
        anyhow::bail!("[operator] listen requires the api role (same pool API process)");
    }

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

    // The ark-only process verifies orders with a public key. It must not
    // even attempt to read the SPc-side signing key's configured/default path.
    let order_key = if roles.contains(&Role::Payout) {
        config.ark.as_ref().and_then(load_order_key)
    } else {
        None
    };
    let max_order_sats = config
        .ark
        .as_ref()
        .map(|a| {
            if a.max_order_sats == 0 {
                5_000_000
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
            budget_wu: config.budget.ceiling_wu,
            overhead_wu: 0,
        })
        // SPEC §5.2 retention/scrub policy, straight from `[accounting]`.
        .with_scrub(ScrubConfig {
            retain_windows: config.accounting.retain_windows,
            scrub_after_blocks: config.accounting.scrub_after_blocks,
        })
        // Identity rows are written with the configured chain code.
        .with_network(network_code(network));
        let svc = if !allow_unchecked {
            svc.with_economic_pool(
                FrontConfig::from_config(&config).map_err(|e| anyhow::anyhow!(e))?,
            )
        } else {
            svc
        };
        let svc = if let Some(metadata) =
            spc_pool::registration::ArkServerMetadata::from_config(&config)
                .map_err(|error| anyhow::anyhow!(error))?
        {
            svc.with_registration_ark_id(metadata.ark_id)
        } else {
            svc
        };
        let svc = if let Some(ark) = config.ark.as_ref().filter(|a| a.enabled) {
            svc.with_aggregate_server(
                ark.server.clone(),
                config.pool.network.parse::<bitcoin::Network>()?,
            )
        } else {
            svc
        };
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
        let order_pubkey = config
            .ark
            .as_ref()
            .filter(|ark| ark.enabled)
            .map(|ark| {
                ark.order_pubkey
                    .parse::<bitcoin::secp256k1::XOnlyPublicKey>()
            })
            .transpose()?;
        let backend = std::sync::Arc::new(
            StoreBackend::with_metrics_config(
                store.clone(),
                spc_pool::api_backend::pool_info(&config),
                serde_json::json!({
                    "budget_wu": config.budget.ceiling_wu,
                    "ceiling_wu": config.budget.ceiling_wu,
                    "max_payout_slots": config.accounting.max_payout_slots,
                }),
                spc_pool::api_backend::MetricsConfig {
                    window_n: config.accounting.window_n,
                    coinbase_threshold_sats: config.accounting.coinbase_threshold_sats,
                    budget_floor_wu: u64::from(config.budget.floor_wu),
                    budget_ceiling_wu: u64::from(config.budget.ceiling_wu),
                },
            )
            .with_order_pubkey(order_pubkey),
        );
        let app = spc_pool::api::router(backend, network);
        let listener = tokio::net::TcpListener::bind(&config.api.listen).await?;
        tracing::info!(addr = %config.api.listen, "api listener ready");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "api server stopped");
            }
        });

        // EXP-OP: the optional, off-by-default loopback-only operator
        // listener runs in this same API process, on its own dedicated
        // listener. The public router above is unchanged; a non-loopback
        // address was already rejected by config validation and is
        // rejected again by `OperatorServer::bind`.
        if let Some(listen) = &config.operator.listen {
            let addr: std::net::SocketAddr = listen
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid [operator] listen: {e}"))?;
            let backend = spc_pool::operator::OperatorBackend::new(
                store.clone(),
                spc_pool::operator::ObservationFacts::from_config(&config),
            );
            let server = spc_pool::operator::OperatorServer::bind(addr, backend).await?;
            tracing::info!(addr = %listen, "operator listener ready");
            operator_task = Some(tokio::spawn(async move {
                if let Err(e) = server.serve().await {
                    tracing::error!(error = %e, "operator listener stopped");
                }
            }));
        }
    }

    // ---- Front role ----
    let front_task = if roles.contains(&Role::Front) {
        if let Some(path) = std::env::var_os("SPC_SOURCE_EVIDENCE_KEY_FILE") {
            spc_pool::evidence::configure_source_signing_key(std::path::Path::new(&path))
                .map_err(anyhow::Error::msg)?;
        }
        // Front requires Redis for publishing slice/block events.
        let ark_metadata = spc_pool::registration::ArkServerMetadata::from_config(&config)
            .map_err(|error| anyhow::anyhow!(error))?;
        let streams = match Streams::connect(&config.storage.redis).await {
            Ok(s) => Some(s),
            Err(e) => {
                if ark_metadata.is_some() {
                    anyhow::bail!("0x5350 registration requires Redis: {e}");
                }
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
        let share_policy = config
            .accounting
            .share_policy(network)
            .map_err(|e| anyhow::anyhow!(e))?;
        let share_difficulty = config.accounting.legacy_share_difficulty();
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
        let authority_secret = config
            .pool
            .authority_secret()
            .map_err(|e| anyhow::anyhow!(e))?;
        let mut sv2 = Sv2Server::new(
            network,
            tx,
            type32.clone(),
            config.sv1.extranonce1_bytes + config.sv1.extranonce2_bytes,
        )
        .with_authority_secret_key(authority_secret)
        .with_share_difficulty(share_difficulty)
        .with_type32_enabled(config.extensions.share_accounting)
        .with_front_id(config.pool.front_id);
        if let (Some(metadata), Some(registration_streams)) =
            (ark_metadata.as_ref(), streams.as_ref())
        {
            sv2 = sv2.with_ark_registration(Arc::new(
                spc_pool::registration::RegistrationService::new(
                    registration_streams.clone(),
                    metadata.ark_id,
                ),
            ));
        }
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
                        // GBT has no negotiated output reservation. This is the
                        // pool-controlled output bound, not extranonce weight;
                        // actual complete block weight is independently checked.
                        (config.accounting.max_payout_slots as u32 + 1) * 43 + 47,
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
        front.set_share_policy(share_policy);
        if !allow_unchecked {
            let observation_streams = streams
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("front observations require Redis"))?;
            front.set_observation_streams(observation_streams.independent().await?);
            front.set_economic_source(
                Store::connect_with_timeout(&config.storage.postgres, STORE_CONNECT_TIMEOUT)
                    .await?,
            );
        }
        if let Some(metadata) = ark_metadata {
            front.set_ark_server(metadata);
        }
        front.set_type32_enabled(config.extensions.share_accounting);
        // The payout role is the source of truth for pending balances and Ark
        // registrations; the front applies the bootstrap snapshot before it
        // builds its first distribution. Postgres/Redis are optional for a dev
        // front, so an unavailable store logs and the front starts from empty
        // RAM state.
        match load_bootstrap(&config).await? {
            Some(snapshot) => {
                front.apply_bootstrap(&snapshot)?;
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
        if streams.is_some() {
            // The reader opens its own connection. Passing a clone of the
            // publisher's `Streams` would share one Redis client: while
            // `XREADGROUP ... BLOCK` holds that client, every inline slice or
            // block `XADD` on the same client waits out the read timeout.
            // Taking the URL (not a `Streams`) makes the accidental clone a
            // compile error.
            spawn_front_event_reader(
                config.storage.redis.clone(),
                front_events_group.clone(),
                front_events_tx.clone(),
            );
        }
        let job = loop {
            match front.refresh().await {
                Ok(job) => break job,
                Err(spc_pool::front::FrontError::AccountingUnavailable(_)) => {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                Err(error) => return Err(error.into()),
            }
        };
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
                        let channel_id = share.channel_id;
                        let sequence = share.share_index;
                        let credited_work = front.job().map(|job| job.work.m_issue);
                        let result = front.on_share(share);
                        let outcome = match &result {
                            Ok(_) => ShareOutcome::Accepted {
                                credited_work: credited_work.unwrap_or(0),
                            },
                            Err(error) => ShareOutcome::Rejected {
                                error_code: front_error_code(error).into(),
                            },
                        };
                        sv2.resolve_share(channel_id, sequence, outcome);
                        match result {
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

        // Connect to Redis (both transports emit receipts through it, and
        // the board loop keeps its durable state in it).
        let streams = Streams::connect(&config.storage.redis)
            .await
            .map_err(|e| anyhow::anyhow!("ark role requires Redis: {e}"))?;

        // Transport selection (SPEC §11.10; config validation already
        // refused unknown values and "command" outside regtest/signet).
        // Both transports re-verify the order's address at the boundary:
        // fail closed on any parse or binding error.
        let transport: Box<dyn ArkTransport> = match ark_cfg.transport.as_str() {
            "command" => {
                let send_command = ark_cfg.send_command.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("ark transport 'command' requires send_command")
                })?;
                Box::new(ExternalCommandTransport::new(
                    send_command.clone(),
                    30, // 30 second timeout for the external command
                ))
            }
            #[cfg(feature = "ark")]
            "bark" => {
                use spc_pool::ark::bark_stock::{load_seed, open_bark_wallet, BarkWalletOptions};
                use spc_pool::ark::bark_transport::BarkClient as _;
                use spc_pool::ark::bark_transport::BarkTransport;
                use spc_pool::ark::board::{
                    descriptor_account_xpub, BoardingLoop, BoardingPolicy, PoolOutputKeychain,
                    RedisBoardRegistry, RpcBoardChain,
                };
                use spc_pool::ark::wallet_control::{Controller, StockOwner};
                // Fail-closed startup (decision record, clause 7): the
                // seed is hex, 64 bytes, owner-only, and must not resolve
                // under the SPc-side secret store.
                let controller = Controller::open(&ark_cfg.datadir)?;
                let seed = load_seed(&ark_cfg.seed_path).map_err(|e| anyhow::anyhow!("{e}"))?;
                // The pool output keychain under the same seed, and the
                // cross-check that its account xpub is what
                // [pool] pool_payout_descriptor commits to: a mismatch
                // would board outputs the SPc side cannot see.
                let keychain = PoolOutputKeychain::from_seed(
                    bitcoin_network(network),
                    &seed,
                    &ark_cfg.pool_output_account,
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                let descriptor_xpub = descriptor_account_xpub(&config.pool.pool_payout_descriptor)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if descriptor_xpub != keychain.account_xpub() {
                    anyhow::bail!(
                        "the ark seed's {} xpub does not match [pool] pool_payout_descriptor; refusing to board outputs the SPc side cannot see",
                        ark_cfg.pool_output_account
                    );
                }
                let client = open_bark_wallet(BarkWalletOptions {
                    network: bitcoin_network(network),
                    seed,
                    datadir: ark_cfg.datadir.clone(),
                    server_address: ark_cfg.server.clone(),
                    bitcoind_url: config.template.rpc_url.clone(),
                    bitcoind_user: config.template.rpc_user.clone(),
                    bitcoind_pass: config.template.rpc_pass.clone(),
                    // bark's own mainnet default; the feerate floor only
                    // bites before the first chain estimate.
                    fallback_fee_sat_vb: 2,
                })
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                // One full sync before the service starts consuming
                // orders: a send against a never-synced wallet would fail
                // on missing inputs, and an unreachable server must be a
                // startup error, not a stream of Failed receipts.
                controller.startup(&client).await?;
                // Native funding handoff for the independently owned pool
                // wallet. Observation only: no second signer or ledger credit.
                {
                    use std::io::Write;
                    use std::os::unix::fs::OpenOptionsExt;
                    let vtxos = client.list_vtxos().await.map_err(anyhow::Error::msg)?;
                    let spendable = vtxos
                        .iter()
                        .filter(|v| v.spendable)
                        .try_fold(0u64, |n, v| n.checked_add(v.sats))
                        .ok_or_else(|| anyhow::anyhow!("Ark balance overflow"))?;
                    let address = client.onchain_address().await.map_err(anyhow::Error::msg)?;
                    let status = serde_json::json!({
                        "schema": "spc.pool.ark.provision/v1",
                        "server": ark_cfg.server,
                        "onchain_funding_address": address.to_string(),
                        "float_spendable_sats": spendable,
                        "float_target_sats": ark_cfg.float_target_sats,
                        "funded_e2e": false,
                        "observed_at_unix": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    });
                    let pending = ark_cfg.datadir.join("provision.json.pending");
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&pending)?;
                    file.write_all(&serde_json::to_vec_pretty(&status)?)?;
                    file.sync_all()?;
                    std::fs::rename(pending, ark_cfg.datadir.join("provision.json"))?;
                }
                // Boarding: the separate refill surface (SPEC §11.3
                // "Refilling (H_f + 100)"), on the same wallet but never
                // reachable from the order stream.
                let board_chain = RpcBoardChain::new(
                    &config.template.rpc_url,
                    &config.template.rpc_user,
                    &config.template.rpc_pass,
                )?;
                let boarding = BoardingLoop::new(
                    board_chain,
                    client.clone(),
                    RedisBoardRegistry::new(streams.clone()),
                    keychain,
                    BoardingPolicy::new(ark_cfg.float_target_sats, ark_cfg.min_remaining_blocks),
                );
                let wallet_owner =
                    StockOwner::new(client.clone(), &config, bitcoin_network(network), ark_id)?;
                tokio::spawn(controller.run(wallet_owner, boarding));
                Box::new(BarkTransport::new(
                    client,
                    ark_id,
                    network != spc_crypto::Network::Mainnet,
                ))
            }
            #[cfg(not(feature = "ark"))]
            "bark" => {
                anyhow::bail!(
                    "ark transport 'bark' requires the build feature 'ark' (stock bark-wallet at the UPSTREAM_DEPS.md pin)"
                )
            }
            other => {
                anyhow::bail!(
                    "unknown ark transport '{other}' (config validation should have caught this)"
                )
            }
        };

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
        let service = Arc::new(tokio::sync::Mutex::new(
            ArkService::new(verifier, dedupe, transport, streams, initial_tip)
                .require_fresh_authorization(),
        ));

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
    if let Some(t) = operator_task {
        // The operator listener is part of this process's lifecycle, not
        // a detached service: it is awaited with the other role tasks and
        // stops with the process.
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
    let retain = if config.accounting.work_unit_version == 2 {
        i64::try_from(V2_RETAINED_SLICES_LIMIT).unwrap_or(i64::MAX)
    } else {
        // Legacy retention is deliberately unchanged.
        i64::from(config.accounting.retain_windows)
    };
    match store.bootstrap(retain).await {
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
///
/// This function deliberately takes the Redis URL rather than a `Streams`.
/// `Streams` clones share one physical client, and a client held in
/// `XREADGROUP ... BLOCK` also serializes unrelated `XADD` calls on the same
/// client. The reader must own an independent connection so slice/block
/// publication is not delayed by the read timeout.
fn spawn_front_event_reader(
    redis_url: String,
    group: String,
    tx: mpsc::Sender<(String, Arc<spc_store::wire::FrontEvent>)>,
) {
    tokio::spawn(async move {
        let streams = match Streams::connect(&redis_url).await {
            Ok(streams) => streams,
            Err(e) => {
                tracing::warn!(error = %e, "front event reader connection failed; live front events disabled");
                return;
            }
        };
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
        bits: 0x200f_ffff,
        curtime: 1_700_000_000,
        subsidy: 5_000_000_000,
        fees: 0,
        coinbase_value: 5_000_000_000,
        transactions: Vec::new(),
        merkle_branches: Vec::new(),
        coinbase_output_max_additional_size: 50_000,
    }
}

fn front_error_code(error: &FrontError) -> &'static str {
    match error {
        FrontError::StaleJob | FrontError::NoJob => "stale-share",
        FrontError::DuplicateShare => "duplicate-share",
        FrontError::LowDifficulty => "difficulty-too-low",
        FrontError::ExtranonceLength => "bad-extranonce-size",
        FrontError::StaleTime => "invalid-ntime",
        FrontError::InvalidVersion => "invalid-version",
        _ => "invalid-share",
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
        match streams.add(stream, &payload).await {
            Ok(id) => spc_pool::evidence::source_publication(stream, &payload, Some(&id)),
            Err(err) => {
                spc_pool::evidence::source_publication(stream, &payload, None);
                tracing::error!(error = %err, stream, "stream publish failed");
            }
        }
    }
}

/// Identity of the last distribution sent to miners and to the JDS. Used to
/// skip republishing when a periodic refresh found nothing new: same height,
/// prevhash, tx set, version, bits, distribution and front job id. Ordinary
/// curtime is not part of this identity, and the issued job's min_ntime
/// stays immutable. The job id is load-bearing: a same-txid witness
/// malleation moves the BIP 141 commitment (hence the job id) while the
/// txid-merkle identity, height, prev-hash, bits and distribution
/// fingerprint all stay equal - only the job id distinguishes the two jobs
/// miners must be told apart (decision 2026-11-08-pool-witness-commitment).
#[derive(Clone, PartialEq, Eq)]
struct Published {
    height: u32,
    prev_hash: [u8; 32],
    /// Merkle identity of the non-coinbase tx set: a same-height tx-set change
    /// must republish the job even though height and prev-hash are unchanged,
    /// because the miner's merkle path changed.
    template: [u8; 32],
    version: u32,
    bits: u32,
    fingerprint: [u8; 32],
    /// The front's current job id (hex of the job sequence). Covers every
    /// job-changing input, including commitment-only changes the fields
    /// above cannot see.
    job_id: String,
}

fn published_key(front: &Front) -> Option<Published> {
    Some(Published {
        height: front.job()?.height,
        prev_hash: front.job()?.prev_hash,
        template: front.template_id(),
        version: front.job()?.version,
        bits: front.job()?.nbits,
        fingerprint: front.distribution()?.fingerprint,
        job_id: front.job()?.job_id.clone(),
    })
}

/// The JD weights distribution for the current template (SPEC §3.5, §4.7):
/// weights from the same routing as the coinbase distribution, and the miner
/// output keys `P_i` from that distribution's entries.
fn jd_distribution(front: &Front, dust_limit: u32) -> Option<spc_ext::SetPayoutDistribution> {
    let weights = front.weights()?;
    let dist = front.distribution()?;
    let pool_script = front.pool_payout_script()?;
    let p_x_by_id: std::collections::BTreeMap<[u8; 32], [u8; 32]> = weights
        .entries
        .iter()
        .filter_map(|entry| entry.p_x.first().map(|p_x| (entry.sp_id, *p_x)))
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
            // The front has invalidated its job too; do not let either ingress
            // continue accepting the previous generation after a failed roll.
            sv1.clear_current().await;
            sv2.clear_current().await;
            *published = None;
            tracing::warn!(error = %e, "template refresh failed; current work invalidated");
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

#[cfg(test)]
mod wiring_guards {
    use super::*;

    /// The reader must never receive a `Streams` clone: clones share one
    /// physical Redis client and would serialize a blocking XREADGROUP with
    /// every inline XADD. Taking an owned URL makes that mistake a type error.
    #[test]
    #[allow(clippy::type_complexity)]
    fn front_event_reader_requires_an_owned_redis_url() {
        let _: fn(
            String,
            String,
            tokio::sync::mpsc::Sender<(String, std::sync::Arc<spc_store::wire::FrontEvent>)>,
        ) = spawn_front_event_reader;
    }
}

/// B1 (Fable rework 2026-11-08): the main-level publication identity carries
/// the front job id, so a commitment-only same-txid witness change - which
/// moves the BIP 141 commitment and the job id while leaving the height,
/// prev-hash, bits, version, fees, txid-merkle identity and distribution
/// fingerprint untouched - still reaches both ingresses. No height or fee
/// change is involved or required.
#[cfg(test)]
mod witness_publication {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash as _;
    use bitcoin::pow::{CompactTarget, Target};
    use bitcoin::transaction::Version;
    use bitcoin::{
        Amount, BlockHash, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    };
    use spc_crypto::Network;
    use stratum_apps::network_helpers::connect_with_noise;
    use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
    use stratum_core::binary_sv2::{self, Str0255, U256};
    use stratum_core::common_messages_sv2::{
        Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    };
    use stratum_core::framing_sv2::framing::MessageFrame;
    use stratum_core::mining_sv2::{
        NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
        SetNewPrevHash, SetTarget, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, MESSAGE_TYPE_SET_TARGET,
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::TcpStream;

    fn front_config() -> FrontConfig {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xprv =
            bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
        let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);
        FrontConfig {
            network: Network::Regtest,
            fee_bps: 100,
            window_n: 8,
            slice_fee_delta_sats: 0,
            max_slice_shares: 65_000,
            dust_limit_sats: 330,
            coinbase_threshold_sats: 330,
            max_payout_slots: 256,
            budget_wu: 100_000,
            overhead_wu: 0,
            pool_payout_descriptor: spc_pool::coinbase::PoolPayoutDescriptor::parse(
                &format!("tr({xpub}/0/*)"),
                Network::Regtest,
            )
            .unwrap(),
            pool_payout_script: Some(
                spc_pool::coinbase::p2tr_script(&[0x99; 32])
                    .to_bytes()
                    .to_vec(),
            ),
            extranonce_len: 8,
            front_id: 0,
            keep_slices: 16,
        }
    }

    fn witness_template(tx: Vec<u8>) -> Template {
        Template {
            height: 800_000,
            prev_hash: [0x11; 32],
            version: 0x2000_0000,
            // Regtest-easy: a short nonce search finds a block-finding share.
            bits: 0x207f_ffff,
            curtime: 1_700_000_000,
            subsidy: 312_500_000,
            fees: 0,
            coinbase_value: 312_500_000,
            transactions: vec![tx],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 50_000,
        }
    }

    /// A one-input transaction whose witness stack is `stack`.
    fn witness_tx(stack: &[Vec<u8>]) -> Vec<u8> {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_byte_array([0x09; 32]),
                    vout: 9,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::from_slice(stack),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    /// A source whose template the test can swap between refreshes.
    struct SwappableSource(std::sync::Mutex<Template>);

    #[async_trait::async_trait]
    impl TemplateSource for SwappableSource {
        async fn get_template(&self) -> Result<Template, spc_pool::template::TemplateError> {
            Ok(self.0.lock().expect("poisoned").clone())
        }
        async fn submit_block(
            &self,
            _block: &[u8],
        ) -> Result<(), spc_pool::template::TemplateError> {
            Ok(())
        }
    }

    fn address() -> String {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap(),
        );
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap(),
        );
        spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap()
    }

    /// Subscribe and authorize an SV1 miner; the notify lines arrive with
    /// each publication.
    async fn sv1_subscribe(
        addr: std::net::SocketAddr,
    ) -> (tokio::io::Lines<BufReader<OwnedReadHalf>>, OwnedWriteHalf) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        write
            .write_all(br#"{"id":1,"method":"mining.subscribe","params":[]}"#)
            .await
            .unwrap();
        write.write_all(b"\n").await.unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let auth = serde_json::json!({"id":2,"method":"mining.authorize","params":[address(),"x"]});
        write.write_all(auth.to_string().as_bytes()).await.unwrap();
        write.write_all(b"\n").await.unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        (lines, write)
    }

    /// The next `mining.notify`, skipping `mining.set_difficulty`.
    async fn next_notify(
        lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    ) -> serde_json::Value {
        loop {
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .expect("sv1 line within 5s")
                .expect("sv1 line")
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            if v["method"] == "mining.notify" {
                return v;
            }
        }
    }

    async fn assert_sv1_quiet(lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>) {
        let result = tokio::time::timeout(Duration::from_millis(400), lines.next_line()).await;
        assert!(result.is_err(), "sv1 must stay quiet when nothing changed");
    }

    async fn send_message<T>(write: &mut NoiseTcpWriteHalf, message: T, ty: u8, channel: bool)
    where
        T: binary_sv2::Serialize + binary_sv2::GetSize,
    {
        write
            .write_frame(MessageFrame::from_message(message, ty, 0, channel).unwrap())
            .await
            .unwrap();
    }

    async fn next_frame(read: &mut NoiseTcpReadHalf) -> (u8, Vec<u8>) {
        let mut frame = read.read_frame().await.unwrap();
        (frame.header().msg_type(), frame.payload().to_vec())
    }

    /// Noise + SetupConnection + OpenExtendedMiningChannel: the channel is
    /// live once the server acknowledges it.
    async fn sv2_open_channel(
        addr: std::net::SocketAddr,
        authority: stratum_apps::key_utils::Secp256k1PublicKey,
    ) -> (NoiseTcpReadHalf, NoiseTcpWriteHalf) {
        let stream = connect_with_noise(TcpStream::connect(addr).await.unwrap(), Some(authority))
            .await
            .unwrap();
        let (mut read, mut write) = stream.into_split();
        let setup = SetupConnection {
            protocol: Protocol::MiningProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: Str0255::try_from("127.0.0.1").unwrap(),
            endpoint_port: 0,
            vendor: Str0255::try_from("lp1-peer").unwrap(),
            hardware_version: Str0255::try_from("1").unwrap(),
            firmware: Str0255::try_from("1").unwrap(),
            device_id: Str0255::try_from("1").unwrap(),
        };
        send_message(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await;
        let (ty, mut payload) = next_frame(&mut read).await;
        assert_eq!(ty, 0x01, "setup connection success");
        let _: SetupConnectionSuccess = binary_sv2::from_bytes(&mut payload).unwrap();
        let identity = address();
        let open = OpenExtendedMiningChannel {
            request_id: 7,
            user_identity: Str0255::try_from(identity.as_str()).unwrap(),
            nominal_hash_rate: 1.0,
            max_target: U256::from(&[0xff; 32]),
            min_extranonce_size: 4,
        };
        send_message(
            &mut write,
            open,
            MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
            false,
        )
        .await;
        loop {
            let (ty, mut payload) = next_frame(&mut read).await;
            match ty {
                MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR => panic!("channel open rejected"),
                0x14 => {
                    let _: OpenExtendedMiningChannelSuccess =
                        binary_sv2::from_bytes(&mut payload).unwrap();
                    return (read, write);
                }
                _ => {}
            }
        }
    }

    /// The job id of the next NewExtendedMiningJob, skipping SetNewPrevHash
    /// and SetTarget frames.
    async fn next_sv2_job(read: &mut NoiseTcpReadHalf) -> u32 {
        loop {
            let (ty, mut payload) = tokio::time::timeout(Duration::from_secs(5), next_frame(read))
                .await
                .expect("sv2 frame within 5s");
            match ty {
                MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                    let job: NewExtendedMiningJob = binary_sv2::from_bytes(&mut payload).unwrap();
                    return job.job_id;
                }
                MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                    let _: SetNewPrevHash = binary_sv2::from_bytes(&mut payload).unwrap();
                }
                MESSAGE_TYPE_SET_TARGET => {
                    let _: SetTarget = binary_sv2::from_bytes(&mut payload).unwrap();
                }
                _ => {}
            }
        }
    }

    async fn assert_sv2_quiet(read: &mut NoiseTcpReadHalf) {
        let result = tokio::time::timeout(Duration::from_millis(400), next_frame(read)).await;
        assert!(result.is_err(), "sv2 must stay quiet when nothing changed");
    }

    /// A fully valid block-finding share against the front's current job
    /// (the same construction a real miner performs: full coinbase, folded
    /// merkle root, and a header hash that meets the network target).
    fn block_finding_share(front: &Front, nonce_from: u32) -> spc_pool::sv1::Share {
        let job = front.job().expect("job");
        let extranonce = vec![0xaa; 8];
        let mut coinbase = job.coinb1.clone();
        coinbase.extend_from_slice(&extranonce);
        coinbase.extend_from_slice(&job.coinb2);
        let mut merkle_root = spc_pool::block::txid(&coinbase);
        for sibling in &job.merkle_branches {
            let mut pair = Vec::with_capacity(64);
            pair.extend_from_slice(&merkle_root);
            pair.extend_from_slice(sibling);
            merkle_root = spc_pool::block::txid(&pair);
        }
        let target = Target::from_compact(CompactTarget::from_consensus(job.nbits));
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
        let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
        for nonce in nonce_from..=nonce_from.saturating_add(4_000_000) {
            let mut header = Vec::with_capacity(80);
            header.extend_from_slice(&job.version.to_le_bytes());
            header.extend_from_slice(&job.prev_hash);
            header.extend_from_slice(&merkle_root);
            header.extend_from_slice(&job.ntime.to_le_bytes());
            header.extend_from_slice(&job.nbits.to_le_bytes());
            header.extend_from_slice(&nonce.to_le_bytes());
            let hash = bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array();
            if target.is_met_by(BlockHash::from_byte_array(hash)) {
                return spc_pool::sv1::Share {
                    sp_id: spc_crypto::sp_id(&scan, &spend),
                    b_scan: scan.serialize(),
                    b_spend: spend.serialize(),
                    channel_id: 1,
                    job_id: job.job_id.clone(),
                    nonce,
                    ntime: job.ntime,
                    version: job.version,
                    extranonce,
                    share_index: nonce,
                    job_fees: 0,
                    coinbase,
                    merkle_root,
                    share_hash: hash,
                };
            }
        }
        panic!("no nonce found");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn commitment_only_witness_change_republishes_both_ingresses() {
        // Two transaction sets with the same txid: only the witness bytes -
        // hence the wtxid, witness root and commitment - differ.
        let original = witness_tx(&[vec![0u8; 4]]);
        let malleated = witness_tx(&[vec![9u8; 4], vec![9u8; 8]]);
        assert_eq!(
            spc_pool::template::txid_of(&original),
            spc_pool::template::txid_of(&malleated),
            "the witness malleation keeps the txid"
        );
        let source = Arc::new(SwappableSource(std::sync::Mutex::new(witness_template(
            original,
        ))));
        let mut front = Front::new(
            front_config(),
            source.clone(),
            Arc::new(std::sync::Mutex::new(
                spc_pool::type32::Type32Store::default(),
            )),
        );

        // The real main publication fan-out: SV1 and SV2 listeners, the JDS,
        // and the dedup key `refresh_and_publish` consults.
        let (share_tx, _share_rx) = mpsc::channel(64);
        let sv1 = Sv1Server::new(Network::Regtest, 4, 4, 1.0, share_tx.clone());
        let (sv1_addr, _sv1_guard) = sv1.bind("127.0.0.1:0").await.unwrap();
        let type32 = Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        ));
        let sv2 = Sv2Server::new(Network::Regtest, share_tx, type32, 8);
        let (sv2_addr, _sv2_guard) = sv2.bind("127.0.0.1:0").await.unwrap();
        let jds = JdsServer::new(0);
        let (_jds_addr, _jds_guard) = jds.bind("127.0.0.1:0").await.unwrap();

        // Ingress clients connect before the first publication.
        let (mut sv1_lines, _sv1_write) = sv1_subscribe(sv1_addr).await;
        let (mut sv2_read, _sv2_write) =
            sv2_open_channel(sv2_addr, sv2.authority_public_key()).await;

        let mut published = None;
        assert!(
            refresh_and_publish(&mut front, &None, &jds, &sv1, &sv2, 330, &mut published).await,
            "the first job is published"
        );
        let notify_a = next_notify(&mut sv1_lines).await;
        let job_a: String = notify_a["params"][0].as_str().unwrap().to_string();
        let sv2_job_a = next_sv2_job(&mut sv2_read).await;
        assert_eq!(
            format!("{sv2_job_a:08x}"),
            job_a,
            "both ingresses carry the same job"
        );

        // The commitment-only change: same height, prev-hash, bits, version,
        // fees and txid-merkle identity. Nothing but the witness moved.
        *source.0.lock().expect("poisoned") = witness_template(malleated);
        assert!(
            refresh_and_publish(&mut front, &None, &jds, &sv1, &sv2, 330, &mut published).await,
            "a commitment-only same-txid witness change must republish"
        );
        let notify_b = next_notify(&mut sv1_lines).await;
        let job_b: String = notify_b["params"][0].as_str().unwrap().to_string();
        let sv2_job_b = next_sv2_job(&mut sv2_read).await;
        assert_ne!(job_b, job_a, "the SV1 notify carries a new job id");
        assert_ne!(sv2_job_b, sv2_job_a, "the SV2 job carries a new job id");
        assert_eq!(format!("{sv2_job_b:08x}"), job_b);

        // An unchanged set does not republish needlessly.
        assert!(
            !refresh_and_publish(&mut front, &None, &jds, &sv1, &sv2, 330, &mut published).await,
            "an unchanged set must not republish"
        );
        assert_sv1_quiet(&mut sv1_lines).await;
        assert_sv2_quiet(&mut sv2_read).await;

        // Per policy: work on the new job is accepted (and assembles a
        // witness block through the real on_share path, commitment
        // re-verified), work on the old job is stale.
        let mut stale = block_finding_share(&front, 1);
        stale.job_id = job_a;
        assert!(matches!(front.on_share(stale), Err(FrontError::StaleJob)));
        let fresh = block_finding_share(&front, 2);
        assert!(
            front.on_share(fresh).unwrap().is_some(),
            "the new commitment's block-finding share assembles a block"
        );
    }
}
