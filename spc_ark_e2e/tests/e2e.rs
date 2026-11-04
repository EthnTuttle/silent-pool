//! End-to-end on regtest for the `spc-pool` ark role's stock bark adapter.
//!
//! One stock captaind (unmodified bark at the UPSTREAM_DEPS.md pin) plays
//! the Ark server. The pool side is the real thing under test, not a
//! mock: [`spc_pool::ark::bark_stock::open_bark_wallet`] opens the role's
//! wallet, [`BoardingLoop`] boards a real coinbase pool output through
//! [`RpcBoardChain`] and an in-memory registry, and
//! [`BarkTransport::send`] pays a real stock `bark::Wallet` (the miner)
//! with a thin arkoor send. The receipt the transport returns is checked
//! against the VTXO the miner actually received.
//!
//! Gated behind `SPC_ARK_E2E=1` (needs bitcoind, captaind and postgres
//! binaries); run via `./run_e2e.sh`, which sets everything up.

use std::sync::Arc;

use bitcoin::{Address, Network, OutPoint};
use bitcoincore_rpc::RpcApi;

use ark_testing::TestContext;

use spc_pool::ark::bark_stock::{open_bark_wallet, BarkWalletOptions};
use spc_pool::ark::bark_transport::{BarkClient, BarkTransport};
use spc_pool::ark::board::{
    BoardRegistry, BoardingLoop, BoardingPolicy, PoolOutput, PoolOutputKeychain, RpcBoardChain,
};
use spc_pool::ark::ArkTransport;

/// In-memory [BoardRegistry]: production is Redis-backed, but the
/// interface is what makes boarding idempotent, so the e2e only needs a
/// faithful implementation of it.
#[derive(Clone, Default)]
struct InMemoryRegistry {
    state: Arc<std::sync::Mutex<RegistryState>>,
}

#[derive(Default)]
struct RegistryState {
    scanned: Option<u32>,
    outputs: Vec<PoolOutput>,
    decided: std::collections::HashSet<OutPoint>,
}

#[async_trait::async_trait]
impl BoardRegistry for InMemoryRegistry {
    async fn scanned_height(&self) -> Result<Option<u32>, String> {
        Ok(self.state.lock().unwrap().scanned)
    }
    async fn set_scanned_height(&self, height: u32) -> Result<(), String> {
        self.state.lock().unwrap().scanned = Some(height);
        Ok(())
    }
    async fn record_pool_output(&self, output: PoolOutput) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if !s.outputs.iter().any(|o| o.outpoint == output.outpoint) {
            s.outputs.push(output);
        }
        Ok(())
    }
    async fn pool_outputs(&self) -> Result<Vec<PoolOutput>, String> {
        Ok(self.state.lock().unwrap().outputs.clone())
    }
    async fn drop_pool_output(&self, outpoint: OutPoint) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .outputs
            .retain(|o| o.outpoint != outpoint);
        Ok(())
    }
    async fn claim_decision(&self, outpoint: OutPoint) -> Result<bool, String> {
        Ok(self.state.lock().unwrap().decided.insert(outpoint))
    }
    async fn release_decision(&self, outpoint: OutPoint) -> Result<(), String> {
        self.state.lock().unwrap().decided.remove(&outpoint);
        Ok(())
    }
}

/// The ark-testing bitcoind uses cookie auth; the role wallet and the
/// RPC board chain take user/pass, so read them off the cookie.
fn cookie_credentials(path: &std::path::Path) -> (String, String) {
    let cookie = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("reading the bitcoind cookie: {e}"));
    let cookie = cookie.trim();
    let (user, pass) = cookie
        .split_once(':')
        .unwrap_or_else(|| panic!("malformed bitcoind cookie: {cookie}"));
    (user.to_string(), pass.to_string())
}

