//! A queue client only. Never opens a seed, stock database, or wallet.
use std::path::PathBuf;

use clap::Parser;
use spc_pool::ark::wallet_control;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    request: PathBuf,
}

fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = spc_pool::config::Config::load(&args.config)?;
    let ark = config
        .ark
        .as_ref()
        .filter(|a| a.enabled && a.transport == "bark")
        .ok_or_else(|| anyhow::anyhow!("stock Ark wallet configuration required"))?;
    let request = wallet_control::read_request(&args.request)?;
    let result = wallet_control::submit(&ark.datadir, &request)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn main() {
    if run().is_err() {
        // Never print parser errors containing private config/request contents.
        eprintln!("pool-wallet-executor: configuration, request, or private queue refused");
        std::process::exit(1);
    }
}
