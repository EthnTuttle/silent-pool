//! Exact selected runtime consumer. All monetary facts come from raw
//! consensus bytes; getmempoolinfo consumes only its integer size field.
use serde_json::Value;
use spc_lab_txd::control::{RpcRequest, MAX_BODY};

pub const CONTRACT: &str = "spc-lab-tx-rpc-consumer/1";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodClass {
    Read,
    Send,
    Bootstrap,
}

fn hash(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn raw(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        !s.is_empty() && s.len().is_multiple_of(2) && s.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

pub fn validate_request(
    request: &RpcRequest,
    bootstrap_open: bool,
) -> Result<MethodClass, &'static str> {
    if request.method.len() > 64
        || serde_json::to_vec(&request.params)
            .map_err(|_| "shape")?
            .len()
            > MAX_BODY
    {
        return Err("shape");
    }
    let p = request.params.as_array().ok_or("shape")?;
    let class = match request.method.as_str() {
        "getrawmempool" | "getmempoolinfo" | "getbestblockhash" if p.is_empty() => {
            MethodClass::Read
        }
        "getblockhash" if p.len() == 1 && p[0].as_u64().is_some() => MethodClass::Read,
        "getblockheader" if p.len() == 1 && hash(&p[0]) => MethodClass::Read,
        "getblock" if p.len() == 2 && hash(&p[0]) && p[1].as_u64() == Some(0) => MethodClass::Read,
        "getrawtransaction"
            if (p.len() == 2 || p.len() == 3)
                && hash(&p[0])
                && p[1].as_u64() == Some(0)
                && (p.len() == 2 || hash(&p[2])) =>
        {
            MethodClass::Read
        }
        "sendrawtransaction" if p.len() == 1 && raw(&p[0]) => MethodClass::Send,
        "getblocktemplate"
            if bootstrap_open
                && request.params
                    == serde_json::json!([{"mode":"template","rules":["segwit"]}]) =>
        {
            MethodClass::Bootstrap
        }
        "submitblock" if bootstrap_open && p.len() == 1 && raw(&p[0]) => MethodClass::Bootstrap,
        _ => return Err("method_or_params_not_allowed"),
    };
    if class == MethodClass::Send {
        if request.send_sequence.is_none_or(|n| n == 0)
            || !request
                .txid
                .as_ref()
                .is_some_and(|s| hash(&Value::String(s.clone())))
        {
            return Err("send_identity_missing");
        }
        let bytes = hex::decode(p[0].as_str().ok_or("shape")?).map_err(|_| "shape")?;
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&bytes).map_err(|_| "shape")?;
        if bitcoin::consensus::serialize(&tx) != bytes
            || Some(tx.compute_txid().to_string()) != request.txid
        {
            return Err("send_bytes_mismatch");
        }
    } else if request.send_sequence.is_some() || request.txid.is_some() {
        return Err("unexpected_send_identity");
    }
    Ok(class)
}

pub fn validate_result(method: &str, result: &Value) -> bool {
    match method {
        "sendrawtransaction" | "getbestblockhash" | "getblockhash" => hash(result),
        "getrawmempool" => result.as_array().is_some_and(|a| a.iter().all(hash)),
        "getmempoolinfo" => result.get("size").and_then(Value::as_u64).is_some(),
        "getrawtransaction" | "getblock" => raw(result),
        "getblockheader" => {
            result.get("height").and_then(Value::as_u64).is_some()
                && result
                    .get("confirmations")
                    .and_then(Value::as_i64)
                    .is_some()
        }
        "getblocktemplate" => result.is_object(),
        "submitblock" => result.is_null() || result.is_string(),
        _ => false,
    }
}
