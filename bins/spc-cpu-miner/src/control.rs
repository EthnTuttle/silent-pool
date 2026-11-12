use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::budget::HashBudget;
use crate::config::{ConfigError, ConnectionConfig};
use crate::ids::{CommandId, ConnectionId, IdentityId, WorkerId};

pub const CONTROL_PROTOCOL: &str = "spc-cpu-miner-control";
pub const CONTROL_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub protocol: String,
    pub version: u16,
    pub command_id: CommandId,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum Operation {
    AddConnection {
        connection: ConnectionConfig,
    },
    RemoveConnection {
        connection_id: ConnectionId,
    },
    SetWorkerBudget {
        worker_id: WorkerId,
        budget: HashBudget,
    },
    PauseWorkers {
        worker_ids: Vec<WorkerId>,
    },
    ResumeWorkers {
        worker_ids: Vec<WorkerId>,
    },
    SwitchPool {
        connection_id: ConnectionId,
        endpoint: String,
        identity: IdentityId,
        authority_public_key: String,
        min_extranonce_size: Option<u16>,
    },
    Snapshot {},
    Stop {},
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CommandValidationError {
    #[error("unsupported control protocol {0:?}")]
    Protocol(String),
    #[error("unsupported control version {0}")]
    Version(u16),
    #[error("command_id is required")]
    Id,
    #[error("operation requires a non-empty id list")]
    EmptyIds,
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Budget(#[from] crate::budget::BudgetError),
}

impl Command {
    pub fn validate(&self) -> Result<(), CommandValidationError> {
        if self.protocol != CONTROL_PROTOCOL {
            return Err(CommandValidationError::Protocol(self.protocol.clone()));
        }
        if self.version != CONTROL_VERSION {
            return Err(CommandValidationError::Version(self.version));
        }
        if self.command_id.trim().is_empty() {
            return Err(CommandValidationError::Id);
        }
        match &self.operation {
            Operation::AddConnection { connection } => connection.validate()?,
            Operation::SetWorkerBudget { budget, .. } => budget.validate()?,
            Operation::PauseWorkers { worker_ids } | Operation::ResumeWorkers { worker_ids }
                if worker_ids.is_empty() =>
            {
                return Err(CommandValidationError::EmptyIds);
            }
            Operation::SwitchPool {
                endpoint,
                identity,
                authority_public_key,
                min_extranonce_size,
                ..
            } => {
                if endpoint.trim().is_empty() {
                    return Err(ConfigError::Endpoint.into());
                }
                if identity.is_empty() || identity.len() > 255 {
                    return Err(ConfigError::Identity.into());
                }
                crate::config::parse_authority_public_key(authority_public_key)?;
                if min_extranonce_size.is_some_and(|size| !(4..=32).contains(&size)) {
                    return Err(ConfigError::ExtranonceSize.into());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn fingerprint(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "protocol": self.protocol,
            "version": self.version,
            "operation": self.operation,
        }))
        .expect("typed command serialization cannot fail")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CommandResult {
    pub protocol: String,
    pub version: u16,
    pub kind: String,
    pub command_id: CommandId,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}

impl CommandResult {
    pub fn succeeded(command_id: impl Into<CommandId>, result: serde_json::Value) -> Self {
        Self::outcome(command_id, "succeeded", Some(result), None)
    }

    pub fn running(command_id: impl Into<CommandId>, result: serde_json::Value) -> Self {
        Self::outcome(command_id, "running", Some(result), None)
    }

    pub fn rejected(
        command_id: impl Into<CommandId>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::outcome(
            command_id,
            "rejected_precondition",
            None,
            Some(CommandError {
                code: code.into(),
                message: message.into(),
            }),
        )
    }

    pub fn failed(
        command_id: impl Into<CommandId>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::outcome(
            command_id,
            "failed",
            None,
            Some(CommandError {
                code: code.into(),
                message: message.into(),
            }),
        )
    }

    pub fn outcome(
        command_id: impl Into<CommandId>,
        outcome: &str,
        result: Option<serde_json::Value>,
        error: Option<CommandError>,
    ) -> Self {
        Self {
            protocol: CONTROL_PROTOCOL.into(),
            version: CONTROL_VERSION,
            kind: "command_result".into(),
            command_id: command_id.into(),
            outcome: outcome.into(),
            result,
            error,
        }
    }
}

struct CommandRecord {
    fingerprint: String,
    result: CommandResult,
}

/// Process-lifetime idempotency. The runner owns durable journaling; this
/// ledger prevents duplicate effects until the process exits.
#[derive(Default)]
pub struct CommandLedger {
    records: HashMap<CommandId, CommandRecord>,
}

impl CommandLedger {
    pub fn begin(&self, command: &Command) -> Option<CommandResult> {
        self.records.get(&command.command_id).map(|record| {
            if record.fingerprint == command.fingerprint() {
                record.result.clone()
            } else {
                CommandResult::rejected(
                    command.command_id.clone(),
                    "command_id_conflict",
                    "command_id was already used with a different payload",
                )
            }
        })
    }

    pub fn complete(&mut self, command: &Command, result: CommandResult) {
        self.records.insert(
            command.command_id.clone(),
            CommandRecord {
                fingerprint: command.fingerprint(),
                result,
            },
        );
    }
}
