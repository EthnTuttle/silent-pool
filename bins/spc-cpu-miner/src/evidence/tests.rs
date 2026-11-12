use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

#[test]
fn genuine_signed_contract_fixture_joins_actual_bytes_and_rejects_cross_record_changes() {
    let bytes = include_bytes!("../../fixtures/native-evidence/record.json");
    let manifest = include_bytes!("../../fixtures/native-evidence/manifest.json");
    let handle: DurableHandle =
        serde_json::from_slice(include_bytes!("../../fixtures/native-evidence/handle.json"))
            .unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../fixtures/native-evidence/provenance.json"
    ))
    .unwrap();
    let record: Record = serde_json::from_slice(bytes).unwrap();
    let request = wire::decode_request(&hex::decode(&record.request_json_hex).unwrap()).unwrap();
    let keys = VerificationKeys::for_regtest_identity(
        &request.identity,
        metadata["collector_pubkey"].as_str().unwrap(),
        metadata["pool_pubkey"].as_str().unwrap(),
    )
    .unwrap();
    let (_, parsed, _) = format::decode_record(bytes, manifest, &handle, &keys).unwrap();
    assert_eq!(
        hex::encode(parsed.signing_digest().unwrap()),
        handle.request_digest
    );
    for change in 0..4 {
        let mut altered: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        match change {
            0 => altered["binding"]["candidate_epoch"] = serde_json::json!(99),
            1 => altered["response_sha256"] = serde_json::json!("00".repeat(32)),
            2 => altered["checkpoint_digest"] = serde_json::json!("00".repeat(32)),
            _ => altered["activation"] = serde_json::json!("activated"),
        }
        let altered = serde_json::to_vec(&altered).unwrap();
        let mut claimed = handle.clone();
        claimed.record_sha256 = format::sha256_hex(&altered);
        assert!(format::decode_record(&altered, manifest, &claimed, &keys).is_err());
    }
    let mut wrong = handle.clone();
    wrong.record_relative_path = "../outside.json".into();
    assert!(format::decode_record(bytes, manifest, &wrong, &keys).is_err());
}

#[test]
fn private_capture_refuses_symlinks_traversal_and_existing_names() {
    let dir = tempfile::Builder::new()
        .prefix("capture-path-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let store = CaptureStore::test_store(dir.path(), 8);
    let info = store.info();
    let real = info.directory.join(&info.manifest_relative_path);
    let linked = dir.path().join("linked");
    symlink(&info.directory, &linked).unwrap();
    assert!(open_directory(&linked).is_err());
    let mut invalid = store.manifest.config.clone();
    invalid.directory = dir.path().join("private/native-evidence/../escape");
    assert!(invalid.validate("test-miner").is_err());
    let original = std::fs::read(&real).unwrap();
    assert!(publish(
        &store.session,
        "manifest.json",
        ".collision.pending",
        b"altered",
        store.manifest.process.uid
    )
    .is_err());
    assert_eq!(original, std::fs::read(&real).unwrap());
    symlink(
        &real,
        info.directory
            .join(format::session_name(&store.manifest.process))
            .join("link.json"),
    )
    .unwrap();
    assert!(read_file(
        &store.session,
        "link.json",
        store.manifest.process.uid,
        65536
    )
    .is_err());
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_file(
        &store.session,
        "manifest.json",
        store.manifest.process.uid,
        65536
    )
    .is_err());
}

#[test]
fn process_rotation_exhaustion_preserves_existing_session_evidence() {
    let dir = tempfile::Builder::new()
        .prefix("capture-rotation-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let store = CaptureStore::test_store(dir.path(), 8);
    let path = store
        .info
        .directory
        .join(&store.info.manifest_relative_path);
    let before = std::fs::read(&path).unwrap();
    let mut config = store.manifest.config.clone();
    config.max_sessions = 1;
    let config_path = store.manifest.config_path.clone();
    let mut process = store.manifest.process.clone();
    process.pid += 1;
    drop(store);
    assert!(CaptureStore::create(config, "test-miner".into(), config_path, process).is_err());
    assert_eq!(before, std::fs::read(path).unwrap());
}
