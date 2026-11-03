//! Minimal JDS listener: publishes `SetPayoutDistribution` and accepts a
//! declaration whose `distribution_id` is current or the same-height previous
//! id. Full payout recomputation is in [`crate::jds::Jds::validate`].

use std::net::SocketAddr;
use std::sync::Arc;

use spc_ext::{DistributionIdTlv, SetPayoutDistribution};
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use stratum_core::binary_sv2::{self, GetSize, Seq064K, B0255, B064K, Str0255};
use stratum_core::codec_sv2::SerializedFrame;
use stratum_core::common_messages_sv2::{
    SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::extensions_sv2::{
    RequestExtensions, RequestExtensionsSuccess, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
    MESSAGE_TYPE_REQUEST_EXTENSIONS, MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
};
use stratum_core::job_declaration_sv2::{
    AllocateMiningJobToken, AllocateMiningJobTokenSuccess, DeclareMiningJob,
    DeclareMiningJobError, DeclareMiningJobSuccess, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN,
    MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS, MESSAGE_TYPE_DECLARE_MINING_JOB,
    MESSAGE_TYPE_DECLARE_MINING_JOB_ERROR, MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS,
};
use stratum_core::mining_sv2::{
    SetCustomMiningJob, SetCustomMiningJobError, SetCustomMiningJobSuccess,
    MESSAGE_TYPE_SET_CUSTOM_MINING_JOB, MESSAGE_TYPE_SET_CUSTOM_MINING_JOB_ERROR,
    MESSAGE_TYPE_SET_CUSTOM_MINING_JOB_SUCCESS,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};

use crate::jds::{JdError, Jds};

pub const EXT_PAYOUTS: u16 = 0x0003;

/// What a test client can observe about a received declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclareNotice {
    pub request_id: u32,
    pub distribution_id: Option<u64>,
    pub wtxids: usize,
}

/// How the server answers declarations (SPEC §3.5 answers every parseable
/// one). `Natural` is production; the other arms let client tests script a
/// decline or a silence deterministically.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AnswerPolicy {
    #[default]
    Natural,
    /// Answer every declaration with this decline code.
    ForceDecline(String),
    /// Never answer declarations (distributions are still served).
    Mute,
}

pub struct JdsServer {
    jds: Arc<Mutex<Jds>>,
    current: Arc<Mutex<Option<Vec<u8>>>>,
    height: Arc<Mutex<u32>>,
    accepted: Arc<Mutex<Option<u64>>>,
    declares: Arc<Mutex<Vec<DeclareNotice>>>,
    answer: Arc<Mutex<AnswerPolicy>>,
    dist_tx: broadcast::Sender<Vec<u8>>,
    pk: Secp256k1PublicKey,
    sk: Secp256k1SecretKey,
}

impl JdsServer {
    pub fn new(front_id: u8) -> Self {
        let secp = secp256k1_legacy::Secp256k1::new();
        let mut rng = rand::thread_rng();
        let kp = secp256k1_legacy::Keypair::new(&secp, &mut rng);
        let (dist_tx, _) = broadcast::channel(16);
        Self {
            jds: Arc::new(Mutex::new(Jds::new(front_id))),
            current: Arc::new(Mutex::new(None)),
            height: Arc::new(Mutex::new(0)),
            accepted: Arc::new(Mutex::new(None)),
            declares: Arc::new(Mutex::new(Vec::new())),
            answer: Arc::new(Mutex::new(AnswerPolicy::default())),
            dist_tx,
            pk: Secp256k1PublicKey(kp.x_only_public_key().0),
            sk: Secp256k1SecretKey(kp.secret_key()),
        }
    }

    pub async fn publish(
        &self,
        height: u32,
        revenue: u64,
        distribution: SetPayoutDistribution,
    ) -> Result<u64, String> {
        let mut jds = self.jds.lock().await;
        let id = jds.publish(height, revenue, distribution);
        let current = jds.current().cloned().ok_or("no current distribution")?;
        drop(jds);
        let payload = current.encode().map_err(|e| e.to_string())?;
        *self.current.lock().await = Some(payload.clone());
        *self.height.lock().await = height;
        let _ = self.dist_tx.send(payload);
        Ok(id)
    }

    pub async fn accepted(&self) -> Option<u64> {
        *self.accepted.lock().await
    }

    /// How many `DeclareMiningJob` messages reached the server.
    pub async fn declare_count(&self) -> usize {
        self.declares.lock().await.len()
    }

