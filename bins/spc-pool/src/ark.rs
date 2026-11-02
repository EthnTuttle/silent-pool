//! `ark` role: verify signed, capped send orders and execute them from the
//! pool's float. The role deliberately sees no identity data (SPEC §11.3).

use std::collections::HashSet;

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
    seen: HashSet<[u8; 16]>,
}

impl OrderVerifier {
    pub fn new(order_pubkey: XOnlyPublicKey, max_order_sats: u64, max_daily_sats: u64) -> Self {
        Self {
            order_pubkey,
            max_order_sats,
            max_daily_sats,
            daily_sats: 0,
            seen: HashSet::new(),
        }
    }

    /// SPEC §11.3 order verification. A `Hold` is a retry, not a rejection.
    pub fn check(&mut self, order: &ArkSendOrder, tip: u32) -> Result<Action, ArkError> {
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
        if self.seen.contains(&order.order_id) {
            return Err(ArkError::Duplicate);
        }
        if self.max_daily_sats > 0
            && self.daily_sats.saturating_add(order.sats) > self.max_daily_sats
        {
            return Err(ArkError::DailyCap);
        }
        self.seen.insert(order.order_id);
        self.daily_sats = self.daily_sats.saturating_add(order.sats);
        Ok(Action::Send)
    }
}

/// The only side effect the role needs: pay an address from the float.
#[async_trait::async_trait]
pub trait ArkTransport: Send + Sync + 'static {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String>;
}

pub struct ArkService<T: ArkTransport> {
    verifier: OrderVerifier,
    transport: T,
    streams: Streams,
    tip: u32,
}

impl<T: ArkTransport> ArkService<T> {
    pub fn new(verifier: OrderVerifier, transport: T, streams: Streams, tip: u32) -> Self {
        Self {
            verifier,
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
                if let Ok(order) = ArkSendOrder::decode(&payload) {
                    self.process(&order).await?;
                }
                self.streams.ack("spc:ark:orders", "ark", &id).await?;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    pub async fn process(&mut self, order: &ArkSendOrder) -> Result<(), ArkError> {
        match self.verifier.check(order, self.tip)? {
            Action::Hold => Ok(()),
            Action::Send => {
                let (vtxo_id, expiry) = self
                    .transport
                    .send(&order.ark_address, order.sats)
                    .await
                    .map_err(ArkError::Transport)?;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{Keypair, Secp256k1, SecretKey};

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
        assert!(matches!(v.check(&o, 800_007), Err(ArkError::Duplicate)));

        let big = order(&kp, 331, 800_006);
        assert!(matches!(v.check(&big, 800_006), Err(ArkError::TooLarge)));

        let mut bad = order(&kp, 300, 800_006);
        bad.sats = 301;
        assert!(matches!(v.check(&bad, 800_006), Err(ArkError::Signature)));

        let zero = order(&kp, 0, 800_006);
        assert!(matches!(v.check(&zero, 800_006), Err(ArkError::TooLarge)));
    }

    #[test]
    fn wrong_pubkey_is_rejected() {
        let kp = key(3);
        let other = key(4);
        let mut v = OrderVerifier::new(other.x_only_public_key().0, 330, 0);
        assert!(matches!(
            v.check(&order(&kp, 10, 0), 0),
            Err(ArkError::Signature)
        ));
    }
}
