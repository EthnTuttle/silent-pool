mod common;

use std::time::Duration;

use spc_lab::adapters::{NodeControl, RpcNodeControl};
use spc_lab::ids::NodeId;
use spc_lab::resource::Endpoint;
use spc_lab::supervisor::{ProcessSpec, Readiness, Supervisor};

/// Explicitly ignored real-process proof. It does not mine, submit a block, or
/// claim an SV2/payout vertical slice. It proves the runner can launch a real
/// rbitcoin node, read its effective profile over authenticated RPC and stop
/// the process group safely.
#[test]
#[ignore = "requires SPC_LAB_RBITCOIN_NODE=/path/to/rbitcoin-node"]
fn real_rbitcoin_baseline_profile_and_shutdown() {
    let program = std::env::var("SPC_LAB_RBITCOIN_NODE")
        .expect("set SPC_LAB_RBITCOIN_NODE to the rbitcoin-node binary");
    eprintln!(
        "proof program={} sha256={}",
        program,
        spc_lab::supervisor::hash_file(std::path::Path::new(&program)).unwrap_or_default()
    );
    let dir = common::TestDir::new("real-rbitcoin");
    let datadir = dir.join("datadir");
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut supervisor = Supervisor::new("real-node-proof");
    let mut spec = ProcessSpec::new("node", "node-a", &program, dir.join("rbitcoin.log"));
    spec.args = vec![
        "--network=regtest".to_string(),
        format!("--datadir={}", datadir.display()),
        "--no-listen".to_string(),
        "--rpc".to_string(),
        format!("--rpc-listen=127.0.0.1:{port}"),
        "--log-level=error".to_string(),
    ];
    spec.readiness = Readiness::Tcp {
        host: "127.0.0.1".to_string(),
        port,
    };
    spec.ready_timeout_ms = 20_000;
    let record = supervisor.launch(spec).unwrap();
    let control = RpcNodeControl::new(
        NodeId::new("node-a").unwrap(),
        Endpoint::loopback(port),
        Some(format!(
            "bearer-file:{}",
            datadir.join("rpc.token").display()
        )),
    );
    assert_eq!(control.height().unwrap(), 0);
    let profile = control.effective_profile().unwrap();
    assert_eq!(profile.network.as_deref(), Some("regtest"));
    assert_eq!(profile.pow_limit_bits.as_deref(), Some("207fffff"));
    assert_eq!(
        profile.no_pow_retargeting,
        Some(true),
        "the default regtest baseline is explicitly not the retarget profile"
    );
    supervisor
        .stop("node", "node-a", Duration::from_secs(5))
        .unwrap();
    assert!(!spc_lab::supervisor::pid_is_alive(record.pid));
}
