//! `ark` role: verify signed, capped send orders and execute them from the
//! pool's float. The role deliberately sees no identity data (SPEC §11.3).

use std::str::FromStr;

use secp256k1::XOnlyPublicKey;
use spc_ext::{ArkSendOrder, ArkSendReceipt, ReceiptStatus};
use spc_store::redis::{RedisError, Streams};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArkError {
    #[error("bad signature")]
    Signature,
    #[error("order too large")]
    TooLarge,
    #[error("order not due")]
    NotDue,
    #[error("duplicate order")]
    Duplicate,
    #[error("daily cap reached")]
    DailyCap,
    #[error("bad address")]
    Address,
    #[error("address commits to a different ark server")]
    WrongServer,
    #[error("transport: {0}")]
    Transport(String),
    #[error("redis: {0}")]
    Redis(#[from] RedisError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Send,
    Hold,
}

pub struct OrderVerifier {
    pub order_pubkey: XOnlyPublicKey,
    /// The configured Ark server the pool pays through (SPEC §6.3 `[ark]
    /// ark_id`): the first 4 bytes of SHA256(server pubkey).
    pub ark_id: [u8; 4],
    pub max_order_sats: u64,
    pub max_daily_sats: u64,
    pub daily_sats: u64,
}

/// Parse the configured `ark_id` (`[ark] ark_id`, SPEC §6.3: 8 hex chars).
/// Fail closed: wrong length or non-hex input is rejected.
pub fn parse_ark_id(s: &str) -> Result<[u8; 4], ArkError> {
    let id = ark::address::ArkId::from_str(s).map_err(|_| ArkError::Address)?;
    Ok(<[u8; 4]>::from(id))
}

impl OrderVerifier {
    pub fn new(
        order_pubkey: XOnlyPublicKey,
        ark_id: [u8; 4],
        max_order_sats: u64,
        max_daily_sats: u64,
    ) -> Self {
        Self {
            order_pubkey,
            ark_id,
            max_order_sats,
            max_daily_sats,
            daily_sats: 0,
        }
    }

    /// SPEC §11.3 order verification, minus duplicate detection: that step
    /// is `OrderDedupe`'s job and MUST be backed by a store that survives
    /// restarts (`SeenOrders` in production), not process memory. A `Hold`
    /// is a retry, not a rejection. Pure — on `Send`, call `account` once
    /// the order cleared the dedupe check.
    pub fn check(&self, order: &ArkSendOrder, tip: u32) -> Result<Action, ArkError> {
        if order.sats == 0 || (self.max_order_sats > 0 && order.sats > self.max_order_sats) {
            return Err(ArkError::TooLarge);
        }
        // Cheap prefix prefilter; the parsed comparison below is
        // authoritative (SPEC §11.3 step 4: the `ark_id` parsed from
        // `ark_address` MUST equal the configured `ark_id`). Any parse
        // failure fails closed.
        if !(order.ark_address.starts_with("ark1") || order.ark_address.starts_with("tark1")) {
            return Err(ArkError::Address);
        }
        let address = ark::Address::from_str(&order.ark_address).map_err(|_| ArkError::Address)?;
        if <[u8; 4]>::from(address.ark_id()) != self.ark_id {
            return Err(ArkError::WrongServer);
        }
        order
            .verify(&self.order_pubkey)
            .map_err(|_| ArkError::Signature)?;
        if order.not_before_height > tip {
            return Ok(Action::Hold);
        }
        if self.max_daily_sats > 0
            && self.daily_sats.saturating_add(order.sats) > self.max_daily_sats
        {
            return Err(ArkError::DailyCap);
        }
        Ok(Action::Send)
    }

    /// Count a passed order against the daily cap.
    pub fn account(&mut self, sats: u64) {
        self.daily_sats = self.daily_sats.saturating_add(sats);
    }
}

/// Duplicate-order detection (SPEC §11.3: "reject duplicate `order_id`
/// within the retry window"). Implementations must survive restarts —
/// `payout` re-emits scheduled sends, so an `ark` reboot must not double-pay
/// from the float (review 7.6).
#[async_trait::async_trait]
pub trait OrderDedupe: Send + Sync {
    /// Mark `order_id` as seen. `Ok(true)` on the first sighting inside the
    /// retry window, `Ok(false)` on a duplicate.
    async fn mark(&self, order_id: &[u8; 16]) -> Result<bool, ArkError>;
}

/// Redis-backed dedupe set with a TTL of `retry_blocks * 600` s, refreshed
/// on every order seen, so duplicates are rejected across restarts.
#[derive(Clone)]
pub struct SeenOrders {
    streams: Streams,
    ttl_secs: u64,
}

impl SeenOrders {
    /// One set per pool; members are the raw 16-byte order ids.
    pub const KEY: &'static str = "spc:ark:seen";

    pub fn new(streams: Streams, retry_blocks: u32) -> Self {
        Self {
            streams,
            ttl_secs: u64::from(retry_blocks) * 600,
        }
    }
}

#[async_trait::async_trait]
impl OrderDedupe for SeenOrders {
    async fn mark(&self, order_id: &[u8; 16]) -> Result<bool, ArkError> {
        // SADD is the atomic check-and-add: false means already seen.
        let added = self.streams.sadd(Self::KEY, &order_id[..]).await?;
        // The window slides with the last order seen; the set holds at most
        // one retry window's worth of orders and expires once sends stop.
        self.streams.expire(Self::KEY, self.ttl_secs).await?;
        Ok(added)
    }
}

/// The verified, deduped send decision, before the receipt is emitted.
/// `Ok(None)` is a `Hold`. A duplicate inside the retry window never
/// reaches the transport.
async fn decide_send<T: ArkTransport, D: OrderDedupe>(
    verifier: &mut OrderVerifier,
    dedupe: &D,
    transport: &T,
    order: &ArkSendOrder,
    tip: u32,
) -> Result<Option<(Vec<u8>, u32)>, ArkError> {
    match verifier.check(order, tip)? {
        Action::Hold => Ok(None),
        Action::Send => {
            // Mark before sending: a stream redelivery or a replay must
            // never double-pay from the float, even if we crash mid-send.
            if !dedupe.mark(&order.order_id).await? {
                return Err(ArkError::Duplicate);
            }
            let sent = transport
                .send(&order.ark_address, order.sats)
                .await
                .map_err(ArkError::Transport)?;
            verifier.account(order.sats);
            Ok(Some(sent))
        }
    }
}

/// The only side effect the role needs: pay an address from the float.
#[async_trait::async_trait]
pub trait ArkTransport: Send + Sync + 'static {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String>;
}

