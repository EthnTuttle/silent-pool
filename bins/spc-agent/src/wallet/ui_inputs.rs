//! Private, disposable input observation. This is never an input authority or
//! ledger. Only the existing successful proof-validation passes populate it.
use super::receive::{ObservedVtxo, VtxoState};
use super::{VtxoRecord, WalletStore};
use anyhow::{ensure, Context};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_ROWS: usize = 4096;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_AGE_MS: u64 = 15_000;

pub(super) fn now_ms() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Rail {
    Derived,
    Normal,
}

#[derive(Clone)]
struct VerifiedInput {
    id: [u8; 36],
    amount: u64,
    expiry: Option<u32>,
    rail: Rail,
}

/// In-memory observation scratch only. The timestamp starts BEFORE the proof
/// reads; finishing a slow validation must not freshen its oldest observation.
pub(super) struct Capture {
    at: Option<u64>,
    rows: Vec<VerifiedInput>,
}
impl Capture {
    pub(super) fn new() -> Self {
        Self {
            at: now_ms().ok(),
            rows: vec![],
        }
    }
    fn push(&mut self, input: VerifiedInput) {
        if self.at.is_none() {
            return;
        }
        if self.rows.len() == MAX_ROWS {
            self.at = None;
            self.rows.clear();
            return;
        }
        self.rows.push(input);
    }
    pub(super) fn derived(&mut self, v: &ObservedVtxo) {
        if v.state == VtxoState::Spendable {
            self.push(VerifiedInput {
                id: v.vtxo_id,
                amount: v.amount_sats,
                expiry: v.expiry.height(),
                rail: Rail::Derived,
            });
        }
    }
    pub(super) fn normal(&mut self, id: [u8; 36], amount: u64, expiry: u32) {
        self.push(VerifiedInput {
            id,
            amount,
            expiry: Some(expiry),
            rail: Rail::Normal,
        });
    }
}

#[derive(Default)]
pub(super) struct Observations {
    derived: Mutex<Option<Capture>>,
    normal: Mutex<Option<Capture>>,
}
impl Observations {
    pub(super) fn invalidate_derived(&self) {
        *self.derived.lock().expect("UI observation lock") = None;
    }
    pub(super) fn invalidate_normal(&self) {
        *self.normal.lock().expect("UI observation lock") = None;
    }
    pub(super) fn derived_validated(&self, capture: Capture) {
        *self.derived.lock().expect("UI observation lock") = Some(capture);
    }
    pub(super) fn normal_validated(&self, capture: Capture) {
        *self.normal.lock().expect("UI observation lock") = Some(capture);
    }

    fn snapshot(
        &self,
        current: &[VtxoRecord],
        reserved: &BTreeSet<Vec<u8>>,
        now: u64,
    ) -> anyhow::Result<PrivateInputs> {
        let derived = self
            .derived
            .lock()
            .map_err(|_| anyhow::anyhow!("UI observation lock"))?;
        let normal = self
            .normal
            .lock()
            .map_err(|_| anyhow::anyhow!("UI observation lock"))?;
        let derived = derived
            .as_ref()
            .context("derived proof observation unavailable")?;
        let normal = normal
            .as_ref()
            .context("normal proof observation unavailable")?;
        let at = derived
            .at
            .context("derived input limit/clock unavailable")?
            .min(normal.at.context("normal input limit/clock unavailable")?);
        ensure!(at <= now && now - at <= MAX_AGE_MS, "input proofs stale");
        ensure!(
            derived
                .rows
                .len()
                .checked_add(normal.rows.len())
                .context("input count overflow")?
                <= MAX_ROWS,
            "input row limit"
        );
        let mut inputs = vec![];
        let mut seen = BTreeSet::new();
        for input in derived.rows.iter().chain(&normal.rows) {
            if reserved.contains(input.id.as_slice()) {
                continue;
            }
            if matches!(input.rail, Rail::Derived) {
                let row = current
                    .iter()
                    .find(|v| v.vtxo_id == input.id)
                    .context("derived observation no longer in wallet")?;
                if row.state != VtxoState::Spendable {
                    continue;
                }
                ensure!(
                    row.amount_sats == input.amount && row.expiry.height() == input.expiry,
                    "derived observation changed"
                );
            }
            // No spend selector accepts unknown expiry in this wallet adapter.
            if input.expiry.is_none() {
                continue;
            }
            ensure!(
                input.amount > 0 && input.expiry != Some(0),
                "invalid input observation"
            );
            ensure!(seen.insert(input.id), "duplicate input across wallet rails");
            inputs.push(Input {
                id: hex::encode(input.id),
                amount_sats: input.amount.to_string(),
                state: "spendable",
                expiry_height: input.expiry,
                rail: input.rail,
            });
        }
        inputs.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(PrivateInputs {
            schema: "spc-agent-private-inputs/v1",
            updated_at_ms: at,
            inputs,
        })
    }
}

