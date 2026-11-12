//! Test-only Nix artifact trust boundary. Runtime environment is not authority.
use bitcoin::hashes::{sha256, Hash, HashEngine};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub schema: String,
    pub kind: String,
    pub binary_path: PathBuf,
    pub binary_sha256: String,
    pub binary_bytes: u64,
    pub source_id: String,
    pub source_path: PathBuf,
    pub source_nar_sha256: String,
    pub derivation_path: PathBuf,
    pub derivation_sha256: String,
    pub output_path: PathBuf,
}

struct Anchors<'a> {
    descriptor: &'a str,
    descriptor_hash: &'a str,
    source: &'a str,
    source_id: &'a str,
    nar_hash: &'a str,
    drv: &'a str,
    drv_hash: &'a str,
    output: &'a str,
}

fn anchors() -> Result<Anchors<'static>, String> {
    let missing = || "missing build-generated native joint artifact anchors".to_string();
    Ok(Anchors {
        descriptor: option_env!("SPC_JOINT_DESCRIPTOR").ok_or_else(missing)?,
        descriptor_hash: option_env!("SPC_JOINT_DESCRIPTOR_SHA256").ok_or_else(missing)?,
        source: option_env!("SPC_JOINT_TEST_SOURCE").ok_or_else(missing)?,
        source_id: option_env!("SPC_BUILD_SOURCE_ID").ok_or_else(missing)?,
        nar_hash: option_env!("SPC_JOINT_SOURCE_NAR_SHA256").ok_or_else(missing)?,
        drv: option_env!("SPC_JOINT_CPU_DRV").ok_or_else(missing)?,
        drv_hash: option_env!("SPC_JOINT_CPU_DRV_SHA256").ok_or_else(missing)?,
        output: option_env!("SPC_JOINT_CPU_OUTPUT").ok_or_else(missing)?,
    })
}

fn require(condition: bool, error: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(error.into())
    }
}

