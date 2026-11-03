//! The transport contract and the dev/test command transport.
//!
//! `ArkTransport` is the whole order-execution contract (decision record
//! 2026-11-02-bark-adapter.md, clause 3): the only effect the order stream
//! can trigger. Order verification, caps and dedupe stay in the role
//! ([`super::OrderVerifier`], [`super::OrderDedupe`]); a transport only
//! pays. The production implementation is the bark-backed
//! [`super::bark_transport::BarkTransport`]; the command transport here is
//! selected by `[ark] transport = "command"`, which config validation
//! refuses outside regtest/signet (SPEC §11.9).

use std::time::Duration;

use tokio::process::Command;

/// The only side effect the role needs: pay an address from the float.
/// Returns `(vtxo_id, expiry_height)` for the receipt: the 36-byte
/// `ark::VtxoId` of a delivered VTXO and the minimum expiry height across
/// delivered VTXOs (decision record, clause 2).
#[async_trait::async_trait]
pub trait ArkTransport: Send + Sync + 'static {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String>;
}

/// Blanket implementation for Box<dyn ArkTransport>
#[async_trait::async_trait]
impl ArkTransport for Box<dyn ArkTransport> {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String> {
        (**self).send(address, sats).await
    }
}

/// External command transport for Ark sends. Dev/test only: config
/// validation refuses `transport = "command"` outside regtest/signet.
/// The command receives JSON on stdin: `{"address": "<ark1...>", "sats": 12345}`
/// It MUST return JSON on stdout: `{"vtxo_id": "<hex>", "expiry_height": <u32>}`
/// Non-zero exit, timeout, invalid JSON, or wrong types are treated as transport failures.
pub struct ExternalCommandTransport {
    command: String,
    timeout: Duration,
}

impl ExternalCommandTransport {
    pub fn new(command: String, timeout_secs: u64) -> Self {
        Self {
            command,
            timeout: Duration::from_secs(timeout_secs),
        }
    }
}

#[async_trait::async_trait]
impl ArkTransport for ExternalCommandTransport {
    async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String> {
        // Defensive re-parse of the address to ensure it's valid before sending
        let _parsed: ark::Address = address
            .parse()
            .map_err(|e| format!("transport: address parse failed: {e}"))?;

        let input = serde_json::json!({
            "address": address,
            "sats": sats,
        });

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&self.command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("transport: failed to spawn command: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "transport: failed to open stdin".to_string())?;
        use tokio::io::AsyncWriteExt;
        let mut stdin = stdin;
        stdin
            .write_all(input.to_string().as_bytes())
            .await
            .map_err(|e| format!("transport: failed to write stdin: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| format!("transport: failed to close stdin: {e}"))?;

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("transport: command timed out after {:?}", self.timeout))?
            .map_err(|e| format!("transport: command failed: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "transport: command exited with {}: {}",
                output.status, stderr
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("transport: invalid JSON output: {e}"))?;

        let vtxo_id_hex = result
            .get("vtxo_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "transport: missing or invalid 'vtxo_id' field".to_string())?;
        let expiry_height = result
            .get("expiry_height")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "transport: missing or invalid 'expiry_height' field".to_string())?;

        let vtxo_id_bytes =
            hex::decode(vtxo_id_hex).map_err(|e| format!("transport: invalid vtxo_id hex: {e}"))?;

        // A real `ark::VtxoId` is 36 bytes; receipts carry exactly one
        // (WIRE-FORMATS.md §3.4).
        if vtxo_id_bytes.len() != 36 {
            return Err(format!(
                "transport: vtxo_id must be 36 bytes (72 hex chars), got {}",
                vtxo_id_bytes.len()
            ));
        }

        let mut vtxo_id = [0u8; 36];
        vtxo_id.copy_from_slice(&vtxo_id_bytes);

        Ok((
            vtxo_id.to_vec(),
            u32::try_from(expiry_height)
                .map_err(|_| "transport: expiry_height exceeds u32".to_string())?,
        ))
    }
}