    /// The received declarations, in arrival order.
    pub async fn declares(&self) -> Vec<DeclareNotice> {
        self.declares.lock().await.clone()
    }

    /// Script the declaration answers (`Natural` by default).
    pub async fn set_answer_policy(&self, policy: AnswerPolicy) {
        *self.answer.lock().await = policy;
    }

    pub async fn bind(
        &self,
        addr: &str,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let server = Arc::new(JdsServerRef {
            jds: self.jds.clone(),
            current: self.current.clone(),
            height: self.height.clone(),
            accepted: self.accepted.clone(),
            declares: self.declares.clone(),
            answer: self.answer.clone(),
            dist_tx: self.dist_tx.clone(),
            pk: self.pk,
            sk: self.sk,
            token_seq: Arc::new(Mutex::new(0)),
            job_seq: Arc::new(Mutex::new(0)),
        });
        let handle = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let server = server.clone();
                tokio::spawn(async move {
                    let _ = server.handle(socket).await;
                });
            }
        });
        Ok((local, handle))
    }
}

struct JdsServerRef {
    jds: Arc<Mutex<Jds>>,
    current: Arc<Mutex<Option<Vec<u8>>>>,
    height: Arc<Mutex<u32>>,
    accepted: Arc<Mutex<Option<u64>>>,
    declares: Arc<Mutex<Vec<DeclareNotice>>>,
    answer: Arc<Mutex<AnswerPolicy>>,
    dist_tx: broadcast::Sender<Vec<u8>>,
    pk: Secp256k1PublicKey,
    sk: Secp256k1SecretKey,
    token_seq: Arc<Mutex<u64>>,
    job_seq: Arc<Mutex<u32>>,
}

