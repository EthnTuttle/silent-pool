use std::sync::{Arc, Mutex};

use spc_accounting::{ShareRec, Slice};
use spc_crypto::Network;
use spc_pool::sv2_server::Sv2Server;
use spc_pool::type32::Type32Store;
use tokio::sync::mpsc;

#[tokio::test]
async fn audit_client_verifies_sampled_shares() {
    let mut slice = Slice::new(1, 800_000, [1; 32], 0, 7, 50_000);
    for i in 0..4u32 {
        slice.push(ShareRec {
            sp_id: [i as u8; 32],
            d: 10,
            job_fees: 50_000,
            job_id: 7,
            nonce: i,
            ntime: 1_700_000_000 + i,
            version: 0x2000_0000,
            extranonce: vec![i as u8; 8],
            share_index: i,
        });
    }
    slice.seal();
    let mut store = Type32Store::default();
    store.add(&slice);
    assert_eq!(store.window().len(), 1);
    assert!(store.window()[0].n_shares == 4);

    let type32 = Arc::new(Mutex::new(store));
    let (tx, _rx) = mpsc::channel(8);
    let server = Sv2Server::new(Network::Regtest, tx, type32);
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
    let report = spc_scan::audit::run(addr, [9; 32], 1, 4).await.unwrap().0;
    assert_eq!(report.slices, 1);
    assert_eq!(report.shares, 4);
    assert_eq!(report.verified, 4);
}