#[derive(Serialize)]
struct Input {
    id: String,
    amount_sats: String,
    state: &'static str,
    expiry_height: Option<u32>,
    rail: Rail,
}
#[derive(Serialize)]
struct PrivateInputs {
    schema: &'static str,
    updated_at_ms: u64,
    inputs: Vec<Input>,
}

/// Bound once to the actual configured wallet datadir, under its existing
/// writer lock. No browser/path parameter can choose the observation target.
pub(super) struct Publisher {
    root: PathBuf,
    directory: File,
    dev: u64,
    ino: u64,
    uid: u32,
}
impl Publisher {
    pub(super) fn open(root: &Path) -> anyhow::Result<Self> {
        ensure!(
            root.is_absolute() && root.canonicalize()? == root,
            "UI provider requires canonical configured datadir"
        );
        let meta = std::fs::symlink_metadata(root)?;
        ensure!(
            meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
            "UI provider datadir must be private and not a symlink"
        );
        // The supported runtime is Linux; proc supplies the effective process
        // ownership without another dependency or a wallet/RPC instance.
        let uid = std::fs::metadata("/proc/self")?.uid();
        ensure!(
            meta.uid() == uid,
            "UI provider datadir is not owned by this process user"
        );
        for name in ["state.sqlite", "bark/db.sqlite"] {
            let p = root.join(name);
            let m = std::fs::symlink_metadata(&p)?;
            ensure!(
                m.is_file() && m.uid() == uid && m.permissions().mode() & 0o777 == 0o600,
                "UI provider is not the configured initialized wallet datadir"
            );
        }
        let directory = File::open(root)?;
        let opened = directory.metadata()?;
        ensure!(
            opened.dev() == meta.dev() && opened.ino() == meta.ino(),
            "UI provider datadir changed while opening"
        );
        Ok(Self {
            root: root.into(),
            directory,
            dev: meta.dev(),
            ino: meta.ino(),
            uid,
        })
    }
    fn check_root(&self) -> anyhow::Result<()> {
        let m = std::fs::symlink_metadata(&self.root)?;
        ensure!(
            m.is_dir()
                && m.dev() == self.dev
                && m.ino() == self.ino
                && m.uid() == self.uid
                && m.permissions().mode() & 0o777 == 0o700
                && self.root.canonicalize()? == self.root,
            "UI provider root changed"
        );
        Ok(())
    }
    pub(super) fn unavailable(&self) -> anyhow::Result<()> {
        self.check_root()?;
        match std::fs::symlink_metadata(self.root.join("ui-inputs.json")) {
            Ok(m) => {
                ensure!(
                    m.is_file() || m.file_type().is_symlink(),
                    "invalid UI observation target"
                );
                std::fs::remove_file(self.root.join("ui-inputs.json"))?;
                self.directory.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
    fn write(&self, snapshot: &PrivateInputs, now: u64) -> anyhow::Result<()> {
        self.check_root()?;
        ensure!(
            snapshot.inputs.len() <= MAX_ROWS
                && snapshot.updated_at_ms <= now
                && now - snapshot.updated_at_ms <= MAX_AGE_MS,
            "invalid or stale UI snapshot"
        );
        let bytes = serde_json::to_vec(snapshot)?;
        ensure!(bytes.len() <= MAX_BYTES, "UI input body limit");
        let path = self.root.join("ui-inputs.json");
        if let Ok(m) = std::fs::symlink_metadata(&path) {
            ensure!(
                m.is_file() && m.uid() == self.uid && m.permissions().mode() & 0o777 == 0o600,
                "unsafe existing UI observation target"
            );
        }
        // Pinned Bark helper uses create_new(0600), file fsync, atomic rename
        // and cleans only its own temp on error. Complete the directory fsync.
        bark::fs_perms::write_atomic_owner_only(&path, &bytes)?;
        self.directory.sync_all()?;
        self.check_root()?;
        Ok(())
    }
    pub(super) fn publish(
        &self,
        observations: &Observations,
        store: &WalletStore,
        extra_reserved: impl IntoIterator<Item = Vec<u8>>,
    ) -> anyhow::Result<()> {
        let result = (|| {
            let mut reserved = store.reserved_inputs()?;
            reserved.extend(extra_reserved);
            reserved.extend(super::maintenance::queued_ui_inputs(
                store,
                &self.root.join("commands"),
            )?);
            let now = now_ms()?;
            let snapshot = observations.snapshot(&store.vtxo_records()?, &reserved, now)?;
            self.write(&snapshot, now_ms()?)
        })();
        if result.is_err() {
            let _ = self.unavailable();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        maintenance::{submit_command, MaintenanceAction, WalletCommand},
        receive::{Expiry, ExpirySource},
        StoreIdentity,
    };
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    fn fixture() -> (tempfile::TempDir, WalletStore, Publisher) {
        let temp = tempfile::tempdir().unwrap();
        let store = WalletStore::open(
            temp.path(),
            &StoreIdentity {
                wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
                sp_id: [2; 32],
                network: "regtest".into(),
            },
        )
        .unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(temp.path().join("bark"))
            .unwrap();
        // The read model must never open/read this database. Production gets
        // here only after the wallet's normal BarkStore/open validation.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temp.path().join("bark/db.sqlite"))
            .unwrap();
        let publisher = Publisher::open(temp.path()).unwrap();
        (temp, store, publisher)
    }
    fn record(id: u8, amount: u64, state: VtxoState) -> VtxoRecord {
        VtxoRecord {
            vtxo_id: [id; 36],
            u_pubkey: PublicKey::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[3; 32]).unwrap(),
            ),
            amount_sats: amount,
            state,
            expiry: Expiry::At {
                height: 1000,
                source: ExpirySource::VtxoAncestry,
            },
        }
    }
    fn observations(at: u64, derived: &[VtxoRecord], normal: &[(u8, u64)]) -> Observations {
        // Explicit verified-selector-state fixture. No blockchain/server proof
        // or funded wallet behavior is claimed by these read-model tests.
        let observations = Observations::default();
        let mut sp = Capture {
            at: Some(at),
            rows: vec![],
        };
        for v in derived {
            sp.derived(&ObservedVtxo {
                vtxo_id: v.vtxo_id,
                u_pubkey: v.u_pubkey,
                amount_sats: v.amount_sats,
                expiry: v.expiry,
                state: v.state,
            });
        }
        let mut stock = Capture {
            at: Some(at),
            rows: vec![],
        };
        for (id, amount) in normal {
            stock.normal([*id; 36], *amount, 1200);
        }
        observations.derived_validated(sp);
        observations.normal_validated(stock);
        observations
    }

    #[test]
    fn ui_inputs_exact_owner_dto_above_javascript_integer_range() {
        let current = vec![record(1, 9_007_199_254_740_993, VtxoState::Spendable)];
        let snapshot = observations(100, &current, &[(2, u64::MAX)])
            .snapshot(&current, &BTreeSet::new(), 101)
            .unwrap();
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"schema":"spc-agent-private-inputs/v1","updated_at_ms":100,"inputs":[
            {"id":"01".repeat(36),"amount_sats":"9007199254740993","state":"spendable","expiry_height":1000,"rail":"derived"},
            {"id":"02".repeat(36),"amount_sats":"18446744073709551615","state":"spendable","expiry_height":1200,"rail":"normal"}]})
        );
    }

    #[test]
    fn ui_inputs_reserved_spent_unknown_and_transferred_value_are_not_selectable() {
        let verified = vec![
            record(1, 100, VtxoState::Spendable),
            record(2, 200, VtxoState::Spendable),
            record(3, 300, VtxoState::Pending),
        ];
        let cache = observations(100, &verified, &[(4, 100)]);
        let mut current = verified.clone();
        current[0].state = VtxoState::Spent; // SP round imported ordinary output 4.
        let reserved = BTreeSet::from([vec![2; 36]]);
        let snapshot = cache.snapshot(&current, &reserved, 100).unwrap();
        assert_eq!(snapshot.inputs.len(), 1);
        assert_eq!(snapshot.inputs[0].id, hex::encode([4; 36]));
        let mut unknown = record(5, 10, VtxoState::Spendable);
        unknown.expiry = Expiry::Unknown;
        assert!(observations(100, &[unknown.clone()], &[])
            .snapshot(&[unknown], &BTreeSet::new(), 100)
            .unwrap()
            .inputs
            .is_empty());
        assert!(cache.snapshot(&verified, &BTreeSet::new(), 15_101).is_err());
    }

    #[test]
    fn ui_inputs_missing_validation_and_true_empty_are_distinct() {
        assert!(Observations::default()
            .snapshot(&[], &BTreeSet::new(), 100)
            .is_err());
        let empty = observations(100, &[], &[])
            .snapshot(&[], &BTreeSet::new(), 100)
            .unwrap();
        assert!(empty.inputs.is_empty());
        assert_eq!(empty.updated_at_ms, 100);
        let cache = observations(100, &[], &[]);
        cache.invalidate_derived();
        assert!(cache.snapshot(&[], &BTreeSet::new(), 100).is_err());
    }

    #[test]
    fn ui_inputs_duplicate_cross_rail_changed_amount_and_limits_fail_closed() {
        let current = vec![record(1, 100, VtxoState::Spendable)];
        assert!(observations(100, &current, &[(1, 100)])
            .snapshot(&current, &BTreeSet::new(), 100)
            .is_err());
        let cache = observations(100, &current, &[]);
        let mut changed = current;
        changed[0].amount_sats = 101;
        assert!(cache.snapshot(&changed, &BTreeSet::new(), 100).is_err());
        let cache = observations(100, &[], &[]);
        let mut oversized = Capture {
            at: Some(100),
            rows: vec![],
        };
        for _ in 0..=MAX_ROWS {
            oversized.normal([1; 36], 1, 1000);
        }
        assert!(oversized.at.is_none());
        cache.normal_validated(oversized);
        assert!(cache.snapshot(&[], &BTreeSet::new(), 100).is_err());
        assert!(observations(101, &[], &[])
            .snapshot(&[], &BTreeSet::new(), 100)
            .is_err());
    }

    #[test]
    fn ui_inputs_private_atomic_file_is_under_configured_datadir_and_not_a_ledger() {
        let (temp, store, publisher) = fixture();
        let before = std::fs::read(temp.path().join("bark/db.sqlite")).unwrap();
        publisher
            .publish(&observations(now_ms().unwrap(), &[], &[]), &store, [])
            .unwrap();
        let path = temp.path().join("ui-inputs.json");
        let bytes = std::fs::read(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["inputs"], serde_json::json!([]));
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read(temp.path().join("bark/db.sqlite")).unwrap(),
            before
        );
        assert!(store.vtxo_records().unwrap().is_empty());
        assert!(!temp
            .path()
            .parent()
            .unwrap()
            .join("ui-inputs.json")
            .exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".temp.")));
    }

    #[test]
    fn ui_inputs_error_removes_observation_without_freshening_old_timestamp() {
        let (temp, store, publisher) = fixture();
        let old = now_ms().unwrap();
        publisher
            .publish(&observations(old, &[], &[]), &store, [])
            .unwrap();
        let cache = observations(old, &[], &[]);
        cache.invalidate_normal();
        assert!(publisher.publish(&cache, &store, []).is_err());
        assert!(!temp.path().join("ui-inputs.json").exists());
        let cache = observations(old.saturating_sub(MAX_AGE_MS + 1), &[], &[]);
        assert!(publisher.publish(&cache, &store, []).is_err());
        assert!(!temp.path().join("ui-inputs.json").exists());
    }

    #[test]
    fn ui_inputs_hygiene_wrong_root_symlink_and_large_body_are_refused() {
        let (temp, store, publisher) = fixture();
        let parent = tempfile::tempdir().unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(temp.path(), &alias).unwrap();
        assert!(Publisher::open(&alias).is_err());
        assert!(Publisher::open(&temp.path().join("bark")).is_err());
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(publisher
            .publish(&observations(now_ms().unwrap(), &[], &[]), &store, [])
            .is_err());
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let outside = parent.path().join("untouched");
        std::fs::write(&outside, b"owner data").unwrap();
        std::os::unix::fs::symlink(&outside, temp.path().join("ui-inputs.json")).unwrap();
        assert!(publisher
            .publish(&observations(now_ms().unwrap(), &[], &[]), &store, [])
            .is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"owner data");
        let at = now_ms().unwrap();
        let snapshot = PrivateInputs {
            schema: "spc-agent-private-inputs/v1",
            updated_at_ms: at,
            inputs: vec![Input {
                id: "x".repeat(MAX_BYTES),
                amount_sats: "1".into(),
                state: "spendable",
                expiry_height: Some(1),
                rail: Rail::Normal,
            }],
        };
        assert!(publisher.write(&snapshot, at).is_err());
        assert!(!temp.path().join("ui-inputs.json").exists());
    }

    #[test]
    fn ui_inputs_concurrent_reader_never_observes_partial_json() {
        let (temp, _store, publisher) = fixture();
        let at = now_ms().unwrap();
        let snapshot = observations(at, &[], &[])
            .snapshot(&[], &BTreeSet::new(), at)
            .unwrap();
        publisher.write(&snapshot, at).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let path = temp.path().join("ui-inputs.json");
        let reader = std::thread::spawn(move || {
            while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
                let value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert_eq!(value["schema"], "spc-agent-private-inputs/v1");
            }
        });
        for _ in 0..30 {
            publisher.write(&snapshot, now_ms().unwrap()).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();
    }

    #[test]
    fn ui_inputs_owner_queue_and_actual_reservations_exclude_normal_inputs() {
        let (temp, store, publisher) = fixture();
        let command = WalletCommand {
            operation_id: hex::encode([1; 32]),
            vtxo_ids: vec![hex::encode([2; 36])],
            action: MaintenanceAction::Refresh,
        };
        submit_command(temp.path(), &command).unwrap();
        let cache = observations(now_ms().unwrap(), &[], &[(2, 100), (3, 200)]);
        publisher.publish(&cache, &store, [vec![3; 36]]).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("ui-inputs.json")).unwrap())
                .unwrap();
        assert_eq!(json["inputs"], serde_json::json!([]));
        store.reserve_maintenance(&command, Some(b"[]")).unwrap();
        std::fs::remove_file(
            temp.path()
                .join("commands")
                .join(format!("{}.json", command.operation_id)),
        )
        .unwrap();
        publisher.publish(&cache, &store, []).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("ui-inputs.json")).unwrap())
                .unwrap();
        assert_eq!(json["inputs"].as_array().unwrap().len(), 1);
        assert_eq!(json["inputs"][0]["id"], hex::encode([3; 36]));
    }
}
