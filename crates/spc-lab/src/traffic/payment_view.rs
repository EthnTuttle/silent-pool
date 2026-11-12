//! Authenticated, source-fresh T1 projection. The caller never supplies a
//! driver, wallet index, path or balance. Kept distinct from SP/Ark earnings.
use crate::access::AuthContext;
use crate::scenario::Scenario;
use serde_json::Value;
use spc_lab_txd::actor_config::AllocationDescription;
pub use spc_lab_txd::payment_view::PaymentWalletView;
use spc_lab_txd::payment_view::{PaymentRead, PaymentReply, MAX_AGE_MS, SCHEMA};

pub fn authorize(auth: &AuthContext, scenario: &Scenario, user: &str) -> Result<(), String> {
    if auth.principal != auth.claim.principal
        || auth.claim.run_id != scenario.run.id
        || (!auth.claim.is_operator() && auth.claim.user_id.as_deref() != Some(user))
        || !scenario.identities.iter().any(|i| i.id.as_str() == user)
    {
        return Err("payment_view_scope_denied".into());
    }
    Ok(())
}

pub fn read_scoped(
    auth: &AuthContext,
    scenario: &Scenario,
    user: &str,
    allocation: &AllocationDescription,
    read: impl FnOnce(&str, PaymentRead) -> Result<Value, String>,
) -> Result<PaymentWalletView, String> {
    authorize(auth, scenario, user)?;
    if allocation.run_id != scenario.run.id.as_str() || allocation.run_seed != scenario.run.seed {
        return Err("payment_view_allocation_mismatch".into());
    }
    let mut users: Vec<_> = scenario
        .identities
        .iter()
        .map(|i| i.id.to_string())
        .collect();
    users.sort();
    if users != allocation.users {
        return Err("payment_view_identity_allocation_mismatch".into());
    }
    let token = allocation
        .reconstruct()
        .map_err(|_| "payment_view_allocation_invalid")?;
    let (driver, index) = allocation
        .owner_of_user(user)
        .map_err(|_| "payment_view_user_unallocated")?;
    let request = PaymentRead {
        user_id: user.into(),
        wallet_index: index,
        allocation_digest: token.digest().into(),
    };
    let value = read(driver, request.clone())?;
    let reply: PaymentReply =
        serde_json::from_value(value).map_err(|_| "payment_view_reply_invalid")?;
    validate_reply(
        reply,
        driver,
        &request,
        spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?,
        &spc_lab_txd::bootstrap::boot_id().map_err(|e| e.to_string())?,
    )
}