pub struct ArkService<T: ArkTransport, D: OrderDedupe> {
    verifier: OrderVerifier,
    dedupe: D,
    transport: T,
    streams: Streams,
    tip: u32,
}

impl<T: ArkTransport, D: OrderDedupe> ArkService<T, D> {
    pub fn new(
        verifier: OrderVerifier,
        dedupe: D,
        transport: T,
        streams: Streams,
        tip: u32,
    ) -> Self {
        Self {
            verifier,
            dedupe,
            transport,
            streams,
            tip,
        }
    }

    pub async fn run(&mut self) -> Result<(), ArkError> {
        self.streams.ensure_group("spc:ark:orders", "ark").await?;
        loop {
            let entries = self
                .streams
                .read_group("spc:ark:orders", "ark", "ark-0", 16)
                .await?;
            for (id, payload) in entries {
                match ArkSendOrder::decode(&payload) {
                    Ok(order) => {
                        // A rejected or failed order is a terminal verdict for
                        // this stream entry: log, ack, and leave re-emission
                        // policy to `payout` (SPEC §11.3). A poison order must
                        // not kill the service (the seen set survives reboots,
                        // so a replayed order WILL be seen again).
                        if let Err(e) = self.process(&order).await {
                            tracing::warn!(error = %e, "ark order not executed");
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "undecodable ark order dropped"),
                }
                self.streams.ack("spc:ark:orders", "ark", &id).await?;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    pub async fn process(&mut self, order: &ArkSendOrder) -> Result<(), ArkError> {
        let Some((vtxo_id, expiry)) = decide_send(
            &mut self.verifier,
            &self.dedupe,
            &self.transport,
            order,
            self.tip,
        )
        .await?
        else {
            return Ok(());
        };
        let receipt = ArkSendReceipt {
            order_id: order.order_id,
            status: ReceiptStatus::Sent,
            vtxo_id,
            expiry_height: expiry,
        };
        let bytes = receipt
            .encode()
            .map_err(|e| ArkError::Transport(e.to_string()))?;
        let _ = self.streams.add("spc:ark:receipts", &bytes).await;
        Ok(())
    }
}

/// Convenience constructor wiring the production Redis-backed dedupe.
impl<T: ArkTransport> ArkService<T, SeenOrders> {
    pub fn with_redis_dedupe(
        verifier: OrderVerifier,
        transport: T,
        streams: Streams,
        tip: u32,
        retry_blocks: u32,
    ) -> Self {
        Self::new(
            verifier,
            SeenOrders::new(streams.clone(), retry_blocks),
            transport,
            streams,
            tip,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark::address::ArkId;
    use bitcoin::secp256k1::{PublicKey, Secp256k1 as BitcoinSecp, SecretKey as BitcoinSecretKey};
    use secp256k1::{Keypair, Secp256k1, SecretKey};
    use std::collections::HashSet;
    use std::sync::Arc;

    fn key(seed: u8) -> Keypair {
        let secp = Secp256k1::new();
        Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
    }

    /// A real payable `tark1` address on a throwaway server key (ark-lib
    /// rejects incomplete builders), so the parse in `check` succeeds.
    /// Returns the string form and that server's `ark_id`.
    fn ark_address(server_seed: u8) -> (String, [u8; 4]) {
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
        let ark_id = <[u8; 4]>::from(ArkId::from_server_pubkey(server_pk));
        (address.to_string(), ark_id)
    }

    fn order(kp: &Keypair, ark_address: &str, sats: u64, due: u32) -> ArkSendOrder {
        let mut o = ArkSendOrder {
            order_id: [7; 16],
            ark_address: ark_address.into(),
            sats,
            not_before_height: due,
            sig: [0; 64],
        };
        o.sign(kp).unwrap();
        o
    }

    fn verifier(kp: &Keypair, ark_id: [u8; 4], max_order: u64, max_daily: u64) -> OrderVerifier {
        OrderVerifier::new(kp.x_only_public_key().0, ark_id, max_order, max_daily)
    }

    #[test]
    fn verifies_caps_signature_and_due_height() {
        let kp = key(3);
        let (addr, ark_id) = ark_address(9);
        let mut v = verifier(&kp, ark_id, 330, 0);
        let o = order(&kp, &addr, 330, 800_006);
        assert_eq!(v.check(&o, 800_005).unwrap(), Action::Hold);
        assert_eq!(v.check(&o, 800_006).unwrap(), Action::Send);
        // `check` is pure; duplicate detection lives in `OrderDedupe`.
        assert_eq!(v.check(&o, 800_007).unwrap(), Action::Send);
        v.account(o.sats);

        let big = order(&kp, &addr, 331, 800_006);
        assert!(matches!(v.check(&big, 800_006), Err(ArkError::TooLarge)));

        let mut bad = order(&kp, &addr, 300, 800_006);
        bad.sats = 301;
        assert!(matches!(v.check(&bad, 800_006), Err(ArkError::Signature)));

        let zero = order(&kp, &addr, 0, 800_006);
        assert!(matches!(v.check(&zero, 800_006), Err(ArkError::TooLarge)));

        // Daily cap counts accounted sends only.
        let mut capped = verifier(&kp, ark_id, 0, 500);
        let one = order(&kp, &addr, 400, 0);
        assert_eq!(capped.check(&one, 0).unwrap(), Action::Send);
        capped.account(one.sats);
        assert!(matches!(capped.check(&one, 0), Err(ArkError::DailyCap)));
    }

    #[test]
    fn wrong_pubkey_is_rejected() {
        let kp = key(3);
        let other = key(4);
        let (addr, ark_id) = ark_address(9);
        let v = verifier(&other, ark_id, 330, 0);
        assert!(matches!(
            v.check(&order(&kp, &addr, 10, 0), 0),
            Err(ArkError::Signature)
        ));
    }

    /// SPEC §11.3 step (4): an order addressed to the configured server
    /// passes; the same order addressed to a foreign `ark_id` is rejected
    /// before anything is sent.
    #[test]
    fn wrong_ark_server_is_rejected() {
        let kp = key(3);
        let (ours_addr, ours_id) = ark_address(9);
        let (foreign_addr, _) = ark_address(10);
        let v = verifier(&kp, ours_id, 330, 0);

        let ours = order(&kp, &ours_addr, 100, 0);
        assert_eq!(v.check(&ours, 0).unwrap(), Action::Send);

        let foreign = order(&kp, &foreign_addr, 100, 0);
        assert!(matches!(v.check(&foreign, 0), Err(ArkError::WrongServer)));
    }

    /// Fail closed: an address that does not parse is never executed,
    /// whether the prefix prefilter catches it or the parser does.
    #[test]
    fn malformed_address_is_rejected() {
        let kp = key(3);
        let (_, ours_id) = ark_address(9);
        let v = verifier(&kp, ours_id, 330, 0);
        // Prefix-valid but not bech32m-decodable.
        let o = order(&kp, "ark1qqqq", 100, 0);
        assert!(matches!(v.check(&o, 0), Err(ArkError::Address)));
        // Prefilter: not an ark address at all.
        let o = order(&kp, "bc1qxyz", 100, 0);
        assert!(matches!(v.check(&o, 0), Err(ArkError::Address)));
    }

    #[test]
    fn parse_ark_id_fails_closed() {
        // The config string round-trips the real `ArkId` (SPEC §6.3).
        let (_, ark_id) = ark_address(9);
        let as_hex = ArkId::from(ark_id).to_string();
        assert_eq!(parse_ark_id(&as_hex).unwrap(), ark_id);
        assert!(parse_ark_id(
            &ark_id
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
        .is_ok());
        assert!(parse_ark_id("").is_err());
        assert!(parse_ark_id("0001").is_err());
        assert!(parse_ark_id("zz0011zz").is_err());
    }

    /// Stand-in for the Redis-backed set: state outlives the service, which
    /// is exactly what `SeenOrders` provides across restarts.
    #[derive(Clone, Default)]
    struct SharedSet(Arc<std::sync::Mutex<HashSet<[u8; 16]>>>);

    #[async_trait::async_trait]
    impl OrderDedupe for SharedSet {
        async fn mark(&self, order_id: &[u8; 16]) -> Result<bool, ArkError> {
            Ok(self.0.lock().unwrap().insert(*order_id))
        }
    }

    struct OkTransport;

    #[async_trait::async_trait]
    impl ArkTransport for OkTransport {
        async fn send(&self, _address: &str, _sats: u64) -> Result<(Vec<u8>, u32), String> {
            Ok((vec![0xAA; 8], 900_000))
        }
    }

    #[tokio::test]
    async fn duplicate_orders_are_rejected_across_restarts() {
        let kp = key(3);
        let (addr, ark_id) = ark_address(9);
        let o = order(&kp, &addr, 100, 10);
        let registry = SharedSet::default();

        // First boot: the order verifies, is marked seen, and is sent.
        let mut v = verifier(&kp, ark_id, 330, 0);
        let sent = decide_send(&mut v, &registry, &OkTransport, &o, 10)
            .await
            .unwrap()
            .expect("send");
        assert!(!sent.0.is_empty());
        assert_eq!(v.daily_sats, 100);

        // "Restart": fresh verifier, same dedupe registry. The replay is a
        // duplicate and nothing is sent, and the daily cap is untouched.
        let mut v = verifier(&kp, ark_id, 330, 500);
        assert!(matches!(
            decide_send(&mut v, &registry, &OkTransport, &o, 11).await,
            Err(ArkError::Duplicate)
        ));
        assert_eq!(v.daily_sats, 0);

        // A genuinely new order on the same registry still passes.
        let mut fresh = order(&kp, &addr, 120, 10);
        fresh.order_id = [8; 16];
        fresh.sign(&kp).unwrap();
        assert!(decide_send(&mut v, &registry, &OkTransport, &fresh, 11)
            .await
            .unwrap()
            .is_some());
        assert_eq!(v.daily_sats, 120);
    }

    #[tokio::test]
    async fn held_orders_are_not_marked() {
        let kp = key(3);
        let (addr, ark_id) = ark_address(9);
        let o = order(&kp, &addr, 100, 800_006);
        let registry = SharedSet::default();
        let mut v = verifier(&kp, ark_id, 330, 0);
        assert!(decide_send(&mut v, &registry, &OkTransport, &o, 800_005)
            .await
            .unwrap()
            .is_none());
        // Once due, the same order still clears the dedupe check.
        assert!(decide_send(&mut v, &registry, &OkTransport, &o, 800_006)
            .await
            .unwrap()
            .is_some());
    }
}
