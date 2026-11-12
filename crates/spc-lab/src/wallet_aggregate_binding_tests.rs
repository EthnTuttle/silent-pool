//! Local source/config/socket fixtures, not funded Ark acceptance.
use super::*;
use serde_json::json;
use std::net::{Ipv4Addr, TcpListener};
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    scenario: Scenario,
    manifest: RunManifest,
    front: toml::Value,
    wallet: toml::Value,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
impl Fixture {
    fn new() -> Self {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let root = std::env::temp_dir().join(format!(
            "spc-aggregate-binding-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        for path in [
            root.join("private"),
            root.join("private/pool-ark-pool-a"),
            root.join("private/wallet-alice"),
        ] {
            std::fs::create_dir(path.clone()).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut scenario: Scenario =
            serde_json::from_str(include_str!("../scenarios/lab-ark-wallets.json")).unwrap();
        let secp = Secp256k1::new();
        let scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
        let spend = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[4; 32]).unwrap());
        let address =
            spc_crypto::address::encode_sp_address(spc_crypto::Network::Regtest, &scan, &spend)
                .unwrap();
        scenario.identities[0].address_source = AddressSource::Explicit {
            address: address.clone(),
        };
        let manifest: RunManifest=serde_json::from_value(json!({
            "schema_version":1,"run_id":scenario.run.id,"owner_instance":"test-owner","created_at":1,"updated_at":1,
            "base_revision":"test","dirty_tree_hash":"test","scenario_sha256":"test","binary_paths_and_hashes":{},
            "resources":{"run_dir":root,"listeners":{
                "front.front-a1.api":{"host":"127.0.0.1","port":38001},
                "front.front-a1.sv2":{"host":"127.0.0.1","port":38002},
                "wallet.alice.status":{"host":"127.0.0.1","port":38003}},
                "redis_instances":[{"id":"redis-pool-a","owner_pool_id":"pool-a","port":38004,"dir":root.join("redis"),"url":"redis://fixture@127.0.0.1:38004/0"}],
                "postgres_databases":[{"id":"pg-pool-a","owner_pool_id":"pool-a","database":"spc_lab_a","role":"spc_lab_a","port":38005,"dir":root.join("pg"),"url":"postgres://fixture@127.0.0.1:38005/spc_lab_a"}],"temp_dirs":[]},
            "effective_profiles":{},"processes":[],"locks":{"path":root.join("owner.lock"),"owner_instance":"test-owner","pid":std::process::id(),"heartbeat_at":1},
            "state":"running","artifact_paths":crate::artifacts::ArtifactPaths::under(root.clone()),"warnings":[]
        })).unwrap();
        let order = scan.x_only_public_key().0.to_string();
        let ark = json!({"enabled":true,"order_pubkey":order,"ark_id":"01020304","server_pubkey":scan.to_string(),"mailbox_pubkey":spend.to_string(),"server":"http://127.0.0.1:39000/"});
        let mut front=toml::Value::try_from(json!({"pool":{"network":"regtest","front_id":1,"listen":"127.0.0.1:38002","roles":["front","payout","api"],"authority_public_key":"fixture-public-authority","pool_payout_descriptor":"fixture-descriptor"},
            "api":{"listen":"127.0.0.1:38001"},"storage":{"redis":manifest.resources.redis_instances[0].url,"postgres":manifest.resources.postgres_databases[0].url},"ark":ark})).unwrap();
        // Public configuration can name the payout secret. The join must not
        // attempt to open it; this path deliberately has no readable target.
        front["ark"].as_table_mut().unwrap().insert(
            "order_key_path".into(),
            "/missing/private/order-secret.hex".into(),
        );
        let role=toml::Value::try_from(json!({"pool":{"network":"regtest","pool_payout_descriptor":"fixture-descriptor"},"ark":ark})).unwrap();
        let role_bytes = toml::to_string(&role).unwrap().into_bytes();
        write(&root.join("private/pool-ark-pool-a/pool.toml"), &role_bytes);
        write(&root.join("private/pool-ark-pool-a/provision.json"),&serde_json::to_vec(&json!({"schema":"spc.lab.pool-ark-provision/v1","pool_id":"pool-a","config_sha256":hex::encode(Sha256::digest(&role_bytes))})).unwrap());
        write(&root.join("private/wallet-alice/provision.json"),&serde_json::to_vec(&json!({"schema":"spc.agent.provision/v1","network":"regtest","sp_address":address,"child_fingerprint":"12345678"})).unwrap());
        let dir = root.join("private/wallet-alice");
        let wallet=toml::Value::try_from(json!({"upstream":{"address":"127.0.0.1:38002","authority_public_key":"fixture-public-authority","sp_address":address},
            "ark":{"network":"regtest","status_listen":"127.0.0.1:38003","ark_id":ark["ark_id"],"server_pubkey":ark["server_pubkey"],"mailbox_pubkey":ark["mailbox_pubkey"],"server_address":ark["server"],
            "scan_xprv":dir.join("scan.xprv"),"ark_xprv":dir.join("ark.xprv"),"seed_child":dir.join("child.seed"),"datadir":dir.join("state"),"aggregate_api":{"origin":"http://127.0.0.1:38001/","order_pubkey":order}}})).unwrap();
        let f = Self {
            root,
            scenario,
            manifest,
            front,
            wallet,
        };
        f.save_front();
        f
    }
    fn save_front(&self) {
        write(
            &self.root.join("private/pool-front-a1.toml"),
            toml::to_string(&self.front).unwrap().as_bytes(),
        );
    }
    fn resolve(&self) -> Result<Resolved> {
        resolve(
            &self.scenario,
            &self.manifest,
            &self.root,
            &self.scenario.wallet_agents[0],
        )
    }
}

