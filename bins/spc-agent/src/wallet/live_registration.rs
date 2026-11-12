//! Native Noise/SV2 transport using AP-2's canonical signed 0x5350 codec.
//! Every exchange receives a fresh durable pool resolution. No cached ack is
//! accepted as liveness; no hashing is performed by this connection.

use anyhow::{bail, ensure};
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};

use super::registration::{RegistrationOutcome, RegistrationPayload};

pub struct LiveRegistration {
    read: NoiseTcpReadHalf,
    write: NoiseTcpWriteHalf,
}

impl LiveRegistration {
    pub async fn connect(config: &crate::config::AgentConfig) -> anyhow::Result<Self> {
        let client = crate::sv2_client::Sv2Client::connect_wallet(
            &config.upstream_target()?,
            &config.net.egress()?,
            &config.upstream.sp_address,
            config.upstream_authority()?,
        )
        .await?;
        let (read, write) = client.into_wallet_connection();
        Ok(Self { read, write })
    }

    pub async fn exchange(
        &mut self,
        payload: &RegistrationPayload,
        request_id: u32,
        scan: &SecretKey,
    ) -> anyhow::Result<RegistrationOutcome> {
        let message = signed_request(payload, request_id, scan)?;
        let payload_bytes = message.encode()?;
        let mut bytes = Vec::with_capacity(6 + payload_bytes.len());
        bytes.extend_from_slice(&spc_ext::EXT_DELIVERY.to_le_bytes());
        bytes.push(spc_ext::MSG_SET_ARK_IDENTITY);
        bytes.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes()[..3]);
        bytes.extend_from_slice(&payload_bytes);
        let frame = stratum_core::codec_sv2::SerializedFrame::from_bytes(bytes)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        for _ in 0..128 {
            let mut frame = self
                .read
                .read_frame()
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if frame.header().ext_type_without_channel_msg() != spc_ext::EXT_DELIVERY {
                continue;
            }
            return classify_response(
                frame.header().msg_type(),
                frame.payload(),
                request_id,
                payload.ark_id,
            );
        }
        bail!("registration reply frame limit")
    }
}

pub fn signed_request(
    payload: &RegistrationPayload,
    request_id: u32,
    scan: &SecretKey,
) -> anyhow::Result<spc_ext::SetArkIdentity> {
    ensure!(
        payload.version == spc_ext::DELIVERY_VERSION,
        "unsupported delivery version"
    );
    ensure!(
        if payload.mailbox_mode == super::MailboxMode::None {
            payload.mailbox_pubkey.is_none() && payload.mailbox_dh.is_none()
        } else {
            payload.mailbox_pubkey.is_some() && payload.mailbox_dh.is_some()
        },
        "bad-mailbox-fields"
    );
    ensure!(
        payload.epoch_fallback == super::EpochFallback::Static
            || payload.mailbox_mode == super::MailboxMode::EpochRegistered,
        "bad-mailbox-fields"
    );
    let mut message = spc_ext::SetArkIdentity {
        ark_id: payload.ark_id,
        b_ark: payload.b_ark.serialize(),
        mailbox_mode: spc_ext::MailboxMode::from_u8(payload.mailbox_mode.as_u8())?,
        mailbox_pubkey: payload
            .mailbox_pubkey
            .map(|p| p.serialize().to_vec())
            .unwrap_or_default(),
        mailbox_dh: payload
            .mailbox_dh
            .map(|p| p.serialize().to_vec())
            .unwrap_or_default(),
        epoch_fallback: spc_ext::EpochFallback::from_u8(payload.epoch_fallback.as_u8())?,
        sig: [0; 64],
        request_id,
    };
    message.sign(
        &payload.sp_id,
        &Keypair::from_secret_key(&Secp256k1::new(), scan),
    )?;
    Ok(message)
}

