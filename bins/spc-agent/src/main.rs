use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use spc_agent::config::AgentConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Role {
    Translator,
    Jdc,
    Wallet,
}

#[derive(Parser)]
#[command(name = "spc-agent", version = spc_agent::VERSION)]
struct Cli {
    #[arg(
        short,
        long,
        env = "SPC_AGENT_CONFIG",
        required_unless_present = "provision_wallet"
    )]
    config: Option<PathBuf>,
    /// One-shot cold regtest provisioning in a separate process.
    #[arg(long, requires = "cold_store", conflicts_with = "config")]
    provision_wallet: Option<PathBuf>,
    #[arg(long, requires = "provision_wallet")]
    cold_store: Option<PathBuf>,
    /// Submit an owner-scoped command JSON to the running wallet's private queue.
    #[arg(long, requires = "config")]
    wallet_command: Option<PathBuf>,
    #[arg(long, value_delimiter = ',', default_value = "translator")]
    roles: Vec<Role>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(wallet) = cli.provision_wallet {
        #[cfg(feature = "ark")]
        {
            let record = spc_agent::wallet::provision::provision(
                &wallet,
                cli.cold_store.as_deref().expect("clap requires cold-store"),
            )?;
            println!("{}", serde_json::to_string(&record)?);
            return Ok(());
        }
        #[cfg(not(feature = "ark"))]
        {
            let _ = wallet;
            anyhow::bail!("wallet provisioning requires the ark feature");
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,spc_agent=debug".into()),
        )
        .init();

    let config = AgentConfig::load(cli.config.as_deref().expect("clap requires config"))?;
    if let Some(path) = cli.wallet_command {
        #[cfg(feature = "ark")]
        {
            let command: spc_agent::wallet::maintenance::WalletCommand =
                serde_json::from_slice(&std::fs::read(path)?)?;
            let datadir = config
                .ark
                .as_ref()
                .and_then(|a| a.datadir.as_deref())
                .ok_or_else(|| anyhow::anyhow!("wallet datadir required"))?;
            spc_agent::wallet::maintenance::submit_command(datadir, &command)?;
            println!("{{\"state\":\"queued\"}}");
            return Ok(());
        }
        #[cfg(not(feature = "ark"))]
        {
            let _ = path;
            anyhow::bail!("wallet commands require ark feature");
        }
    }
    let policy = config.own_payout_policy().map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!(
        version = spc_agent::VERSION,
        upstream = %config.upstream.address,
        require_own_payout = ?policy,
        roles = ?cli.roles,
        "spc-agent starting"
    );
    if cli.roles.contains(&Role::Wallet) {
        // The wallet role holds spendable VTXO secrets: it never co-runs
        // with the mining-side roles in one process (fail closed, SPEC
        // 11.5 isolation).
        let names: Vec<&str> = cli
            .roles
            .iter()
            .map(|r| match r {
                Role::Translator => "translator",
                Role::Jdc => "jdc",
                Role::Wallet => "wallet",
            })
            .collect();
        spc_agent::validate_role_selection(&names).map_err(|e| anyhow::anyhow!(e))?;
        #[cfg(feature = "ark")]
        {
            return spc_agent::wallet::run_from_config(config).await;
        }
        #[cfg(not(feature = "ark"))]
        {
            let _ = config;
            anyhow::bail!("this build has no wallet role; rebuild spc-agent with --features ark");
        }
    }
    if cli.roles.contains(&Role::Translator) {
        if cli.roles.contains(&Role::Jdc) {
            let jdc_config = config.clone();
            return tokio::select! {
                result = spc_agent::jdc::run(jdc_config) => result,
                result = spc_agent::translator::run(config) => result,
            };
        }
        return spc_agent::translator::run(config).await;
    }
    if cli.roles.contains(&Role::Jdc) {
        return spc_agent::jdc::run(config).await;
    }
    anyhow::bail!("no runnable role selected; use --roles translator, jdc or wallet")
}
