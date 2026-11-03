//! End-to-end SV1 path with a deterministic template source: connect,
//! authorize, receive a job, mine a share, submit, and assemble a block.

use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use spc_crypto::{Network, SendKey};
use spc_pool::front::{Front, FrontConfig};
use spc_pool::sv1_server::Sv1Server;
use spc_pool::template::{MockTemplateSource, Template, TemplateSource};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
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
        pool_payout_descriptor: test_pool_payout_descriptor(),
        pool_payout_script: Some(
            spc_pool::coinbase::p2tr_script(&[0x99; 32])
                .to_bytes()
                .to_vec(),
        ),
        extranonce_len: 8,
        front_id: 0,
        keep_slices: 16,
    }
}

/// A fail-closed parsed `[pool] pool_payout_descriptor` stand-in for the
/// in-process tests (the explicit script override wins over it here).
fn test_pool_payout_descriptor() -> spc_pool::coinbase::PoolPayoutDescriptor {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let xprv = bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
    let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);
    spc_pool::coinbase::PoolPayoutDescriptor::parse(&format!("tr({xpub}/0/*)"), Network::Regtest)
        .unwrap()
}

#[tokio::test]
async fn sv1_miner_finds_a_block() {
    let source = Arc::new(MockTemplateSource::new(template()));
    let mut front = Front::new(
        config(),
        source.clone(),
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
    );
    let job = front.refresh().await.unwrap();

    let (tx, mut rx) = mpsc::channel(8);
    let server = Sv1Server::new(Network::Regtest, 4, 4, 1.0, tx);
    let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
    server.publish(job.clone()).await;

    let stream = TcpStream::connect(addr).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    // Subscribe.
    write
        .write_all(br#"{"id":1,"method":"mining.subscribe","params":[]}"#)
        .await
        .unwrap();
    write.write_all(b"\n").await.unwrap();
    let sub: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let extranonce1 = sub["result"][1].as_str().unwrap().to_string();
    let extranonce2_size = sub["result"][2].as_u64().unwrap() as usize;
    assert_eq!(extranonce1.len(), 8);
    assert_eq!(extranonce2_size, 4);

    // Authorize with a valid regtest identity.
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
    let address = spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap();
    let auth = serde_json::json!({"id":2,"method":"mining.authorize","params":[address,"x"]});
    write.write_all(auth.to_string().as_bytes()).await.unwrap();
    write.write_all(b"\n").await.unwrap();
    let auth: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(auth["result"], true);

    // set_difficulty then notify.
    let _ = lines.next_line().await.unwrap().unwrap();
    let notify: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let params = notify["params"].as_array().unwrap();
    let job_id = params[0].as_str().unwrap().to_string();
    let coinb1 = hex_decode(params[2].as_str().unwrap());
    let coinb2 = hex_decode(params[3].as_str().unwrap());
    let version = u32::from_str_radix(params[5].as_str().unwrap(), 16).unwrap();
    let nbits = u32::from_str_radix(params[6].as_str().unwrap(), 16).unwrap();
    let ntime = u32::from_str_radix(params[7].as_str().unwrap(), 16).unwrap();

    let extranonce2 = vec![0xaa; extranonce2_size];
    let mut coinbase = coinb1.clone();
    coinbase.extend_from_slice(&hex_decode(&extranonce1));
    coinbase.extend_from_slice(&extranonce2);
    coinbase.extend_from_slice(&coinb2);
    let txid = bitcoin::hashes::sha256d::Hash::hash(&coinbase).to_byte_array();

    let target = Target::from_compact(CompactTarget::from_consensus(nbits));
    let mut nonce = 0u32;
    let local_hash;
    loop {
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&version.to_le_bytes());
        header.extend_from_slice(&job.prev_hash);
        header.extend_from_slice(&txid);
        header.extend_from_slice(&ntime.to_le_bytes());
        header.extend_from_slice(&nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let h = bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array();
        if target.is_met_by(BlockHash::from_byte_array(h)) {
            local_hash = h;
            break;
        }
        nonce = nonce.wrapping_add(1);
        if nonce > 2_000_000 {
            panic!("no nonce found");
        }
    }

    let submit = serde_json::json!({
        "id": 3, "method": "mining.submit",
        "params": ["w", job_id, hex(&extranonce2), format!("{ntime:08x}"), format!("{nonce:08x}")]
    });
    write
        .write_all(submit.to_string().as_bytes())
        .await
        .unwrap();
    write.write_all(b"\n").await.unwrap();
    let submit: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(submit["result"], true, "{submit}");

    let share = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(share.coinbase, coinbase, "coinbase mismatch");
    assert_eq!(share.merkle_root, txid, "merkle mismatch");
    assert_eq!(share.version, version, "version mismatch");
    assert_eq!(share.ntime, ntime, "ntime mismatch");
    assert_eq!(share.nonce, nonce, "nonce mismatch");
    let mut h2 = Vec::with_capacity(80);
    h2.extend_from_slice(&share.version.to_le_bytes());
    h2.extend_from_slice(&job.prev_hash);
    h2.extend_from_slice(&share.merkle_root);
    h2.extend_from_slice(&share.ntime.to_le_bytes());
    h2.extend_from_slice(&job.nbits.to_le_bytes());
    h2.extend_from_slice(&share.nonce.to_le_bytes());
    assert_eq!(
        hex(&bitcoin::hashes::sha256d::Hash::hash(&h2).to_byte_array()),
        hex(&share.share_hash),
        "server hash is not from its own fields"
    );
    assert_eq!(
        hex(&share.share_hash),
        hex(&local_hash),
        "server/local hash mismatch"
    );
    let block = front.on_share(share).unwrap().expect("block");
    assert_eq!(block.header[4..36], job.prev_hash);
    source.submit_block(&block.block).await.unwrap();
    assert_eq!(source.submitted().len(), 1);

    // The pool's own scan can recover the output for this miner.
    let send = SendKey::generate(800_000).unwrap();
    let _ = send; // the job's key is internal; the scan path is covered by spc-scan
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