pub fn classify_response(
    ty: u8,
    bytes: &[u8],
    request_id: u32,
    ark_id: [u8; 4],
) -> anyhow::Result<RegistrationOutcome> {
    match spc_ext::DeliveryMessage::decode(ty, bytes)? {
        spc_ext::DeliveryMessage::SetArkIdentitySuccess(s) => {
            ensure!(
                s.request_id == request_id && s.ark_id == ark_id && s.registration_rev > 0,
                "registration reply correlation mismatch"
            );
            Ok(match s.state {
                spc_ext::RegistrationSuccessState::Applied => RegistrationOutcome::Applied,
                spc_ext::RegistrationSuccessState::AlreadyApplied => {
                    RegistrationOutcome::AlreadyApplied
                }
                spc_ext::RegistrationSuccessState::Superseded => RegistrationOutcome::Superseded,
            })
        }
        spc_ext::DeliveryMessage::SetArkIdentityError(e) => {
            ensure!(
                e.request_id == request_id,
                "registration reply correlation mismatch"
            );
            let error_code = String::from_utf8(e.error_code)?;
            Ok(if e.retryable {
                RegistrationOutcome::Retryable { error_code }
            } else {
                match error_code.as_str() {
                    "registration-mismatch" => RegistrationOutcome::Mismatch,
                    "stale-request" => RegistrationOutcome::Stale,
                    _ => RegistrationOutcome::Terminal { error_code },
                }
            })
        }
        _ => bail!("unexpected registration response"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::PublicKey;

    #[test]
    fn derived_mailbox_requires_d_and_counter_is_signed() {
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[1; 32]).unwrap();
        let ark = SecretKey::from_slice(&[2; 32]).unwrap();
        let mailbox = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
        let mut payload = RegistrationPayload::epoch_derived(
            [4; 32],
            [5; 4],
            PublicKey::from_secret_key(&secp, &ark),
            mailbox,
            super::super::EpochFallback::Static,
        );
        assert!(signed_request(&payload, 1, &scan).is_err());
        payload.mailbox_dh = Some(spc_crypto::mailbox_blinding_hint(&ark, &mailbox).unwrap());
        let mut wire = signed_request(&payload, 1, &scan).unwrap();
        let scan_pub = Keypair::from_secret_key(&secp, &scan).x_only_public_key().0;
        wire.verify(&payload.sp_id, &scan_pub).unwrap();
        wire.request_id = 2;
        assert!(wire.verify(&payload.sp_id, &scan_pub).is_err());
    }

    #[test]
    fn replies_are_strictly_correlated_and_superseded_is_not_ready() {
        let mut success = spc_ext::SetArkIdentitySuccess {
            request_id: 7,
            registration_rev: 9,
            ark_id: [8; 4],
            state: spc_ext::RegistrationSuccessState::Applied,
        };
        let ty = spc_ext::MSG_SET_ARK_IDENTITY_SUCCESS;
        assert!(classify_response(ty, &success.encode().unwrap(), 6, [8; 4]).is_err());
        assert!(classify_response(ty, &success.encode().unwrap(), 7, [9; 4]).is_err());
        success.state = spc_ext::RegistrationSuccessState::Superseded;
        assert_eq!(
            classify_response(ty, &success.encode().unwrap(), 7, [8; 4]).unwrap(),
            RegistrationOutcome::Superseded
        );
        success.registration_rev = 0;
        assert!(classify_response(ty, &success.encode().unwrap(), 7, [8; 4]).is_err());
    }

    #[test]
    fn commit_unknown_is_retryable_without_wire_hints() {
        let error = spc_ext::SetArkIdentityError::new(3, true, "registration-commit-unknown", "");
        assert_eq!(
            classify_response(
                spc_ext::MSG_SET_ARK_IDENTITY_ERROR,
                &error.encode().unwrap(),
                3,
                [1; 4]
            )
            .unwrap(),
            RegistrationOutcome::Retryable {
                error_code: "registration-commit-unknown".into()
            }
        );
    }
}
