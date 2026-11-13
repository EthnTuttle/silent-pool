use super::*;

/// Selected only by the source-bound gate as a real supervised subprocess.
/// `-- --config PATH` leaves the actual config pair visible in kernel argv.
/// No test-only startup implementation: load + NativeRuntime::start are real.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "supervised subprocess helper for native_run_token_gate"]
async fn native_bootstrap_probe() {
    let args: Vec<_> = std::env::args_os().collect();
    let paths: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "--config")
        .collect();
    assert_eq!(paths.len(), 1);
    let config_path = PathBuf::from(&paths[0][1]);
    let config = crate::config::AgentConfig::load(&config_path).unwrap();
    let runtime = NativeRuntime::start(&config)
        .await
        .unwrap()
        .expect("audit configured");
    let instance = wire::decode_instance(
        &io::private_read(
            &runtime.context.root.join(wire::INSTANCE_FILENAME),
            io::uid().unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(instance, runtime.context.instance);
    assert_eq!(
        instance.process.run_token_sha256,
        config
            .ark
            .as_ref()
            .unwrap()
            .native_audit
            .as_ref()
            .unwrap()
            .expected_run_token()
            .unwrap()
    );
    runtime.context.endpoint().unwrap();
    private_write(
        &runtime.context.root.join("native-bootstrap-probe.json"),
        &serde_json::to_vec(&instance).unwrap(),
    );
    // Parent owns this child and releases it after validating publication.
    let mut release = [0];
    std::io::Read::read_exact(&mut std::io::stdin(), &mut release).unwrap();
    runtime.stop().await.unwrap();
}

#[test]
fn opaque_marker_requires_exact_expected_digest_and_unique_bounded_environment() {
    for token in [
        "run:literal-owner".to_string(),
        hex::encode(io::hash(b"run:traffic-owner")),
    ] {
        let expected = io::hash(token.as_bytes());
        let env = zeroize::Zeroizing::new(format!("A=B\0SPC_LAB_RUN_TOKEN={token}\0").into_bytes());
        assert_eq!(io::observed_run_token(&env, &expected).unwrap(), expected);
        assert!(io::observed_run_token(&env, &io::hash(b"another-run:owner")).is_err());
        assert!(io::observed_run_token(&env, &[0; 32]).is_err());
    }
    let expected = io::hash(b"run:owner");
    for env in [
        b"A=B\0".as_slice(),
        b"SPC_LAB_RUN_TOKEN=\0",
        b"SPC_LAB_RUN_TOKEN=run:owner\0SPC_LAB_RUN_TOKEN=run:owner\0",
        b"SPC_LAB_RUN_TOKEN=run:owner\0SPC_LAB_RUN_TOKEN=foreign\0",
    ] {
        assert!(io::observed_run_token(env, &expected).is_err());
    }
    let oversized = zeroize::Zeroizing::new(format!("SPC_LAB_RUN_TOKEN={}\0", "x".repeat(1025)));
    assert!(io::observed_run_token(oversized.as_bytes(), &io::hash(&vec![b'x'; 1025])).is_err());
}

#[test]
fn native_private_config_requires_canonical_controller_digest() {
    let base = serde_json::json!({"run_id":"run", "identity_id":"alice", "pool_id":"pool",
        "front_id":"front", "run_token_hash":hex::encode(io::hash(b"opaque")),
        "collector_socket":"/private/collector.sock", "pool_pubkey":key(9).x_only_public_key().0.to_string(),
        "collector_pubkey":key(10).x_only_public_key().0.to_string()});
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(7).public_key(),
        &key(8).public_key(),
    )
    .unwrap();
    for bad in ["", "abc", &"00".repeat(32), &"AB".repeat(32)] {
        let mut value = base.clone();
        value["run_token_hash"] = bad.into();
        let profile: crate::config::NativeAudit = serde_json::from_value(value).unwrap();
        assert!(profile
            .validate(&address, spc_crypto::Network::Regtest)
            .is_err());
    }
    let mut value = base;
    value.as_object_mut().unwrap().remove("run_token_hash");
    assert!(serde_json::from_value::<crate::config::NativeAudit>(value).is_err());
}

