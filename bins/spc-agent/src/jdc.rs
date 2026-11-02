//! Minimal JDC client: connects to the JDS, negotiates, and receives
//! `SetPayoutDistribution`. Job construction/declaration is the next layer.

use std::net::SocketAddr;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use spc_ext::{DistributionIdTlv, SetPayoutDistribution};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Seq064K, Str0255, B0255, B064K, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::job_declaration_sv2::{
    AllocateMiningJobToken, AllocateMiningJobTokenSuccess, DeclareMiningJob,
    MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS,
    MESSAGE_TYPE_DECLARE_MINING_JOB,
};
use thiserror::Error;
use tokio::net::TcpStream;

use crate::verify::{self, Policy};

const EXT_PAYOUTS: u16 = 0x0003;

/// Miner-side key material for own-payout verification (SPEC §3.6): the
/// BIP 352 scan secret key and spend public key.
#[derive(Clone, Debug)]
pub struct OwnKeys {
    pub b_scan: SecretKey,
    pub b_spend: PublicKey,
}

impl OwnKeys {
    pub fn new(b_scan: SecretKey, b_spend: PublicKey) -> Self {
        Self { b_scan, b_spend }
    }
}

#[derive(Debug, Error)]
pub enum JdcError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("no distribution")]
    Empty,
    #[error("own payout derivation failed")]
    OwnDerivation,
    #[error("own payout absent from the coinbase outputs")]
    OwnAbsent,
    #[error("coinbase build: {0}")]
    Coinbase(String),
}

pub struct Jdc {
    read: NoiseTcpReadHalf,
    write: NoiseTcpWriteHalf,
    last: Option<SetPayoutDistribution>,
}