impl JdsServerRef {
    async fn handle(&self, socket: TcpStream) -> Result<(), String> {
        let stream =
            stratum_apps::network_helpers::accept_noise_connection(socket, self.pk, self.sk, 3600)
                .await
                .map_err(|e| format!("noise: {e}"))?;
        let (mut read, mut write) = stream.into_split();
        let mut jobs = self.dist_tx.subscribe();

        loop {
            tokio::select! {
                frame = read.read_frame() => {
                    let mut frame = frame.map_err(|e| format!("read: {e}"))?;
                    let msg_type = frame.header().msg_type();
                    let ext = frame.header().ext_type_without_channel_msg();
                    let mut payload = frame.payload().to_vec();
                    match (ext, msg_type) {
                        (0, MESSAGE_TYPE_SETUP_CONNECTION) => {
                            let _: SetupConnection = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("setup: {e}"))?;
                            write_msg(&mut write, SetupConnectionSuccess { used_version: 2, flags: 0 }, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS).await?;
                            if let Some(p) = self.current.lock().await.clone() {
                                write_dist(&mut write, &p).await?;
                            }
                        }
                        (EXT_PAYOUTS, 0) => { /* client must not publish */ }
                        (EXTENSION_TYPE_EXTENSIONS_NEGOTIATION, MESSAGE_TYPE_REQUEST_EXTENSIONS) => {
                            let msg: RequestExtensions = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("extensions: {e}"))?;
                            let requested: Vec<u16> =
                                msg.requested_extensions.iter().copied().collect();
                            // 0x0001 negotiates, 0x0003 publishes payouts. An
                            // unsupported 0x0020 is simply omitted from the
                            // success list; the JDC only requires 0x0003.
                            let supported: Vec<u16> = requested
                                .iter()
                                .copied()
                                .filter(|e| matches!(*e, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION | EXT_PAYOUTS))
                                .collect();
                            let success = RequestExtensionsSuccess {
                                request_id: msg.request_id,
                                supported_extensions: Seq064K::new(supported)
                                    .map_err(|e| format!("extensions: {e}"))?,
                            };
                            write_msg_ext(
                                &mut write,
                                success,
                                MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
                                EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
                            )
                            .await?;
                        }
                        (_, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN) => {
                            let msg: AllocateMiningJobToken = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("allocate: {e}"))?;
                            let mut seq = self.token_seq.lock().await;
                            *seq = seq.wrapping_add(1);
                            let token = format!("spc-token-{}", *seq);
                            drop(seq);
                            let success = AllocateMiningJobTokenSuccess {
                                request_id: msg.request_id,
                                mining_job_token: B0255::new(token.as_bytes())
                                    .map_err(|e| format!("token: {e}"))?,
                                coinbase_outputs: B064K::new(&[])
                                    .map_err(|e| format!("outputs: {e}"))?,
                            };
                            write_msg(
                                &mut write,
                                success,
                                MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS,
                            )
                            .await?;
                        }
                        (_, MESSAGE_TYPE_DECLARE_MINING_JOB) => {
                            let msg: DeclareMiningJob = binary_sv2::from_bytes(&mut payload)
                                .map_err(|e| format!("declare: {e}"))?;
                            {
                                let mut declares = self.declares.lock().await;
                                declares.push(DeclareNotice {
                                    request_id: msg.request_id,
                                    distribution_id: find_tlv(msg.excess_data.as_bytes()),
                                    wtxids: msg.wtxid_list.len(),
                                });
                            }
                            match self.answer.lock().await.clone() {
                                // Simulated silence: nothing is answered at all.
                                AnswerPolicy::Mute => {}
                                AnswerPolicy::ForceDecline(code) => {
                                    send_declare_error(&mut write, msg.request_id, &code).await?;
                                }
                                AnswerPolicy::Natural => {
                                    let height = *self.height.lock().await;
                                    let result = {
                                        let jds = self.jds.lock().await;
                                        check_declare(&jds, height, &msg)
                                    };
                                    match result {
                                        Ok(id) => {
                                            *self.accepted.lock().await = Some(id);
                                            let success = DeclareMiningJobSuccess {
                                                request_id: msg.request_id,
                                                // The pool allows mining on
                                                // the declared job with the
                                                // same token.
                                                new_mining_job_token: msg.mining_job_token.clone(),
                                            };
                                            write_msg(
                                                &mut write,
                                                success,
                                                MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS,
                                            )
                                            .await?;
                                        }
                                        Err(decline) => {
                                            tracing::warn!(height, code = decline.code(), %decline, "declared coinbase declined");
                                            send_declare_error(&mut write, msg.request_id, decline.code()).await?;
                                        }
                                    }
                                }
                            }
                        }
                        (_, MESSAGE_TYPE_SET_CUSTOM_MINING_JOB) => {
                            let height = *self.height.lock().await;
                            let mut id_buf = payload.clone();
                            let parsed: Result<SetCustomMiningJob, _> =
                                binary_sv2::from_bytes(&mut id_buf);
                            let result = {
                                let jds = self.jds.lock().await;
                                check_custom_job(&jds, height, &payload)
                            };
                            match result {
                                Ok(id) => {
                                    *self.accepted.lock().await = Some(id);
                                    if let Ok(msg) = parsed {
                                        let mut seq = self.job_seq.lock().await;
                                        *seq = seq.wrapping_add(1);
                                        let success = SetCustomMiningJobSuccess {
                                            channel_id: msg.channel_id,
                                            request_id: msg.request_id,
                                            job_id: *seq,
                                        };
                                        write_msg(
                                            &mut write,
                                            success,
                                            MESSAGE_TYPE_SET_CUSTOM_MINING_JOB_SUCCESS,
                                        )
                                        .await?;
                                    }
                                }
                                Err(decline) => {
                                    tracing::warn!(height, code = decline.code(), %decline, "SetCustomMiningJob declined");
                                    if let Ok(msg) = parsed {
                                        send_custom_job_error(
                                            &mut write,
                                            msg.channel_id,
                                            msg.request_id,
                                            decline.code(),
                                        )
                                        .await?;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                dist = jobs.recv() => {
                    match dist {
                        Ok(p) => write_dist(&mut write, &p).await?,
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                }
            }
        }
        Ok(())
    }
}

/// A decline the JDS can answer (SPEC §3.5 "Declaration answers"): the wire
/// `error_code` plus a detail string for logs. `Malformed` covers anything
/// about the declaration itself (codec, TLV); `Rejected` is the distribution
/// validation result.
#[derive(Debug, PartialEq, Eq)]
pub enum Decline {
    Malformed(String),
    Rejected(JdError),
}

impl Decline {
    /// The stable wire code a JDC can act on.
    pub fn code(&self) -> &'static str {
        match self {
            Decline::Malformed(_) => "invalid-job",
            Decline::Rejected(e) => e.decline_code(),
        }
    }
}

impl std::fmt::Display for Decline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decline::Malformed(detail) => write!(f, "invalid-job: {detail}"),
            Decline::Rejected(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Decline {}

/// Validate a Full-Template declaration (`DeclareMiningJob`), returning the
/// distribution id it declared under on success.
fn check_declare(
    jds: &Jds,
    height: u32,
    msg: &DeclareMiningJob<'_>,
) -> Result<u64, Decline> {
    let id = find_tlv(msg.excess_data.as_bytes()).ok_or_else(|| {
        Decline::Malformed("DeclareMiningJob is missing the distribution_id TLV".into())
    })?;
    let outputs = parse_coinbase_outputs(msg.coinbase_tx_suffix.as_bytes())
        .map_err(Decline::Malformed)?;
    jds.validate_declared(id, height, &outputs)
        .map_err(Decline::Rejected)?;
    Ok(id)
}

/// Validate a Coinbase-only declaration (`SetCustomMiningJob`).
///
/// The extension appends the `distribution_id` TLV after the message payload
/// (PR #203 §6); the TLV is not part of the `mining_sv2` struct, so the
/// declared outputs are `coinbase_tx_outputs` (a consensus-serialized
/// `Vec<TxOut>`, i.e. count + outputs) and the trailing bytes must contain
/// exactly one well-formed `0x0003`/`0x01` TLV.
fn check_custom_job(jds: &Jds, height: u32, payload: &[u8]) -> Result<u64, Decline> {
    // `from_bytes` leaves trailing bytes untouched, so decode from a copy and
    // use the message's serialized size to locate the appended TLV.
    let mut buf = payload.to_vec();
    let msg: SetCustomMiningJob = binary_sv2::from_bytes(&mut buf)
        .map_err(|e| Decline::Malformed(format!("SetCustomMiningJob decode: {e}")))?;
    let consumed = msg.get_size();
    let outputs_bytes = msg.coinbase_tx_outputs.as_bytes().to_vec();
    let trailing = payload.get(consumed..).ok_or_else(|| {
        Decline::Malformed("SetCustomMiningJob message size does not match its payload".into())
    })?;
    let id = distribution_id_from_trailing(trailing).map_err(Decline::Malformed)?;
    let (outputs, consumed_outputs) =
        parse_outputs(&outputs_bytes, 0).map_err(Decline::Malformed)?;
    if consumed_outputs != outputs_bytes.len() {
        return Err(Decline::Malformed(
            "trailing bytes after SetCustomMiningJob coinbase_tx_outputs".into(),
        ));
    }
    jds.validate_declared(id, height, &outputs)
        .map_err(Decline::Rejected)?;
    Ok(id)
}

/// Parse one or more appended TLVs and return the `distribution_id`.
///
/// Fail-closed: truncated headers/values, a duplicate id, a wrong length or a
/// missing id are all rejected with a specific reason.
fn distribution_id_from_trailing(bytes: &[u8]) -> Result<u64, String> {
    if bytes.is_empty() {
        return Err("SetCustomMiningJob is missing the appended distribution_id TLV".into());
    }
    let mut off = 0usize;
    let mut found = None;
    while off < bytes.len() {
        let rest = bytes
            .get(off..)
            .ok_or("SetCustomMiningJob TLV offset out of range")?;
        if rest.len() < 5 {
            return Err("truncated TLV header after SetCustomMiningJob".into());
        }
        let ext_type = u16::from_le_bytes([rest[0], rest[1]]);
        let field_type = rest[2];
        let len = u16::from_le_bytes([rest[3], rest[4]]) as usize;
        let value = rest
            .get(5..5 + len)
            .ok_or("truncated TLV value after SetCustomMiningJob")?;
        if ext_type == EXT_PAYOUTS && field_type == 0x01 {
            if found.is_some() {
                return Err("duplicate distribution_id TLV on SetCustomMiningJob".into());
            }
            if len != 8 {
                return Err(format!("distribution_id TLV must be 8 bytes, got {len}"));
            }
            let id = u64::from_le_bytes(
                value
                    .try_into()
                    .map_err(|_| "distribution_id TLV length mismatch")?,
            );
            found = Some(id);
        }
        off = off
            .checked_add(5 + len)
            .ok_or("SetCustomMiningJob TLV offset overflow")?;
    }
    found.ok_or_else(|| "SetCustomMiningJob is missing the appended distribution_id TLV".into())
}

fn parse_coinbase_outputs(suffix: &[u8]) -> Result<Vec<spc_ext::TxOut>, String> {
    // suffix: sequence (4) || output count || outputs || locktime
    Ok(parse_outputs(suffix, 4)?.0)
}

/// Parse a compact-size output list starting at `off`; returns the outputs and
/// the offset just past the last one (the caller decides whether trailing
/// bytes are allowed).
fn parse_outputs(buf: &[u8], mut off: usize) -> Result<(Vec<spc_ext::TxOut>, usize), String> {
    let count = read_compact(buf, &mut off).ok_or("output count")?;
    let mut out = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let value_bytes = buf.get(off..off + 8).ok_or("value")?;
        let value = u64::from_le_bytes(value_bytes.try_into().map_err(|_| "value")?);
        off += 8;
        let n = read_compact(buf, &mut off).ok_or("script len")?;
        let script = buf.get(off..off + n).ok_or("script")?.to_vec();
        off += n;
        out.push(spc_ext::TxOut { script, value });
    }
    Ok((out, off))
}

fn read_compact(buf: &[u8], off: &mut usize) -> Option<usize> {
    let first = *buf.get(*off)?;
    *off += 1;
    match first {
        0..=0xfc => Some(first as usize),
        0xfd => {
            let b = buf.get(*off..*off + 2)?;
            *off += 2;
            Some(u16::from_le_bytes([b[0], b[1]]) as usize)
        }
        0xfe => {
            let b = buf.get(*off..*off + 4)?;
            *off += 4;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        }
        0xff => {
            let b = buf.get(*off..*off + 8)?;
            *off += 8;
            usize::try_from(u64::from_le_bytes(b.try_into().ok()?)).ok()
        }
    }
}

fn find_tlv(bytes: &[u8]) -> Option<u64> {
    if bytes.len() == 13 {
        return DistributionIdTlv::decode(bytes).ok().map(|t| t.id);
    }
    // The TLV may be appended to a larger excess-data field.
    bytes
        .windows(13)
        .rev()
        .find_map(|w| DistributionIdTlv::decode(w).ok().map(|t| t.id))
}

async fn write_msg<T>(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
) -> Result<(), String>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    write_msg_ext(write, msg, msg_type, 0).await
}

async fn write_msg_ext<T>(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
    ext_type: u16,
) -> Result<(), String>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = stratum_core::framing_sv2::framing::MessageFrame::from_message(
        msg, msg_type, ext_type, false,
    )
    .map_err(|e| format!("frame: {e}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("write: {e}"))
}

async fn write_dist(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    payload: &[u8],
) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(6 + payload.len());
    bytes.extend_from_slice(&EXT_PAYOUTS.to_le_bytes());
    bytes.push(0);
    let len = u32::try_from(payload.len()).map_err(|_| "too long")?;
    bytes.extend_from_slice(&len.to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    let frame = SerializedFrame::from_bytes(bytes).map_err(|e| format!("frame: {e:?}"))?;
    write
        .write_frame(frame)
        .await
        .map_err(|e| format!("write: {e}"))
}

/// Answer a Full-Template declaration with a coded decline (SPEC §3.5); the
/// JDS is silent only when the frame itself could not be decoded.
async fn send_declare_error(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    request_id: u32,
    code: &str,
) -> Result<(), String> {
    let msg = DeclareMiningJobError {
        request_id,
        error_code: Str0255::try_from(code).map_err(|e| format!("code: {e:?}"))?,
        error_details: B064K::new(&[]).map_err(|e| format!("details: {e}"))?,
    };
    write_msg(write, msg, MESSAGE_TYPE_DECLARE_MINING_JOB_ERROR).await
}

/// Answer the MP custom-job path with a coded decline (SPEC §3.5).
async fn send_custom_job_error(
    write: &mut stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf,
    channel_id: u32,
    request_id: u32,
    code: &str,
) -> Result<(), String> {
    let msg = SetCustomMiningJobError {
        channel_id,
        request_id,
        error_code: Str0255::try_from(code).map_err(|e| format!("code: {e:?}"))?,
    };
    write_msg(write, msg, MESSAGE_TYPE_SET_CUSTOM_MINING_JOB_ERROR).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinbase::{build_jd_template, write_compact_size};
    use crate::jds::{JdError, Jds};
    use bitcoin::script::ScriptBuf;
    use spc_ext::{compute_payout_amounts, TxOut};
    use stratum_core::binary_sv2::{Seq0255, B0255, U256};
    use stratum_core::mining_sv2::SetCustomMiningJob;

    fn dist() -> SetPayoutDistribution {
        SetPayoutDistribution {
            distribution_id: 0,
            pool_payout: TxOut {
                script: vec![0x51, 0x20, 1, 1],
                value: 100,
            },
            payouts: vec![TxOut {
                script: vec![0x51, 0x20, 2, 2],
                value: 300,
            }],
            dust_limits: vec![0],
            additional_outputs: vec![TxOut {
                script: vec![0x6a, 0x01, 2, 3],
                value: 0,
            }],
        }
    }

    fn coinbase_outputs(d: &SetPayoutDistribution) -> Vec<(ScriptBuf, u64)> {
        let a = compute_payout_amounts(1_000, d.pool_payout.value, &[300], &[0], 0).unwrap();
        vec![
            (ScriptBuf::from_bytes(d.pool_payout.script.clone()), a.pay_p),
            (
                ScriptBuf::from_bytes(d.payouts[0].script.clone()),
                a.pays[0].unwrap(),
            ),
            (
                ScriptBuf::from_bytes(d.additional_outputs[0].script.clone()),
                0,
            ),
        ]
    }

    #[test]
    fn declared_coinbase_parses_and_validates() {
        let mut jds = Jds::new(3);
        let d = dist();
        let id = jds.publish(800_000, 1_000, d.clone());
        let outputs = coinbase_outputs(&d);
        let template = build_jd_template(2, 800_000, 8, &outputs, 0).unwrap();
        let parsed = parse_coinbase_outputs(&template.suffix).unwrap();
        assert_eq!(parsed.len(), outputs.len());
        assert_eq!(parsed[0].value, outputs[0].1);
        assert_eq!(parsed[0].script, outputs[0].0.as_bytes());

        assert_eq!(jds.validate_declared(id, 800_000, &parsed), Ok(()));

        // Height straddling is stale even for the current id.
        assert_eq!(
            jds.validate_declared(id, 800_001, &parsed),
            Err(JdError::Stale)
        );

        // Mutated amounts and reordered outputs are invalid.
        let mut bad = parsed.clone();
        bad[0].value += 1;
        assert_eq!(
            jds.validate_declared(id, 800_000, &bad),
            Err(JdError::Invalid)
        );
        let mut swapped = parsed.clone();
        swapped.swap(0, 1);
        assert_eq!(
            jds.validate_declared(id, 800_000, &swapped),
            Err(JdError::Invalid)
        );
    }

    #[test]
    fn declared_coinbase_tolerates_a_duplicate_trailing_carrier() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let mut outputs = coinbase_outputs(&d);
        outputs.push((
            ScriptBuf::from_bytes(d.additional_outputs[0].script.clone()),
            0,
        ));
        let template = build_jd_template(2, 1, 8, &outputs, 0).unwrap();
        let parsed = parse_coinbase_outputs(&template.suffix).unwrap();
        assert_eq!(jds.validate_declared(id, 1, &parsed), Ok(()));
    }

    #[test]
    fn malformed_coinbase_suffixes_are_rejected() {
        assert!(parse_coinbase_outputs(&[]).is_err());
        assert!(parse_coinbase_outputs(&[0, 0, 0, 0]).is_err());
        // One output declared but no bytes for its value.
        assert!(parse_coinbase_outputs(&[0, 0, 0, 0, 1]).is_err());
        // Script length runs past the suffix.
        assert!(parse_coinbase_outputs(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0x51]).is_err());
    }

    fn plain_outputs(d: &SetPayoutDistribution) -> Vec<(Vec<u8>, u64)> {
        coinbase_outputs(d)
            .into_iter()
            .map(|(script, value)| (script.into_bytes(), value))
            .collect()
    }

    fn serialize_outputs(outputs: &[(Vec<u8>, u64)]) -> Vec<u8> {
        let mut out = Vec::new();
        write_compact_size(&mut out, outputs.len() as u64);
        for (script, value) in outputs {
            out.extend_from_slice(&value.to_le_bytes());
            write_compact_size(&mut out, script.len() as u64);
            out.extend_from_slice(script);
        }
        out
    }

    fn custom_job_payload(outputs: &[(Vec<u8>, u64)], tlv: Option<u64>) -> Vec<u8> {
        let encoded = serialize_outputs(outputs);
        let msg = SetCustomMiningJob {
            channel_id: 0,
            request_id: 1,
            token: B0255::new(b"token").unwrap(),
            version: 0x2000_0000,
            prev_hash: U256::from(&[0u8; 32]),
            min_ntime: 0,
            nbits: 0,
            coinbase_tx_version: 2,
            coinbase_prefix: B0255::new(b"").unwrap(),
            coinbase_tx_input_n_sequence: u32::MAX,
            coinbase_tx_outputs: B064K::new(&encoded).unwrap(),
            coinbase_tx_locktime: 0,
            merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
        };
        let mut bytes = binary_sv2::to_bytes(msg).unwrap();
        if let Some(id) = tlv {
            bytes.extend_from_slice(&DistributionIdTlv { id }.encode());
        }
        bytes
    }

    #[test]
    fn custom_job_valid_declaration_is_accepted() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let outputs = plain_outputs(&d);
        let payload = custom_job_payload(&outputs, Some(id));
        assert_eq!(check_custom_job(&jds, 1, &payload), Ok(id));
    }

