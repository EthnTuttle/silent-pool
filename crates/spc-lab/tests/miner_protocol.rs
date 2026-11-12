mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use spc_lab::adapters::{MinerControl, MinerOperation};
use spc_lab::ids::MinerId;
use spc_lab::supervisor::{ProcessSpec, Readiness, Supervisor};

#[test]
fn runner_speaks_the_published_control_envelope_and_correlates_events() {
    let dir = common::TestDir::new("miner-protocol");
    let mut supervisor = Supervisor::new("miner-protocol-token");
    let mut spec = ProcessSpec::new(
        "miner",
        "miner-fixture",
        common::fixture_bin(),
        dir.join("miner.log"),
    );
    spec.args = vec![
        "miner-jsonl".to_string(),
        "--miner-id".to_string(),
        "miner-fixture".to_string(),
        "--events".to_string(),
        dir.join("control.jsonl").display().to_string(),
    ];
    spec.pipe_stdin = true;
    spec.pipe_stdout = true;
    spec.control = spc_lab::supervisor::ControlTransport::Pipe;
    spec.readiness = Readiness::Immediate;
    spec.ready_timeout_ms = 3_000;
    spec.env = BTreeMap::new();
    let launched = supervisor.launch_with_pipes(spec).unwrap();
    let control = MinerControl::spawn(
        MinerId::new("miner-fixture").unwrap(),
        "incarnation-1".to_string(),
        launched.stdin.unwrap(),
        launched.stdout.unwrap(),
        dir.join("miner.log"),
    )
    .unwrap();

    let response = control
        .request(MinerOperation::Snapshot, serde_json::json!({}))
        .unwrap();
    assert_eq!(response.protocol, "spc-cpu-miner-control");
    assert_eq!(response.version, 1);
    assert_eq!(response.kind, "command_result");
    assert!(response.is_success());
    assert_eq!(response.result.unwrap()["snapshot_version"], 1);

    let add = serde_json::json!({
        "connection": {
            "id": "conn-alice",
            "endpoint": "127.0.0.1:33445",
            "identity": "tsp1q-fixture",
            "authority_public_key": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "min_extranonce_size": 4,
            "reconnect": {
                "enabled": true,
                "initial_backoff_ms": 100,
                "max_backoff_ms": 5000,
                "handshake_timeout_ms": 10000
            },
            "workers": []
        }
    });
    control.request(MinerOperation::AddConnection, add).unwrap();
    let receiver = control.event_receiver();
    let event = {
        let receiver = receiver.lock().unwrap();
        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("published event")
    };
    assert_eq!(event.protocol, "spc-cpu-miner-events");
    assert_eq!(event.version, 1);
    assert_eq!(event.kind, "event");
    assert_eq!(event.event.name, "connection_state");
    assert_eq!(event.miner_id, MinerId::new("miner-fixture").unwrap());

    control.stop().unwrap();
    let _ = supervisor.stop("miner", "miner-fixture", Duration::from_secs(2));
    let control_log = std::fs::read_to_string(dir.join("control.jsonl")).unwrap();
    assert!(control_log.contains("\"protocol\":\"spc-cpu-miner-control\""));
    assert!(control_log.contains("\"operation\":\"add_connection\""));
    assert!(control_log.contains("\"operation\":\"snapshot\""));
}
