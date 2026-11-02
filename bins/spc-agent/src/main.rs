use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use spc_agent::config::AgentConfig;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Role {
    Translator,
    Jdc,
    Wallet,
}

#[derive(Parser)]
#[command(name = "spc-agent", version = spc_agent::VERSION)]
struct Cli {
    #[arg(short, long, env = "SPC_AGENT_CONFIG")]
    config: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "translator")]
    roles: Vec<Role>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,spc_agent=debug".into()),
        )
        .init();

    let config = AgentConfig::load(&cli.config)?;
    let policy = config.own_payout_policy().map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!(
        version = spc_agent::VERSION,
        upstream = %config.upstream.address,
        require_own_payout = ?policy,
        roles = ?cli.roles,
        "spc-agent starting (config validated; roles wired in later packets)"
    );
    Ok(())
}
