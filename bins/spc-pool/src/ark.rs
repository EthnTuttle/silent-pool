//! `ark` role: verify signed, capped send orders and execute them from the
//! pool's float. The role deliberately sees no identity data (SPEC §11.3).

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
    pub max_order_sats: u64,
    pub max_daily_sats: u64,
    pub daily_sats: u64,
}

impl OrderVerifier {
    pub fn new(order_pubkey: XOnlyPublicKey, max_order_sats: u64, max_daily_sats: u64) -> Self {
        Self {
            order_pubkey,
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
        if !(order.ark_address.starts_with("ark1") || order.ark_address.starts_with("tark1")) {
            return Err(ArkError::Address);
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
    use secp256k1::{Keypair, Secp256k1, SecretKey};
    use std::collections::HashSet;
    use std::sync::Arc;

    fn key(seed: u8) -> Keypair {
        let secp = Secp256k1::new();
        Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap())
    }

    fn order(kp: &Keypair, sats: u64, due: u32) -> ArkSendOrder {
        let mut o = ArkSendOrder {
            order_id: [7; 16],
            ark_address: "ark1qqqq".into(),
            sats,
            not_before_height: due,
            sig: [0; 64],
        };
        o.sign(kp).unwrap();
        o
    }

    #[test]
    fn verifies_caps_signature_and_due_height() {
        let kp = key(3);
        let mut v = OrderVerifier::new(kp.x_only_public_key().0, 330, 0);
        let o = order(&kp, 330, 800_006);
        assert_eq!(v.check(&o, 800_005).unwrap(), Action::Hold);
        assert_eq!(v.check(&o, 800_006).unwrap(), Action::Send);
        // `check` is pure; duplicate detection lives in `OrderDedupe`.
        assert_eq!(v.check(&o, 800_007).unwrap(), Action::Send);
        v.account(o.sats);

        let big = order(&kp, 331, 800_006);
        assert!(matches!(v.check(&big, 800_006), Err(ArkError::TooLarge)));

        let mut bad = order(&kp, 300, 800_006);
        bad.sats = 301;
        assert!(matches!(v.check(&bad, 800_006), Err(ArkError::Signature)));

        let zero = order(&kp, 0, 800_006);
        assert!(matches!(v.check(&zero, 800_006), Err(ArkError::TooLarge)));

        // Daily cap counts accounted sends only.
        let mut capped = OrderVerifier::new(kp.x_only_public_key().0, 0, 500);
        let one = order(&kp, 400, 0);
        assert_eq!(capped.check(&one, 0).unwrap(), Action::Send);
        capped.account(one.sats);
        assert!(matches!(capped.check(&one, 0), Err(ArkError::DailyCap)));
    }

    #[test]
    fn wrong_pubkey_is_rejected() {
        let kp = key(3);
        let other = key(4);
        let v = OrderVerifier::new(other.x_only_public_key().0, 330, 0);
        assert!(matches!(
            v.check(&order(&kp, 10, 0), 0),
            Err(ArkError::Signature)
        ));
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
        let o = order(&kp, 100, 10);
        let registry = SharedSet::default();

        // First boot: the order verifies, is marked seen, and is sent.
        let mut v = OrderVerifier::new(kp.x_only_public_key().0, 330, 0);
        let sent = decide_send(&mut v, &registry, &OkTransport, &o, 10)
            .await
            .unwrap()
            .expect("send");
        assert!(!sent.0.is_empty());
        assert_eq!(v.daily_sats, 100);

        // "Restart": fresh verifier, same dedupe registry. The replay is a
        // duplicate and nothing is sent, and the daily cap is untouched.
        let mut v = OrderVerifier::new(kp.x_only_public_key().0, 330, 500);
        assert!(matches!(
            decide_send(&mut v, &registry, &OkTransport, &o, 11).await,
            Err(ArkError::Duplicate)
        ));
        assert_eq!(v.daily_sats, 0);

        // A genuinely new order on the same registry still passes.
        let mut fresh = order(&kp, 120, 10);
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
        let o = order(&kp, 100, 800_006);
        let registry = SharedSet::default();
        let mut v = OrderVerifier::new(kp.x_only_public_key().0, 330, 0);
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
