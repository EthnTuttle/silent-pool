//! The production Ark transport: a stock bark wallet behind a narrow seam.
//!
//! [`BarkClient`] is the seam between order policy (verification, caps,
//! dedupe - all in this role) and the wallet: the stock `bark-wallet`
//! implementation lives in [`super::bark_stock`] behind the `ark` feature,
//! and tests mock the client (decision record 2026-11-02-bark-adapter.md,
//! clause 8). Nothing bark-shaped crosses the seam: methods take and
//! return plain data, so the mapping from a verified order to a bark
//! payment request is testable without a wallet, a server or a chain.
//!
//! Receipt semantics (clause 2): stock `Wallet::send_arkoor_payment`
//! returns no receipt, and a wallet-side before/after diff cannot
//! recover one either - bark only persists *change* VTXOs of a send, so
//! the destination VTXOs never appear in the sender's wallet. The seam
//! therefore returns the delivered VTXOs from the send itself
//! ([`BarkClient::send_arkoor_payment`], implemented as a thin arkoor
//! send over fully-public bark APIs in [`super::bark_stock`]), and the
//! transport validates them directly. The receipt records the first
//! delivered VTXO id (sorted, for determinism) and the minimum
//! `expiry_height` across delivered VTXOs. A chained send (bark chains
//! payments when no single float VTXO covers the amount) delivers more
//! than one VTXO: a float-hygiene failure, not a normal event, counted
//! in `spc_ark_chained_sends_total` and logged loudly.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use ark::vtxo::policy::VtxoPolicyKind;

use super::transport::ArkTransport;
use super::ArkError;

/// One VTXO the wallet tracks, as the send and board paths need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedVtxo {
    /// 36-byte VTXO id (`ark::VtxoId::to_bytes`).
    pub id: [u8; 36],
    pub sats: u64,
    /// The VTXO's expiry height (the receipt takes the minimum across a
    /// chained send's delivered VTXOs).
    pub expiry_height: u32,
    /// The VTXO's user pubkey: a delivered VTXO of an order carries the
    /// destination `U_i` (or blinded mailbox key); change VTXOs carry
    /// wallet keys.
    pub user_pubkey: [u8; 33],
    /// True when the VTXO can be selected as an input (float balance).
    pub spendable: bool,
}

/// A verified order mapped to exactly one bark payment request:
/// destination = the order's `ark_address` (`U_i` / blinded mailbox),
/// amount = its sats, capped by the verifier before it gets here.
#[derive(Clone, Debug)]
pub struct ArkPayment {
    pub address: ark::Address,
    pub sats: u64,
}

/// The bark operations the ark role needs, as plain data. Implemented by
/// the stock wallet client ([`super::bark_stock::BarkWalletClient`]) and
/// by mocks in tests. Boarding operations are here because boarding is
/// the same wallet, not the same trait as [`ArkTransport`]: the order
/// stream cannot reach them at all (decision record, clause 3).
#[async_trait::async_trait]
pub trait BarkClient: Send + Sync {
    /// Every VTXO the wallet currently tracks.
    async fn list_vtxos(&self) -> Result<Vec<TrackedVtxo>, String>;
    /// Out-of-round payment to `payment.address`; returns once the send
    /// is finalized (cosign, register, mailbox delivery). The returned
    /// VTXOs are the ones delivered to the destination - the raw
    /// material of the receipt. Fails closed: a send that cannot report
    /// its delivered VTXOs is an error, never a guessed receipt.
    async fn send_arkoor_payment(&self, payment: &ArkPayment) -> Result<Vec<TrackedVtxo>, String>;
    /// The funding address for the next board and the expiry height its
    /// VTXO will carry. Must be paired with the next [`Self::board_psbt`]
    /// call: the PSBT must pay exactly this address.
    async fn board_funding_address(&self) -> Result<(bitcoin::Address, u32), String>;
    /// Register a finalized, fully signed PSBT as a board; bark
    /// broadcasts it itself.
    async fn board_psbt(&self, psbt: &bitcoin::Psbt, expiry_height: u32) -> Result<(), String>;
    /// An on-chain address this wallet controls (board change, SPEC
    /// §11.3 "the remainder to an on-chain address `ark` controls").
    async fn onchain_address(&self) -> Result<bitcoin::Address, String>;
    /// Sync the wallet and run bark maintenance (float hygiene: refresh
    /// VTXOs on depth and before expiry, decision record clause 4).
    async fn maintenance(&self) -> Result<(), String>;
    /// The fee rate to use for a board transaction, sats per vbyte.
    async fn feerate_sat_vb(&self) -> Result<u64, String>;
}

