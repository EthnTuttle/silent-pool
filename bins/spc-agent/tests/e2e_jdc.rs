use spc_ext::{SetPayoutDistribution, TxOut};
use spc_pool::jds_server::JdsServer;

#[tokio::test]
async fn jdc_receives_the_published_distribution() {
    let server = JdsServer::new(1);
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
    let dist = SetPayoutDistribution {
        distribution_id: 0,
        pool_payout: TxOut {
            script: vec![0x51, 0x20, 1, 1],
            value: 100,
        },
        payouts: vec![TxOut {
            script: vec![0x51, 0x20, 2, 2],
            value: 300,
        }],
        dust_limits: vec![330],
        additional_outputs: vec![],
    };
    let id = server.publish(800_000, dist.clone()).await.unwrap();
    let jdc = spc_agent::jdc::Jdc::connect(addr).await.unwrap();
    let got = jdc.last().unwrap();
    assert_eq!(got.distribution_id, id);
    assert_eq!(got.payouts.len(), 1);
    assert_eq!(got.pool_payout.value, 100);
}

#[tokio::test]
async fn jds_accepts_a_declaration_with_the_current_id() {
    let server = JdsServer::new(0);
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
    let dist = SetPayoutDistribution {
        distribution_id: 0,
        pool_payout: TxOut {
            script: vec![0x51, 0x20, 1, 1],
            value: 100,
        },
        payouts: vec![],
        dust_limits: vec![],
        additional_outputs: vec![],
    };
    let id = server.publish(1, dist).await.unwrap();
    let mut jdc = spc_agent::jdc::Jdc::connect(addr).await.unwrap();
    let token = jdc.allocate_token("dev").await.unwrap();
    let (prefix, suffix) =
        spc_agent::jdc::Jdc::build_coinbase(&dist_for_build(), 1_000, 1, &[7u8; 32], 8).unwrap();
    jdc.declare_job(&token, id, &prefix, &suffix).await.unwrap();

    for _ in 0..50 {
        if server.accepted().await == Some(id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("JDS did not accept the declaration");
}

fn dist_for_build() -> SetPayoutDistribution {
    SetPayoutDistribution {
        distribution_id: 0,
        pool_payout: TxOut {
            script: vec![0x51, 0x20, 1, 1],
            value: 100,
        },
        payouts: vec![TxOut {
            script: vec![0x51, 0x20, 2, 2],
            value: 300,
        }],
        dust_limits: vec![0],
        additional_outputs: vec![],
    }
}
