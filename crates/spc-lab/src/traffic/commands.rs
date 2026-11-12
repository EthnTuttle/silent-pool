//! Typed durable traffic command payloads (traffic plan Section 12).
//!
//! These are the payload types for the `CommandKind` additions T1 wires
//! into `command.rs` under the serial merge contract. They carry no
//! behavior: no signing, broadcasting or process control lives here. The
//! existing command-id idempotency semantics apply unchanged - the same
//! command id with the same payload returns the stored outcome, the same
//! id with a different payload is a durable conflict (detected through
//! the command log's canonical payload hash, so these types are
//! `deny_unknown_fields` with a fixed field order).

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ids::{ActorId, IdentityId, NodeId};
use crate::traffic::spec::{
    FEE_RATE_SATS_VB_MAX, FEE_RATE_SATS_VB_MIN, ORDINARY_TX_PER_SEC_MAX, ORDINARY_TX_PER_SEC_MIN,
};
use crate::traffic::TrafficError;

/// `pause_tx_driver { driver_id }` - quiesce barrier per traffic plan 6.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseTxDriver {
    pub driver_id: ActorId,
}

/// `resume_tx_driver { driver_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeTxDriver {
    pub driver_id: ActorId,
}

/// `set_tx_rate { driver_id, ordinary_tx_per_sec }` - bounded by the
/// preset cap; rejected above it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTxRate {
    pub driver_id: ActorId,
    pub ordinary_tx_per_sec: f64,
}

/// `send_user_payment { user, destination_address, amount_sats,
/// fee_rate_sats_vb }` - manual UI path, one real signed payment from an
/// SPc user's payment wallet to an explicit destination.
///
/// Plan 6.6/6.3 wrote `{user, wallet, amount, fee}` while plan 12 wrote
/// `{user, amount, fee}` with the destination only implied; the parent
/// resolved the ambiguity (2026-11-08) in favor of an explicit
/// destination field. The paying user (and their per-identity payment
/// wallet) stays the source of funds; `destination_address` is where the
/// signed output goes. There is no default destination and the field is
/// never silently ignored.
///
/// T0 validates the address format, network (regtest, the lab's only
/// network) and output script class (P2PKH, P2WPKH or P2TR);
/// the remaining T1-before-signing checks (destination wallet/UTXO
/// admission, exact output construction) are the signer's domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendUserPayment {
    pub user: IdentityId,
    /// Explicit, non-empty, bounded destination address.
    pub destination_address: String,
    pub amount_sats: u64,
    pub fee_rate_sats_vb: u64,
}

/// `bootstrap_fund { node_id }` - phase-level command executing traffic
/// plan Section 5 (T2-L behavior).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapFund {
    pub driver_id: ActorId,
    pub node_id: NodeId,
}

/// The `inject_tx` payload for the `"rpc-signed"` adapter: a declared,
/// pre-signed raw transaction routed through the driver's standard state
/// machine. Any other adapter name keeps today's fail-closed engine
/// rejection.
///
/// Validation here is the fail-closed hex shape only: full consensus
/// decode of the transaction is T1's signing/journal domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcSignedTxPayload {
    pub raw_tx_hex: String,
}

impl PauseTxDriver {
    pub fn validate(&self) -> Result<(), TrafficError> {
        validate_driver_id(&self.driver_id)
    }
}

impl ResumeTxDriver {
    pub fn validate(&self) -> Result<(), TrafficError> {
        validate_driver_id(&self.driver_id)
    }
}

impl SetTxRate {
    pub fn validate(&self) -> Result<(), TrafficError> {
        validate_driver_id(&self.driver_id)?;
        let rate = self.ordinary_tx_per_sec;
        if !rate.is_finite() || rate < ORDINARY_TX_PER_SEC_MIN {
            return Err(TrafficError::new(
                "traffic_rate_bounds_invalid",
                format!("set_tx_rate {rate} is below the minimum of {ORDINARY_TX_PER_SEC_MIN}"),
            ));
        }
        if rate > ORDINARY_TX_PER_SEC_MAX {
            return Err(TrafficError::new(
                "traffic_rate_exceeds_preset_cap",
                format!("set_tx_rate {rate} exceeds the preset cap of {ORDINARY_TX_PER_SEC_MAX}"),
            ));
        }
        Ok(())
    }
}

