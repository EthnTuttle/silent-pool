//! Narrow adapter to the AP-3b owner's private command queue. Queueing is
//! Running, never Succeeded. Only an actual confirmed agent result completes.
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::command::{CommandKind, CommandOutcome, LabCommand, OutcomeState};
use crate::ids::IdentityId;
use crate::wallet_projection::{inputs, private_dir, private_read, WalletBinding};

pub fn identity(kind: &CommandKind) -> Option<&IdentityId> {
    match kind {
        CommandKind::WalletRefresh { identity_id, .. }
        | CommandKind::WalletOffboard { identity_id, .. }
        | CommandKind::WalletReadInputs { identity_id } => Some(identity_id),
        _ => None,
    }
}

#[derive(Serialize)]
struct AgentCommand<'a> {
    operation_id: String,
    vtxo_ids: &'a [String],
    action: Action<'a>,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action<'a> {
    Refresh,
    Offboard { address: &'a str },
}

pub fn operation_id(command: &LabCommand) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            "spc-lab-wallet/v1",
            &command.run_id,
            &command.actor,
            &command.command_id,
        ))
        .expect("command serialization"),
    ))
}

pub fn validate(kind: &CommandKind) -> Result<(), &'static str> {
    let ids = match kind {
        CommandKind::WalletReadInputs { .. } => return Ok(()),
        CommandKind::WalletRefresh { vtxo_ids, .. } => vtxo_ids,
        CommandKind::WalletOffboard {
            vtxo_ids, address, ..
        } => {
            let parsed = address
                .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
                .map_err(|_| "invalid_offboard_destination")?
                .require_network(bitcoin::Network::Regtest)
                .map_err(|_| "invalid_offboard_network")?;
            // Keep this initial UI policy narrow and explicit: native regtest
            // witness destinations. No ambiguous testnet/regtest base58.
            if !address.starts_with("bcrt1") || !parsed.script_pubkey().is_witness_program() {
                return Err("unsupported_offboard_destination");
            }
            vtxo_ids
        }
        _ => return Err("not_a_wallet_command"),
    };
    let mut seen = std::collections::BTreeSet::new();
    if ids.is_empty() || ids.len() > 64 {
        return Err("wallet_needs_explicit_inputs");
    }
    for id in ids {
        if id.len() != 72
            || !id.bytes().all(|c| c.is_ascii_hexdigit())
            || !seen.insert(id.to_ascii_lowercase())
        {
            return Err("wallet_inputs_invalid");
        }
    }
    Ok(())
}

