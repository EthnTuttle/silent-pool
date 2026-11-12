//! Scripted process fixture for LP-3 tests.
//!
//! This is deliberately not a miner, node or pool. It is a real child process
//! that binds a loopback port or speaks the runner's JSONL control envelope so
//! supervision, adoption and HTTP/SSE can be tested without pretending that
//! simulated hashes are live proof-of-work.

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::path::PathBuf;

fn main() {
    let _ = rustix::process::umask(rustix::fs::Mode::from_bits_truncate(0o077));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "hold".to_string());
    match mode.as_str() {
        "hold" | "redis" | "postgres" | "node" | "pool" | "front" | "ark" => hold(&args),
        "miner-jsonl" => miner_jsonl(&args),
        other => {
            eprintln!("unknown fixture mode {other:?}");
            std::process::exit(2);
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|value| value == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn hold(args: &[String]) {
    let port = arg(args, "--port").and_then(|value| value.parse::<u16>().ok());
    let height_file = arg(args, "--height-file").map(PathBuf::from);
    if let Some(path) = &height_file {
        let _ = std::fs::write(path, b"0");
    }
    let listener = port.map(|port| {
        TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|error| {
            eprintln!("fixture could not bind 127.0.0.1:{port}: {error}");
            std::process::exit(3);
        })
    });
    if let Some(ready) = arg(args, "--ready-file") {
        if let Some(parent) = PathBuf::from(&ready).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(ready, b"ready\n");
    }
    // Keep the listener variable alive for the process lifetime.
    let _listener = listener;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn miner_jsonl(args: &[String]) {
    let events = arg(args, "--events").map(PathBuf::from);
    let miner_id = arg(args, "--miner-id").unwrap_or_else(|| "fixture-miner".to_string());
    // Additive test hook for the EXP-ACCESS worker projection: when one or
    // more `--snapshot-worker <id>` arguments are present, the v1 snapshot
    // answer carries real worker rows (requested/achieved/active_job_id) so
    // the miner-poll projection can be exercised against fixtures.
    let snapshot_workers: Vec<String> = args
        .windows(2)
        .filter(|window| window[0] == "--snapshot-worker")
        .map(|window| window[1].clone())
        .collect();
    if let Some(ready) = arg(args, "--ready-file") {
        if let Some(parent) = PathBuf::from(&ready).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(ready, b"ready\n");
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut event_id = 0u64;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(path) = &events {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({
                        "protocol": "spc-cpu-miner-control",
                        "version": 1,
                        "kind": "command_result",
                        "command_id": "unknown",
                        "outcome": "failed",
                        "error": {"code": "invalid_json", "message": error.to_string()},
                    })
                );
                let _ = stdout.flush();
                continue;
            }
        };
        let command_id = value
            .get("command_id")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        if value.get("protocol").and_then(|value| value.as_str()) != Some("spc-cpu-miner-control")
            || value.get("version").and_then(|value| value.as_u64()) != Some(1)
        {
            let _ = writeln!(
                stdout,
                "{}",
                serde_json::json!({
                    "protocol": "spc-cpu-miner-control",
                    "version": 1,
                    "kind": "command_result",
                    "command_id": command_id,
                    "outcome": "rejected_precondition",
                    "error": {"code": "invalid_command", "message": "protocol/version mismatch"},
                })
            );
            let _ = stdout.flush();
            continue;
        }
        let operation = value
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let result = if operation == "snapshot" {
            let workers: Vec<serde_json::Value> = snapshot_workers
                .iter()
                .map(|worker| {
                    serde_json::json!({
                        "id": worker,
                        "connection_id": "conn-fixture",
                        "paused": false,
                        "requested": {
                            "hashes_per_second": 100000,
                            "total_attempts": 1000000
                        },
                        "attempted": "0",
                        "elapsed_ms": 5,
                        "achieved_hps": 4242,
                        "budget_hps": null,
                        "saturated": false,
                        "queue_depth": 0,
                        "active_job_id": "job-fixture-1",
                        "stale_cancelled": 0,
                    })
                })
                .collect();
            serde_json::json!({
                "snapshot_version": 1,
                "miner_id": miner_id,
                "occurred_at_mono_ms": 0,
                "events_dropped_total": 0,
                "stopping": false,
                "connections": [],
                "workers": workers,
            })
        } else {
            serde_json::json!({})
        };
        let _ = writeln!(
            stdout,
            "{}",
            serde_json::json!({
                "protocol": "spc-cpu-miner-control",
                "version": 1,
                "kind": "command_result",
                "command_id": command_id,
                "outcome": "succeeded",
                "result": result,
            })
        );
        let _ = stdout.flush();
        if operation == "add_connection" {
            let dropped = arg(args, "--dropped-before")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            for _ in 0..2 {
                event_id += 1;
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({
                        "protocol": "spc-cpu-miner-events",
                        "version": 1,
                        "kind": "event",
                        "source": "process",
                        "event_id": event_id,
                        "source_cursor": event_id,
                        "miner_id": miner_id,
                        "occurred_at_wall_ms": 1,
                        "occurred_at_mono_ms": event_id,
                        "dropped_before": dropped,
                        "event": {
                            "name": "connection_state",
                            "payload": {
                                "connection_id": "fixture-connection",
                                "session_slot": null,
                                "state": "connecting",
                                "detail": "fixture control-only",
                            }
                        },
                    })
                );
            }
            if args.iter().any(|value| value == "--emit-invalid") {
                let _ = writeln!(stdout, "not-json-fixture-line");
            }
            let _ = stdout.flush();
        }
        if operation == "stop" {
            break;
        }
    }
    // EOF is a normal driver stop: the real spc-cpu-miner closes on control
    // EOF. The runner therefore treats a pipe-backed adopted miner as stale.
}
