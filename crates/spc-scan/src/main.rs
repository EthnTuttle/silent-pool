use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};
use spc_scan::{
    export_tweaks, parse_scan_key, scan_block, scan_light, BlockSource, FileSource, LightProof,
    TweakFormat,
};

#[derive(Parser)]
#[command(name = "spc-scan", version = spc_scan::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find this miner's SPc outputs in a block range.
    Find {
        #[arg(long)]
        address: String,
        #[arg(long)]
        scan_key: String,
        #[arg(long)]
        from: u32,
        #[arg(long)]
        to: Option<u32>,
        #[arg(long)]
        rpc: Option<String>,
        #[arg(long)]
        rpc_user: Option<String>,
        #[arg(long)]
        rpc_pass: Option<String>,
        #[arg(long)]
        blocks: Option<PathBuf>,
    },
    /// Scan exported header||branch||coinbase proofs.
    Light {
        #[arg(long)]
        address: String,
        #[arg(long)]
        scan_key: String,
        #[arg(long)]
        proofs: PathBuf,
    },
    /// Export (outpoint, tweak) pairs for a wallet import.
    ExportTweaks {
        #[arg(long)]
        address: String,
        #[arg(long)]
        scan_key: String,
        #[arg(long)]
        txid: String,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long)]
        blocks: PathBuf,
    },
    /// Generate a fresh scan/spend keypair and its SP address (dev/test).
    GenAddress {
        #[arg(long, default_value = "regtest")]
        network: String,
    },
    /// Run the embedded vector checks.
    Vectors,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GenAddress { network } => {
            let net = match network.as_str() {
                "mainnet" => spc_crypto::Network::Mainnet,
                "testnet4" | "testnet" => spc_crypto::Network::Testnet,
                "signet" => spc_crypto::Network::Signet,
                _ => spc_crypto::Network::Regtest,
            };
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let scan = bitcoin::secp256k1::SecretKey::new(&mut rand::thread_rng());
            let spend = bitcoin::secp256k1::SecretKey::new(&mut rand::thread_rng());
            let b_scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan);
            let b_spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend);
            let address = spc_crypto::address::encode_sp_address(net, &b_scan, &b_spend)?;
            println!(
                "{}",
                serde_json::json!({
                    "address": address,
                    "b_scan": spc_scan::hex(&b_scan.serialize()),
                    "b_spend": spc_scan::hex(&b_spend.serialize()),
                    "scan_key": spc_scan::hex(&scan.secret_bytes()),
                    "spend_key": spc_scan::hex(&spend.secret_bytes()),
                })
            );
        }
        Command::Vectors => {
            // The crate's own test suite is the authoritative vector run; this
            // command exists so an operator can smoke-test an installed binary.
            println!(
                "spc-scan {}: run `cargo test -p spc-scan` for the full vectors",
                spc_scan::VERSION
            );
        }
        Command::Find {
            address,
            scan_key,
            from,
            to,
            rpc,
            rpc_user,
            rpc_pass,
            blocks,
        } => {
            let (b_spend, kind) = parse_address(&address)?;
            let b_scan = parse_scan_key(&scan_key).map_err(|e| anyhow::anyhow!(e))?;
            let source: Box<dyn BlockSource> = match (blocks, rpc) {
                (Some(dir), _) => Box::new(FileSource::open(&dir).map_err(|e| anyhow::anyhow!(e))?),
                (None, Some(url)) => {
                    #[cfg(feature = "rpc")]
                    {
                        let auth = match (rpc_user, rpc_pass) {
                            (Some(u), Some(p)) => bitcoincore_rpc::Auth::UserPass(u, p),
                            _ => bitcoincore_rpc::Auth::None,
                        };
                        Box::new(
                            spc_scan::RpcSource::new(&url, auth).map_err(|e| anyhow::anyhow!(e))?,
                        )
                    }
                    #[cfg(not(feature = "rpc"))]
                    {
                        let _ = (url, rpc_user, rpc_pass);
                        anyhow::bail!("this spc-scan was built without the `rpc` feature");
                    }
                }
                (None, None) => anyhow::bail!("one of --blocks or --rpc is required"),
            };
            let tip = source.tip_height().map_err(|e| anyhow::anyhow!(e))?;
            let to = to.unwrap_or(tip);
            let mut total = 0usize;
            for height in from..=to {
                let block = source.block_at(height).map_err(|e| anyhow::anyhow!(e))?;
                let outcome = scan_block(&block, &b_scan, &b_spend)?;
                if outcome.is_spc {
                    println!(
                        "{}",
                        serde_json::json!({
                            "height": height,
                            "address_kind": kind,
                            "a_send": outcome.a_x.map(|x| spc_scan::hex(&x)),
                            "found": outcome.found,
                        })
                    );
                    total += outcome.found.len();
                }
            }
            eprintln!("spc-scan: {total} output(s) found");
        }
        Command::Light {
            address,
            scan_key,
            proofs,
        } => {
            let (b_spend, _) = parse_address(&address)?;
            let b_scan = parse_scan_key(&scan_key).map_err(|e| anyhow::anyhow!(e))?;
            let bytes = std::fs::read(&proofs)?;
            let mut off = 0usize;
            let mut total = 0usize;
            while off + 4 <= bytes.len() {
                let len = u32::from_le_bytes([
                    bytes[off],
                    bytes[off + 1],
                    bytes[off + 2],
                    bytes[off + 3],
                ]) as usize;
                off += 4;
                let end = off
                    .checked_add(len)
                    .ok_or_else(|| anyhow::anyhow!("bad proof length"))?;
                let proof = LightProof::decode(
                    bytes
                        .get(off..end)
                        .ok_or_else(|| anyhow::anyhow!("truncated proofs"))?,
                )?;
                off = end;
                let outcome = scan_light(&proof, &b_scan, &b_spend)?;
                if outcome.is_spc {
                    println!("{}", serde_json::json!({ "found": outcome.found }));
                    total += outcome.found.len();
                }
            }
            eprintln!("spc-scan: {total} output(s) found");
        }
        Command::ExportTweaks {
            address,
            scan_key,
            txid,
            format,
            blocks,
        } => {
            let (b_spend, _) = parse_address(&address)?;
            let b_scan = parse_scan_key(&scan_key).map_err(|e| anyhow::anyhow!(e))?;
            let source = FileSource::open(&blocks).map_err(|e| anyhow::anyhow!(e))?;
            let mut found = Vec::new();
            for height in 0..=source.tip_height().map_err(|e| anyhow::anyhow!(e))? {
                let block = source.block_at(height).map_err(|e| anyhow::anyhow!(e))?;
                found.extend(scan_block(&block, &b_scan, &b_spend)?.found);
            }
            let txid = bitcoin::Txid::from_str(&txid).map_err(|e| anyhow::anyhow!(e))?;
            let fmt = match format.as_str() {
                "sparrow" => TweakFormat::Sparrow,
                _ => TweakFormat::Json,
            };
            println!("{}", export_tweaks(&found, &txid, fmt));
        }
    }
    Ok(())
}

fn parse_address(addr: &str) -> anyhow::Result<(bitcoin::secp256k1::PublicKey, &'static str)> {
    let network = if addr.starts_with("sp1") {
        spc_crypto::Network::Mainnet
    } else if addr.starts_with("tsp1") {
        spc_crypto::Network::Testnet
    } else {
        anyhow::bail!("address must start with sp1 or tsp1");
    };
    let parsed = spc_crypto::parse_sp_address(addr, network).map_err(|e| anyhow::anyhow!(e))?;
    Ok((
        parsed.b_spend,
        if network == spc_crypto::Network::Mainnet {
            "mainnet"
        } else {
            "test"
        },
    ))
}