fn validate_reply(
    mut reply: PaymentReply,
    driver: &str,
    request: &PaymentRead,
    now: u64,
    boot: &str,
) -> Result<PaymentWalletView, String> {
    let view = &mut reply.view;
    if reply.driver_id != driver
        || reply.wallet_index != request.wallet_index
        || reply.allocation_digest != request.allocation_digest
        || view.user_id != request.user_id
        || view.schema != SCHEMA
        || view.source != "T1 test-payment funds"
        || !matches!(view.state.as_str(), "available" | "not_available" | "stale")
        || !matches!(view.stage.as_str(), "active" | "not_active")
        || view.unconfirmed_sats.is_some()
        || view.pending_sats.is_some()
    {
        return Err("payment_view_reply_binding".into());
    }
    if let Some(amount) = &view.spendable_sats {
        if !amount
            .parse::<u64>()
            .is_ok_and(|n| n.to_string() == *amount)
        {
            return Err("payment_view_amount_invalid".into());
        }
    }
    if let Some(address) = &view.receive_address {
        use std::str::FromStr;
        if !bitcoin::Address::from_str(address)
            .ok()
            .and_then(|a| a.require_network(bitcoin::Network::Regtest).ok())
            .is_some_and(|a| a.script_pubkey().is_p2pkh())
        {
            return Err("payment_view_address_invalid".into());
        }
    }
    let age = reply
        .observation_monotonic_ms
        .and_then(|stamp| now.checked_sub(stamp));
    view.source_age_ms = age;
    let fresh = reply.observation_boot_id.as_deref() == Some(boot)
        && age.is_some_and(|age| age <= MAX_AGE_MS);
    if !fresh {
        view.state = if reply.observation_monotonic_ms.is_some() {
            "stale"
        } else {
            "not_available"
        }
        .into();
        view.reason = Some("observation_stale_or_unavailable".into());
        view.spendable_sats = None;
        view.receive_address = None;
    } else if view.state == "available"
        && (view.stage != "active"
            || view.spendable_sats.is_none()
            || view.receive_address.is_none()
            || view.source_observed_at_ms.is_none())
    {
        return Err("payment_view_available_without_facts".into());
    }
    Ok(reply.view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{AccessClaim, ACCESS_CLAIM_SCHEMA};
    use bitcoin::hashes::Hash;
    use spc_lab_txd::actor_config::AllocationActor;
    fn source() -> (Scenario, AllocationDescription, AuthContext) {
        let scenario: Scenario = serde_json::from_str(include_str!(
            "../../tests/fixtures/scenario_v1_two_pools.json"
        ))
        .unwrap();
        let mut users: Vec<_> = scenario
            .identities
            .iter()
            .map(|i| i.id.to_string())
            .collect();
        users.sort();
        let claim:AccessClaim=serde_json::from_value(serde_json::json!({"schema":ACCESS_CLAIM_SCHEMA,"run_id":scenario.run.id,"principal":users[0],"user_id":users[0],"actor":users[0],"token":"aa".repeat(32),"may":["view_own_status"]})).unwrap();
        let allocation = AllocationDescription {
            run_id: scenario.run.id.to_string(),
            run_seed: scenario.run.seed,
            network: bitcoin::Network::Regtest,
            caps: Default::default(),
            users,
            actors: vec![
                AllocationActor {
                    driver_id: "a".into(),
                    network_wallets: 1,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: true,
                    lookahead: 2,
                    fee_policy: Default::default(),
                },
                AllocationActor {
                    driver_id: "b".into(),
                    network_wallets: 1,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: false,
                    lookahead: 2,
                    fee_policy: Default::default(),
                },
            ],
        };
        let auth = AuthContext {
            principal: claim.principal.clone(),
            claim,
        };
        (scenario, allocation, auth)
    }
    fn reply(driver: &str, r: &PaymentRead) -> PaymentReply {
        PaymentReply {
            driver_id: driver.into(),
            wallet_index: r.wallet_index,
            allocation_digest: r.allocation_digest.clone(),
            observation_boot_id: Some("boot".into()),
            observation_monotonic_ms: Some(100),
            journal_seq: 5,
            view: PaymentWalletView {
                schema: SCHEMA.into(),
                user_id: r.user_id.clone(),
                state: "available".into(),
                source: "T1 test-payment funds".into(),
                stage: "active".into(),
                spendable_sats: Some("9007199254741115".into()),
                unconfirmed_sats: None,
                pending_sats: None,
                receive_address: Some(
                    bitcoin::Address::p2pkh(
                        bitcoin::PubkeyHash::from_byte_array([7; 20]),
                        bitcoin::Network::Regtest,
                    )
                    .to_string(),
                ),
                source_observed_at_ms: Some(1000),
                source_age_ms: None,
                reason: None,
            },
        }
    }
    #[test]
    fn user_scope_precedes_io_operator_mapping_is_explicit_and_wrong_driver_refuses() {
        let (scenario, allocation, mut auth) = source();
        let alice = allocation.users[0].clone();
        let bob = allocation.users[1].clone();
        assert!(
            read_scoped(&auth, &scenario, &bob, &allocation, |_, _| panic!(
                "cross-user I/O"
            ))
            .is_err()
        );
        assert!(
            read_scoped(&auth, &scenario, &alice, &allocation, |driver, r| {
                assert_eq!(driver, "a");
                assert_eq!(r.wallet_index, 0);
                Ok(serde_json::to_value(reply("b", &r)).unwrap())
            })
            .is_err()
        );
        auth.claim.may = vec!["operator".into()];
        auth.principal = "operator".into();
        auth.claim.principal = auth.principal.clone();
        let view = read_scoped(&auth, &scenario, &bob, &allocation, |driver, r| {
            assert_eq!(driver, "b");
            assert_eq!(r.wallet_index, 1);
            let mut p = reply(driver, &r);
            p.observation_boot_id = Some(spc_lab_txd::bootstrap::boot_id().unwrap());
            p.observation_monotonic_ms = Some(spc_lab_txd::bootstrap::monotonic_ms().unwrap());
            Ok(serde_json::to_value(p).unwrap())
        })
        .unwrap();
        assert_eq!(view.user_id, bob);
        assert_eq!(view.state, "available");
        assert!(
            read_scoped(&auth, &scenario, "unknown", &allocation, |_, _| panic!(
                "undeclared I/O"
            ))
            .is_err()
        );
    }
    #[test]
    fn stale_unknown_wrong_index_and_exact_large_decimal_fail_closed() {
        let request = PaymentRead {
            user_id: "alice".into(),
            wallet_index: 0,
            allocation_digest: "aa".repeat(32),
        };
        let valid = reply("a", &request);
        let view = validate_reply(valid.clone(), "a", &request, 101, "boot").unwrap();
        assert_eq!(
            serde_json::to_value(view).unwrap()["spendable_sats"],
            "9007199254741115"
        );
        for (now, boot) in [(15101, "boot"), (99, "boot"), (101, "different")] {
            let view = validate_reply(valid.clone(), "a", &request, now, boot).unwrap();
            assert_eq!(view.state, "stale");
            assert!(view.spendable_sats.is_none());
            assert!(view.receive_address.is_none());
        }
        let mut bad = valid.clone();
        bad.wallet_index = 1;
        assert!(validate_reply(bad, "a", &request, 101, "boot").is_err());
        let mut bad = valid.clone();
        bad.view.unconfirmed_sats = Some("0".into());
        assert!(validate_reply(bad, "a", &request, 101, "boot").is_err());
        for amount in ["1.0", "01", "18446744073709551616", "-1"] {
            let mut bad = valid.clone();
            bad.view.spendable_sats = Some(amount.into());
            assert!(validate_reply(bad, "a", &request, 101, "boot").is_err());
        }
        let mut malformed = serde_json::to_value(valid).unwrap();
        malformed["raw_recovery_map"] = serde_json::json!({});
        assert!(serde_json::from_value::<PaymentReply>(malformed).is_err());
    }
}