#[tokio::test]
async fn spc_pool_ark_role_boards_and_pays_over_stock_captaind() {
    if std::env::var("SPC_ARK_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping: set SPC_ARK_E2E=1 (or use ./run_e2e.sh) to run the ark e2e");
        return;
    }

    // --- The world: one stock server, one stock miner wallet, one chain.
    let ctx = TestContext::new("spc_ark_e2e/board_and_send").await;
    let srv = ctx.captaind("server").create().await;
    let ark_info = srv.ark_info().await;
    let ark_id = <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
        ark_info.server_pubkey,
    ));
    let miner = ctx.bark_sdk("miner", &srv).create().await;

    let rpc = bitcoincore_rpc::Client::new(
        &ctx.bitcoind().rpc_url(),
        bitcoincore_rpc::Auth::CookieFile(ctx.bitcoind().rpc_cookie()),
    )
    .unwrap();
    let (rpc_user, rpc_pass) = cookie_credentials(&ctx.bitcoind().rpc_cookie());

    // --- The ark role: its seed backs both the bark wallet and the
    // pool output keychain (m/86'/1'/1', the account the front's
    // `pool_payout_descriptor` commits to).
    let mut seed = [0u8; 64];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut seed);
    let keychain = PoolOutputKeychain::from_seed(Network::Regtest, &seed, "m/86'/1'/1'").unwrap();

    // --- Mint the pool's coinbase: the next block pays its subsidy to
    // the pool output script of its own height (index = height, SPEC
    // §11.3), then mature it (H_f + 100).
    let height = u32::try_from(rpc.get_block_count().unwrap() + 1).unwrap();
    let pool_addr =
        Address::from_script(&keychain.script_pubkey(height), Network::Regtest).unwrap();
    rpc.generate_to_address(1, &pool_addr).unwrap();
    ctx.generate_blocks(100).await;
    println!("pool coinbase at height {height}, chain matured");

    // --- Open the production wallet client behind the seam.
    let client = open_bark_wallet(BarkWalletOptions {
        network: Network::Regtest,
        seed,
        datadir: ctx.datadir.join("ark-role"),
        server_address: srv.ark_url(),
        bitcoind_url: ctx.bitcoind().rpc_url(),
        bitcoind_user: rpc_user.clone(),
        bitcoind_pass: rpc_pass.clone(),
        fallback_fee_sat_vb: 5,
    })
    .await
    .unwrap();

    // --- Board: the loop scans the chain, finds the matured pool
    // output and boards the float shortfall. The pay window is scaled
    // to the test server's vtxo lifetime (432 blocks), everything else
    // is the production policy.
    let chain = RpcBoardChain::new(&ctx.bitcoind().rpc_url(), &rpc_user, &rpc_pass).unwrap();
    let registry = InMemoryRegistry::default();
    let float_target_sats = 1_000_000;
    let policy = BoardingPolicy::new(float_target_sats, 100);
    let board = BoardingLoop::new(chain, client.clone(), registry.clone(), keychain, policy);
    board.tick().await.unwrap();
    println!("board tick 1 done");

    // The funding tx needs confirmations (bark testing: 3) before the
    // boarded VTXO turns spendable; maintenance (sync) finalizes it.
    ctx.generate_blocks(3).await;
    let mut float_sats = 0u64;
    for attempt in 0..10 {
        client.maintenance().await.unwrap();
        float_sats = client
            .list_vtxos()
            .await
            .unwrap()
            .into_iter()
            .filter(|v| v.spendable)
            .map(|v| v.sats)
            .sum();
        if float_sats == float_target_sats {
            break;
        }
        assert!(
            attempt < 9,
            "the boarded VTXO never became spendable: float is {float_sats} sats"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    println!("float after boarding: {float_sats} sats");

    // A second tick must not re-board: the pool output is spent by the
    // board tx, so the loop drops it (outpoint left the active chain).
    board.tick().await.unwrap();
    assert!(
        registry.pool_outputs().await.unwrap().is_empty(),
        "the boarded pool output must be dropped from the registry"
    );
    let float_after = client
        .list_vtxos()
        .await
        .unwrap()
        .into_iter()
        .filter(|v| v.spendable)
        .map(|v| v.sats)
        .sum::<u64>();
    assert_eq!(
        float_after, float_target_sats,
        "a second tick must not change the float"
    );

    // --- Send: the production transport pays the miner's real ark
    // address and returns a receipt recovered from the delivered VTXOs.
    let transport = BarkTransport::new(client.clone(), ark_id, true);
    let miner_addr = miner.new_address().await.unwrap().to_string();
    let payout_sats = 500_000u64;
    let (vtxo_id, expiry) = transport.send(&miner_addr, payout_sats).await.unwrap();
    assert_eq!(vtxo_id.len(), 36, "the receipt carries a real VtxoId");
    println!(
        "arkoor send finalized: vtxo {} expiry {expiry}",
        hex::encode(&vtxo_id[..8])
    );

    // --- The miner actually received it: the stock wallet's spendable
    // VTXO is the one the receipt names, for the ordered amount.
    let mut received = Vec::new();
    for attempt in 0..10 {
        miner.sync().await;
        received = miner
            .spendable_vtxos()
            .await
            .unwrap()
            .into_iter()
            .filter(|v| v.amount().to_sat() == payout_sats)
            .collect();
        if !received.is_empty() {
            break;
        }
        assert!(attempt < 9, "the miner never received the payout");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    assert_eq!(received.len(), 1, "exactly one payout VTXO");
    let received = &received[0];
    assert_eq!(
        received.id().to_bytes().to_vec(),
        vtxo_id,
        "the receipt's vtxo_id is the VTXO the miner holds"
    );
    assert_eq!(
        received.expiry_height().to_u32(),
        expiry,
        "the receipt's expiry is the VTXO's expiry"
    );
    assert_eq!(
        transport.chained_sends_total(),
        0,
        "no chained send: one float VTXO covered it"
    );

    // The send consumed the boarded float and left its change as the
    // new spendable float.
    let float_after_send = client
        .list_vtxos()
        .await
        .unwrap()
        .into_iter()
        .filter(|v| v.spendable)
        .map(|v| v.sats)
        .sum::<u64>();
    assert_eq!(
        float_after_send,
        float_target_sats - payout_sats,
        "the send's change is the remaining float"
    );

    println!("e2e ok: boarded {float_target_sats} sats, paid {payout_sats} sats, receipt verified");
}
