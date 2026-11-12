mod common;

use std::os::unix::fs::PermissionsExt;

#[test]
fn private_manifest_preserves_storage_credentials_for_restore_with_mode_0600() {
    let dir = common::TestDir::new("manifest-credentials");
    let engine = common::prepare_engine(&dir, "scenario_v1_one_pool_two_miners.json");
    let mut private_manifest = engine.manifest_snapshot();
    let marker = "lp-final-manifest-secret";
    private_manifest.resources.redis_instances[0].url =
        format!("redis://:{marker}@10.250.0.2:41234/0");
    let path = dir.join("manifest-with-storage-auth.json");
    private_manifest.save(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600,
        "run manifests containing resource credentials must stay private"
    );
    let restored = spc_lab::manifest::RunManifest::load(&path).unwrap();
    assert!(
        restored.resources.redis_instances[0].url.contains(marker),
        "the private resource connection state remains restorable"
    );
    engine.refresh_artifacts().unwrap();
    let public = std::fs::read_dir(dir.join("shareable"))
        .expect("shareable directory")
        .map(|entry| entry.unwrap().path())
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !public.contains(marker),
        "a public projection leaked the Redis password"
    );
    assert!(path.exists());
}
