use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Role {
    Translator,
    Jdc,
    Wallet,
}

#[derive(Parser)]
#[command(name = "spc-agent", version = spc_crypto::VERSION)]
struct Cli {
    #[arg(short, long, env = "SPC_AGENT_CONFIG")]
    config: Option<PathBuf>,
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
    tracing::info!(
        version = spc_crypto::VERSION,
        roles = ?cli.roles,
        config = ?cli.config,
        "spc-agent starting (WP0 skeleton)"
    );
    Ok(())
}
