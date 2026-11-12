//! EXP-OP operator runtime wiring: the host CLI flag is off by default,
//! rejects the namespace topology explicitly, and reaches `Runtime::prepare`
//! where the per-front api-role requirement fails closed.

use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spc-lab"))
}

fn base_runtime_args(dir: &std::path::Path, scenario: &std::path::Path) -> Vec<String> {
    let dummy = |name: &str| dir.join(name).to_string_lossy().into_owned();
    for name in ["node", "pool", "miner", "redis-server", "redis-cli"] {
        std::fs::write(dir.join(name), b"").unwrap();
    }
    let pg = dir.join("pg");
    std::fs::create_dir_all(&pg).unwrap();
    for tool in ["initdb", "postgres", "createdb", "psql"] {
        std::fs::write(pg.join(tool), b"").unwrap();
    }
    vec![
        "runtime".to_string(),
        "--scenario".to_string(),
        scenario.to_string_lossy().into_owned(),
        "--run-dir".to_string(),
        dir.join("run").to_string_lossy().into_owned(),
        "--node".to_string(),
        dummy("node"),
        "--pool".to_string(),
        dummy("pool"),
        "--miner".to_string(),
        dummy("miner"),
        "--postgres-bin-dir".to_string(),
        pg.to_string_lossy().into_owned(),
        "--redis-server".to_string(),
        dummy("redis-server"),
        "--redis-cli".to_string(),
        dummy("redis-cli"),
        "--base-revision".to_string(),
        "cli-test".to_string(),
        "--dirty-tree-hash".to_string(),
        "clean".to_string(),
    ]
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "spc-lab-cli-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn runtime_help_documents_the_off_by_default_operator_ui_flag() {
    let output = lab().args(["runtime", "--help"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("--operator-ui"), "help text: {text}");
}

#[test]
fn operator_ui_conflicts_with_topology() {
    let dir = temp_dir("conflict");
    let mut args = base_runtime_args(&dir, dir.join("scenario.json").as_path());
    args.push("--operator-ui".to_string());
    args.push("--topology".to_string());
    let output = lab().args(&args).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("cannot be used with"),
        "conflict error: {text}"
    );
}

#[test]
fn operator_ui_flag_reaches_runtime_prepare_and_fails_closed_without_api_role() {
    let dir = temp_dir("reaches-prepare");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/spc-lab/tests/fixtures/scenario_v1_one_pool_two_miners.json"
    );
    let no_api = std::fs::read_to_string(fixture).unwrap().replace(
        "\"roles\": [\"front\", \"payout\", \"api\"]",
        "\"roles\": [\"front\", \"payout\"]",
    );
    let scenario = dir.join("scenario-no-api.json");
    std::fs::write(&scenario, no_api).unwrap();
    let mut args = base_runtime_args(&dir, &scenario);
    args.push("--operator-ui".to_string());
    let output = lab().args(&args).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("no api role"), "prepare error: {text}");
    assert!(
        !dir.join("run").exists(),
        "the rejection must happen before the run directory exists"
    );
}
