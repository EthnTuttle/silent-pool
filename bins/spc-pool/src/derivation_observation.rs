//! Public inputs at the actual front cache lookup; no wallet/send secrets.
use crate::registration::ArkServerMetadata;
use serde_json::{json, Value};
use spc_accounting::MinerKeys;
use spc_store::models::{ArkDeliveryRow, ArkEpochMailboxRow, EpochFallback, MailboxMode};

pub fn inputs(
    id: &[u8; 32],
    keys: Option<&MinerKeys>,
    delivery: Option<&ArkDeliveryRow>,
    server: Option<&ArkServerMetadata>,
    epoch: Option<&ArkEpochMailboxRow>,
) -> Value {
    let registration=delivery.map(|d|json!({"ark_id":hex::encode(d.ark_id),"b_ark":hex::encode(&d.b_ark),
        "mailbox_mode":d.mailbox_mode.as_str(),"mailbox_m":d.mailbox_m.as_ref().map(hex::encode),
        "mailbox_d":d.mailbox_d.as_ref().map(hex::encode),"epoch_fallback":d.epoch_fallback.as_i16(),
        "commit":d.registration_commit}));
    let selection = delivery.map(|d| match d.mailbox_mode {
        MailboxMode::None => "none",
        MailboxMode::Static => "static",
        MailboxMode::EpochDerived => "epoch_derived",
        MailboxMode::EpochRegistered if epoch.is_some() => "registered_epoch",
        MailboxMode::EpochRegistered if d.epoch_fallback == EpochFallback::Hold => {
            "hold_missing_epoch"
        }
        MailboxMode::EpochRegistered => "static_fallback",
    });
    let observed_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    json!({"sp_id":hex::encode(id),"input_scope":"actual_front_cache","observed_unix_ms":observed_unix_ms,
        "scan_public":keys.map(|k|hex::encode(k.b_scan.serialize())),
        "spend_public":keys.map(|k|hex::encode(k.b_spend.serialize())),
        "coinbase_key_status":if keys.is_some(){"available"}else{"missing_miner_keys"},
        "registration":registration,"registration_presence":if delivery.is_some(){"present"}else{"absent_in_front_cache"},
        "server":server.map(|s|json!({"ark_id":hex::encode(s.ark_id),"server_pubkey":hex::encode(s.server_pubkey.serialize()),
            "mailbox_pubkey":hex::encode(s.mailbox_pubkey.serialize()),"testnet":s.testnet})),
        "epoch_mailbox":epoch.map(|e|json!({"height":e.height,"mailbox_mi":hex::encode(e.mailbox_mi)})),
        "mailbox_selection":selection,"destination_bytes":null,"outcome":"not_attempted"})
}