pub fn apply(binding: Option<WalletBinding>, command: &LabCommand, replay: bool) -> CommandOutcome {
    let make = |state, detail| CommandOutcome::new(command.command_id.clone(), state, detail);
    let Some(binding) = binding else {
        if replay {
            return make(
                OutcomeState::Unknown,
                "wallet provider not registered; result unavailable, retain original command",
            );
        }
        return make(
            OutcomeState::RejectedPrecondition,
            "wallet provider not registered",
        );
    };
    if validate(&command.kind).is_err() {
        return make(
            OutcomeState::RejectedPrecondition,
            "wallet command validation failed",
        );
    }
    if matches!(command.kind, CommandKind::WalletReadInputs { .. }) {
        return match inputs(&binding) {
            Ok(_) => make(
                OutcomeState::Succeeded,
                "private wallet inputs available through scoped wallets view",
            ),
            Err(_) => make(
                OutcomeState::RejectedPrecondition,
                "private agent input provider unavailable or stale",
            ),
        };
    }
    let op = operation_id(command);
    if replay {
        return result(&binding, command);
    }
    let (ids, action) = match &command.kind {
        CommandKind::WalletRefresh { vtxo_ids, .. } => (vtxo_ids, Action::Refresh),
        CommandKind::WalletOffboard {
            vtxo_ids, address, ..
        } => (vtxo_ids, Action::Offboard { address }),
        _ => unreachable!(),
    };
    let Ok(summary) = inputs(&binding) else {
        return make(
            OutcomeState::RejectedPrecondition,
            "private agent input provider unavailable or stale",
        );
    };
    if ids.iter().any(|id| {
        !summary
            .inputs
            .iter()
            .any(|row| row.id.eq_ignore_ascii_case(id) && row.state == "spendable")
    }) {
        return make(
            OutcomeState::RejectedPrecondition,
            "selected wallet inputs unavailable for this action",
        );
    }
    let rails: std::collections::BTreeSet<_> = summary
        .inputs
        .iter()
        .filter(|row| ids.iter().any(|id| row.id.eq_ignore_ascii_case(id)))
        .map(|row| row.rail.as_str())
        .collect();
    if rails.len() != 1 {
        return make(
            OutcomeState::RejectedPrecondition,
            "select wallet inputs from one rail only",
        );
    }
    let payload = serde_json::to_vec(&AgentCommand {
        operation_id: op.clone(),
        vtxo_ids: ids,
        action,
    })
    .expect("agent command serialization");
    let write = || -> Result<(), &'static str> {
        private_dir(&binding.datadir)?;
        let dir = binding.datadir.join("commands");
        if !dir.exists() {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(|_| "queue_unavailable")?;
        }
        private_dir(&dir)?;
        let destination = dir.join(format!("{op}.json"));
        if destination.exists() {
            return if private_read(&destination)? == payload {
                Ok(())
            } else {
                Err("payload_conflict")
            };
        }
        let temp = dir.join(format!("{op}.ui.tmp"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|_| "queue_unavailable")?;
        file.write_all(&payload)
            .and_then(|_| file.sync_all())
            .map_err(|_| "queue_unavailable")?;
        if std::fs::hard_link(&temp, &destination).is_err() {
            // Only this call's create_new temp is ours. Never unlink the
            // destination or an agent result, including on publication races.
            let _ = std::fs::remove_file(&temp);
            return Err("queue_unavailable");
        }
        std::fs::remove_file(&temp).map_err(|_| "queue_unavailable")?;
        std::fs::File::open(&dir)
            .and_then(|f| f.sync_all())
            .map_err(|_| "queue_unavailable")?;
        Ok(())
    };
    match write() {
        Ok(()) => make(
            OutcomeState::Running,
            "submitted to agent queue; awaiting actual wallet result",
        ),
        Err(_) => make(
            OutcomeState::Unknown,
            "wallet queue outcome unknown; retain this command id and payload",
        ),
    }
}

fn result(binding: &WalletBinding, command: &LabCommand) -> CommandOutcome {
    let state = (|| {
        private_dir(&binding.datadir)?;
        private_dir(&binding.datadir.join("commands"))?;
        let bytes = private_read(
            &binding
                .datadir
                .join("commands")
                .join(format!("{}.result", operation_id(command))),
        )?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid_result")?;
        if value["operation_id"].as_str() != Some(&operation_id(command)) {
            return Err("invalid_result");
        }
        Ok(value["state"].as_str().unwrap_or("unknown").to_string())
    })();
    let (state, detail) = match state.as_deref() {
        Ok("round_confirmed") if matches!(command.kind, CommandKind::WalletRefresh { .. }) => {
            (OutcomeState::Succeeded, "agent confirmed refresh round")
        }
        Ok("offboard_confirmed") if matches!(command.kind, CommandKind::WalletOffboard { .. }) => {
            (OutcomeState::Succeeded, "agent confirmed offboard")
        }
        Ok("rejected_no_effect" | "noop") => (
            OutcomeState::RejectedPrecondition,
            "agent reports no wallet effect",
        ),
        Ok("stock_submitted") => (
            OutcomeState::Running,
            "agent submitted; confirmation not yet observed",
        ),
        _ => (
            OutcomeState::Unknown,
            "agent result pending or unknown; retain original command",
        ),
    };
    CommandOutcome::new(command.command_id.clone(), state, detail)
}
