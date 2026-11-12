//! Pool-side `0x5350` v1 registration and Ark destination construction.
//!
//! Postgres is authoritative. This module only publishes validated requests
//! and reads the scoped Redis notification cache; a timeout never proves that
//! a registration is absent.

use std::time::Duration;

use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use spc_ext::{
    correlated_request_id, correspondent_error_code, DeliveryMessage, RegistrationSuccessState,
    SetArkIdentityError, SetArkIdentitySuccess, MSG_SET_ARK_IDENTITY,
};
use spc_store::models::{
    ArkDeliveryRow, ArkEpochMailboxRow, ArkRegistrationAck, EpochFallback, MailboxMode,
};
use spc_store::redis::Streams;
use spc_store::wire::{ArkEpochRegEvent, ArkRegEvent};

use crate::config::{Config, Role};
use crate::identity::ChannelIdentity;

pub const REGISTRATION_ACK_TIMEOUT: Duration = Duration::from_secs(5);

#[async_trait]
pub trait ArkRegistrationHandler: Send + Sync {
    async fn handle(
        &self,
        identity: &ChannelIdentity,
        msg_type: u8,
        payload: &[u8],
    ) -> DeliveryMessage;
}

#[derive(Clone)]
pub struct RegistrationService {
    streams: Streams,
    ark_id: [u8; 4],
    registration_stream: String,
    ack_key: String,
    ack_timeout: Duration,
}

impl RegistrationService {
    pub fn new(streams: Streams, ark_id: [u8; 4]) -> Self {
        Self {
            streams,
            ark_id,
            registration_stream: "spc:arkreg".to_string(),
            ack_key: "spc:arkreg:ack".to_string(),
            ack_timeout: REGISTRATION_ACK_TIMEOUT,
        }
    }

    pub fn with_names(
        mut self,
        registration_stream: impl Into<String>,
        ack_key: impl Into<String>,
    ) -> Self {
        self.registration_stream = registration_stream.into();
        self.ack_key = ack_key.into();
        self
    }

    pub fn with_ack_timeout(mut self, timeout: Duration) -> Self {
        self.ack_timeout = timeout;
        self
    }

    fn response_error(
        &self,
        epoch: bool,
        request_id: u32,
        retryable: bool,
        code: &str,
        detail: &str,
    ) -> DeliveryMessage {
        let error = SetArkIdentityError::new(request_id, retryable, code, detail);
        if epoch {
            DeliveryMessage::RegisterEpochMailboxesError(error)
        } else {
            DeliveryMessage::SetArkIdentityError(error)
        }
    }

