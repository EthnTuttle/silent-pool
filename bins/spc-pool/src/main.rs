use std::path::PathBuf;

use clap::Parser;
use spc_pool::config::Config;

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

fn main() -> anyhow::Result<()> {
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
            .map(|r| {
                spc_pool::config::Role::parse(r)
                    .ok_or_else(|| anyhow::anyhow!("unknown role '{r}'"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };

    tracing::info!(
        version = spc_pool::VERSION,
        network = %config.pool.network,
        front_id = config.pool.front_id,
        roles = ?roles,
        "spc-pool starting (WP5 skeleton: config validated, listeners next)"
    );
    Ok(())
}