/// Map an order's address string and sats to a payment request, fail
/// closed: this is the SPEC §11.3 step-(4) parse again, defensively, at
/// the transport boundary - the verifier already checked it, and the
/// transport re-checks so a future caller cannot skip verification.
/// Rejects a non-`ark1`/`tark1` prefix, an undecodable address, the
/// wrong network, a foreign `ark_id`, and any policy other than `Pubkey`
/// (an arkoor payment to `U_i` is always a plain pubkey policy).
pub fn map_payment(
    address: &str,
    sats: u64,
    ark_id: [u8; 4],
    testnet: bool,
) -> Result<ArkPayment, ArkError> {
    if !(address.starts_with("ark1") || address.starts_with("tark1")) {
        return Err(ArkError::Address);
    }
    let parsed = ark::Address::from_str(address).map_err(|_| ArkError::Address)?;
    if parsed.is_testnet() != testnet {
        return Err(ArkError::Address);
    }
    if <[u8; 4]>::from(parsed.ark_id()) != ark_id {
        return Err(ArkError::WrongServer);
    }
    if parsed.policy().policy_type() != VtxoPolicyKind::Pubkey {
        return Err(ArkError::Address);
    }
    Ok(ArkPayment {
        address: parsed,
        sats,
    })
}

/// The receipt recovered from a send, before it is trimmed to the wire
/// shape `ArkTransport::send` returns.
struct Delivered {
    vtxo_id: [u8; 36],
    expiry_height: u32,
    /// True when the send chained: more than one delivered VTXO.
    chained: bool,
}

/// Validate the VTXOs a send reports as delivered and recover the
/// receipt. Fails closed: a send that delivered nothing, delivered to
/// any key other than the destination's, or delivered a sum other than
/// the order's sats is ambiguous, never a `Sent` receipt.
fn delivered_receipt(
    delivered: Vec<TrackedVtxo>,
    payment: &ArkPayment,
    sats: u64,
) -> Result<Delivered, String> {
    let destination = payment.address.policy().user_pubkey().serialize();
    for v in &delivered {
        if v.user_pubkey != destination {
            return Err(format!(
                "a delivered VTXO ({}) is held by a key other than the destination; refusing to fabricate a receipt",
                hex::encode(&v.id[..4])
            ));
        }
    }
    if delivered.is_empty() {
        return Err(
            "send finalized but delivered no VTXO to the destination; refusing to fabricate a receipt"
                .to_string(),
        );
    }
    let sum: u64 = delivered.iter().map(|v| v.sats).sum();
    if sum != sats {
        return Err(format!(
            "delivered VTXOs sum to {sum} sats but the order was {sats} sats; refusing to fabricate a receipt"
        ));
    }
    // Deterministic "first": sort by id so restarts and mocks agree.
    let mut delivered = delivered;
    delivered.sort_by_key(|v| v.id);
    let expiry_height = delivered.iter().map(|v| v.expiry_height).min().unwrap_or(0);
    Ok(Delivered {
        vtxo_id: delivered[0].id,
        expiry_height,
        chained: delivered.len() > 1,
    })
}

/// The production [`ArkTransport`] over a [`BarkClient`].
pub struct BarkTransport<C: BarkClient> {
    client: C,
    ark_id: [u8; 4],
    testnet: bool,
    /// `spc_ark_chained_sends_total`: float-hygiene failures, not normal
    /// events (decision record, clause 2). Alarmed on increment.
    chained_sends: AtomicU64,
}

