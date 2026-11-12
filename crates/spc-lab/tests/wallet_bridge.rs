use serde_json::json;
use spc_lab::scenario::Scenario;
use spc_lab::wallet_bridge::{status_ready, validate};

fn scenario() -> Scenario {
    serde_json::from_str(include_str!("../scenarios/lab-ark-wallets.json")).unwrap()
}

#[test]
fn four_explicit_wallets_do_not_change_worker_hash_budgets() {
    let s = scenario();
    s.validate().unwrap();
    assert_eq!(s.wallet_agents.len(), 4);
    assert_eq!(s.identities.len(), 4);
    assert_eq!(s.miners.len(), 4);
    assert_eq!(s.miners.iter().map(|m| m.workers.len()).sum::<usize>(), 4);
    assert_eq!(
        s.miners
            .iter()
            .flat_map(|m| &m.workers)
            .map(|w| w
                .budget
                .hashes_per_second
                .unwrap()
                .to_string()
                .parse::<u64>()
                .unwrap())
            .sum::<u64>(),
        8
    );
}

#[test]
fn missing_duplicate_and_wrong_pool_wallets_reject_before_launch() {
    let mut s = scenario();
    s.wallet_agents.pop();
    assert!(validate(&s).is_err());
    let mut s = scenario();
    s.wallet_agents.push(s.wallet_agents[0].clone());
    assert!(validate(&s).is_err());
    let mut s = scenario();
    s.wallet_agents[0].front_id = "front-b1".parse().unwrap();
    assert!(validate(&s).is_err());
    // AP-4 enables every pool in a local-provider run even when the original
    // infrastructure scenario omitted optional pool.ark_backend_id.
    let mut s = scenario();
    for p in &mut s.pools {
        p.ark_backend_id = None;
    }
    s.wallet_agents.clear();
    assert!(validate(&s).is_err());
}

#[test]
fn process_or_historical_ack_is_not_current_readiness() {
    assert!(!status_ready(
        &json!({"state":"connected","registered":true}),
        100
    ));
    assert!(!status_ready(
        &json!({"state":"connected","registered":true,"registration_live_until_unix_ms":99}),
        100
    ));
    assert!(!status_ready(
        &json!({"state":"dormant","registered":true,"registration_live_until_unix_ms":200}),
        100
    ));
    assert!(!status_ready(
        &json!({"state":"connected","registered":false,"registration_live_until_unix_ms":200}),
        100
    ));
    assert!(status_ready(
        &json!({"state":"connected","registered":true,"registration_live_until_unix_ms":200}),
        100
    ));
}