    #[test]
    fn custom_job_mutated_or_reordered_payout_is_rejected() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let outputs = plain_outputs(&d);

        let mut mutated = outputs.clone();
        mutated[0].1 += 1;
        let err = check_custom_job(&jds, 1, &custom_job_payload(&mutated, Some(id))).unwrap_err();
        assert_eq!(err.code(), "invalid-payout-distribution");

        let mut swapped = outputs.clone();
        swapped.swap(0, 1);
        let err = check_custom_job(&jds, 1, &custom_job_payload(&swapped, Some(id))).unwrap_err();
        assert_eq!(err.code(), "invalid-payout-distribution");

        // Dropping the carrier is also a mismatch.
        let mut missing = outputs.clone();
        missing.pop();
        let err = check_custom_job(&jds, 1, &custom_job_payload(&missing, Some(id))).unwrap_err();
        assert_eq!(err.code(), "invalid-payout-distribution");
    }

    #[test]
    fn declaring_before_any_publication_is_a_coded_empty_decline() {
        let jds = Jds::new(0);
        let d = dist();
        let outputs = plain_outputs(&d);
        let err = check_custom_job(&jds, 1, &custom_job_payload(&outputs, Some(7))).unwrap_err();
        assert_eq!(err.code(), "no-payout-distribution");
        assert!(matches!(err, Decline::Rejected(JdError::Empty)));
    }

    #[test]
    fn custom_job_without_or_with_a_bad_tlv_is_rejected() {
        let mut jds = Jds::new(0);
        let d = dist();
        let id = jds.publish(1, 1_000, d.clone());
        let outputs = plain_outputs(&d);

        let err = check_custom_job(&jds, 1, &custom_job_payload(&outputs, None)).unwrap_err();
        assert_eq!(err.code(), "invalid-job");
        assert!(
            err.to_string()
                .contains("missing the appended distribution_id TLV"),
            "{err}"
        );

        // Truncated TLV header.
        let mut payload = custom_job_payload(&outputs, Some(id));
        payload.truncate(payload.len() - 10);
        assert!(check_custom_job(&jds, 1, &payload).is_err());

        // Wrong id at the current height is stale.
        let err = check_custom_job(&jds, 1, &custom_job_payload(&outputs, Some(id + 1)))
            .unwrap_err();
        assert_eq!(err.code(), "stale-payout-distribution");
        // Height straddling is stale.
        let err = check_custom_job(&jds, 2, &custom_job_payload(&outputs, Some(id))).unwrap_err();
        assert_eq!(err.code(), "stale-payout-distribution");
    }

    #[test]
    fn custom_job_appended_tlv_is_parsed_strictly() {
        let id = 0x0102_0304_0506_0708u64;
        let tlv = DistributionIdTlv { id }.encode();
        assert_eq!(distribution_id_from_trailing(&tlv), Ok(id));

        // Unknown TLV before the id is skipped.
        let mut mixed = vec![0x20, 0x00, 0x02, 0x02, 0x00, 0xaa, 0xbb];
        mixed.extend_from_slice(&tlv);
        assert_eq!(distribution_id_from_trailing(&mixed), Ok(id));

        // Duplicate ids and wrong lengths are rejected.
        let mut duplicate = tlv.clone();
        duplicate.extend_from_slice(&tlv);
        assert!(distribution_id_from_trailing(&duplicate).is_err());
        let mut wrong_len = vec![0x03, 0x00, 0x01, 0x04, 0x00, 0x01, 0x02, 0x03, 0x04];
        assert!(distribution_id_from_trailing(&wrong_len).is_err());
        wrong_len.clear();
        assert!(distribution_id_from_trailing(&wrong_len).is_err());
    }

    /// SPEC §3.5 "Declaration answers": a hand-rolled client gets a real
    /// `DeclareMiningJob.Error` frame whose `error_code` distinguishes the
    /// decline reasons from each other - and the typed message itself from a
    /// transport error (which would surface as a read failure, not a frame).
    #[tokio::test]
    async fn declarations_are_answered_with_coded_replies() {
        use stratum_apps::network_helpers::connect_with_noise;
        use stratum_core::common_messages_sv2::{Protocol, SetupConnection};
        use stratum_core::framing_sv2::framing::MessageFrame;
        use tokio::net::TcpStream;

        type ReadHalf = stratum_apps::network_helpers::noise_stream::NoiseTcpReadHalf;
        type WriteHalf = stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf;

        async fn send_declare(
            write: &mut WriteHalf,
            request_id: u32,
            tlv: Option<u64>,
            outputs: &[(ScriptBuf, u64)],
        ) {
            use stratum_core::framing_sv2::framing::MessageFrame;
            let template = build_jd_template(2, 1, 8, outputs, 0).unwrap();
            let tlv_bytes = tlv
                .map(|id| DistributionIdTlv { id }.encode())
                .unwrap_or_default();
            let msg = DeclareMiningJob {
                request_id,
                mining_job_token: B0255::new(b"token").unwrap(),
                version: 0x2000_0000,
                coinbase_tx_prefix: B064K::new(&template.prefix).unwrap(),
                coinbase_tx_suffix: B064K::new(&template.suffix).unwrap(),
                wtxid_list: Seq064K::try_from(Vec::<U256>::new()).unwrap(),
                excess_data: B064K::new(&tlv_bytes).unwrap(),
            };
            let frame =
                MessageFrame::from_message(msg, MESSAGE_TYPE_DECLARE_MINING_JOB, 0, false).unwrap();
            write.write_frame(frame).await.unwrap();
        }

        async fn read_answer(read: &mut ReadHalf, request_id: u32) -> Option<String> {
            loop {
                let mut f = read.read_frame().await.unwrap();
                match f.header().msg_type() {
                    MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS => {
                        let mut p = f.payload().to_vec();
                        let m: DeclareMiningJobSuccess = binary_sv2::from_bytes(&mut p).unwrap();
                        assert_eq!(m.request_id, request_id);
                        return None;
                    }
                    MESSAGE_TYPE_DECLARE_MINING_JOB_ERROR => {
                        let mut p = f.payload().to_vec();
                        let m: DeclareMiningJobError = binary_sv2::from_bytes(&mut p).unwrap();
                        assert_eq!(m.request_id, request_id);
                        return Some(m.error_code.as_utf8_or_hex().to_string());
                    }
                    _ => {}
                }
            }
        }

        let server = JdsServer::new(0);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let d = dist();
        let id = server.publish(1, 1_000, d.clone()).await.unwrap();

        let stream = TcpStream::connect(addr).await.unwrap();
        let stream = connect_with_noise(stream, None).await.unwrap();
        let (mut read, mut write) = stream.into_split();

        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: Protocol::JobDeclarationProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("test"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        let frame =
            MessageFrame::from_message(setup, MESSAGE_TYPE_SETUP_CONNECTION, 0, false).unwrap();
        write.write_frame(frame).await.unwrap();
        // SetupConnection.Success, then the current distribution.
        read.read_frame().await.unwrap();
        read.read_frame().await.unwrap();

        // Valid declaration -> Success echoing the request id.
        send_declare(&mut write, 10, Some(id), &coinbase_outputs(&d)).await;
        assert_eq!(read_answer(&mut read, 10).await, None);

        // Unknown id -> stale.
        send_declare(&mut write, 11, Some(id + 1_000_000), &coinbase_outputs(&d)).await;
        assert_eq!(
            read_answer(&mut read, 11).await,
            Some("stale-payout-distribution".to_string())
        );

        // Missing TLV -> invalid-job.
        send_declare(&mut write, 12, None, &coinbase_outputs(&d)).await;
        assert_eq!(read_answer(&mut read, 12).await, Some("invalid-job".to_string()));

        // Mutated amount -> invalid-payout-distribution.
        let mut bogus = coinbase_outputs(&d);
        bogus[0].1 += 1;
        send_declare(&mut write, 13, Some(id), &bogus).await;
        assert_eq!(
            read_answer(&mut read, 13).await,
            Some("invalid-payout-distribution".to_string())
        );

        assert_eq!(server.declare_count().await, 4);
        assert_eq!(server.accepted().await, Some(id));
    }
}
