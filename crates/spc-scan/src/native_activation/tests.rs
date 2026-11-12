use super::*;
use crate::source_feed::{
    sign, Checkpoint, CommittedEvidence, Consumed, TraceSeal, VerifiedSourceEvidence,
};
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}
fn fixture() -> (CandidateRequest, Response, VerificationKeys) {
    let (wallet, collector, pool) = (key(7), key(8), key(9));
    let identity = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &wallet.public_key(),
        &key(10).public_key(),
    )
    .unwrap();
    let tx = bitcoin::Transaction {
        version: bitcoin::transaction::Version::ONE,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn {
            previous_output: bitcoin::OutPoint::null(),
            script_sig: bitcoin::script::Builder::new()
                .push_int(1)
                .push_slice([0; 8])
                .into_script(),
            sequence: bitcoin::Sequence::MAX,
            witness: bitcoin::Witness::new(),
        }],
        output: vec![bitcoin::TxOut {
            value: bitcoin::Amount::from_sat(1),
            script_pubkey: bitcoin::script::Builder::new().push_int(1).into_script(),
        }],
    };
    let raw = serialize(&tx);
    let request = CandidateRequest {
        schema: SCHEMA.into(),
        run_id: "run".into(),
        miner_id: "miner".into(),
        connection_id: "connection".into(),
        identity: identity.clone(),
        wallet_instance: [21; 16],
        peer_generation: 1,
        connection_config_digest: [22; 32],
        expires_at_unix_ms: 20_000,
        session_epoch: 2,
        candidate_epoch: 3,
        nonce: [4; 16],
        job: ReceivedJob {
            job_id: 5,
            prev_hash: [6; 32],
            version: 1,
            nbits: 0x207fffff,
            ntime: 100,
            coinbase_prefix: raw[..44].to_vec(),
            coinbase_suffix: raw[52..].to_vec(),
            merkle_path: vec![],
            share_target_be: [255; 32],
            pool_extranonce: vec![1; 4],
            miner_extranonce_len: 4,
        },
    };
    let mut trace = TraceSeal {
        session: [5; 16],
        seq: 6,
        digest: [7; 32],
        signature: String::new(),
    };
    trace.signature = sign(trace.signing_digest(), &pool);
    let mut checkpoint = Checkpoint {
        attempt: request.nonce,
        wire_job_digest: request.job.digest(),
        job_id: request.full_job_id().unwrap(),
        generation: 9,
        source_watermark: 6,
        history_watermark: 10,
        bundle_digest: [11; 32],
        history_digest: [12; 32],
        expires_at_unix_ms: 20_000,
        complete: true,
        pool: trace,
        signature: String::new(),
    };
    checkpoint.signature = sign(checkpoint.signing_digest(), &collector);
    let mut consumed = Consumed {
        attempt: checkpoint.attempt,
        job_id: checkpoint.job_id,
        generation: 9,
        bundle_digest: checkpoint.bundle_digest,
        history_digest: checkpoint.history_digest,
        checkpoint_digest: checkpoint.signing_digest(),
        window: crate::source_audit::WindowFacts::from_work(9, 0),
        signature: String::new(),
    };
    consumed.signature = sign(consumed.signing_digest(), &wallet);
    let commit = source_feed::committed_digest(consumed.attempt, consumed.signing_digest());
    let mut receipt = ActivationReceipt {
        request_digest: request.signing_digest().unwrap(),
        candidate_epoch: request.candidate_epoch,
        nonce: request.nonce,
        evidence: VerifiedSourceEvidence {
            schema: source_feed::SCHEMA.into(),
            checkpoint,
            committed: CommittedEvidence {
                attempt: consumed.attempt,
                ack_digest: consumed.signing_digest(),
                signature: sign(commit, &collector),
            },
            consumed,
        },
        wallet_signature: String::new(),
    };
    receipt.wallet_signature = sign(activation_digest(&request, &receipt).unwrap(), &wallet);
    let keys = VerificationKeys::for_regtest_identity(
        &identity,
        &collector.x_only_public_key().0.to_string(),
        &pool.x_only_public_key().0.to_string(),
    )
    .unwrap();
    (
        request,
        Response::Approved {
            schema: SCHEMA.into(),
            capability: CAPABILITY.into(),
            receipt,
        },
        keys,
    )
}
#[test]
fn actual_signatures_and_full_job_id_are_bound() {
    let (request, response, keys) = fixture();
    let verified = verify_response(&request, &response, &keys, 10_000).unwrap();
    assert_eq!(verified.job_id, (1u64 << 32) | 5);
    assert!(verified.window.underfilled);
    assert_eq!(verified.window.shortfall, 9);
    let decoded = decode_request(&encode_request(&request).unwrap()).unwrap();
    let reply = decode_response(&encode_response(&response).unwrap()).unwrap();
    assert!(verify_response(&decoded, &reply, &keys, 10_000).is_ok());
    assert!(verify_response(&request, &response, &keys, 20_000).is_err());
    assert!(verify_response(
        &request,
        &response,
        &VerificationKeys {
            wallet: key(11).x_only_public_key().0,
            ..keys
        },
        10_000
    )
    .is_err());
    assert!(verify_response(
        &request,
        &response,
        &VerificationKeys {
            pool: key(11).x_only_public_key().0,
            ..keys
        },
        10_000
    )
    .is_err());
}
#[test]
fn nonce_epoch_session_identity_and_every_received_field_replay_fail() {
    let (request, response, keys) = fixture();
    for mutation in 0..17 {
        let mut altered = request.clone();
        match mutation {
            0 => altered.nonce[0] ^= 1,
            1 => altered.candidate_epoch += 1,
            2 => altered.session_epoch += 1,
            3 => altered.run_id.push('x'),
            4 => altered.miner_id.push('x'),
            5 => altered.connection_id.push('x'),
            6 => altered.job.share_target_be[0] ^= 1,
            7 => altered.job.pool_extranonce[0] ^= 1,
            8 => altered.job.prev_hash[0] ^= 1,
            9 => altered.job.job_id += 1,
            10 => altered.job.version += 1,
            11 => altered.job.ntime += 1,
            12 => altered.identity.push('x'),
            13 => altered.wallet_instance[0] ^= 1,
            14 => altered.peer_generation += 1,
            15 => altered.connection_config_digest[0] ^= 1,
            _ => altered.expires_at_unix_ms += 1,
        }
        assert!(
            verify_response(&altered, &response, &keys, 10_000).is_err(),
            "{mutation}"
        );
    }
}
#[test]
fn corrupted_checkpoint_ack_and_commit_never_authorize() {
    let (request, response, keys) = fixture();
    let original = encode_response(&response).unwrap();
    for field in [
        "generation",
        "job_id",
        "source_watermark",
        "history_watermark",
        "expires_at_unix_ms",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["receipt"]["evidence"]["checkpoint"][field] = serde_json::json!(123);
        let changed = decode_response(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(verify_response(&request, &changed, &keys, 10_000).is_err());
    }
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["receipt"]["evidence"]["committed"]["ack_digest"][0] = serde_json::json!(255);
    assert!(verify_response(
        &request,
        &decode_response(&serde_json::to_vec(&value).unwrap()).unwrap(),
        &keys,
        10_000
    )
    .is_err());
}
#[test]
fn strict_json_rejects_unknown_duplicate_and_decimal_epoch_before_receipt() {
    let (request, _, _) = fixture();
    let text = String::from_utf8(encode_request(&request).unwrap()).unwrap();
    assert!(decode_request(
        text.replacen("\"candidate_epoch\":3", "\"candidate_epoch\":3.0", 1)
            .as_bytes()
    )
    .is_err());
    assert!(decode_request(text.replacen("{", "{\"extra\":true,", 1).as_bytes()).is_err());
    assert!(decode_request(
        text.replacen("\"job_id\":5", "\"job_id\":5,\"job_id\":5", 1)
            .as_bytes()
    )
    .is_err());
    assert!(decode_response(&vec![b' '; MAX_RESPONSE_BYTES + 1]).is_err());
}

#[test]
fn v2_window_facts_cannot_be_relabelled_or_omitted() {
    let (request, response, keys) = fixture();
    let original = encode_response(&response).unwrap();
    for change in 0..4 {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match change {
            0 => value["receipt"]["evidence"]["schema"] = serde_json::json!("spc.source-feed/1"),
            1 => {
                value["receipt"]["evidence"]["consumed"]["window"]["actual_sum"] =
                    serde_json::json!("9")
            }
            2 => {
                value["receipt"]["evidence"]["consumed"]["window"]["underfilled"] =
                    serde_json::json!(false)
            }
            _ => {
                value["receipt"]["evidence"]["consumed"]
                    .as_object_mut()
                    .unwrap()
                    .remove("window");
            }
        }
        let result = decode_response(&serde_json::to_vec(&value).unwrap())
            .and_then(|r| verify_response(&request, &r, &keys, 10_000));
        assert!(result.is_err());
    }
}

#[test]
fn kernel_peer_and_wallet_restart_binding_refuse_old_requests() {
    let (request, _, keys) = fixture();
    let process = ProcessIdentity {
        pid: 123,
        uid: 1000,
        start_time_ticks: 456,
        boot_id: "00000000-0000-0000-0000-000000000001".into(),
        executable_sha256: [1; 32],
        run_token_sha256: [2; 32],
        config_sha256: [3; 32],
    };
    let wallet = WalletInstance {
        schema: INSTANCE_SCHEMA.into(),
        instance_id: request.wallet_instance,
        run_id: request.run_id.clone(),
        identity_id: "identity".into(),
        identity: request.identity.clone(),
        pool_id: "pool".into(),
        front_id: "front".into(),
        process: process.clone(),
        config_path: "/private/wallet/config.toml".into(),
        socket_path: "/private/wallet/native-audit.sock".into(),
        socket_device: 1,
        socket_inode: 2,
        collector_pubkey: keys.collector.to_string(),
        pool_pubkey: keys.pool.to_string(),
    };
    let peer = PeerBinding {
        miner_id: request.miner_id.clone(),
        connection_id: request.connection_id.clone(),
        identity: request.identity.clone(),
        pool_id: wallet.pool_id.clone(),
        front_id: wallet.front_id.clone(),
        generation: request.peer_generation,
        process: process.clone(),
        config_path: "/private/cpu/config.toml".into(),
        connection_config_digest: request.connection_config_digest,
    };
    let registry = PeerRegistry {
        schema: REGISTRY_SCHEMA.into(),
        run_id: request.run_id.clone(),
        wallet_instance: wallet.instance_id,
        generation: 1,
        peers: vec![peer],
    };
    registry
        .authorize(&wallet, &request, &process, 10_000)
        .unwrap();
    let mut restarted = wallet.clone();
    restarted.instance_id[0] ^= 1;
    assert!(registry
        .authorize(&restarted, &request, &process, 10_000)
        .is_err());
    let mut foreign = process.clone();
    foreign.pid += 1;
    assert!(registry
        .authorize(&wallet, &request, &foreign, 10_000)
        .is_err());
    assert!(registry
        .authorize(&wallet, &request, &process, 20_000)
        .is_err());
    let mut replay = request.clone();
    replay.peer_generation += 1;
    assert!(registry
        .authorize(&wallet, &replay, &process, 10_000)
        .is_err());
}
