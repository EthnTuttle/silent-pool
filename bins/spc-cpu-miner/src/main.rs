use std::path::PathBuf;
use std::time::Instant;

use anyhow::{anyhow, Context};
use clap::Parser;
use spc_cpu_miner::config::MinerConfig;
use spc_cpu_miner::control::{
    Command, CommandResult, Operation, CONTROL_PROTOCOL, CONTROL_VERSION,
};
use spc_cpu_miner::miner::Miner;
use spc_cpu_miner::report::{write_output, EventSink, OutputMessage};
use tokio::io::{stdin, stdout, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

#[derive(Debug, Parser)]
#[command(
    name = "spc-cpu-miner",
    about = "native SV2 extended-channel CPU miner"
)]
struct Args {
    /// Strict TOML configuration (config_version = 1).
    #[arg(long)]
    config: PathBuf,
    /// Validate configuration and exit without starting network or workers.
    #[arg(long)]
    check_config: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = MinerConfig::load(&args.config).map_err(|error| anyhow!(error.to_string()))?;
    if args.check_config {
        return Ok(());
    }

    let (output_tx, output_rx) = mpsc::channel(config.event_queue_capacity);
    let writer = tokio::spawn(write_output(output_rx, stdout()));
    let events = EventSink::new(output_tx.clone(), config.miner_id.clone(), Instant::now());
    let mut miner = Miner::new(config.miner_id.clone(), events);

    for connection in config.connections {
        let command = Command {
            protocol: CONTROL_PROTOCOL.into(),
            version: CONTROL_VERSION,
            command_id: format!("config:{}", connection.id).into(),
            operation: Operation::AddConnection { connection },
        };
        let result = miner.execute(command).await;
        if result.outcome != "running" {
            return Err(anyhow!(
                "initial connection failed: {}",
                result
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "unknown startup error".into())
            ));
        }
    }

    let mut lines = BufReader::new(stdin()).lines();
    loop {
        if miner.is_stopping() {
            break;
        }
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line.context("read control stdin")? else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let (result, stop) = match serde_json::from_str::<Command>(&line) {
                    Ok(command) => {
                        let stop = matches!(command.operation, Operation::Stop {});
                        let result = miner.execute(command).await;
                        (result, stop)
                    }
                    Err(error) => {
                        let command_id = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|value| value.get("command_id")?.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        (
                            CommandResult::rejected(command_id, "invalid_json", error.to_string()),
                            false,
                        )
                    }
                };
                output_tx.send(OutputMessage::Command(result)).await.map_err(|_| anyhow!("control output closed"))?;
                if stop { break; }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("install ctrl-c handler")?;
                break;
            }
        }
    }

    miner.stop_connections().await;
    drop(miner);
    drop(output_tx);
    let _ = writer.await;
    Ok(())
}