impl<C: BarkClient> BarkTransport<C> {
    pub fn new(client: C, ark_id: [u8; 4], testnet: bool) -> Self {
        Self {
            client,
            ark_id,
            testnet,
            chained_sends: AtomicU64::new(0),
        }
    }

    /// `spc_ark_chained_sends_total`.
    pub fn chained_sends_total(&self) -> u64 {
        self.chained_sends.load(Ordering::Acquire)
    }
}

#[async_trait::async_trait]
impl<C: BarkClient + 'static> ArkTransport for BarkTransport<C> {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String> {
        // Fail closed on any parse or binding error: the transport never
        // pays an address it cannot fully re-verify (SPEC §11.3).
        let payment = map_payment(address, sats, self.ark_id, self.testnet)
            .map_err(|e| format!("bark transport rejected the order's address: {e}"))?;
        let delivered = self.client.send_arkoor_payment(&payment).await?;
        let receipt = delivered_receipt(delivered, &payment, sats)?;
        if receipt.chained {
            let n = self.chained_sends.fetch_add(1, Ordering::AcqRel) + 1;
            // Loud on purpose: a chained send means no single float VTXO
            // covered the order (float hygiene, decision record clause 4).
            tracing::error!(
                chained_sends_total = n,
                sats,
                "spc_ark_chained_sends_total: arkoor payment chained across multiple float VTXOs"
            );
        }
        Ok((receipt.vtxo_id.to_vec(), receipt.expiry_height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1 as BitcoinSecp, SecretKey as BitcoinSecretKey};
    use secp256k1::{Keypair, Secp256k1, SecretKey};
    use spc_ext::ArkSendOrder;

    fn key(seed: u8) -> Keypair {
        let secp = Secp256k1::new();
        Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
    }

    /// A real payable `tark1` address on a throwaway server key (ark-lib
    /// rejects incomplete builders), mirroring the mod tests.
    fn ark_address(server_seed: u8) -> (String, [u8; 4], PublicKey) {
        let secp = BitcoinSecp::new();
        let sk = |seed: u8| BitcoinSecretKey::from_slice(&[seed; 32]).unwrap();
        let pk = |sk: &BitcoinSecretKey| PublicKey::from_secret_key(&secp, sk);
        let server_pk = pk(&sk(server_seed));
        let user_pk = pk(&sk(server_seed.wrapping_add(1)));
        let blinded_id = ark::mailbox::BlindedMailboxIdentifier::from_pubkey(pk(&sk(
            server_seed.wrapping_add(2)
        )));
        let address = ark::Address::builder()
            .testnet(true)
            .server_pubkey(server_pk)
            .pubkey_policy(user_pk)
            .delivery(ark::address::VtxoDelivery::ServerMailbox { blinded_id })
            .into_address()
            .unwrap();
        let ark_id = <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server_pk));
        (address.to_string(), ark_id, user_pk)
    }

    fn order(kp: &Keypair, ark_address: &str, sats: u64) -> ArkSendOrder {
        let mut o = ArkSendOrder {
            order_id: [7; 16],
            ark_address: ark_address.into(),
            sats,
            not_before_height: 0,
            sig: [0; 64],
        };
        o.sign(kp).unwrap();
        o
    }

    fn vtxo(id: u8, sats: u64, expiry: u32, user: PublicKey) -> TrackedVtxo {
        let mut vid = [0u8; 36];
        vid[0] = id;
        TrackedVtxo {
            id: vid,
            sats,
            expiry_height: expiry,
            user_pubkey: user.serialize(),
            spendable: true,
        }
    }

    /// A recording mock of the seam: payments are captured, sends deliver
    /// configurable VTXOs to the destination policy key.
    struct MockBarkClient {
        vtxos: std::sync::Arc<tokio::sync::Mutex<Vec<TrackedVtxo>>>,
        payments: std::sync::Arc<tokio::sync::Mutex<Vec<ArkPayment>>>,
        /// How many VTXOs one send delivers (chaining).
        delivered_per_send: usize,
        /// VTXO expiry handed to delivered VTXOs.
        expiry: u32,
        /// When set, one delivered VTXO carries this many sats instead
        /// of the order's share (fail-closed coverage).
        delivered_sats: Option<u64>,
        /// When set, the send delivers to a foreign key instead of the
        /// destination (fail-closed coverage).
        deliver_to_foreign_key: bool,
    }

    impl MockBarkClient {
        fn new() -> Self {
            Self {
                vtxos: Default::default(),
                payments: Default::default(),
                delivered_per_send: 1,
                expiry: 900_000,
                delivered_sats: None,
                deliver_to_foreign_key: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl BarkClient for MockBarkClient {
        async fn list_vtxos(&self) -> Result<Vec<TrackedVtxo>, String> {
            Ok(self.vtxos.lock().await.clone())
        }

        async fn send_arkoor_payment(
            &self,
            payment: &ArkPayment,
        ) -> Result<Vec<TrackedVtxo>, String> {
            self.payments.lock().await.push(payment.clone());
            let user = if self.deliver_to_foreign_key {
                // A key the destination does not control.
                let secp = BitcoinSecp::new();
                PublicKey::from_secret_key(
                    &secp,
                    &BitcoinSecretKey::from_slice(&[0xfe; 32]).unwrap(),
                )
            } else {
                payment.address.policy().user_pubkey()
            };
            let sats = self
                .delivered_sats
                .unwrap_or(payment.sats / self.delivered_per_send as u64);
            let mut delivered = Vec::new();
            for i in 0..self.delivered_per_send {
                let id = (self.vtxos.lock().await.len() % 255) as u8 + i as u8;
                delivered.push(vtxo(id, sats, self.expiry, user));
            }
            Ok(delivered)
        }

        async fn board_funding_address(&self) -> Result<(bitcoin::Address, u32), String> {
            unimplemented!("boarding is exercised in board.rs tests")
        }

        async fn board_psbt(
            &self,
            _psbt: &bitcoin::Psbt,
            _expiry_height: u32,
        ) -> Result<(), String> {
            unimplemented!()
        }

        async fn onchain_address(&self) -> Result<bitcoin::Address, String> {
            unimplemented!()
        }

        async fn maintenance(&self) -> Result<(), String> {
            Ok(())
        }

        async fn feerate_sat_vb(&self) -> Result<u64, String> {
            Ok(1)
        }
    }

    #[test]
    fn maps_a_verified_order_to_a_bark_payment_request() {
        let (addr, ark_id, _) = ark_address(9);
        let payment = map_payment(&addr, 1234, ark_id, true).unwrap();
        assert_eq!(payment.sats, 1234);
        assert_eq!(payment.address.to_string(), addr);

        // Fail closed: foreign server, wrong network, garbage, non-ark
        // prefix (SPEC §11.3 step 4; the verifier rejects these first,
        // the transport must not pay any of them either).
        let (_, foreign_id, _) = ark_address(10);
        assert!(matches!(
            map_payment(&addr, 1, foreign_id, true),
            Err(ArkError::WrongServer)
        ));
        assert!(matches!(
            map_payment(&addr, 1, ark_id, false),
            Err(ArkError::Address)
        ));
        assert!(matches!(
            map_payment("ark1qqqq", 1, ark_id, true),
            Err(ArkError::Address)
        ));
        assert!(matches!(
            map_payment("bc1qxyz", 1, ark_id, true),
            Err(ArkError::Address)
        ));
    }

    #[tokio::test]
    async fn send_maps_the_order_and_returns_a_real_receipt() {
        let (addr, ark_id, user) = ark_address(9);
        let client = MockBarkClient::new();
        let transport = BarkTransport::new(client, ark_id, true);

        let (vtxo_id, expiry) = transport.send(&addr, 1000).await.unwrap();
        assert_eq!(vtxo_id.len(), 36, "one real VtxoId");
        assert_eq!(expiry, 900_000);
        let client = transport.client; // moved-out check below
        let payments = client.payments.lock().await;
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].sats, 1000);
        assert_eq!(payments[0].address.policy().user_pubkey(), user);
    }

    #[tokio::test]
    async fn a_chained_send_records_the_first_id_and_minimum_expiry() {
        let (addr, ark_id, _) = ark_address(9);
        let mut client = MockBarkClient::new();
        client.delivered_per_send = 2;
        client.expiry = 800_000;
        let transport = BarkTransport::new(client, ark_id, true);

        let (vtxo_id, expiry) = transport.send(&addr, 1000).await.unwrap();
        assert_eq!(expiry, 800_000, "minimum across delivered VTXOs");
        assert_eq!(vtxo_id.len(), 36);
        assert_eq!(transport.chained_sends_total(), 1, "chained sends alarm");
    }

    #[tokio::test]
    async fn a_send_that_delivers_the_wrong_sum_fails_closed() {
        let (addr, ark_id, _) = ark_address(9);
        let mut client = MockBarkClient::new();
        // Deliver to the right key but one sat short.
        client.delivered_sats = Some(999);
        let transport = BarkTransport::new(client, ark_id, true);
        let err = transport.send(&addr, 1000).await.unwrap_err();
        assert!(err.contains("refusing to fabricate"), "{err}");
    }

    #[tokio::test]
    async fn a_send_that_delivers_to_a_foreign_key_fails_closed() {
        let (addr, ark_id, _) = ark_address(9);
        let mut client = MockBarkClient::new();
        client.deliver_to_foreign_key = true;
        let transport = BarkTransport::new(client, ark_id, true);
        let err = transport.send(&addr, 1000).await.unwrap_err();
        assert!(err.contains("refusing to fabricate"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_send_is_a_transport_error_and_emits_no_receipt() {
        let (addr, ark_id, _) = ark_address(9);
        #[derive(Default)]
        struct Failing;
        #[async_trait::async_trait]
        impl BarkClient for Failing {
            async fn list_vtxos(&self) -> Result<Vec<TrackedVtxo>, String> {
                Ok(Vec::new())
            }
            async fn send_arkoor_payment(
                &self,
                _p: &ArkPayment,
            ) -> Result<Vec<TrackedVtxo>, String> {
                Err("server unreachable".to_string())
            }
            async fn board_funding_address(&self) -> Result<(bitcoin::Address, u32), String> {
                unimplemented!()
            }
            async fn board_psbt(&self, _p: &bitcoin::Psbt, _e: u32) -> Result<(), String> {
                unimplemented!()
            }
            async fn onchain_address(&self) -> Result<bitcoin::Address, String> {
                unimplemented!()
            }
            async fn maintenance(&self) -> Result<(), String> {
                Ok(())
            }
            async fn feerate_sat_vb(&self) -> Result<u64, String> {
                Ok(1)
            }
        }
        let transport = BarkTransport::new(Failing, ark_id, true);
        assert!(transport.send(&addr, 1000).await.is_err());
    }

    /// The seam sits behind the verifier: a fully verified order flows
    /// through `decide_send` into the transport and produces a receipt
    /// with a real 36-byte VTXO id; the mapping is exactly the mock's
    /// recorded payment.
    #[tokio::test]
    async fn the_seam_sits_behind_the_verifier() {
        use crate::ark::{decide_send, OrderDedupe, OrderVerifier};
        use std::collections::HashSet;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct SharedSet(Arc<Mutex<HashSet<[u8; 16]>>>);
        #[async_trait::async_trait]
        impl OrderDedupe for SharedSet {
            async fn mark(&self, order_id: &[u8; 16]) -> Result<bool, crate::ark::ArkError> {
                Ok(self.0.lock().unwrap().insert(*order_id))
            }
        }

        let kp = key(3);
        let (addr, ark_id, _) = ark_address(9);
        let o = order(&kp, &addr, 100);
        let mut v = OrderVerifier::new(kp.x_only_public_key().0, ark_id, 330, 0);
        let dedupe = SharedSet::default();
        let client = MockBarkClient::new();
        let sent = decide_send(
            &mut v,
            &dedupe,
            &BarkTransport::new(client, ark_id, true),
            &o,
            0,
        )
        .await
        .unwrap()
        .expect("send");
        assert_eq!(sent.0.len(), 36);
        assert_eq!(sent.1, 900_000);
    }
}
