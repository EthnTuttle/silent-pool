//! Bounded public-evidence I/O, always outside the native connection selector.
use bitcoin::hashes::{sha256, Hash, HashEngine};
use rustix::fs::{mkdirat, openat, renameat_with, FlockOperation, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Serialize};
use spc_scan::{
    native_activation::{self as wire, ProcessIdentity, VerificationKeys},
    native_evidence::{self as format, Binding, Config, DurableHandle, Manifest, Record},
};
use std::os::unix::fs::MetadataExt;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::Semaphore;

fn unavailable<T>(_: T) -> &'static str {
    "native_evidence_unavailable"
}
fn private_metadata(file: &File, uid: u32, directory: bool) -> Result<(), &'static str> {
    let m = file.metadata().map_err(unavailable)?;
    if m.uid() != uid
        || m.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || (directory && !m.is_dir())
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err("native_evidence_permissions");
    }
    Ok(())
}
fn open_directory(path: &Path) -> Result<File, &'static str> {
    if !path.is_absolute() {
        return Err("native_evidence_path");
    }
    let mut dir = File::from(
        rustix::fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(unavailable)?,
    );
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                dir = File::from(
                    openat(
                        &dir,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(unavailable)?,
                )
            }
            _ => return Err("native_evidence_path"),
        }
    }
    Ok(dir)
}
fn read_file(dir: &File, name: &str, uid: u32, bound: usize) -> Result<Vec<u8>, &'static str> {
    let mut file = File::from(
        openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(unavailable)?,
    );
    private_metadata(&file, uid, false)?;
    let mut data = Vec::new();
    (&mut file)
        .take(bound as u64 + 1)
        .read_to_end(&mut data)
        .map_err(unavailable)?;
    private_metadata(&file, uid, false)?;
    if data.len() > bound {
        return Err("native_evidence_bound");
    }
    Ok(data)
}
fn publish(
    dir: &File,
    name: &str,
    stage: &str,
    bytes: &[u8],
    uid: u32,
) -> Result<(), &'static str> {
    let mut file = File::from(
        openat(
            dir,
            stage,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )
        .map_err(unavailable)?,
    );
    private_metadata(&file, uid, false)?;
    file.write_all(bytes).map_err(unavailable)?;
    file.sync_all().map_err(unavailable)?;
    renameat_with(dir, stage, dir, name, RenameFlags::NOREPLACE)
        .map_err(|_| "native_evidence_conflict")?;
    dir.sync_all().map_err(unavailable)?;
    Ok(())
}
fn file_hash(path: &Path) -> Result<[u8; 32], &'static str> {
    let mut f = File::open(path).map_err(unavailable)?;
    let mut engine = sha256::Hash::engine();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(unavailable)?;
        if n == 0 {
            break;
        }
        engine.input(&b[..n]);
    }
    Ok(sha256::Hash::from_engine(engine).to_byte_array())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureInfo {
    pub schema: String,
    pub capability: String,
    pub directory: PathBuf,
    pub manifest_relative_path: String,
    pub manifest_sha256: String,
}
struct Budget {
    attempts: u64,
    bytes: u64,
    retained: BTreeMap<String, DurableHandle>,
}
pub struct CaptureStore {
    manifest: Manifest,
    manifest_bytes: Vec<u8>,
    info: CaptureInfo,
    root: File,
    session: File,
    budget: Mutex<Budget>,
    slot: Arc<Semaphore>,
    #[cfg(test)]
    delay: Mutex<Option<Arc<TestDelay>>>,
}
impl CaptureStore {
    pub async fn open(
        config: Config,
        miner_id: String,
        config_path: PathBuf,
    ) -> Result<Arc<Self>, &'static str> {
        tokio::task::spawn_blocking(move || Self::open_blocking(config, miner_id, config_path))
            .await
            .map_err(unavailable)?
    }
    fn open_blocking(
        config: Config,
        miner_id: String,
        config_path: PathBuf,
    ) -> Result<Arc<Self>, &'static str> {
        config.validate(&miner_id)?;
        let pid = std::process::id();
        let uid = std::fs::metadata("/proc/self").map_err(unavailable)?.uid();
        let cfgdir = open_directory(config_path.parent().ok_or("native_evidence_path")?)?;
        private_metadata(&cfgdir, uid, true)?;
        let config_bytes = read_file(
            &cfgdir,
            config_path
                .file_name()
                .and_then(|p| p.to_str())
                .ok_or("native_evidence_path")?,
            uid,
            wire::MAX_REQUEST_BYTES,
        )?;
        let stat = std::fs::read_to_string("/proc/self/stat").map_err(unavailable)?;
        let ticks = stat
            .rsplit_once(')')
            .ok_or("native_evidence_process")?
            .1
            .split_whitespace()
            .nth(19)
            .ok_or("native_evidence_process")?
            .parse()
            .map_err(unavailable)?;
        let token = std::env::var("SPC_LAB_RUN_TOKEN").map_err(unavailable)?;
        if token.is_empty() {
            return Err("native_evidence_process");
        }
        let process = ProcessIdentity {
            pid,
            uid,
            start_time_ticks: ticks,
            boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map_err(unavailable)?
                .trim()
                .into(),
            executable_sha256: file_hash(Path::new("/proc/self/exe"))?,
            run_token_sha256: sha256::Hash::hash(token.as_bytes()).to_byte_array(),
            config_sha256: sha256::Hash::hash(&config_bytes).to_byte_array(),
        };
        Self::create(config, miner_id, config_path, process)
    }
    fn create(
        config: Config,
        miner_id: String,
        config_path: PathBuf,
        process: ProcessIdentity,
    ) -> Result<Arc<Self>, &'static str> {
        config.validate(&miner_id)?;
        process.validate()?;
        let uid = process.uid;
        let root = open_directory(&config.directory)?;
        private_metadata(&root, uid, true)?;
        rustix::fs::flock(&root, FlockOperation::NonBlockingLockExclusive).map_err(unavailable)?;
        let mut count = 0;
        for entry in std::fs::read_dir(&config.directory)
            .map_err(unavailable)?
            .take(config.max_sessions as usize + 1)
        {
            entry.map_err(unavailable)?;
            count += 1;
        }
        if count >= config.max_sessions as usize {
            return Err("native_evidence_session_budget");
        }
        let name = format::session_name(&process);
        mkdirat(&root, name.as_str(), Mode::from_bits_truncate(0o700))
            .map_err(|_| "native_evidence_session_exists")?;
        let session = File::from(
            openat(
                &root,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(unavailable)?,
        );
        private_metadata(&session, uid, true)?;
        let manifest = Manifest {
            schema: format::MANIFEST_SCHEMA.into(),
            kind: "capture_session".into(),
            capability: format::CAPABILITY.into(),
            capture_id: hex::encode(rand::random::<[u8; 16]>()),
            miner_id,
            config,
            process,
            config_path,
        };
        manifest.validate()?;
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(unavailable)?;
        if manifest_bytes.len() > format::MAX_MANIFEST_BYTES {
            return Err("native_evidence_bound");
        }
        publish(
            &session,
            "manifest.json",
            ".manifest.pending",
            &manifest_bytes,
            uid,
        )?;
        root.sync_all().map_err(unavailable)?;
        let info = CaptureInfo {
            schema: format::LOCATOR_SCHEMA.into(),
            capability: format::CAPABILITY.into(),
            directory: manifest.config.directory.clone(),
            manifest_relative_path: format!("{name}/manifest.json"),
            manifest_sha256: format::sha256_hex(&manifest_bytes),
        };
        Ok(Arc::new(Self {
            budget: Mutex::new(Budget {
                attempts: 0,
                bytes: manifest_bytes.len() as u64,
                retained: BTreeMap::new(),
            }),
            manifest,
            manifest_bytes,
            info,
            root,
            session,
            slot: Arc::new(Semaphore::new(1)),
            #[cfg(test)]
            delay: Mutex::new(None),
        }))
    }
    pub fn info(&self) -> CaptureInfo {
        self.info.clone()
    }
    pub fn matches(&self, run: &str, miner: &str) -> bool {
        self.manifest.config.run_id == run && self.manifest.miner_id == miner
    }

    fn check_location(&self) -> Result<(), &'static str> {
        let root = open_directory(&self.manifest.config.directory)?;
        let name = format::session_name(&self.manifest.process);
        let session = File::from(
            openat(
                &root,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(unavailable)?,
        );
        for (expected, actual) in [(&self.root, &root), (&self.session, &session)] {
            private_metadata(actual, self.manifest.process.uid, true)?;
            let a = expected.metadata().map_err(unavailable)?;
            let b = actual.metadata().map_err(unavailable)?;
            if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                return Err("native_evidence_directory_changed");
            }
        }
        Ok(())
    }

    pub async fn retain(
        self: &Arc<Self>,
        request_bytes: Vec<u8>,
        response_bytes: Vec<u8>,
        keys: VerificationKeys,
        verified_at_unix_ms: u64,
        deadline: Instant,
        current: Arc<AtomicBool>,
    ) -> Result<DurableHandle, &'static str> {
        // A cancelled fsync keeps its slot until it ends; retries cannot spawn
        // unbounded blocking operations. Async slot wait uses candidate timeout.
        let slot = self
            .slot
            .clone()
            .acquire_owned()
            .await
            .map_err(unavailable)?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            store.retain_blocking(
                request_bytes,
                response_bytes,
                keys,
                verified_at_unix_ms,
                deadline,
                &current,
            )
        })
        .await
        .map_err(unavailable)?
    }
    fn retain_blocking(
        &self,
        request_bytes: Vec<u8>,
        response_bytes: Vec<u8>,
        keys: VerificationKeys,
        verified_at_unix_ms: u64,
        deadline: Instant,
        current: &AtomicBool,
    ) -> Result<DurableHandle, &'static str> {
        let request = wire::decode_request(&request_bytes)?;
        let response = wire::decode_response(&response_bytes)?;
        let proof = wire::verify_response(&request, &response, &keys, verified_at_unix_ms)?;
        let eligible = || {
            if !current.load(Ordering::SeqCst)
                || Instant::now() >= deadline
                || crate::native_gate::now_ms()? >= proof.expires_at_unix_ms
            {
                return Err("native_evidence_expired_or_cancelled");
            }
            Ok(())
        };
        eligible()?;
        if !self.matches(&request.run_id, &request.miner_id) {
            return Err("native_evidence_run_binding");
        }
        let uid = self.manifest.process.uid;
        private_metadata(&self.root, uid, true)?;
        private_metadata(&self.session, uid, true)?;
        self.check_location()?;
        let existing_manifest = read_file(
            &self.session,
            "manifest.json",
            uid,
            format::MAX_MANIFEST_BYTES,
        )?;
        if existing_manifest != self.manifest_bytes {
            return Err("native_evidence_manifest_changed");
        }
        let digest = hex::encode(proof.request_digest);
        let filename = format!("{digest}.json");
        let mut budget = self.budget.lock().map_err(unavailable)?;
        if let Some(handle) = budget.retained.get(&digest) {
            let bytes = read_file(&self.session, &filename, uid, format::MAX_RECORD_BYTES)?;
            let (record, _, _) =
                format::decode_record(&bytes, &self.manifest_bytes, handle, &keys)?;
            if record.request_json_hex != hex::encode(&request_bytes)
                || record.response_json_hex != hex::encode(&response_bytes)
            {
                return Err("native_evidence_conflict");
            }
            eligible()?;
            return Ok(handle.clone());
        }
        if budget.attempts >= self.manifest.config.max_records {
            return Err("native_evidence_record_budget");
        }
        let sequence = budget.attempts + 1;
        let record = Record {
            schema: format::SCHEMA.into(),
            kind: "approved_exchange".into(),
            version: 1,
            activation: "not_asserted".into(),
            capture_id: self.manifest.capture_id.clone(),
            receipt_sequence: sequence,
            manifest_sha256: self.info.manifest_sha256.clone(),
            request_digest: digest.clone(),
            request_sha256: format::sha256_hex(&request_bytes),
            response_sha256: format::sha256_hex(&response_bytes),
            checkpoint_digest: hex::encode(proof.checkpoint_digest),
            binding: Binding::from(&request),
            verified_at_unix_ms,
            request_json_hex: hex::encode(request_bytes),
            response_json_hex: hex::encode(response_bytes),
        };
        let bytes = serde_json::to_vec(&record).map_err(unavailable)?;
        if bytes.len() > format::MAX_RECORD_BYTES {
            return Err("native_evidence_bound");
        }
        let charged = budget
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or("native_evidence_byte_budget")?;
        if charged > self.manifest.config.max_bytes {
            return Err("native_evidence_byte_budget");
        }
        budget.attempts = sequence;
        budget.bytes = charged;
        #[cfg(test)]
        {
            let delay = self.delay.lock().unwrap().take();
            if let Some(delay) = delay {
                delay.block();
            }
        }
        eligible()?;
        publish(
            &self.session,
            &filename,
            &format!(".{digest}-{sequence}.pending"),
            &bytes,
            uid,
        )?;
        let name = format::session_name(&self.manifest.process);
        let handle = DurableHandle {
            schema: format::HANDLE_SCHEMA.into(),
            capability: format::CAPABILITY.into(),
            capture_id: self.manifest.capture_id.clone(),
            receipt_sequence: sequence,
            manifest_relative_path: self.info.manifest_relative_path.clone(),
            manifest_sha256: self.info.manifest_sha256.clone(),
            record_relative_path: format!("{name}/{filename}"),
            record_sha256: format::sha256_hex(&bytes),
            request_digest: digest.clone(),
        };
        let readback = read_file(&self.session, &filename, uid, format::MAX_RECORD_BYTES)?;
        format::decode_record(&readback, &self.manifest_bytes, &handle, &keys)?;
        self.check_location()?;
        budget.retained.insert(digest, handle.clone());
        eligible()?;
        Ok(handle)
    }
}

