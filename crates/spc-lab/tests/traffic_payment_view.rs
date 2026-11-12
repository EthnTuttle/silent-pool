//! Actual authenticated HTTP adapter, with no native provider or processes.
//! Positive money proof remains the actor read-model and funded runtime's job.
mod common;
use spc_lab::access::{AccessClaim, AuthContext};
use spc_lab::http::{HttpServer, WebAssets};
use std::io::{Read, Write};

fn get(addr: std::net::SocketAddr, path: &str, token: Option<&str>) -> (u16, serde_json::Value) {
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let end = bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
    let status = std::str::from_utf8(&bytes[..end])
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes[end + 4..]).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn authenticated_projection_is_own_only_and_absent_native_provider_stays_unavailable() {
    let dir = common::TestDir::new("t1-payment-view");
    let mut scenario = common::fixture_scenario("scenario_v1_one_pool_two_miners.json");
    scenario.expanded_access = true;
    let engine = common::prepare_engine_with(&dir, scenario, common::fixture_config());
    let server = HttpServer::start(engine.clone(), "127.0.0.1:0", WebAssets::fallback()).unwrap();
    let claims: Vec<AccessClaim> = std::fs::read_dir(dir.join("private/access"))
        .unwrap()
        .map(|p| serde_json::from_slice(&std::fs::read(p.unwrap().path()).unwrap()).unwrap())
        .collect();
    let user = claims.iter().find(|c| c.is_user()).unwrap();
    let other = claims
        .iter()
        .find(|c| c.is_user() && c.principal != user.principal)
        .unwrap();
    let operator = claims.iter().find(|c| c.is_operator()).unwrap();
    let address = server
        .url()
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    let prefix = format!("/api/lab/v1/runs/{}", engine.run_id());
    assert_eq!(get(address, &format!("{prefix}/projection"), None).0, 401);
    let before = engine.manifest_snapshot();
    let commands = std::fs::read(&before.artifact_paths.private_commands).unwrap();
    for _ in 0..2 {
        let (status, value) = get(address, &format!("{prefix}/projection"), Some(&user.token));
        assert_eq!(status, 200);
        assert_eq!(value["user_id"], user.user_id.as_ref().unwrap().as_str());
        assert_eq!(
            value["payment_wallet"]["user_id"],
            user.user_id.as_ref().unwrap().as_str()
        );
        assert_eq!(value["payment_wallet"]["state"], "not_available");
        assert!(value["payment_wallet"]["spendable_sats"].is_null());
        assert!(value["payment_wallet"]["unconfirmed_sats"].is_null());
    }
    assert_eq!(
        get(
            address,
            &format!(
                "{prefix}/projection?user_id={}",
                other.user_id.as_ref().unwrap()
            ),
            Some(&user.token)
        )
        .0,
        400
    );
    let auth = AuthContext {
        principal: user.principal.clone(),
        claim: user.clone(),
    };
    assert!(engine
        .t1_payment_wallet(&auth, other.user_id.as_ref().unwrap())
        .is_err());
    assert_eq!(
        std::fs::read(&before.artifact_paths.private_commands).unwrap(),
        commands
    );
    let (_, catalog) = get(address, &format!("{prefix}/catalog"), Some(&operator.token));
    assert_eq!(catalog["traffic_controls"]["available"], false);
    assert_eq!(
        catalog["traffic_controls"]["drivers"],
        serde_json::json!([])
    );
    assert_eq!(
        get(address, &format!("{prefix}/catalog"), Some(&user.token)).0,
        403
    );
}
