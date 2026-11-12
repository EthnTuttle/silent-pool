//! Shared helpers for the AP-3a wallet integration tests.
//!
//! Everything here is test scaffolding: it provisions key files from a
//! deterministic master seed (the committed fixture's seed, or a test
//! seed for non-recovery scenarios) and provides mock transports. The
//! production code paths under test are exactly the ones `--roles
//! wallet` runs.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::Path;

use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use spc_agent::wallet::receive::{ObservedVtxo, ReceiveBackend};
use spc_agent::wallet::registration::{
    MailboxMode, RegistrationOutcome, RegistrationPayload, RegistrationTransport,
};
use spc_agent::wallet::state::RecordWrite;
use spc_agent::wallet::{EpochFallback, KeySources, WalletRole, WalletSettings};
use spc_crypto::{Network, SpAddress};

/// Write a key file with strict 0600 permissions.
pub fn write_secret(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

/// Provision key files for `master_seed` under `dir`, exactly the way
/// the (pure) provisioner would, and return the agent-side sources
/// plus the `sp_address` the pool would hand out.
pub fn provision_files(
    dir: &Path,
    master_seed: &[u8; 64],
    network: Network,
) -> (KeySources, String) {
    let keys = spc_crypto::provision(master_seed, network).unwrap();
    let secp = Secp256k1::new();
    let scan = spc_crypto::hd::spc_branch_secret(&keys.scan_branch).unwrap();
    let ark = spc_crypto::hd::spc_branch_secret(&keys.ark_branch).unwrap();
    let spend = spc_crypto::hd::spc_branch_secret(&keys.spend_branch).unwrap();
    let scan_pub = PublicKey::from_secret_key(&secp, &scan);
    let spend_pub = PublicKey::from_secret_key(&secp, &spend);
    let ark_pub = PublicKey::from_secret_key(&secp, &ark);
    assert_ne!(ark_pub, spend_pub, "fixture sanity: ark != spend");

    let sources = KeySources {
        scan_xprv: dir.join("scan.xprv"),
        ark_xprv: dir.join("ark.xprv"),
        child_seed: dir.join("child.seed"),
        sp_address: spc_crypto::address::encode_sp_address(network, &scan_pub, &spend_pub).unwrap(),
        network,
    };
    write_secret(&sources.scan_xprv, &keys.scan_branch.to_string());
    write_secret(&sources.ark_xprv, &keys.ark_branch.to_string());
    write_secret(&sources.child_seed, &hex::encode(keys.child.seed));
    let sp_address = sources.sp_address.clone();
    (sources, sp_address)
}

/// Wallet settings for a test agent.
pub fn test_settings(
    sources: &KeySources,
    datadir: &Path,
    status_listen: SocketAddr,
) -> WalletSettings {
    WalletSettings {
        scan_xprv: sources.scan_xprv.clone(),
        ark_xprv: sources.ark_xprv.clone(),
        child_seed: sources.child_seed.clone(),
        datadir: datadir.to_path_buf(),
        status_listen,
        mailbox_mode: MailboxMode::EpochDerived,
        epoch_fallback: EpochFallback::Hold,
        ark_id: [0xa1, 0xb2, 0xc3, 0xd4],
        network: sources.network,
        sp_address: sources.sp_address.clone(),
    }
}

/// Parse the public pair back out of an `sp_address` (test assertions
/// and U derivation).
pub fn parse_sp(sp_address: &str, network: Network) -> SpAddress {
    spc_crypto::parse_sp_address(sp_address, network).unwrap()
}

/// A scripted registration transport: consumes queued results in order
/// and records every send.
pub struct MockTransport {
    pub results: Vec<Result<RegistrationOutcome, String>>,
    pub sent: Vec<([u8; 4], u32)>, // (payload digest prefix, request id)
}

impl MockTransport {
    pub fn new(results: Vec<Result<RegistrationOutcome, String>>) -> Self {
        MockTransport {
            results,
            sent: Vec::new(),
        }
    }
}

impl RegistrationTransport for MockTransport {
    fn send(
        &mut self,
        payload: &RegistrationPayload,
        request_id: u32,
    ) -> anyhow::Result<RegistrationOutcome> {
        let prefix: [u8; 4] = payload.local_digest()[..4].try_into().unwrap();
        self.sent.push((prefix, request_id));
        if self.results.is_empty() {
            return Ok(RegistrationOutcome::Retryable {
                error_code: "registration-busy".into(),
            });
        }
        self.results.remove(0).map_err(|e| anyhow::anyhow!(e))
    }
}

/// A scripted receive backend (the AP-3b live RPC seam).
pub struct MockReceiveBackend {
    pub observations: Vec<ObservedVtxo>,
}

impl ReceiveBackend for MockReceiveBackend {
    fn poll_mailbox(
        &mut self,
        _mailbox_epoch_height: Option<u32>,
    ) -> anyhow::Result<Vec<ObservedVtxo>> {
        Ok(self.observations.clone())
    }
}

/// A deterministic miner-side `A_send` for fixture blocks.
pub fn a_send(seed: u8) -> PublicKey {
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[seed; 32]).unwrap();
    PublicKey::from_secret_key(&secp, &secret)
}

/// RecordWrite predicates (kept readable at call sites).
pub trait RecordWriteExt {
    fn is_inserted(&self) -> bool;
    fn is_already_present(&self) -> bool;
    fn is_updated(&self) -> bool;
}

impl RecordWriteExt for RecordWrite {
    fn is_inserted(&self) -> bool {
        matches!(self, RecordWrite::Inserted)
    }
    fn is_already_present(&self) -> bool {
        matches!(self, RecordWrite::AlreadyPresent)
    }
    fn is_updated(&self) -> bool {
        matches!(self, RecordWrite::Updated)
    }
}

/// Test conveniences over the running role.
pub trait WalletCommonExt {
    /// Derive the `U -> u` pair for a pool block output (`k = 0`) and
    /// insert it into the store, exactly as the receive path does.
    fn derive_u(&self, height: u32, a_send: &PublicKey) -> PublicKey;
    /// GET /status from the role's loopback endpoint, returning the
    /// body text.
    fn http_status_json(&self) -> impl std::future::Future<Output = String> + Send;
}

impl WalletCommonExt for WalletRole {
    fn derive_u(&self, height: u32, a_send: &PublicKey) -> PublicKey {
        let keys = self.keys();
        spc_agent::wallet::state::insert_derived_u(
            self.store(),
            &keys.b_scan,
            &keys.b_ark,
            a_send,
            height,
            0,
        )
        .unwrap()
        .expect("deterministic fixture derivation cannot fail")
    }

    async fn http_status_json(&self) -> String {
        let addr = self.status_addr().expect("role has a status server bound");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET /status HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        text.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
    }
}
