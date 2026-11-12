//! `spc-lab` CLI: one supervised run path, headless execution, and the same
//! snapshot/SSE contract for an attached browser or recorded-run review.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use spc_lab::artifacts::ArtifactPaths;
use spc_lab::command::{now_ms, LabCommand};
use spc_lab::engine::{EngineOptions, RunEngine};
use spc_lab::evidence::AssertionEvaluation;
use spc_lab::http::{HttpServer, WebAssets};
use spc_lab::ids::{ActorId, CommandId};
use spc_lab::scenario::Scenario;
use spc_lab::LabConfig;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");
const LAB_HTML: &str = include_str!("../static/lab.html");
const LAB_JS: &str = include_str!("../static/lab.js");
const LAB_CSS: &str = include_str!("../static/lab.css");

#[derive(Parser)]
#[command(name = "spc-lab", version = spc_lab::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Parse and validate a versioned scenario without starting a process.
    Validate {
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Prepare and run a scenario. The default is a headless run that writes
    /// machine-readable artifacts.
    Run {
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long, default_value_t = false)]
        headless: bool,
        /// Bind the loopback HTTP/SSE view while the run is live.
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        duration_secs: Option<u64>,
        /// Leave processes alive for a later adopt/reap.
        #[arg(long, default_value_t = false)]
        keep: bool,
    },
    /// Attach the HTTP/SSE view to an existing run directory.
    Serve {
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: String,
        #[arg(long)]
        duration_secs: Option<u64>,
    },
    /// Verify and adopt processes from a run manifest.
    Adopt {
        #[arg(long)]
        run_dir: PathBuf,
    },
    /// Stop only verified processes from a run manifest; never kill by PID alone.
    Reap {
        #[arg(long)]
        run_dir: PathBuf,
    },
    /// Print the same snapshot the browser receives.
    Snapshot {
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long)]
        cursor: Option<u64>,
    },
    /// Replay the evidence stream as SSE on stdout.
    Events {
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    /// Submit one typed command to an attached run.
    Command {
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long)]
        json: String,
    },
    /// Print the durable run manifest (private by default).
    Manifest {
        #[arg(long)]
        run_dir: PathBuf,
    },
    /// Perform one JSON-RPC call.  The helper exists so that a caller can
    /// reach a namespace local endpoint by running this binary inside the
    /// namespace.
    Rpc {
        #[arg(long, default_value_t = false)]
        typed: bool,
        #[arg(long, default_value_t = 1)]
        request_id: u64,
        #[arg(long)]
        url: String,
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "[]")]
        params: String,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        pass: Option<String>,
        /// Read HTTP Basic credentials from this private mode-0600 cookie;
        /// the password never appears in helper argv or environment.
        #[arg(long, conflicts_with_all = ["user", "pass"])]
        auth_file: Option<PathBuf>,
    },
    /// Perform one HTTP request and print the body.
    Http {
        #[arg(long)]
        url: String,
        #[arg(long, default_value = "GET")]
        method: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        pass: Option<String>,
    },
    /// Probe a TCP endpoint and exit 0 when it accepted a connection.
    ///
    /// With `--pid` the probe also proves that the listening socket belongs
    /// to that process.  Run inside a namespace, the listener ownership is
    /// checked against the namespace's own socket table.
    ProbeTcp {
        #[arg(long)]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long, default_value_t = 2000)]
        timeout_ms: u64,
        #[arg(long)]
        pid: Option<u32>,
    },
    /// Internal actor-lifetime helper. It sets PR_SET_PDEATHSIG and execs the
    /// requested program without running a pre-exec callback in the parent.
    #[command(name = "actor-exec", hide = true)]
    ActorExec {
        #[arg(long)]
        parent_pid: u32,
        #[arg(long)]
        parent_start_ticks: u64,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// Start the built-in real-process host runtime: rendered node, pool,
    /// miner, Redis and PostgreSQL configs, live evidence collectors and an
    /// optional browser control surface.
    Runtime {
        #[arg(long, env = "SPC_LAB_TXD")]
        txd: Option<PathBuf>,
        /// Native interpreter/program for the canonical probe adapter.
        #[arg(long, env = "SPC_LAB_TRAFFIC_PROBE")]
        traffic_probe: Option<PathBuf>,
        /// Prefix arguments, e.g. the frozen probe.py path for a Python adapter.
        #[arg(long)]
        traffic_probe_arg: Vec<String>,
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        run_dir: PathBuf,
        #[arg(long, env = "SPC_LAB_RBITCOIN_NODE")]
        node: PathBuf,
        #[arg(long, env = "SPC_LAB_POOL")]
        pool: PathBuf,
        #[arg(long, env = "SPC_LAB_CPU_MINER")]
        miner: PathBuf,
        #[arg(long, env = "SPC_LAB_SCANNER")]
        scanner: Option<PathBuf>,
        /// Pinned Nix stock captaind package; required for a local Ark scenario.
        #[arg(long, requires = "grpcurl")]
        captaind: Option<PathBuf>,
        #[arg(long)]
        wallet_agent_bin: Option<PathBuf>,
        #[arg(long, requires = "captaind")]
        grpcurl: Option<PathBuf>,
        #[arg(long, env = "SPC_LAB_POSTGRES_BIN")]
        postgres_bin_dir: PathBuf,
        #[arg(long, env = "SPC_LAB_REDIS_SERVER")]
        redis_server: PathBuf,
        #[arg(long, env = "SPC_LAB_REDIS_CLI")]
        redis_cli: PathBuf,
        #[arg(long, default_value = "0x200fffff")]
        pow_bits: String,
        #[arg(long)]
        base_revision: String,
        #[arg(long)]
        dirty_tree_hash: String,
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        duration_secs: Option<u64>,
        /// Read stdin so a typed `stop` submits the durable stop command.
        #[arg(long, default_value_t = false)]
        interactive: bool,
        /// Place the actors in network namespaces owned by the namespace
        /// broker. The controller stays in the host user/network namespaces;
        /// only actors enter the broker user namespace and their endpoint
        /// network namespace at launch.
        #[arg(long, default_value_t = false)]
        topology: bool,
        /// Render the approved EXP-OP read-only operator page into every
        /// pool config (one extra loopback listener per front) and print
        /// its URL after a route-readiness proof. Host runs only; off by
        /// default.
        #[arg(long, default_value_t = false, conflicts_with = "topology")]
        operator_ui: bool,
        /// The namespace broker implementation (the approved LP-4a backend).
        #[arg(long)]
        broker_script: Option<PathBuf>,
        /// The `unshare` binary that creates the broker's namespaces.
        #[arg(long)]
        unshare: Option<PathBuf>,
        /// The `nsenter` binary used to join a namespace.
        #[arg(long)]
        nsenter: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Validate { scenario, config } => {
            let scenario = read_scenario(&scenario)?;
            let mut report = scenario
                .validate()
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let preflight = if let Some(config) = config {
                let config = read_config(&config)?;
                Some(
                    spc_lab::config::preflight(&scenario, &config)
                        .map_err(|error| anyhow::anyhow!("{error}"))?,
                )
            } else {
                None
            };
            if let Some(preflight) = &preflight {
                report.warnings.extend(preflight.warnings.clone());
            }
            let mut seen = std::collections::BTreeSet::new();
            report
                .warnings
                .retain(|warning| seen.insert(warning.clone()));
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "valid": true,
                    "scenario_version": scenario.scenario_version,
                    "run_id": scenario.run.id,
                    "required_capabilities": report.required_capabilities,
                    "warnings": report.warnings,
                    "preflight": preflight.map(|report| serde_json::json!({
                        "capabilities": report.capabilities,
                        "effective_profiles": report.effective_profiles.keys().map(ToString::to_string).collect::<Vec<_>>(),
                    })),
                }))?
            );
        }
        Commands::Run {
            scenario,
            config,
            run_dir,
            headless,
            listen,
            duration_secs,
            keep,
        } => {
            let scenario = read_scenario(&scenario)?;
            let config = read_config(&config)?;
            let lifetime_helper = if scenario.traffic.is_some() {
                Some(
                    std::env::current_exe()
                        .context("traffic run requires its real actor-exec helper")?,
                )
            } else {
                None
            };
            let options = EngineOptions {
                namespaces: Default::default(),
                nsenter: None,
                user_namespace: None,
                user_namespace_owner: None,
                namespace_owners: Default::default(),
                namespace_broker: None,
                lifetime_helper,
                addresses: Default::default(),
                link_backend: None,
                run_dir: run_dir.clone(),
                config,
                base_revision: env_or_git(
                    "SPC_LAB_BASE_REVISION",
                    &["rev-parse", "--short", "HEAD"],
                ),
                dirty_tree_hash: env_or_git("SPC_LAB_DIRTY_TREE_HASH", &["status", "--porcelain"]),
                owner_instance: format!("cli-{}", std::process::id()),
                evidence_capacity: 100_000,
                operator_ui: false,
            };
            let engine = RunEngine::prepare(scenario, options)?;
            let start = LabCommand::new(
                engine.run_id().clone(),
                ActorId::trusted("cli"),
                spc_lab::CommandKind::StartRun {
                    reason: Some("primary CLI run".to_string()),
                },
            );
            let start_outcome = engine.submit(start);
            if start_outcome.state != spc_lab::OutcomeState::Succeeded {
                anyhow::bail!("durable start command failed: {start_outcome:?}");
            }
            let server = if let Some(listen) = listen {
                let server = HttpServer::start(
                    Arc::clone(&engine),
                    listen,
                    WebAssets {
                        index_html: INDEX_HTML,
                        app_js: APP_JS,
                        style_css: STYLE_CSS,
                        lab_html: LAB_HTML,
                        lab_js: LAB_JS,
                        lab_css: LAB_CSS,
                    },
                )?;
                println!("lab view: {}", server.url());
                Some(server)
            } else {
                None
            };
            let _ = headless;
            execute_phases(&engine)?;
            if let Some(seconds) = duration_secs {
                wait_with_assertions(&engine, Duration::from_secs(seconds));
            }
            let assertions = engine.assertions();
            if !keep {
                let stop = LabCommand::new(
                    engine.run_id().clone(),
                    ActorId::trusted("cli"),
                    spc_lab::CommandKind::StopRun {
                        reason: "primary CLI run completed".to_string(),
                    },
                );
                let stop_outcome = engine.submit(stop);
                if stop_outcome.state != spc_lab::OutcomeState::Succeeded {
                    anyhow::bail!("durable stop command failed: {stop_outcome:?}");
                }
            }
            print_result(&engine, &assertions);
            if keep {
                println!("run kept at {}", engine.run_dir.display());
                // Keep the server alive for a bounded duration if requested;
                // otherwise return while leaving owned processes for adoption.
                if let Some(server) = server {
                    if let Some(seconds) = duration_secs {
                        std::thread::sleep(Duration::from_secs(seconds));
                        server.shutdown();
                    }
                }
            }
        }
        Commands::Serve {
            run_dir,
            listen,
            duration_secs,
        } => {
            let engine =
                RunEngine::attach_read_only(&run_dir, &format!("serve-{}", std::process::id()))?;
            let server = HttpServer::start(
                Arc::clone(&engine),
                listen,
                WebAssets {
                    index_html: INDEX_HTML,
                    app_js: APP_JS,
                    style_css: STYLE_CSS,
                    lab_html: LAB_HTML,
                    lab_js: LAB_JS,
                    lab_css: LAB_CSS,
                },
            )?;
            println!("lab view: {}", server.url());
            match duration_secs {
                Some(seconds) => std::thread::sleep(Duration::from_secs(seconds)),
                None => loop {
                    std::thread::park();
                },
            }
            server.shutdown();
        }
        Commands::Adopt { run_dir } => {
            let engine = RunEngine::attach(&run_dir, &format!("adopt-{}", std::process::id()))?;
            let results = engine.adopt_processes()?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            println!(
                "adopt is a one-shot verification; it does not leave a control daemon. Use `run --listen` for live controls, or `serve` for read-only review."
            );
        }
        Commands::Reap { run_dir } => {
            // Cleanup-purpose attach: a stopped run has no live processes to
            // re-adopt, so the strict live-adopt gate would refuse; here the
            // persisted bindings are validated and the census loaded
            // verbatim, and reap reports missing nodes as exited.
            let engine =
                RunEngine::attach_for_cleanup(&run_dir, &format!("reap-{}", std::process::id()))?;
            let results = engine.reap()?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        Commands::Snapshot { run_dir, cursor } => {
            let engine =
                RunEngine::attach_read_only(&run_dir, &format!("snapshot-{}", std::process::id()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&engine.snapshot(cursor))?
            );
        }
        Commands::Events { run_dir, after } => {
            let engine =
                RunEngine::attach_read_only(&run_dir, &format!("events-{}", std::process::id()))?;
            let (subscription, _) = engine
                .evidence()
                .subscribe(after)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            for event in subscription.replay {
                println!("{}", serde_json::to_string(&*event)?);
            }
            // This CLI attaches read-only and owns no producer; replay the
            // retained tail and exit instead of blocking forever on a finished
            // (or kept) run.
        }
        Commands::Command { run_dir, json } => {
            let engine = RunEngine::attach(&run_dir, &format!("command-{}", std::process::id()))?;
            let mut value: serde_json::Value =
                serde_json::from_str(&json).context("command JSON is not valid JSON")?;
            let object = value
                .as_object_mut()
                .context("command JSON must be an object")?;
            object
                .entry("command_id")
                .or_insert_with(|| serde_json::Value::String(CommandId::generate().to_string()));
            object
                .entry("run_id")
                .or_insert_with(|| serde_json::Value::String(engine.run_id().to_string()));
            object
                .entry("issued_at_wall")
                .or_insert_with(|| serde_json::Value::Number(now_ms().into()));
            object
                .entry("actor")
                .or_insert_with(|| serde_json::Value::String(ActorId::trusted("cli").to_string()));
            let command: LabCommand = serde_json::from_value(value)
                .context("command does not match the typed command envelope")?;
            println!("{}", serde_json::to_string_pretty(&engine.submit(command))?);
        }
        Commands::ActorExec {
            parent_pid,
            parent_start_ticks,
            argv,
        } => {
            spc_lab::namespace::actor_exec(parent_pid, parent_start_ticks, &argv)
                .map_err(anyhow::Error::msg)?;
        }
        Commands::Runtime {
            txd,
            traffic_probe,
            traffic_probe_arg,
            scenario,
            run_dir,
            node,
            pool,
            miner,
            scanner,
            captaind,
            wallet_agent_bin,
            grpcurl,
            postgres_bin_dir,
            redis_server,
            redis_cli,
            pow_bits,
            base_revision,
            dirty_tree_hash,
            listen,
            duration_secs,
            interactive,
            topology,
            operator_ui,
            broker_script,
            unshare,
            nsenter,
        } => {
            let binaries = spc_lab::runtime::RuntimeBinaries {
                txd,
                traffic_probe,
                traffic_probe_args: traffic_probe_arg,
                node,
                pool,
                miner,
                scanner,
                postgres_bin_dir,
                redis_server,
                redis_cli,
            };
            let topology_options = if topology {
                Some(spc_lab::runtime::TopologyRuntimeOptions {
                    broker_script: broker_script
                        .unwrap_or_else(|| PathBuf::from("scripts/lab/lib/lab_backend.py")),
                    unshare: unshare
                        .or_else(|| spc_lab::namespace::resolve_tool("unshare"))
                        .context("--topology requires `unshare` on PATH or --unshare")?,
                    nsenter: nsenter
                        .or_else(|| spc_lab::namespace::resolve_tool("nsenter"))
                        .context("--topology requires `nsenter` on PATH or --nsenter")?,
                })
            } else {
                None
            };
            let runtime = spc_lab::runtime::Runtime::prepare(spc_lab::runtime::RuntimeOptions {
                wallet_agent: wallet_agent_bin,
                ark: captaind.zip(grpcurl).map(|(captaind, grpcurl)| {
                    spc_lab::ark_service::ArkBinaries { captaind, grpcurl }
                }),
                scenario_path: scenario,
                run_dir,
                binaries,
                pow_bits,
                base_revision,
                dirty_tree_hash,
                evidence_capacity: 200_000,
                topology: topology_options,
                operator_ui,
            })?;
            runtime.run_with_assets(
                listen,
                duration_secs.map(Duration::from_secs),
                interactive,
                WebAssets {
                    index_html: INDEX_HTML,
                    app_js: APP_JS,
                    style_css: STYLE_CSS,
                    lab_html: LAB_HTML,
                    lab_js: LAB_JS,
                    lab_css: LAB_CSS,
                },
            )?;
        }
        Commands::Rpc {
            typed,
            request_id,
            url,
            method,
            params,
            user,
            pass,
            auth_file,
        } => {
            let parsed =
                spc_lab::namespace::parse_rpc_params(&params).map_err(anyhow::Error::msg)?;
            let body = serde_json::json!({
                "jsonrpc": "1.0",
                "id": request_id,
                "method": method,
                "params": parsed,
            });
            let (user, pass) = match auth_file {
                Some(path) => {
                    let (user, pass) =
                        spc_lab::namespace::basic_auth_file(&path).map_err(anyhow::Error::msg)?;
                    (Some(user), Some(pass))
                }
                None => (user, pass),
            };
            if typed {
                let address = url
                    .strip_prefix("http://")
                    .and_then(|s| s.strip_suffix('/'))
                    .context("typed RPC requires an explicit http://host:port/ endpoint")?;
                let address: std::net::SocketAddr = address.parse()?;
                let control = spc_lab::adapters::RpcNodeControl::new(
                    spc_lab::ids::NodeId::new("namespace-helper")?,
                    spc_lab::resource::Endpoint {
                        host: address.ip().to_string(),
                        port: address.port(),
                    },
                    user.map(|u| format!("basic:{u}:{}", pass.unwrap_or_default())),
                );
                println!(
                    "{}",
                    serde_json::to_string(&control.call_traffic(
                        request_id,
                        &method,
                        body["params"].clone()
                    ))?
                );
                return Ok(());
            }
            let response = spc_lab::namespace::http_request(
                &url,
                "POST",
                Some(&body.to_string()),
                user.as_deref(),
                pass.as_deref(),
            )
            .map_err(anyhow::Error::msg)?;
            print!("{response}");
        }
        Commands::Http {
            url,
            method,
            body,
            user,
            pass,
        } => {
            let response = spc_lab::namespace::http_request(
                &url,
                &method,
                if body.is_empty() {
                    None
                } else {
                    Some(body.as_str())
                },
                user.as_deref(),
                pass.as_deref(),
            )
            .map_err(anyhow::Error::msg)?;
            print!("{response}");
        }
        Commands::ProbeTcp {
            host,
            port,
            timeout_ms,
            pid,
        } => {
            let ok = spc_lab::namespace::probe_tcp(&host, port, timeout_ms)
                .map_err(anyhow::Error::msg)?;
            let owned = match pid {
                Some(pid) => spc_lab::namespace::listener_owned_by(pid, port),
                None => true,
            };
            if !ok || !owned {
                std::process::exit(1);
            }
        }
        Commands::Manifest { run_dir } => {
            let engine =
                RunEngine::attach_read_only(&run_dir, &format!("manifest-{}", std::process::id()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&engine.manifest_snapshot())?
            );
        }
    }
    Ok(())
}