#[cfg(test)]
impl CaptureStore {
    pub(crate) fn test_store(base: &Path, max_records: u64) -> Arc<Self> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let root = base.join("private/native-evidence/test-miner");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .unwrap();
        for path in [
            base.join("private"),
            base.join("private/native-evidence"),
            root.clone(),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let uid = std::fs::metadata("/proc/self").unwrap().uid();
        Self::create(
            Config {
                schema: format::CONFIG_SCHEMA.into(),
                run_id: "test-run".into(),
                directory: root,
                run_manifest: base.join("run-manifest.json"),
                max_records,
                max_bytes: 268435456,
                max_sessions: 32,
            },
            "test-miner".into(),
            base.join("private/miner.toml"),
            ProcessIdentity {
                pid: 1234,
                uid,
                start_time_ticks: 55,
                boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
                executable_sha256: [1; 32],
                run_token_sha256: [2; 32],
                config_sha256: [3; 32],
            },
        )
        .unwrap()
    }
    pub(crate) fn delay_next(&self, delay: Arc<TestDelay>) {
        *self.delay.lock().unwrap() = Some(delay);
    }
    pub(crate) fn exhaust_bytes_for_test(&self) {
        self.budget.lock().unwrap().bytes = self.manifest.config.max_bytes;
    }
}

#[cfg(test)]
pub(crate) struct TestDelay {
    pub entered: tokio::sync::Notify,
    released: (Mutex<bool>, std::sync::Condvar),
}
#[cfg(test)]
impl TestDelay {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: (Mutex::new(false), std::sync::Condvar::new()),
        })
    }
    fn block(&self) {
        self.entered.notify_one();
        let mut yes = self.released.0.lock().unwrap();
        while !*yes {
            yes = self.released.1.wait(yes).unwrap();
        }
    }
    pub(crate) fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
    pub(crate) fn guard(self: &Arc<Self>) -> TestRelease {
        TestRelease(self.clone())
    }
}
#[cfg(test)]
pub(crate) struct TestRelease(Arc<TestDelay>);
#[cfg(test)]
impl Drop for TestRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(test)]
#[path = "evidence/tests.rs"]
mod tests;