impl SendUserPayment {
    pub fn validate(&self) -> Result<(), TrafficError> {
        if let Err(e) = IdentityId::new(self.user.as_str()) {
            return Err(TrafficError::new(
                "traffic_payment_user_invalid",
                format!("send_user_payment user is not a valid identity id: {e}"),
            ));
        }
        validate_destination_address(&self.destination_address)?;
        if self.amount_sats == 0 {
            return Err(TrafficError::new(
                "traffic_payment_amount_invalid",
                "send_user_payment amount_sats must be at least 1",
            ));
        }
        if !(FEE_RATE_SATS_VB_MIN..=FEE_RATE_SATS_VB_MAX).contains(&self.fee_rate_sats_vb) {
            return Err(TrafficError::new(
                "traffic_fee_bounds_invalid",
                format!(
                    "send_user_payment fee_rate_sats_vb {} is outside \
                     {FEE_RATE_SATS_VB_MIN}..={FEE_RATE_SATS_VB_MAX}",
                    self.fee_rate_sats_vb
                ),
            ));
        }
        Ok(())
    }
}

impl BootstrapFund {
    pub fn validate(&self) -> Result<(), TrafficError> {
        validate_driver_id(&self.driver_id)?;
        if self.node_id.as_str().trim().is_empty() {
            return Err(TrafficError::new(
                "traffic_node_id_invalid",
                "bootstrap_fund node_id is empty",
            ));
        }
        Ok(())
    }
}

impl RpcSignedTxPayload {
    pub fn validate(&self) -> Result<(), TrafficError> {
        let hex = self.raw_tx_hex.trim();
        if hex.is_empty()
            || !hex.len().is_multiple_of(2)
            || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(TrafficError::new(
                "traffic_payload_invalid",
                "inject_tx rpc-signed payload must be non-empty even-length hex",
            ));
        }
        Ok(())
    }
}

fn validate_driver_id(driver_id: &ActorId) -> Result<(), TrafficError> {
    if let Err(e) = ActorId::new(driver_id.as_str()) {
        return Err(TrafficError::new(
            "traffic_driver_id_invalid",
            format!("command driver_id is not a valid actor id: {e}"),
        ));
    }
    Ok(())
}

/// Hard bound on the destination address string. A real bitcoin
/// address is far shorter; the bound exists so a malformed endpoint can
/// never ride an unbounded freeform field into the command log.
pub const DESTINATION_ADDRESS_MAX_LEN: usize = 128;

/// Fail-closed destination-address validation (F5): non-empty, bounded,
/// no control characters; parses as a bitcoin address; valid for
/// regtest (the lab's only network); and a standard P2PKH, P2WPKH or P2TR
/// output script (T1-DEST; witness inputs remain gated by T4). The
/// remaining checks (destination wallet admission, exact output
/// construction) are T1's, before signing.
fn validate_destination_address(destination: &str) -> Result<(), TrafficError> {
    if destination.is_empty()
        || destination.len() > DESTINATION_ADDRESS_MAX_LEN
        || destination.chars().any(|c| c.is_control())
    {
        return Err(TrafficError::new(
            "traffic_payment_destination_invalid",
            format!(
                "send_user_payment destination_address must be a non-empty address string of \
                 at most {DESTINATION_ADDRESS_MAX_LEN} bytes without control characters"
            ),
        ));
    }
    let unchecked = bitcoin::Address::from_str(destination).map_err(|e| {
        TrafficError::new(
            "traffic_payment_destination_invalid",
            format!(
                "send_user_payment destination_address does not parse as a bitcoin address: {e}"
            ),
        )
    })?;
    let address = unchecked
        .require_network(bitcoin::Network::Regtest)
        .map_err(|_| {
            TrafficError::new(
                "traffic_payment_destination_network_invalid",
                "send_user_payment destination_address is not valid for regtest (the lab's \
             only network)",
            )
        })?;
    if !spc_lab_txd::builder::supported_destination_script(&address.script_pubkey()) {
        return Err(TrafficError::new(
            "traffic_payment_destination_script_unsupported",
            "send_user_payment destination_address must be a standard P2PKH, P2WPKH or P2TR address",
        ));
    }
    Ok(())
}