    async fn wait_for_ack(
        &self,
        epoch: bool,
        sp_id: &[u8; 32],
        request_id: u32,
        digest: &[u8; 32],
        attempt: &[u8; 16],
    ) -> DeliveryMessage {
        let deadline = tokio::time::Instant::now() + self.ack_timeout;
        loop {
            match self
                .streams
                .hget(
                    &self.ack_key,
                    &crate::payout::registration_ack_field(sp_id, request_id, attempt),
                )
                .await
            {
                Ok(Some(value)) => match serde_json::from_str::<ArkRegistrationAck>(&value) {
                    Ok(ack)
                        if ack.request_id == request_id
                            && ack.sp_id == hex::encode(sp_id)
                            && ack.signed_fields_digest == hex::encode(digest)
                            && ack.attempt == hex::encode(attempt)
                            && ack.expires_at > unix_secs() =>
                    {
                        let state = match ack.state.as_str() {
                            "applied" => Some(RegistrationSuccessState::Applied),
                            "already_applied" => Some(RegistrationSuccessState::AlreadyApplied),
                            "superseded" => Some(RegistrationSuccessState::Superseded),
                            _ => None,
                        };
                        if let Some(state) = state {
                            let success = SetArkIdentitySuccess {
                                request_id,
                                registration_rev: ack.registration_rev,
                                ark_id: self.ark_id,
                                state,
                            };
                            return if epoch {
                                DeliveryMessage::RegisterEpochMailboxesSuccess(success)
                            } else {
                                DeliveryMessage::SetArkIdentitySuccess(success)
                            };
                        }
                        if let Some(code) = ack.error_code {
                            return self.response_error(
                                epoch,
                                request_id,
                                false,
                                &code,
                                "durable registration request resolved without a new effect",
                            );
                        }
                    }
                    Ok(_) => {
                        // Another identity/request or a stale field can never
                        // satisfy this waiter. Keep waiting only for our exact
                        // digest and correlation pair.
                    }
                    Err(_) => {
                        return self.response_error(
                            epoch,
                            request_id,
                            true,
                            "store-unavailable",
                            "registration acknowledgment is not decodable",
                        )
                    }
                },
                Ok(None) => {}
                Err(_) => {
                    return self.response_error(
                        epoch,
                        request_id,
                        true,
                        "store-unavailable",
                        "registration acknowledgment cache is unavailable",
                    )
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return self.response_error(
                    epoch,
                    request_id,
                    true,
                    "registration-commit-unknown",
                    "no durable acknowledgment within the deadline; retry the same request id",
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn publish_and_wait(
        &self,
        epoch: bool,
        sp_id: &[u8; 32],
        request_id: u32,
        digest: [u8; 32],
        attempt: &[u8; 16],
        payload: Vec<u8>,
    ) -> DeliveryMessage {
        if let Err(error) = self.streams.add(&self.registration_stream, &payload).await {
            tracing::warn!(%error, "registration publication failed");
            return self.response_error(
                epoch,
                request_id,
                true,
                "store-unavailable",
                "registration request could not be published",
            );
        }
        self.wait_for_ack(epoch, sp_id, request_id, &digest, attempt)
            .await
    }
}

#[async_trait]
impl ArkRegistrationHandler for RegistrationService {
    async fn handle(
        &self,
        identity: &ChannelIdentity,
        msg_type: u8,
        payload: &[u8],
    ) -> DeliveryMessage {
        let epoch = msg_type == spc_ext::MSG_REGISTER_EPOCH_MAILBOXES;
        if msg_type != MSG_SET_ARK_IDENTITY && !epoch {
            return self.response_error(
                false,
                correlated_request_id(msg_type, payload),
                false,
                "invalid-message",
                "unexpected 0x5350 message type",
            );
        }
        let attempt: [u8; 16] = rand::random();
        let decoded = match DeliveryMessage::decode(msg_type, payload) {
            Ok(message) => message,
            Err(_) => {
                return self.response_error(
                    epoch,
                    correlated_request_id(msg_type, payload),
                    false,
                    correspondent_error_code(msg_type, payload),
                    "invalid versioned registration frame",
                )
            }
        };
        match decoded {
            DeliveryMessage::SetArkIdentity(request) => {
                let scan = match secp256k1::PublicKey::from_slice(&identity.b_scan) {
                    Ok(scan) => scan.x_only_public_key().0,
                    Err(_) => {
                        return self.response_error(
                            false,
                            request.request_id,
                            false,
                            "invalid-point",
                            "channel scan key is invalid",
                        )
                    }
                };
                if let Some(code) =
                    request.error_code(&identity.sp_id, &scan, &identity.b_spend, &self.ark_id)
                {
                    return self.response_error(false, request.request_id, false, code, code);
                }
                let digest = match request.signed_fields_digest(&identity.sp_id) {
                    Ok(digest) => digest,
                    Err(_) => {
                        return self.response_error(
                            false,
                            request.request_id,
                            false,
                            "invalid-message",
                            "cannot compute signed-fields digest",
                        )
                    }
                };
                let event = ArkRegEvent {
                    sp_id: identity.sp_id,
                    b_scan: identity.b_scan,
                    b_spend: identity.b_spend,
                    ark_id: request.ark_id,
                    b_ark: request.b_ark,
                    mailbox_mode: request.mailbox_mode as u8,
                    mailbox_pubkey: request.mailbox_pubkey.clone(),
                    mailbox_dh: request.mailbox_dh.clone(),
                    epoch_fallback: request.epoch_fallback as u8,
                    sig: request.sig,
                    request_id: request.request_id,
                    attempt,
                };
                let payload = match event.encode() {
                    Ok(payload) => payload,
                    Err(_) => {
                        return self.response_error(
                            false,
                            request.request_id,
                            false,
                            "invalid-message",
                            "cannot encode registration stream entry",
                        )
                    }
                };
                self.publish_and_wait(
                    false,
                    &identity.sp_id,
                    request.request_id,
                    digest,
                    &attempt,
                    payload,
                )
                .await
            }
            DeliveryMessage::RegisterEpochMailboxes(request) => {
                let scan = match secp256k1::PublicKey::from_slice(&identity.b_scan) {
                    Ok(scan) => scan.x_only_public_key().0,
                    Err(_) => {
                        return self.response_error(
                            true,
                            request.request_id,
                            false,
                            "invalid-point",
                            "channel scan key is invalid",
                        )
                    }
                };
                if request.verify(&identity.sp_id, &scan).is_err() {
                    return self.response_error(
                        true,
                        request.request_id,
                        false,
                        "bad-signature",
                        "epoch mailbox signature does not match channel identity",
                    );
                }
                let digest = match request.signed_fields_digest(&identity.sp_id) {
                    Ok(digest) => digest,
                    Err(_) => {
                        return self.response_error(
                            true,
                            request.request_id,
                            false,
                            "invalid-message",
                            "cannot compute signed-fields digest",
                        )
                    }
                };
                let event = ArkEpochRegEvent {
                    sp_id: identity.sp_id,
                    entries: request.entries,
                    sig: request.sig,
                    request_id: request.request_id,
                    attempt,
                };
                let payload = match event.encode() {
                    Ok(payload) => payload,
                    Err(_) => {
                        return self.response_error(
                            true,
                            request.request_id,
                            false,
                            "invalid-message",
                            "cannot encode epoch mailbox stream entry",
                        )
                    }
                };
                self.publish_and_wait(
                    true,
                    &identity.sp_id,
                    request.request_id,
                    digest,
                    &attempt,
                    payload,
                )
                .await
            }
            _ => self.response_error(
                epoch,
                correlated_request_id(msg_type, payload),
                false,
                "invalid-message",
                "client sent a server-only 0x5350 message",
            ),
        }
    }
}

/// Public ArkInfo metadata needed by the SPc front. No Ark wallet seed or order
/// secret is present in this value.
#[derive(Clone, Debug)]
pub struct ArkServerMetadata {
    pub ark_id: [u8; 4],
    pub server_pubkey: PublicKey,
    pub mailbox_pubkey: PublicKey,
    pub testnet: bool,
}

impl ArkServerMetadata {
    pub fn from_config(config: &Config) -> Result<Option<Self>, String> {
        let Some(ark) = config.ark.as_ref().filter(|ark| ark.enabled) else {
            return Ok(None);
        };
        let ark_id =
            crate::ark::parse_ark_id(&ark.ark_id).map_err(|error| format!("ark_id: {error}"))?;
        let server =
            hex::decode(&ark.server_pubkey).map_err(|error| format!("server_pubkey: {error}"))?;
        let server_pubkey =
            PublicKey::from_slice(&server).map_err(|error| format!("server_pubkey: {error}"))?;
        let mailbox =
            hex::decode(&ark.mailbox_pubkey).map_err(|error| format!("mailbox_pubkey: {error}"))?;
        let mailbox_pubkey =
            PublicKey::from_slice(&mailbox).map_err(|error| format!("mailbox_pubkey: {error}"))?;
        let expected = <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server_pubkey));
        if expected != ark_id {
            return Err("ark_id does not match server_pubkey".into());
        }
        Ok(Some(Self {
            ark_id,
            server_pubkey,
            mailbox_pubkey,
            testnet: config.network().map_err(|error| error.to_string())?
                != spc_crypto::Network::Mainnet,
        }))
    }
}

/// Advertise only when a real front handler can be constructed from config.
pub fn registration_advertised(config: &Config) -> bool {
    config
        .roles()
        .is_ok_and(|roles| roles.contains(&Role::Front))
        && matches!(ArkServerMetadata::from_config(config), Ok(Some(_)))
}

/// Derive one payable address from public registration + server metadata.
/// `Ok(None)` is only the explicit `epoch_registered` + `hold` missing-key
/// branch; every invalid point/scalar/server field fails closed as an error.
pub fn derive_ark_destination(
    metadata: &ArkServerMetadata,
    height: u32,
    ecdh: &spc_crypto::SharedSecret,
    delivery: &ArkDeliveryRow,
    epoch_mailbox: Option<&ArkEpochMailboxRow>,
) -> Result<Option<Vec<u8>>, spc_crypto::Error> {
    use ark::address::VtxoDelivery;
    use ark::mailbox::MailboxIdentifier;
    use spc_crypto::ark_feature::{build_bark_address, pool_blinded_mailbox_id};
    use spc_crypto::{ark_user_pubkey, derived_epoch_mailbox_pubkey, tier_tweaks};

    let b_ark = PublicKey::from_slice(&delivery.b_ark)
        .map_err(|_| spc_crypto::Error::Carrier("invalid B_ark registration".into()))?;
    let tweaks = tier_tweaks(ecdh, 0)?;
    let user = ark_user_pubkey(&b_ark, &tweaks.t_ark)?;
    let point = |bytes: &Option<Vec<u8>>| -> Result<PublicKey, spc_crypto::Error> {
        let bytes = bytes
            .as_deref()
            .ok_or_else(|| spc_crypto::Error::Carrier("missing mailbox field".into()))?;
        PublicKey::from_slice(bytes)
            .map_err(|_| spc_crypto::Error::Carrier("invalid mailbox point".into()))
    };
    let mailbox = match delivery.mailbox_mode {
        MailboxMode::None => {
            let address = ark::Address::new(
                metadata.testnet,
                ark::address::ArkId::from_server_pubkey(metadata.server_pubkey),
                ark::VtxoPolicy::new_pubkey(user),
                Vec::new(),
            );
            return Ok(Some(address.to_string().into_bytes()));
        }
        MailboxMode::Static => point(&delivery.mailbox_m)?,
        MailboxMode::EpochDerived => {
            derived_epoch_mailbox_pubkey(&point(&delivery.mailbox_m)?, ecdh)?
        }
        MailboxMode::EpochRegistered => {
            if let Some(entry) = epoch_mailbox.filter(|entry| entry.height == height) {
                PublicKey::from_slice(&entry.mailbox_mi)
                    .map_err(|_| spc_crypto::Error::Carrier("invalid epoch mailbox point".into()))?
            } else if delivery.epoch_fallback == EpochFallback::Hold {
                return Ok(None);
            } else {
                point(&delivery.mailbox_m)?
            }
        }
    };
    let hint = point(&delivery.mailbox_d)?;
    let blinded = pool_blinded_mailbox_id(
        &MailboxIdentifier::from_pubkey(mailbox),
        &hint,
        &tweaks.t_ark,
        &metadata.mailbox_pubkey,
    )?;
    let address = build_bark_address(&metadata.server_pubkey, &user, blinded, metadata.testnet)?;
    // Keep the delivery import anchored to the intended server-mailbox shape;
    // `build_bark_address` has already validated the complete address fields.
    debug_assert!(!matches!(
        address.delivery().first(),
        Some(VtxoDelivery::Unknown { .. }) | None
    ));
    Ok(Some(address.to_string().into_bytes()))
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    fn public(seed: u8) -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[seed; 32]).unwrap(),
        )
    }

