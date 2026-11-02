use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use spc_agent::config::{AgentConfig, Downstream, Upstream};
use spc_crypto::Network;
use spc_pool::front::{Front, FrontConfig};
use spc_pool::sv2_server::Sv2Server;
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

fn front_config() -> FrontConfig {
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
async fn sv1_miner_reaches_the_pool_through_the_translator() {
    let source = Arc::new(MockTemplateSource::new(template()));
    let mut front = Front::new(
        front_config(),
        source.clone(),
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
    );
    let job = front.refresh().await.unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let server = Sv2Server::new(
        Network::Regtest,
        tx,
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
        8,
    );
    let (pool_addr, _ph) = server.bind("127.0.0.1:0").await.unwrap();
    server.publish(job).await;

    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
    let address = spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap();

    let agent_cfg = AgentConfig {
        upstream: Upstream {
            address: pool_addr.to_string(),
            authority_public_key:
                "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            sp_address: address,
            require_own_payout: "warn".into(),
            request_share_accounting: true,
        },
        downstream: Downstream {
            listen: "127.0.0.1:0".into(),
        },
        jd: None,
        ark: None,
    };
    agent_cfg.validate().unwrap();
    let (agent_addr, task) = spc_agent::translator::bind_and_run(agent_cfg)
        .await
        .unwrap();

    let stream = TcpStream::connect(agent_addr).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    write
        .write_all(br#"{"id":1,"method":"mining.subscribe","params":[]}"#)
        .await
        .unwrap();
    write.write_all(b"\n").await.unwrap();
    let sub: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let extranonce1 = sub["result"][1].as_str().unwrap().to_string();
    assert_eq!(extranonce1, "");
    let extranonce2_size = sub["result"][2].as_u64().unwrap() as usize;
    assert_eq!(extranonce2_size, 4);

    let auth = serde_json::json!({"id":2,"method":"mining.authorize","params":[
        spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap(), "x"
    ]});
    write.write_all(auth.to_string().as_bytes()).await.unwrap();
    write.write_all(b"\n").await.unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let _ = lines.next_line().await.unwrap().unwrap(); // difficulty
    let notify: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let p = notify["params"].as_array().unwrap();
    let job_id = p[0].as_str().unwrap().to_string();
    let coinb1 = hex_decode(p[2].as_str().unwrap());
    let coinb2 = hex_decode(p[3].as_str().unwrap());
    let version = u32::from_str_radix(p[5].as_str().unwrap(), 16).unwrap();
    let nbits = u32::from_str_radix(p[6].as_str().unwrap(), 16).unwrap();
    let ntime = u32::from_str_radix(p[7].as_str().unwrap(), 16).unwrap();

    let extranonce2 = vec![0xaa; extranonce2_size];
    let mut coinbase = coinb1.clone();
    coinbase.extend_from_slice(&extranonce2);
    coinbase.extend_from_slice(&coinb2);
    let txid = bitcoin::hashes::sha256d::Hash::hash(&coinbase).to_byte_array();
    let target = Target::from_compact(CompactTarget::from_consensus(nbits));
    let mut nonce = 0u32;
    loop {
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&version.to_le_bytes());
        header.extend_from_slice(&[0x11; 32]);
        header.extend_from_slice(&txid);
        header.extend_from_slice(&ntime.to_le_bytes());
        header.extend_from_slice(&nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let h = bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array();
        if target.is_met_by(BlockHash::from_byte_array(h)) {
            break;
        }
        nonce += 1;
        if nonce > 2_000_000 {
            panic!("no nonce");
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
    let _: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();

    let share = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let block = front.on_share(share).unwrap().expect("block");
    source.submit_block(&block.block).await.unwrap();
    task.abort();
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