#[tokio::test]
async fn unaudited_profile_has_no_native_marker_requirement() {
    let address = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(7).public_key(),
        &key(8).public_key(),
    )
    .unwrap();
    let config: crate::config::AgentConfig = serde_json::from_value(serde_json::json!({
        "upstream":{"address":"127.0.0.1:34254", "sp_address":address,
            "authority_public_key":"9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"},
        "ark":{"enabled":true,"network":"regtest"}}))
    .unwrap();
    config.validate().unwrap();
    assert!(NativeRuntime::start(&config).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_marker_does_not_bypass_foreign_process_config_or_registry() {
    for mutation in [
        "actual_foreign_marker",
        "token",
        "pid",
        "start",
        "boot",
        "executable",
        "config",
        "config_path",
        "instance",
    ] {
        let mut f = Fixture::new().await;
        let child = if mutation == "actual_foreign_marker" {
            cpu_with_token(
                &f.runtime.context,
                &f.request,
                true,
                "foreign-run:other-owner",
            )
        } else {
            cpu(&f.runtime.context, &f.request, true)
        };
        let path = f.runtime.context.root.join(wire::REGISTRY_FILENAME);
        let uid = io::uid().unwrap();
        let mut registry = wire::decode_registry(&io::private_read(&path, uid).unwrap()).unwrap();
        let peer = &mut registry.peers[0];
        match mutation {
            "actual_foreign_marker" => {}
            "token" => peer.process.run_token_sha256 = io::hash(b"foreign"),
            "pid" => peer.process.pid = std::process::id(),
            "start" => peer.process.start_time_ticks += 1,
            "boot" => peer.process.boot_id = "00000000-0000-0000-0000-000000000001".into(),
            "executable" => peer.process.executable_sha256 = [99; 32],
            "config" => peer.process.config_sha256 = [99; 32],
            "config_path" => {
                peer.config_path = f
                    .runtime
                    .context
                    .private
                    .join("foreign.toml")
                    .display()
                    .to_string()
            }
            "instance" => registry.wallet_instance = [99; 16],
            _ => unreachable!(),
        }
        io::atomic_private(&path, &serde_json::to_vec(&registry).unwrap(), uid).unwrap();
        let reply = tokio::select! {r=result(child)=>r,r=f.runtime.serve_owner(&f.keys)=>panic!("owner stopped: {r:?}")};
        assert!(
            matches!(
                reply,
                Response::Unknown {
                    code: UnknownCode::Rejected,
                    ..
                }
            ),
            "{mutation}"
        );
        assert!(f.runtime.context.tracker.lock().unwrap().latest.is_empty());
        f.runtime.stop().await.unwrap();
    }
}

#[test]
#[ignore = "requires enabled user/network namespaces; explicit host gate"]
fn foreign_network_namespace_is_rejected_before_token_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = root.join("cpu.toml");
    private_write(&config, b"namespace probe config");
    let mut child = Command::new("unshare")
        .args([
            "--user",
            "--map-root-user",
            "--net",
            "python3",
            "-c",
            "import sys; print('READY',flush=True); sys.stdin.buffer.read(1)",
            "--config",
        ])
        .arg(&config)
        .env("SPC_LAB_RUN_TOKEN", fixture_run_token().as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut ready)
        .unwrap();
    if ready != "READY\n" {
        let output = child.wait_with_output().unwrap();
        panic!(
            "required namespace gate unavailable: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let observed = io::process(
        child.id(),
        io::uid().unwrap(),
        &root,
        Some(&config),
        &io::hash(fixture_run_token().as_bytes()),
    );
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    // Host namespace mismatch or an unreadable foreign namespace both reject;
    // neither may reach the token/registry authorization path.
    let error = observed.unwrap_err().to_string();
    assert_ne!(error, "native process run scope");
}
