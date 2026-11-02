use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Role {
    Front,
    Payout,
    Api,
    Ark,
}

#[derive(Parser)]
#[command(name = "spc-pool", version = spc_crypto::VERSION)]
struct Cli {
    /// TOML configuration file.
    #[arg(short, long, env = "SPC_POOL_CONFIG")]
    config: Option<PathBuf>,
    /// Roles to run in this process (default: front,payout,api).
    #[arg(long, value_delimiter = ',', default_value = "front,payout,api")]
    roles: Vec<Role>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,spc_pool=debug".into()),
        )
        .init();
    tracing::info!(
        version = spc_crypto::VERSION,
        roles = ?cli.roles,
        config = ?cli.config,
        "spc-pool starting (WP0 skeleton)"
    );
    Ok(())
}
