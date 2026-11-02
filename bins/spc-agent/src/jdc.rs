//! Minimal JDC client: connects to the JDS, negotiates, and receives
//! `SetPayoutDistribution`. Job construction/declaration is the next layer.

use std::net::SocketAddr;

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

const EXT_PAYOUTS: u16 = 0x0003;

#[derive(Debug, Error)]
pub enum JdcError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("no distribution")]
    Empty,
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
    /// from the template; the carrier is the JD-mode `OP_RETURN`.
    pub fn build_coinbase(
        distribution: &SetPayoutDistribution,
        template_revenue: u64,
        height: u32,
        a_send_x: &[u8; 32],
        extranonce_len: usize,
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
        let template = spc_pool::coinbase::build_pool_template(
            2,
            height,
            a_send_x,
            extranonce_len,
            &outputs,
            0,
        );
        Ok((template.prefix, template.suffix))
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