#[test]
fn leased_api_and_actual_public_key_join_without_opening_private_order_key() {
    let f = Fixture::new();
    let resolved = f.resolve().unwrap();
    assert_eq!(resolved.binding.origin, "http://127.0.0.1:38001/");
    assert_ne!(
        resolved.binding.origin,
        field(&f.wallet, "ark", "server_address").unwrap()
    );
    assert_ne!(
        resolved.binding.origin,
        format!("http://{}/", resolved.status)
    );
    persisted_config(&resolved, &f.wallet).unwrap();
    assert!(
        !f.root.join("private/wallet-alice/scan.xprv").exists(),
        "runtime joins public provisioning; online WalletKeys performs secret validation"
    );
}

#[test]
fn wrong_pool_front_role_or_listener_mapping_rejects() {
    let mut f = Fixture::new();
    f.scenario.wallet_agents[0].front_id = "front-b1".parse().unwrap();
    assert!(f.resolve().is_err());
    f.scenario.wallet_agents[0].front_id = "front-a1".parse().unwrap();
    f.scenario.fronts[0].roles.retain(|r| *r != FrontRole::Api);
    assert!(f.resolve().is_err());
    f.scenario.fronts[0].roles.push(FrontRole::Api);
    f.front["api"]["listen"] = "127.0.0.1:39000".into();
    f.save_front();
    assert!(f.resolve().is_err());
    f.front["api"]["listen"] = "127.0.0.1:38001".into();
    f.save_front();
    f.manifest.resources.redis_instances[0].owner_pool_id = "pool-b".parse().unwrap();
    assert!(f.resolve().is_err());
}

#[test]
fn injected_origin_key_and_foreign_namespace_never_become_wallet_config() {
    let mut f = Fixture::new();
    for listen in [
        "evil.example:38001",
        "0.0.0.0:38001",
        "127.0.0.1:0",
        "http://127.0.0.1:38001/path",
    ] {
        f.front["api"]["listen"] = listen.into();
        f.save_front();
        assert!(f.resolve().is_err());
    }
    f.front["api"]["listen"] = "127.0.0.1:38001".into();
    f.front["ark"]["order_pubkey"] = "01".repeat(32).into();
    f.save_front();
    assert!(f.resolve().is_err());
    let mut f = Fixture::new();
    f.manifest.placement = Some(json!({"namespace":"foreign"}));
    assert!(f.resolve().is_err());
    f.manifest.placement = None;
    f.manifest
        .resources
        .listeners
        .get_mut("front.front-a1.api")
        .unwrap()
        .host = "10.0.0.1".into();
    assert!(f.resolve().is_err());
}

#[test]
fn persisted_binding_validation_is_read_only_and_refuses_mutation_or_missing_binding() {
    let f = Fixture::new();
    let resolved = f.resolve().unwrap();
    let path = f.root.join("private/wallet-alice/agent.toml");
    let bytes = toml::to_string(&f.wallet).unwrap().into_bytes();
    write(&path, &bytes);
    let saved = config(&f.root.join("private"), &path).unwrap();
    persisted_config(&resolved, &saved).unwrap();
    for origin in [
        "http://127.0.0.1:38003/",
        "https://attacker.invalid/",
        "http://user:pass@127.0.0.1:38001/",
        "http://127.0.0.1:38001/receipt",
    ] {
        let mut changed = saved.clone();
        changed["ark"]["aggregate_api"]["origin"] = origin.into();
        assert!(persisted_config(&resolved, &changed).is_err());
    }
    let mut changed = saved.clone();
    changed["ark"]["aggregate_api"]
        .as_table_mut()
        .unwrap()
        .remove("order_pubkey");
    assert!(persisted_config(&resolved, &changed).is_err());
    let mut changed = saved.clone();
    changed["ark"]
        .as_table_mut()
        .unwrap()
        .remove("aggregate_api");
    assert!(persisted_config(&resolved, &changed).is_err());
    let mut changed = saved.clone();
    changed["ark"]["scan_xprv"] = "/another/wallet/scan.xprv".into();
    assert!(persisted_config(&resolved, &changed).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert!(
        !f.root.join("private/wallet-alice/state").exists(),
        "attach validation did not provision a wallet"
    );
}

#[test]
fn public_provisioning_and_private_source_permissions_are_binding() {
    let mut f = Fixture::new();
    f.scenario.identities[0].address_source = AddressSource::Explicit {
        address: "other".into(),
    };
    assert!(f.resolve().is_err());
    let f = Fixture::new();
    let path = f.root.join("private/pool-front-a1.toml");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(f.resolve().is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let saved = path.with_extension("saved");
    std::fs::rename(&path, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &path).unwrap();
    assert!(f.resolve().is_err());
}

#[test]
fn listener_ownership_requires_exact_loopback_socket_not_only_port() {
    let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(addr) = socket.local_addr().unwrap() else {
        panic!("IPv4 listener");
    };
    assert!(owns_listener(std::process::id(), addr).unwrap());
    assert!(!owns_listener(
        std::process::id(),
        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), addr.port())
    )
    .unwrap());
    assert!(owns_listener(u32::MAX, addr).is_err());
    drop(socket);
    assert!(!owns_listener(std::process::id(), addr).unwrap());
}
