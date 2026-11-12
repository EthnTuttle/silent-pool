//! Host-local kernel/config facts for the native wallet endpoint. Requests never
//! supply paths or observed process facts. Crypto/wire types are the owner codec.
use anyhow::{ensure, Context};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use spc_scan::native_activation::{self as wire, ProcessIdentity};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) fn hash(bytes: &[u8]) -> [u8; 32] {
    sha256::Hash::hash(bytes).to_byte_array()
}
pub(super) fn uid() -> anyhow::Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}
pub(super) fn private_dir(path: &Path, uid: u32) -> anyhow::Result<()> {
    ensure!(
        path.is_absolute() && path.canonicalize()? == path,
        "native directory path"
    );
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && m.uid() == uid && m.mode() & 0o777 == 0o700,
        "native directory ownership"
    );
    Ok(())
}
pub(super) fn private_read(path: &Path, uid: u32) -> anyhow::Result<Vec<u8>> {
    ensure!(path.canonicalize()? == path, "native input path");
    private_dir(path.parent().context("native input parent")?, uid)?;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file()
            && m.uid() == uid
            && m.mode() & 0o777 == 0o600
            && m.nlink() == 1
            && m.len() <= wire::MAX_REQUEST_BYTES as u64,
        "native input ownership/bound"
    );
    let mut bytes = vec![];
    (&mut f)
        .take(wire::MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let a = f.metadata()?;
    let current = std::fs::symlink_metadata(path)?;
    ensure!(
        bytes.len() as u64 == m.len()
            && a.len() == m.len()
            && a.mtime() == m.mtime()
            && a.mtime_nsec() == m.mtime_nsec()
            && a.ctime() == m.ctime()
            && a.ctime_nsec() == m.ctime_nsec()
            && current.dev() == m.dev()
            && current.ino() == m.ino(),
        "native input changed during read"
    );
    Ok(bytes)
}
pub(super) fn atomic_private(path: &Path, bytes: &[u8], uid: u32) -> anyhow::Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= wire::MAX_REQUEST_BYTES,
        "native publication bound"
    );
    let parent = path.parent().context("native publication parent")?;
    private_dir(parent, uid)?;
    if path.symlink_metadata().is_ok() {
        private_read(path, uid)?;
    }
    let temporary = parent.join(format!(
        ".native-wallet-{}",
        hex::encode(rand::random::<[u8; 16]>())
    ));
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
fn bounded(path: &Path, max: u64) -> anyhow::Result<Vec<u8>> {
    let mut b = vec![];
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut b)?;
    ensure!(b.len() as u64 <= max, "native proc bound");
    Ok(b)
}
pub(super) fn start_ticks(pid: u32) -> anyhow::Result<u64> {
    let b = bounded(Path::new(&format!("/proc/{pid}/stat")), 16384)?;
    let s = std::str::from_utf8(&b)?;
    let fields = s
        .get(s.rfind(')').context("native proc stat")? + 1..)
        .context("native proc stat")?
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure!(
        fields.first().is_some_and(|s| *s != "Z" && *s != "X"),
        "native process not live"
    );
    Ok(fields.get(19).context("native start token")?.parse()?)
}
pub(super) fn process(
    pid: u32,
    expected_uid: u32,
    private: &Path,
    expected_config: Option<&Path>,
    run_id: &str,
) -> anyhow::Result<(ProcessIdentity, PathBuf)> {
    let before = start_ticks(pid)?;
    let proc = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        std::fs::metadata(&proc)?.uid() == expected_uid,
        "native kernel UID mismatch"
    );
    ensure!(
        std::fs::read_link(proc.join("ns/net"))? == std::fs::read_link("/proc/self/ns/net")?,
        "native namespace unsupported"
    );
    let cmd = bounded(&proc.join("cmdline"), 1024 * 1024)?;
    let args = cmd
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    let paths = args
        .windows(2)
        .filter(|w| w[0] == b"--config")
        .collect::<Vec<_>>();
    ensure!(paths.len() == 1, "native process config ambiguous");
    let path = PathBuf::from(std::str::from_utf8(paths[0][1])?);
    ensure!(
        path.is_absolute()
            && path.starts_with(private)
            && expected_config.is_none_or(|p| p == path),
        "native config scope mismatch"
    );
    let config = private_read(&path, expected_uid)?;
    let env = zeroize::Zeroizing::new(bounded(&proc.join("environ"), 1024 * 1024)?);
    let tokens = env
        .split(|b| *b == 0)
        .filter_map(|v| v.strip_prefix(b"SPC_LAB_RUN_TOKEN="))
        .collect::<Vec<_>>();
    ensure!(
        tokens.len() == 1 && tokens[0].starts_with(format!("{run_id}:").as_bytes()),
        "native process run scope"
    );
    let mut executable = std::fs::File::open(proc.join("exe"))?;
    let mut engine = sha256::Hash::engine();
    let mut chunk = [0u8; 65536];
    loop {
        let n = executable.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        engine.input(&chunk[..n]);
    }
    let observed = ProcessIdentity {
        pid,
        uid: expected_uid,
        start_time_ticks: before,
        boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .into(),
        executable_sha256: sha256::Hash::from_engine(engine).to_byte_array(),
        run_token_sha256: hash(tokens[0]),
        config_sha256: hash(&config),
    };
    observed.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        start_ticks(pid)? == before && std::fs::metadata(&proc)?.uid() == expected_uid,
        "native process changed during observation"
    );
    Ok((observed, path))
}
