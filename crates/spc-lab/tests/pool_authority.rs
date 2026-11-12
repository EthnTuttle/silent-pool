mod common;

use std::os::unix::fs::PermissionsExt;

use spc_lab::ids::PoolId;

#[test]
fn each_pool_gets_a_distinct_private_authority_pair() {
    let dir = common::TestDir::new("pool-authority");
    let engine = common::prepare_engine(&dir, "scenario_v1_two_pools.json");
    let authorities = engine.public_authorities();
    assert_eq!(authorities.len(), 2);
    let pool_a = authorities.get(&PoolId::new("pool-a").unwrap()).unwrap();
    let pool_b = authorities.get(&PoolId::new("pool-b").unwrap()).unwrap();
    assert_ne!(pool_a.authority_public_key, pool_b.authority_public_key);
    assert_ne!(pool_a.public_hex, pool_b.public_hex);

    for pool in ["pool-a", "pool-b"] {
        let path = dir
            .path()
            .join("private")
            .join(format!("pool-authority-{pool}.json"));
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(
            metadata.permissions().mode() & 0o077,
            0,
            "{path:?} must not be group/world readable"
        );
        let body = std::fs::read_to_string(&path).unwrap();
        // The secret exists only in the private file, never in a shared
        // manifest. The fragment is also private.
        assert!(body.contains("secret_base58"));
        let fragment = dir
            .path()
            .join("private")
            .join(format!("pool-authority-{pool}.toml.fragment"));
        let fragment_meta = std::fs::metadata(&fragment).unwrap();
        assert_eq!(fragment_meta.permissions().mode() & 0o077, 0);
        let fragment_body = std::fs::read_to_string(&fragment).unwrap();
        assert!(fragment_body.contains("[pool]"));
        assert!(fragment_body.contains("authority_secret_key"));
    }
    engine.start().unwrap();
    engine.stop().unwrap();
    let shareable =
        std::fs::read_to_string(dir.path().join("shareable").join("manifest.json")).unwrap();
    assert!(shareable.contains("authority_public_key"));
    assert!(!shareable.contains("secret_base58"));
    assert!(!shareable.contains("authority_secret_key"));
}
