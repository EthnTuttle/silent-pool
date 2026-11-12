//! Local non-cloneable grant context and wallet-bound operation permit.
use bitcoin::secp256k1::XOnlyPublicKey;
use spc_ext::{
    authorization::{order_digest, AuthorizationGrant, AuthorizationRequest},
    ArkSendOrder,
};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};
/// Closed local admission outcomes. Only DeadlineBeforeRequest proves that
/// the stock capacity wait ended before any authorization request was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CapacityError {
    #[error("wallet operation capacity deadline before authorization request")]
    DeadlineBeforeRequest,
    #[error("wallet readiness read deadline before authorization request")]
    ReadinessDeadline,
    #[error("current wallet proof or provider policy unavailable before authorization request")]
    ReadinessUnavailable,
    #[error("eligible wallet inputs do not cover amount plus actual quoted fee")]
    InsufficientFloat,
    #[error("quoted arkoor fee is unsupported by the pinned thin-send adapter")]
    UnsupportedFeePolicy,
    #[error("foreign wallet operation permit")]
    ForeignPermit,
    #[error("transport does not implement send-capacity admission")]
    Unsupported,
}
impl CapacityError {
    pub fn held_before_request(self) -> bool {
        matches!(
            self,
            Self::DeadlineBeforeRequest
                | Self::ReadinessDeadline
                | Self::ReadinessUnavailable
                | Self::InsufficientFloat
                | Self::UnsupportedFeePolicy
        )
    }
    pub fn reason(self) -> &'static str {
        match self {
            Self::DeadlineBeforeRequest => "wallet_operation_capacity_deadline",
            Self::ReadinessDeadline => "wallet_readiness_deadline",
            Self::ReadinessUnavailable => "wallet_proof_or_policy_unavailable",
            Self::InsufficientFloat => "eligible_float_insufficient",
            Self::UnsupportedFeePolicy => "unsupported_quoted_fee_policy",
            Self::ForeignPermit => "foreign_wallet_permit",
            Self::Unsupported => "unsupported_admission",
        }
    }
}
pub struct SendAuthorization {
    order: ArkSendOrder,
    grant: AuthorizationGrant,
    request: AuthorizationRequest,
    key: XOnlyPublicKey,
    escaped: std::sync::atomic::AtomicBool,
}
impl SendAuthorization {
    pub(crate) fn new(
        order: &ArkSendOrder,
        grant: &AuthorizationGrant,
        request: &AuthorizationRequest,
        key: XOnlyPublicKey,
    ) -> Result<Self, String> {
        if order_digest(order).map_err(|_| "order digest")? != request.order_digest
            || request.order_id != order.order_id
        {
            return Err("grant/order binding".into());
        }
        let context = Self {
            order: order.clone(),
            grant: grant.clone(),
            request: request.clone(),
            key,
            escaped: std::sync::atomic::AtomicBool::new(false),
        };
        context.validate(&order.ark_address, order.sats)?;
        Ok(context)
    }
    pub fn validate(&self, address: &str, sats: u64) -> Result<(), String> {
        if self.escaped.load(std::sync::atomic::Ordering::Acquire) {
            return Err("grant attempt already crossed cosign escape".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "grant clock unavailable")?
            .as_secs();
        if address != self.order.ark_address || sats != self.order.sats {
            return Err("grant payment context mismatch".into());
        }
        self.order
            .verify(&self.key)
            .map_err(|_| "grant order signature")?;
        self.grant
            .verify_for(&self.request, now, &self.key)
            .map_err(|_| "existing grant expired or invalid".into())
    }
    pub fn action_id(&self) -> String {
        format!(
            "spc-{}-{}",
            hex::encode(self.request.order_id),
            hex::encode(self.request.nonce)
        )
    }
    /// Called immediately before the first cosign RPC. Durable authorization
    /// and worker pin already exist; response loss never resets this attempt.
    pub fn enter_cosign(&self, address: &str, sats: u64) -> Result<(), String> {
        self.validate(address, sats)?;
        self.escaped
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "duplicate cosign escape".to_string())?;
        Ok(())
    }
}