fn digest(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn store_root(path: &Path) -> bool {
    let Some(value) = path.to_str().and_then(|s| s.strip_prefix("/nix/store/")) else {
        return false;
    };
    let Some((hash, name)) = value.split_once('-') else {
        return false;
    };
    hash.len() == 32
        && hash
            .bytes()
            .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+._?=-".contains(&b))
}

fn immutable_file(path: &Path, bound: u64, executable: bool) -> Result<std::fs::File, String> {
    require(
        path.canonicalize().map_err(|e| e.to_string())? == path,
        "artifact path alias",
    )?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let mode = meta.permissions().mode();
    require(
        meta.is_file()
            && meta.len() > 0
            && meta.len() <= bound
            && mode & 0o222 == 0
            && (!executable || mode & 0o111 != 0),
        "artifact file metadata",
    )?;
    Ok(file)
}

fn file_hash(mut file: std::fs::File) -> Result<String, String> {
    let mut engine = sha256::Hash::engine();
    let mut chunk = [0; 65536];
    loop {
        let len = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        engine.input(&chunk[..len]);
    }
    Ok(sha256::Hash::from_engine(engine).to_string())
}

fn decode(bytes: &[u8], a: &Anchors<'_>) -> Result<Artifact, String> {
    require(
        bytes.len() <= 8192
            && digest(a.descriptor_hash, 64)
            && sha256::Hash::hash(bytes).to_string() == a.descriptor_hash,
        "descriptor digest",
    )?;
    let d: Artifact = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    require(
        d.schema == "spc.native-joint-artifact/1" && d.kind == "same-source-nix",
        "artifact schema/kind",
    )?;
    require(
        d.source_id == a.source_id
            && (d.source_id == "nix-snapshot"
                || d.source_id
                    .strip_prefix("git:")
                    .is_some_and(|id| digest(id, 40))),
        "artifact source identity",
    )?;
    require(
        d.source_path == Path::new(a.source)
            && store_root(&d.source_path)
            && d.source_nar_sha256 == a.nar_hash
            && digest(&d.source_nar_sha256, 64),
        "artifact source binding",
    )?;
    require(
        d.derivation_path == Path::new(a.drv)
            && store_root(&d.derivation_path)
            && d.derivation_path
                .extension()
                .is_some_and(|ext| ext == "drv")
            && d.derivation_sha256 == a.drv_hash
            && digest(&d.derivation_sha256, 64),
        "artifact derivation binding",
    )?;
    require(
        d.output_path == Path::new(a.output)
            && store_root(&d.output_path)
            && d.binary_path == d.output_path.join("bin/spc-cpu-miner"),
        "artifact binary path",
    )?;
    require(
        digest(&d.binary_sha256, 64) && d.binary_bytes > 0 && d.binary_bytes <= 1024 * 1024 * 1024,
        "artifact binary fields",
    )?;
    Ok(d)
}

fn verify_files(d: &Artifact) -> Result<(), String> {
    require(
        file_hash(immutable_file(&d.derivation_path, 16 * 1024 * 1024, false)?)?
            == d.derivation_sha256,
        "derivation digest",
    )?;
    let file = immutable_file(&d.binary_path, 1024 * 1024 * 1024, true)?;
    require(
        file.metadata().map_err(|e| e.to_string())?.len() == d.binary_bytes,
        "binary size",
    )?;
    require(file_hash(file)? == d.binary_sha256, "binary digest")
}

pub(super) fn load() -> Result<Artifact, String> {
    let a = anchors()?;
    let path = Path::new(a.descriptor);
    require(
        path.file_name()
            .is_some_and(|name| name == "descriptor.json")
            && path.parent().is_some_and(store_root),
        "descriptor store path",
    )?;
    let mut bytes = Vec::new();
    immutable_file(path, 8192, false)?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let d = decode(&bytes, &a)?;
    verify_files(&d)?;
    Ok(d)
}

// These cases run against the build-generated descriptor, including real file
// hashing. Re-anchoring mutated JSON below exercises semantic checks as well
// as the outer digest check; production load() never accepts such anchors.
#[cfg(test)]
mod tests {
    use super::*;

    fn changed(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Artifact, String> {
        let mut value = serde_json::to_value(load().expect("Nix artifact required")).unwrap();
        edit(&mut value);
        let bytes = serde_json::to_vec(&value).unwrap();
        let hash = sha256::Hash::hash(&bytes).to_string();
        let a = Anchors {
            descriptor_hash: &hash,
            ..anchors().unwrap()
        };
        decode(&bytes, &a)
    }

    #[test]
    fn wrong_binary_path_fails() {
        assert!(changed(|v| v["binary_path"] = "/bin/true".into()).is_err());
    }
    #[test]
    fn wrong_binary_digest_fails() {
        let mut d = load().unwrap();
        d.binary_sha256 = "0".repeat(64);
        assert!(verify_files(&d).is_err());
    }
    #[test]
    fn wrong_source_fails() {
        assert!(changed(|v| v["source_id"] = format!("git:{}", "0".repeat(40)).into()).is_err());
        assert!(changed(|v| v["source_path"] = "/wrong-source".into()).is_err());
        assert!(changed(|v| v["source_nar_sha256"] = "0".repeat(64).into()).is_err());
    }
    #[test]
    fn wrong_derivation_fails() {
        assert!(changed(|v| v["derivation_sha256"] = "0".repeat(64).into()).is_err());
        assert!(changed(|v| v["output_path"] = "/wrong-output".into()).is_err());
    }
    #[test]
    fn unknown_schema_and_fields_fail() {
        assert!(changed(|v| v["schema"] = "spc.native-joint-artifact/2".into()).is_err());
        assert!(changed(|v| v["runtime_expected_hash"] = "untrusted".into()).is_err());
    }
    #[test]
    fn wrong_descriptor_digest_fails() {
        let d = load().unwrap();
        assert!(decode(
            &serde_json::to_vec(&d).unwrap(),
            &Anchors {
                descriptor_hash: &"0".repeat(64),
                ..anchors().unwrap()
            }
        )
        .is_err());
    }
    #[test]
    fn missing_artifact_fails() {
        let mut d = load().unwrap();
        d.binary_path = d.output_path.join("absent-artifact/spc-cpu-miner");
        assert!(verify_files(&d).is_err());
        assert!(
            immutable_file(&d.output_path.join("absent-descriptor.json"), 8192, false).is_err()
        );
    }
}