fn read_scenario(path: &Path) -> Result<Scenario> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading scenario {}", path.display()))?;
    Scenario::from_json(&text).map_err(|error| anyhow::anyhow!("{error}"))
}

fn read_config(path: &Path) -> Result<LabConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading adapter config {}", path.display()))?;
    LabConfig::from_json(&text).map_err(|error| anyhow::anyhow!("{error}"))
}

fn execute_phases(engine: &RunEngine) -> Result<()> {
    for phase in &engine.scenario.phases {
        for command in &phase.commands {
            let mut value = command.clone();
            let object = value
                .as_object_mut()
                .context("phase command must be an object")?;
            object
                .entry("command_id")
                .or_insert_with(|| serde_json::Value::String(CommandId::generate().to_string()));
            object
                .entry("run_id")
                .or_insert_with(|| serde_json::Value::String(engine.run_id().to_string()));
            object
                .entry("issued_at_wall")
                .or_insert_with(|| serde_json::Value::Number(now_ms().into()));
            object.entry("actor").or_insert_with(|| {
                serde_json::Value::String(ActorId::trusted("phase").to_string())
            });
            let command: LabCommand = serde_json::from_value(value)
                .context("phase command does not match the typed envelope")?;
            let outcome = engine.submit(command);
            println!(
                "phase {} command {:?}",
                phase.id,
                serde_json::to_value(outcome)?
            );
        }
    }
    Ok(())
}

fn wait_with_assertions(engine: &RunEngine, duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        if engine
            .assertions()
            .iter()
            .all(|(_, evaluation)| matches!(evaluation, AssertionEvaluation::Passed))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn print_result(engine: &RunEngine, assertions: &[(String, AssertionEvaluation)]) {
    let artifacts = ArtifactPaths::under(&engine.run_dir);
    let value = serde_json::json!({
        "run_id": engine.run_id(),
        "run_dir": engine.run_dir,
        "state": engine.manifest_snapshot().state,
        "assertions": assertions,
        "shareable_manifest": artifacts.shareable_manifest,
        "private_evidence": artifacts.private_evidence,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
}

fn env_or_git(variable: &str, args: &[&str]) -> String {
    if let Ok(value) = std::env::var(variable) {
        return value;
    }
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".to_string())
}