impl Jdc {
    pub async fn connect(addr: SocketAddr) -> Result<Self, JdcError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let stream = connect_with_noise(stream, None)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let (mut read, mut write) = stream.into_split();
        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: Protocol::JobDeclarationProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("spc-agent"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        let frame = MessageFrame::from_message(
            setup,
            stratum_core::common_messages_sv2::MESSAGE_TYPE_SETUP_CONNECTION,
            0,
            false,
        )
        .map_err(|e| JdcError::Codec(e.to_string()))?;
        write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let mut f = read
            .read_frame()
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        if f.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
            return Err(JdcError::Codec("expected SetupConnection.Success".into()));
        }
        let mut p = f.payload().to_vec();
        let _: SetupConnectionSuccess =
            binary_sv2::from_bytes(&mut p).map_err(|e| JdcError::Codec(e.to_string()))?;
        let mut jdc = Self {
            read,
            write,
            last: None,
        };
        jdc.next_distribution().await?;
        Ok(jdc)
    }

    pub async fn allocate_token(&mut self, user_identifier: &str) -> Result<Vec<u8>, JdcError> {
        let msg = AllocateMiningJobToken {
            user_identifier: Str0255::try_from(user_identifier)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            request_id: 1,
        };
        let frame =
            MessageFrame::from_message(msg, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN, 0, false)
                .map_err(|e| JdcError::Codec(e.to_string()))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        loop {
            let mut f = self
                .read
                .read_frame()
                .await
                .map_err(|e| JdcError::Network(e.to_string()))?;
            if f.header().msg_type() == MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS {
                let mut p = f.payload().to_vec();
                let success: AllocateMiningJobTokenSuccess =
                    binary_sv2::from_bytes(&mut p).map_err(|e| JdcError::Codec(e.to_string()))?;
                return Ok(success.mining_job_token.as_bytes().to_vec());
            }
        }
    }

    /// Build the coinbase the JDC will declare. `a_send_x` and the height come
    /// from the template; the carrier is the JD-mode `OP_RETURN` output (the
    /// scriptSig carries only the BIP34 height push and the extranonce, SPEC
    /// §2.4.2). Before building, the miner's own `P` is derived for
    /// `(height, A_send)` and checked against the outputs (SPEC §3.6):
    /// `Policy::Refuse` fails the build when the payout is absent.
    pub fn build_coinbase(
        distribution: &SetPayoutDistribution,
        template_revenue: u64,
        height: u32,
        a_send_x: &[u8; 32],
        extranonce_len: usize,
        own_keys: &OwnKeys,
        policy: Policy,
    ) -> Result<(Vec<u8>, Vec<u8>), JdcError> {
        let fixed = distribution
            .fixed_value()
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        let weights: Vec<u64> = distribution.payouts.iter().map(|p| p.value).collect();
        let amounts = spc_ext::compute_payout_amounts(
            template_revenue,
            distribution.pool_payout.value,
            &weights,
            &distribution.dust_limits,
            fixed,
        )
        .map_err(|e| JdcError::Codec(e.to_string()))?;
        let mut outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = Vec::new();
        outputs.push((
            bitcoin::script::ScriptBuf::from_bytes(distribution.pool_payout.script.clone()),
            amounts.pay_p,
        ));
        for (out, pay) in distribution.payouts.iter().zip(&amounts.pays) {
            if let Some(v) = pay {
                outputs.push((
                    bitcoin::script::ScriptBuf::from_bytes(out.script.clone()),
                    *v,
                ));
            }
        }
        for out in &distribution.additional_outputs {
            outputs.push((
                bitcoin::script::ScriptBuf::from_bytes(out.script.clone()),
                out.value,
            ));
        }
        outputs.push((spc_crypto::emit_op_return(a_send_x), 0));

        Self::verify_own_payout(height, a_send_x, &outputs, own_keys, policy)?;

        let template =
            spc_pool::coinbase::build_jd_template(2, height, extranonce_len, &outputs, 0)
                .map_err(|e| JdcError::Coinbase(e.to_string()))?;
        Ok((template.prefix, template.suffix))
    }

    /// Derive the miner's own `P` for `(height, A_send)` and check that it
    /// appears among `outputs` (SPEC §3.6). `Policy::Refuse` fails when the
    /// payout is absent; `Policy::Warn` allows a missing payout (a small
    /// miner is legitimately below the threshold or trimmed).
    pub fn verify_own_payout(
        height: u32,
        a_send_x: &[u8; 32],
        outputs: &[(bitcoin::script::ScriptBuf, u64)],
        own_keys: &OwnKeys,
        policy: Policy,
    ) -> Result<(), JdcError> {
        let own = verify::derive_own(height, a_send_x, &own_keys.b_scan, &own_keys.b_spend)
            .map_err(|_| JdcError::OwnDerivation)?;
        let views: Vec<verify::OutputView<'_>> = outputs
            .iter()
            .map(|(script, value)| verify::OutputView {
                script_pubkey: script.as_bytes(),
                value: *value,
            })
            .collect();
        let found = verify::find_own(&views, &own).is_some();
        verify::check_policy(policy, found).map_err(|_| JdcError::OwnAbsent)
    }

    /// Declare a job with the current distribution id. The caller supplies the
    /// token from `allocate_token` and the prefix/suffix from `build_coinbase`.
    pub async fn declare_job(
        &mut self,
        token: &[u8],
        id: u64,
        coinbase_prefix: &[u8],
        coinbase_suffix: &[u8],
    ) -> Result<(), JdcError> {
        let tlv = DistributionIdTlv { id }.encode();
        let msg = DeclareMiningJob {
            request_id: 2,
            mining_job_token: B0255::new(token).map_err(|e| JdcError::Codec(e.to_string()))?,
            version: 0x2000_0000,
            coinbase_tx_prefix: B064K::new(coinbase_prefix)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            coinbase_tx_suffix: B064K::new(coinbase_suffix)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            wtxid_list: Seq064K::try_from(Vec::<U256>::new())
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            excess_data: B064K::new(&tlv).map_err(|e| JdcError::Codec(e.to_string()))?,
        };
        let frame = MessageFrame::from_message(msg, MESSAGE_TYPE_DECLARE_MINING_JOB, 0, false)
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))
    }

    pub async fn next_distribution(&mut self) -> Result<SetPayoutDistribution, JdcError> {
        loop {
            let mut f = self
                .read
                .read_frame()
                .await
                .map_err(|e| JdcError::Network(e.to_string()))?;
            if f.header().ext_type_without_channel_msg() == EXT_PAYOUTS
                && f.header().msg_type() == 0
            {
                let p = f.payload().to_vec();
                let d = SetPayoutDistribution::decode(&p)
                    .map_err(|e| JdcError::Codec(e.to_string()))?;
                self.last = Some(d.clone());
                return Ok(d);
            }
        }
    }

    pub fn last(&self) -> Option<&SetPayoutDistribution> {
        self.last.as_ref()
    }

    /// Reserved for the declaration layer; the write half is kept so the JDC
    /// can send the TLV without reconnecting.
    pub fn write_half(&mut self) -> &mut NoiseTcpWriteHalf {
        &mut self.write
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;
    use spc_ext::TxOut;

    fn keys() -> OwnKeys {
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend = SecretKey::from_slice(&[0x12; 32]).unwrap();
        OwnKeys::new(b_scan, PublicKey::from_secret_key(&secp, &b_spend))
    }

    fn dist_with_payout(script: Vec<u8>) -> SetPayoutDistribution {
        SetPayoutDistribution {
            distribution_id: 0,
            pool_payout: TxOut {
                script: vec![0x51, 0x20, 1, 1],
                value: 100,
            },
            payouts: vec![TxOut { script, value: 300 }],
            dust_limits: vec![0],
            additional_outputs: vec![],
        }
    }

    #[test]
    fn missing_own_payout_under_refuse_fails() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        // The only miner output is not our P.
        let dist = dist_with_payout_script();
        let err = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Refuse,
        )
        .unwrap_err();
        assert!(matches!(err, JdcError::OwnAbsent));
    }

    #[test]
    fn missing_own_payout_under_warn_succeeds() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let dist = dist_with_payout_script();
        let (prefix, suffix) = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Warn,
        )
        .unwrap();
        // JD coinbase: the scriptSig is the height push + extranonce only.
        assert_eq!(prefix[41], 12); // 4-byte height push + 8-byte extranonce
        assert_eq!(&prefix[42..], &[0x03, 0x00, 0x35, 0x0c]);
        // A_send rides in the OP_RETURN output, never in the scriptSig.
        let a_x = send.x_only();
        assert!(!prefix.windows(32).any(|w| w == a_x));
        assert!(suffix
            .windows(37)
            .any(|w| w == spc_crypto::emit_op_return(a_x).as_bytes()));
    }

    #[test]
    fn present_own_payout_under_refuse_succeeds() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let own = verify::derive_own(800_000, send.x_only(), &own_keys.b_scan, &own_keys.b_spend)
            .unwrap();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&own.p_x);
        let dist = dist_with_payout(script);
        assert!(Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Refuse,
        )
        .is_ok());
    }

    fn dist_with_payout_script() -> SetPayoutDistribution {
        dist_with_payout(vec![0x51, 0x20, 2, 2])
    }
}
