//! Hash-bound, public canonical reports from the actual sealed metadata node.
//! This checks the production J1 consumer; it is not fresh-run J2 acceptance.
mod common;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spc_lab::traffic::projection::{
    check_traffic_capability, project_traffic_capability, AP1A_HISTORICAL_R2_CONTRACT_SHA256,
    AP1A_TRAFFIC_CONTRACT_SHA256, TRAFFIC_CONSUMER,
};

fn scenario() -> spc_lab::scenario::Scenario {
    let mut scenario = common::fixture_scenario("scenario_v1_two_pools.json");
    scenario.traffic=Some(serde_json::from_value(json!({
        "version":1,"required_capability":"spc.node.regtest.mempool_rpc.v1",
        "protocol":{"control":"spc-lab-tx-control/v1","history_version":3},
        "bootstrap":{"enabled":true,"mining_mode":"native","funding_blocks":101,"distribution_outputs_per_wallet":8,"excluded_from_statistics":true},
        "driver":{"actors":[{"id":"txd-0","network_wallets":8,"user_payment_wallets":scenario.identities.len(),"hosts_bootstrap_funding":true}],
            "user_payment_wallets":scenario.identities.len(),"history":"traffic/"},
        "rate":{"ordinary_tx_per_sec":0.2,"pacing":"fixed_jitter"},
        "fees":{"rate_sats_vb":{"min":1,"max":20},"max_fee_sats_per_tx":100000},
        "sizes":{"classes_vb":[{"vb":220,"mix":1.0}],"max_vb":1000},"spend_types":["p2pkh"],
        "ark":{"enabled":false,"registration_before_mining":true,"chain_effect_budget_tx_per_sec":0.0},
        "caps":{"max_inflight_broadcasts":1,"max_unknown_retries":3,"max_mempool_backlog_tx":1000,"reorg_resubmit_max":3},
        "pause_semantics":"no_broadcast_after_ack"
    })).unwrap());
    scenario.validate().unwrap();
    scenario
}

#[test]
#[ignore = "requires immutable actual-node reports; explicitly run by the hash-bound joined gate"]
fn actual_default_and_compat_reports_recognize_without_funded_proof() {
    let root = std::path::PathBuf::from(
        std::env::var_os("TRAFFIC_J1_REPORT_ROOT").expect("explicit immutable report root"),
    );
    let scenario = scenario();
    let mut consensus = None;
    for (case, expected_hash) in [
        (
            "default",
            "4b12400733335ba11192b0646a8e8cd3bfe16c7fcbcfe8c58fbce1aee88052f0",
        ),
        (
            "bark-v31",
            "98595842284f331d9069f5edc41d919f303b404a132594441a6330d9883bf4c3",
        ),
    ] {
        let bytes = std::fs::read(root.join(case).join("cli/report.json")).unwrap();
        assert_eq!(hex::encode(Sha256::digest(&bytes)), expected_hash);
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        let record = project_traffic_capability(&bytes).unwrap();
        assert_eq!(record.contract_hash, AP1A_TRAFFIC_CONTRACT_SHA256);
        assert_eq!(record.traffic_consumer.as_deref(), Some(TRAFFIC_CONSUMER));
        assert_eq!(
            record.node_binary_sha256.as_deref(),
            Some("36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9")
        );
        assert!(record.recognized, "{case}");
        assert!(!record.behavior_proven);
        assert!(!record.verdict.full_acceptance);
        assert_eq!(
            record.consumer_model_failures,
            report["consumer_model_failures"]
                .as_array()
                .unwrap()
                .clone()
        );
        assert!(record.consumer_model_failures.iter().any(
            |row| row["detail"] == "shape: maxdatacarriersize: expected JSON integer, got null"
        ));
        check_traffic_capability(&scenario, Some(&record)).unwrap();
        let facts = report["node"]["effective_consensus"].clone();
        if let Some(previous) = &consensus {
            assert_eq!(previous, &facts);
        } else {
            consensus = Some(facts);
        }

        for field in ["traffic_consumer", "consumer_model_failures"] {
            let mut bad = report.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(project_traffic_capability(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        for id in [
            "getmempoolinfo.traffic-consumer",
            "getrawtransaction.blockhash.traffic-consumer",
            "getblockheader.traffic-consumer",
            "getbestblockhash",
        ] {
            let mut bad = report.clone();
            bad["checks"]
                .as_array_mut()
                .unwrap()
                .retain(|r| r["id"] != id);
            let missing = project_traffic_capability(&serde_json::to_vec(&bad).unwrap()).unwrap();
            assert!(!missing.recognized);
            assert!(check_traffic_capability(&scenario, Some(&missing)).is_err());
        }
        let mut bad = report.clone();
        bad["contract_hash"] = json!("aa".repeat(32));
        let unknown = project_traffic_capability(&serde_json::to_vec(&bad).unwrap()).unwrap();
        assert_eq!(
            check_traffic_capability(&scenario, Some(&unknown))
                .unwrap_err()
                .code(),
            "traffic_capability_lineage_unknown"
        );
        bad["contract_hash"] = json!(AP1A_HISTORICAL_R2_CONTRACT_SHA256);
        let mixed = project_traffic_capability(&serde_json::to_vec(&bad).unwrap()).unwrap();
        assert_eq!(
            check_traffic_capability(&scenario, Some(&mixed))
                .unwrap_err()
                .code(),
            "traffic_capability_contract_historical"
        );
        let mut forged = record;
        forged.behavior_proven = true;
        assert!(check_traffic_capability(&scenario, Some(&forged)).is_err());
    }
}