/// Not Clone; guarded execution verifies the issuing wallet's exact mutex.
// The default build exposes the transport type but has no stock-wallet issuer.
#[cfg_attr(not(feature = "ark"), allow(dead_code))]
pub struct WalletOperationPermit {
    guard: OwnedMutexGuard<()>,
}
#[cfg_attr(not(feature = "ark"), allow(dead_code))]
impl WalletOperationPermit {
    pub(crate) async fn acquire_for_send(lock: Arc<Mutex<()>>) -> Result<Self, CapacityError> {
        Self::acquire_with_deadline(lock, std::time::Duration::from_secs(30)).await
    }
    /// Shared production timeout branch; tests bound the duration, not the
    /// outcome. This helper has no order, nonce, issuer or wallet-effect API.
    pub(crate) async fn acquire_with_deadline(
        lock: Arc<Mutex<()>>,
        deadline: std::time::Duration,
    ) -> Result<Self, CapacityError> {
        tokio::time::timeout(deadline, Self::acquire(lock))
            .await
            .map_err(|_| CapacityError::DeadlineBeforeRequest)
    }
    pub(crate) async fn acquire(lock: Arc<Mutex<()>>) -> Self {
        Self {
            guard: lock.lock_owned().await,
        }
    }
    pub(crate) fn require_wallet(&self, lock: &Arc<Mutex<()>>) -> Result<(), String> {
        if !Arc::ptr_eq(OwnedMutexGuard::mutex(&self.guard), lock) {
            return Err("foreign wallet operation permit".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grant(seconds: u64) -> SendAuthorization {
        use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[7; 32]).unwrap());
        let mut order = ArkSendOrder {
            order_id: [1; 16],
            ark_address: "test-local-fence".into(),
            sats: 100,
            not_before_height: 1,
            sig: [0; 64],
        };
        order.sign(&key).unwrap();
        let request = AuthorizationRequest {
            order_id: order.order_id,
            order_digest: order_digest(&order).unwrap(),
            nonce: [2; 16],
        };
        let mut grant = AuthorizationGrant {
            request: request.clone(),
            revision: 1,
            expires_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + seconds,
            signature: [0; 64],
        };
        grant.sign(&key);
        SendAuthorization::new(&order, &grant, &request, key.x_only_public_key().0).unwrap()
    }
    #[tokio::test]
    async fn post_lock_and_post_preparation_expiry_have_no_escape() {
        let lock = Arc::new(Mutex::new(()));
        let guard = lock.lock().await;
        let auth = grant(1);
        let expected_action = auth.action_id();
        let pending_lock = lock.clone();
        let task = tokio::spawn(async move {
            let permit = WalletOperationPermit::acquire(pending_lock.clone()).await;
            permit.require_wallet(&pending_lock).unwrap();
            assert!(auth.validate("test-local-fence", 100).is_err());
            assert!(!auth.escaped.load(std::sync::atomic::Ordering::Acquire));
            auth.action_id()
        });
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        drop(guard);
        assert_eq!(task.await.unwrap(), expected_action);
        let _permit = WalletOperationPermit::acquire(lock).await;
        let auth = grant(1);
        auth.validate("test-local-fence", 100).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        assert!(auth.enter_cosign("test-local-fence", 100).is_err());
        assert!(!auth.escaped.load(std::sync::atomic::Ordering::Acquire));
    }
    #[tokio::test]
    async fn foreign_permit_refused_and_cosign_escape_is_single_use_after_expiry() {
        let wallet = Arc::new(Mutex::new(()));
        let foreign = Arc::new(Mutex::new(()));
        let permit = WalletOperationPermit::acquire(wallet.clone()).await;
        assert!(permit.require_wallet(&foreign).is_err());
        permit.require_wallet(&wallet.clone()).unwrap();
        let auth = grant(1);
        let action = auth.action_id();
        auth.enter_cosign("test-local-fence", 100).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        assert!(auth.enter_cosign("test-local-fence", 100).is_err());
        assert_eq!(
            auth.action_id(),
            action,
            "reconciliation retains the same action without another grant"
        );
        assert!(auth.escaped.load(std::sync::atomic::Ordering::Acquire));
    }
}