    fn metadata() -> ArkServerMetadata {
        let server_pubkey = public(9);
        ArkServerMetadata {
            ark_id: <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server_pubkey)),
            server_pubkey,
            mailbox_pubkey: public(10),
            testnet: false,
        }
    }

    #[test]
    fn frozen_static_destination_matches_the_approved_vector() {
        let root =
            bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Bitcoin, &[0x42; 32]).unwrap();
        let secp = Secp256k1::new();
        let hardened = |index| bitcoin::bip32::ChildNumber::from_hardened_idx(index).unwrap();
        let normal = bitcoin::bip32::ChildNumber::from_normal_idx(0).unwrap();
        let derive = |path: Vec<bitcoin::bip32::ChildNumber>| {
            root.derive_priv(&secp, &path).unwrap().private_key
        };
        let sp = |branch| {
            derive(vec![
                hardened(352),
                hardened(0),
                hardened(0),
                hardened(branch),
                normal,
            ])
        };
        let b_scan = sp(1);
        let b_ark = sp(2);
        let mailbox = derive(vec![hardened(350), hardened(1)]);
        let a_send = SecretKey::from_slice(&[7; 32]).unwrap();
        let b_scan_pub = PublicKey::from_secret_key(&secp, &b_scan);
        let b_ark_pub = PublicKey::from_secret_key(&secp, &b_ark);
        let mailbox_pub = PublicKey::from_secret_key(&secp, &mailbox);
        let hint = spc_crypto::mailbox_blinding_hint(&b_ark, &public(10)).unwrap();
        let ecdh = spc_crypto::SharedSecret::pool_side(800_000, &a_send, &b_scan_pub).unwrap();
        let mut delivery = ArkDeliveryRow {
            sp_id: [0; 32],
            ark_id: [0; 4],
            b_ark: b_ark_pub.serialize().to_vec(),
            mailbox_mode: MailboxMode::Static,
            mailbox_m: Some(mailbox_pub.serialize().to_vec()),
            mailbox_d: Some(hint.serialize().to_vec()),
            epoch_fallback: EpochFallback::Static,
        };
        let address = derive_ark_destination(&metadata(), 800_000, &ecdh, &delivery, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(address).unwrap(),
            "ark1pw4zjupfzqqpgwa8ay9vc42v5pq6gs7ydgty86dztz9hz25kjydkrv39agl6njdezqyprdqnynftx9g0p6lj4jqpnykkjlczlfxl8526956r88l8tn5qfggqlrm0es"
        );
        delivery.mailbox_mode = MailboxMode::EpochDerived;
        let epoch_address = derive_ark_destination(&metadata(), 800_000, &ecdh, &delivery, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(epoch_address).unwrap(),
            "ark1pw4zjupfzqqpgwa8ay9vc42v5pq6gs7ydgty86dztz9hz25kjydkrv39agl6njdezqypganfurycc9966gv3dnvken6mpr52nn72fl76jchg4nrn2p9utmtcsjjclk"
        );
        delivery.mailbox_mode = MailboxMode::EpochRegistered;
        let epoch_mailbox = ArkEpochMailboxRow {
            sp_id: [0; 32],
            height: 800_000,
            mailbox_mi: hex::decode(
                "03f4da68f5676d28c34153149dcb5cacce602cd9b0b515f5c57666c3fbe6daefc3",
            )
            .unwrap()
            .try_into()
            .unwrap(),
        };
        let registered_address =
            derive_ark_destination(&metadata(), 800_000, &ecdh, &delivery, Some(&epoch_mailbox))
                .unwrap()
                .unwrap();
        assert_eq!(
            String::from_utf8(registered_address).unwrap(),
            "ark1pw4zjupfzqqpgwa8ay9vc42v5pq6gs7ydgty86dztz9hz25kjydkrv39agl6njdezqypkq5uanxjd4x6scu4mex7vndqdrxljd0c4j60vvquaaqpzusqx8hq6au2cp"
        );
    }

    #[test]
    fn none_has_empty_delivery_and_epoch_registered_hold_has_no_destination() {
        let secp = Secp256k1::new();
        let a_send = SecretKey::from_slice(&[7; 32]).unwrap();
        let b_scan = SecretKey::from_slice(&[1; 32]).unwrap();
        let ecdh = spc_crypto::SharedSecret::pool_side(
            800_000,
            &a_send,
            &PublicKey::from_secret_key(&secp, &b_scan),
        )
        .unwrap();
        let mut delivery = ArkDeliveryRow {
            sp_id: [0; 32],
            ark_id: [0; 4],
            b_ark: public(2).serialize().to_vec(),
            mailbox_mode: MailboxMode::None,
            mailbox_m: None,
            mailbox_d: None,
            epoch_fallback: EpochFallback::Hold,
        };
        let address = derive_ark_destination(&metadata(), 800_000, &ecdh, &delivery, None)
            .unwrap()
            .unwrap();
        let address = String::from_utf8(address)
            .unwrap()
            .parse::<ark::Address>()
            .unwrap();
        assert!(address.delivery().is_empty());

        delivery.mailbox_mode = MailboxMode::EpochRegistered;
        delivery.mailbox_m = Some(public(3).serialize().to_vec());
        delivery.mailbox_d = Some(public(4).serialize().to_vec());
        assert_eq!(
            derive_ark_destination(&metadata(), 800_000, &ecdh, &delivery, None).unwrap(),
            None
        );
    }
}
