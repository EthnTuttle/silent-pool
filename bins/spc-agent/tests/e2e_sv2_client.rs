use std::sync::Arc;

use bitcoin::hashes::Hash;
use std::time::Duration;

use spc_agent::sv2_client::{share_hash, PoolShare, Sv2Client};
use spc_crypto::Network;
use spc_pool::front::{Front, FrontConfig};
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template, TemplateSource};
use tokio::sync::mpsc;

fn template() -> Template {
    Template {
        height: 800_000,
        prev_hash: [0x11; 32],
        version: 0x2000_0000,
        bits: 0x207f_ffff,
        curtime: 1_700_000_000,
        subsidy: 312_500_000,
        fees: 0,
        coinbase_value: 312_500_000,
        transactions: vec![],
        merkle_branches: vec![],
        coinbase_output_max_additional_size: 50_000,
    }
}

fn config() -> FrontConfig {
    FrontConfig {
        network: Network::Regtest,
        fee_bps: 100,
        window_n: 8,
        slice_fee_delta_sats: 0,
        max_slice_shares: 65_000,
        dust_limit_sats: 330,
        coinbase_threshold_sats: 330,
        max_payout_slots: 256,
        budget_wu: 100_000,
        overhead_wu: 0,
        pool_payout_script: spc_pool::coinbase::p2tr_script(&[0x99; 32])
            .to_bytes()
            .to_vec(),
        extranonce_len: 8,
        front_id: 0,
        keep_slices: 16,
    }
}

#[tokio::test]
async fn agent_client_gets_a_job_and_submits_a_block() {
    let source = Arc::new(MockTemplateSource::new(template()));
    let mut front = Front::new(config(), source.clone());
    let job = front.refresh().await.unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let server = Sv2Server::new(Network::Regtest, tx);
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
    server.publish(job).await;

    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
    let address = spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap();

    let mut client = Sv2Client::connect(addr, &address, Network::Regtest)
        .await
        .unwrap();
    let job = client.next_job().await.unwrap();
    assert_eq!(client.extranonce_prefix.len(), 4);
    assert_eq!(client.extranonce_size, 4);

    let extranonce = vec![0xaa; 4];
    let mut coinbase = job.coinbase_prefix.clone();
    coinbase.extend_from_slice(&client.extranonce_prefix);
    coinbase.extend_from_slice(&extranonce);
    coinbase.extend_from_slice(&job.coinbase_suffix);

    let target = spc_pool::sv2_server::target_from_bits(job.nbits);
    let mut nonce = 0u32;
    let mut found = false;
    while nonce < 2_000_000 {
        let h = share_hash(
            job.version,
            job.prev_hash,
            &coinbase,
            &job.merkle_path,
            job.ntime,
            job.nbits,
            nonce,
        );
        if target.is_met_by(bitcoin::BlockHash::from_byte_array(h)) {
            found = true;
            break;
        }
        nonce += 1;
    }
    assert!(found, "no nonce");

    client
        .submit(PoolShare {
            job_id: job.job_id,
            nonce,
            ntime: job.ntime,
            version: job.version,
            extranonce,
            sequence_number: 0,
        })
        .await
        .unwrap();

    let share = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let block = front.on_share(share).unwrap().expect("block");
    source.submit_block(&block.block).await.unwrap();
    assert_eq!(source.submitted().len(), 1);
}
