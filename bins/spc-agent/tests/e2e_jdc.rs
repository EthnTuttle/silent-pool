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
    use spc_ext::DistributionIdTlv;
    use stratum_core::binary_sv2::{Seq064K, B0255, B064K, U256};
    use stratum_core::framing_sv2::framing::MessageFrame;
    use stratum_core::job_declaration_sv2::{DeclareMiningJob, MESSAGE_TYPE_DECLARE_MINING_JOB};

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

    let tlv = DistributionIdTlv { id }.encode();
    let msg = DeclareMiningJob {
        request_id: 5,
        mining_job_token: B0255::new(b"token").unwrap(),
        version: 0x2000_0000,
        coinbase_tx_prefix: B064K::new(&[]).unwrap(),
        coinbase_tx_suffix: B064K::new(&[]).unwrap(),
        wtxid_list: Seq064K::try_from(Vec::<U256>::new()).unwrap(),
        excess_data: B064K::new(&tlv).unwrap(),
    };
    let frame = MessageFrame::from_message(msg, MESSAGE_TYPE_DECLARE_MINING_JOB, 0, false).unwrap();
    jdc.write_half().write_frame(frame).await.unwrap();

    for _ in 0..50 {
        if server.accepted().await == Some(id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("JDS did not accept the declaration");
}
