//! Single-owner actor loop. Chain commits, intent admission and the actual
//! RPC handoff are serialized here, not by a journal guard returned earlier.
use crate::actor_config::{confine_root, ActorConfig};
use crate::builder::PaymentCategory;
use crate::control::{
    invalid, write_message, Message, RequestSequence, RpcOutcome, RpcRequest, PROTOCOL,
};
use crate::keys::WalletRole;
use crate::wallet::{PaymentOutcome, PaymentRequest, SignedPaymentView, TrafficWalletSet};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

mod bootstrap_control;
mod pump;

pub struct Actor<W> {
    config: ActorConfig,
    incarnation: String,
    output: W,
    input: Receiver<io::Result<Message>>,
    queued: VecDeque<Message>,
    requests: RequestSequence,
    next_rpc: u64,
    next_event: u64,
    wallet: Option<TrafficWalletSet>,
    active: bool,
    schedule: Option<crate::schedule::Schedule>,
    index: Option<crate::observer::CanonicalIndex>,
    next_observation: Instant,
    next_retry: Instant,
    bootstrap_window: Option<crate::bootstrap::BootstrapRequest>,
    hash_consumption: Option<crate::bootstrap::BatchConsumption>,
    measured_start_ms: Option<u64>,
    next_rate_report: Instant,
    current_command: Option<String>,
    payment_observation: Option<crate::payment_view::ObservationStamp>,
}

impl<W: Write> Actor<W> {
    pub fn new(
        config: ActorConfig,
        incarnation: String,
        input: Receiver<io::Result<Message>>,
        output: W,
    ) -> io::Result<Self> {
        config.validate()?;
        crate::keys::validate_run_id(&incarnation).map_err(|_| invalid("invalid incarnation"))?;
        Ok(Self {
            config,
            incarnation,
            input,
            output,
            queued: VecDeque::new(),
            requests: RequestSequence::default(),
            next_rpc: 1,
            next_event: 1,
            wallet: None,
            active: false,
            schedule: None,
            index: None,
            next_observation: Instant::now(),
            next_retry: Instant::now(),
            bootstrap_window: None,
            hash_consumption: None,
            measured_start_ms: None,
            next_rate_report: Instant::now(),
            current_command: None,
            payment_observation: None,
        })
    }

    fn event(&mut self, name: &str, payload: Value) -> io::Result<()> {
        let event_id = self.next_event;
        self.next_event = self
            .next_event
            .checked_add(1)
            .ok_or_else(|| invalid("event sequence exhausted"))?;
        write_message(
            &mut self.output,
            &Message::Event {
                protocol: PROTOCOL.into(),
                incarnation: self.incarnation.clone(),
                driver_id: self.config.driver_id.clone(),
                event_id,
                name: name.into(),
                payload,
            },
        )
    }

    pub fn run(mut self) -> io::Result<()> {
        let own = self
            .config
            .allocation
            .actors
            .iter()
            .find(|a| a.driver_id == self.config.driver_id)
            .unwrap();
        self.event("hello", json!({"protocol_name":"spc-lab-tx-control", "protocol_major":1, "protocol_minor":0,
            "driver_id":self.config.driver_id, "network_wallets":own.network_wallets, "user_payment_wallets":own.user_payment_wallets,
            "capabilities":["ordinary_churn","user_payment","inject_tx","bootstrap_fund","payment_wallet_read_v1"]}))?;
        loop {
            let message = match self.queued.pop_front() {
                Some(message) => message,
                None => match self.input.recv_timeout(Duration::from_millis(100)) {
                    Ok(message) => message?,
                    Err(RecvTimeoutError::Timeout) => {
                        self.pump()?;
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(invalid("controller pipe closed"))
                    }
                },
            };
            message.validate_binding(&self.config.driver_id, &self.incarnation)?;
            let Message::Request {
                request_id,
                command_id,
                operation,
                payload,
                ..
            } = message
            else {
                return Err(invalid("unsolicited response/event from controller"));
            };
            self.requests.accept(request_id)?;
            let stop = operation == "stop";
            self.current_command = command_id.clone();
            let result = match self.dispatch(&operation, command_id.as_deref(), payload) {
                Ok(value) => value,
                Err(error) => json!({"status":"failed", "code":error.to_string()}),
            };
            self.current_command = None;
            write_message(
                &mut self.output,
                &Message::Response {
                    protocol: PROTOCOL.into(),
                    incarnation: self.incarnation.clone(),
                    driver_id: self.config.driver_id.clone(),
                    request_id,
                    command_id,
                    result,
                },
            )?;
            if stop {
                return Ok(());
            }
        }
    }

    fn dispatch(
        &mut self,
        operation: &str,
        command: Option<&str>,
        payload: Value,
    ) -> io::Result<Value> {
        if operation == "hello_ack" {
            if self.wallet.is_some()
                || payload.get("accepted") != Some(&Value::Bool(true))
                || payload.get("history_version").and_then(Value::as_u64) != Some(3)
                || payload.get("protocol_name").and_then(Value::as_str)
                    != Some("spc-lab-tx-control")
                || payload.get("protocol_major").and_then(Value::as_u64) != Some(1)
            {
                return Err(invalid("hello acknowledgment mismatch"));
            }
            let allocation = self.config.validate()?;
            let root = self.config.root()?;
            confine_root(&self.config.run_dir, &root)?;
            let history = crate::journal::TrafficJournal::path_in(&root);
            let wallet = if history.exists() {
                TrafficWalletSet::reopen(&allocation, &self.config.driver_id, &root)
            } else {
                TrafficWalletSet::provision(&allocation, &self.config.driver_id, &root)
            }
            .map_err(|e| invalid(&e.to_string()))?;
            let paused = wallet.journal().barrier_active();
            self.wallet = Some(wallet);
            return Ok(
                json!({"status":"provisioned", "paused":paused, "allocation_digest":allocation.digest(),
                "last_used_send_sequence":self.wallet.as_ref().unwrap().journal().next_send_sequence()-1}),
            );
        }
        if operation == "stop" {
            self.active = false;
            return Ok(json!({"status":"stopped"}));
        }
        if operation == "payment_wallet" {
            return self.payment_wallet(payload);
        }
        if matches!(
            operation,
            "bootstrap_rpc"
                | "funding_address"
                | "observe_bootstrap_coinbase"
                | "distribute"
                | "bootstrap_status"
        ) {
            return self.bootstrap_control(operation, command, payload);
        }
        let version_rejection = operation == "acceptance_version_rejection_v1";
        if version_rejection && !self.config.lab_acceptance {
            return Err(invalid("acceptance_construction_disabled"));
        }
        if operation == "send_user_payment" || version_rejection {
            // Reject invalid output intent before even a fresh chain observation
            // can append to the journal. The complete typed/ownership/request
            // checks below still run before signing, after the normal rescan.
            let address = payload
                .get("destination_address")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("destination_invalid"))?;
            let amount = payload
                .get("amount_sats")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("destination_amount_invalid"))?;
            self.wallet
                .as_ref()
                .ok_or_else(|| invalid("hello_required"))?
                .checked_destination(
                    bitcoin::Address::from_str(address)
                        .map_err(|_| invalid("destination_invalid"))?,
                    amount,
                )
                .map_err(|e| invalid(&e.to_string()))?;
        }
        if matches!(operation, "send_user_payment" | "inject") || version_rejection {
            self.observe_authority().map_err(|e| invalid(&e))?;
        }
        let wallet = self
            .wallet
            .as_ref()
            .ok_or_else(|| invalid("hello_required"))?;
        match operation {
            "snapshot" => Ok(self.public_snapshot()),
            "bootstrap" => Err(invalid("traffic_bootstrap_executor_unavailable")),
            "bootstrap_window" => {
                let request: crate::bootstrap::BootstrapRequest = serde_json::from_value(payload)?;
                if command != Some(request.command_id.as_str())
                    || request.driver_id != self.config.driver_id
                    || request.node_id != self.config.authority_node
                    || self.active
                    || !self
                        .config
                        .allocation
                        .actors
                        .iter()
                        .any(|a| a.driver_id == self.config.driver_id && a.hosts_bootstrap_funding)
                    || request.boot_id != crate::bootstrap::boot_id()?
                    || request.deadline_monotonic_ms <= crate::bootstrap::monotonic_ms()?
                {
                    return Err(invalid("bootstrap_window_binding_invalid"));
                }
                if self.bootstrap_window.as_ref().is_some_and(|old| {
                    old.command_id != request.command_id
                        || old.deadline_monotonic_ms != request.deadline_monotonic_ms
                        || old.boot_id != request.boot_id
                }) {
                    return Err(invalid("bootstrap_window_conflict"));
                }
                if self.hash_consumption.is_none() {
                    self.hash_consumption = Some(crate::bootstrap::BatchConsumption::new(
                        request.command_id.clone(),
                        self.incarnation.clone(),
                    ));
                }
                self.bootstrap_window = Some(request);
                Ok(json!({"status":"bootstrap_window_open"}))
            }
            "reserve_hashes" | "reconcile_hashes" | "budget_status" => {
                let window = self
                    .bootstrap_window
                    .as_ref()
                    .ok_or_else(|| invalid("bootstrap_window_closed"))?;
                if command != Some(window.command_id.as_str())
                    || window.boot_id != crate::bootstrap::boot_id()?
                {
                    return Err(invalid("bootstrap_operation_mismatch"));
                }
                if operation == "reserve_hashes"
                    && crate::bootstrap::monotonic_ms()? >= window.deadline_monotonic_ms
                {
                    return Err(invalid("bootstrap_deadline"));
                }
                let wire_operation = match operation {
                    "reserve_hashes" => "hash_reserve",
                    "reconcile_hashes" => "hash_reconcile",
                    _ => "hash_status",
                };
                let value = self.exchange(wire_operation, payload)?;
                if value["status"].as_str() != Some("ok") {
                    return Err(invalid("hash_authorization_unavailable"));
                }
                if operation == "reserve_hashes" {
                    let status: crate::bootstrap::HashReservationStatus =
                        serde_json::from_value(value["value"].clone())?;
                    let execute = self
                        .hash_consumption
                        .as_mut()
                        .ok_or_else(|| invalid("hash_consumption_missing"))?
                        .consume(&status)
                        .map_err(|e| invalid(&e))?;
                    Ok(json!({"status":"authorized", "execute":execute,"reservation":status}))
                } else {
                    Ok(value["value"].clone())
                }
            }
            "observe" => {
                self.next_observation = Instant::now();
                self.index = None;
                Ok(json!({"status":"observation_requested"}))
            }
            // Only the completion-driven controller path can issue activate.
            "activate" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Activation {
                    monotonic_ms: u64,
                    boot_id: String,
                }
                let activation: Activation = serde_json::from_value(payload)?;
                if activation.boot_id != crate::bootstrap::boot_id()?
                    || activation.monotonic_ms > crate::bootstrap::monotonic_ms()?
                {
                    return Err(invalid("activation_clock_binding_invalid"));
                }
                if wallet.journal().tip_height().is_none()
                    || wallet.journal().summary().funding_observations == 0
                {
                    return Err(invalid("traffic_funding_unconfirmed"));
                }
                self.schedule = Some(crate::schedule::Schedule::restore(
                    self.config.allocation.run_seed,
                    self.config.driver_id.clone(),
                    self.config.ordinary_interval_ms,
                    activation.monotonic_ms,
                    &wallet.journal().recovery_snapshot(),
                )?);
                self.bootstrap_window = None;
                self.measured_start_ms = Some(activation.monotonic_ms);
                self.active = true;
                Ok(json!({"status":"active"}))
            }
            "pause" => {
                if !self.active {
                    return Err(invalid("traffic_not_active"));
                }
                let command = command.ok_or_else(|| invalid("durable_command_id_required"))?;
                wallet
                    .pause_sends(command, crate::bootstrap::monotonic_ms()?)
                    .map_err(|e| invalid(&e.to_string()))?;
                let snapshot = wallet.journal().recovery_snapshot();
                let latest_send = snapshot
                    .transactions
                    .iter()
                    .filter_map(|(txid, tx)| tx.last_send.map(|(seq, _)| (seq, txid)))
                    .max_by_key(|(seq, _)| *seq)
                    .map(|(_, txid)| txid.to_string());
                Ok(
                    json!({"status":"paused", "command_id":command, "wall_ms":crate::now_unix_ms(),
                    "monotonic_ms":crate::bootstrap::monotonic_ms()?, "last_used_send_sequence":snapshot.next_send_sequence-1,
                    "last_txid":snapshot.last_broadcast_txid.map(|id|id.to_string()), "last_intent_txid":latest_send,
                    "journal_seq":snapshot.committed_seq}),
                )
            }
            "resume" => {
                let command = command.ok_or_else(|| invalid("durable_command_id_required"))?;
                wallet
                    .resume_sends(command)
                    .map_err(|e| invalid(&e.to_string()))?;
                Ok(json!({"status":"resumed"}))
            }
            "send_user_payment" | "acceptance_version_rejection_v1" => {
                if !self.active || wallet.journal().barrier_active() {
                    return Err(invalid("traffic_not_active"));
                }
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Payment {
                    user: String,
                    driver_id: String,
                    destination_address: String,
                    amount_sats: u64,
                    fee_rate_sats_vb: u64,
                }
                let p: Payment = serde_json::from_value(payload)?;
                if command.is_some_and(|id| id.starts_with(crate::schedule::COMMAND_PREFIX)) {
                    return Err(invalid("reserved_command_namespace"));
                }
                let (owner, index) = self.config.allocation.owner_of_user(&p.user)?;
                if owner != self.config.driver_id || p.driver_id != owner {
                    return Err(invalid("payment_driver_mismatch"));
                }
                let request = PaymentRequest {
                    command_id: command
                        .ok_or_else(|| invalid("durable_command_id_required"))?
                        .into(),
                    payer_role: WalletRole::UserPayment { index },
                    category: PaymentCategory::UserPayment,
                    destinations: vec![wallet
                        .checked_destination(
                            bitcoin::Address::from_str(&p.destination_address)
                                .map_err(|_| invalid("destination_invalid"))?,
                            p.amount_sats,
                        )
                        .map_err(|e| invalid(&e.to_string()))?],
                    fee_rate_sats_vb: p.fee_rate_sats_vb,
                };
                let signed = if version_rejection {
                    wallet
                        .sign_version_rejection_for_acceptance(&request, self.config.lab_acceptance)
                } else {
                    wallet.sign_payment(&request)
                }
                .map_err(|e| invalid(&e.to_string()))?;
                match signed {
                    PaymentOutcome::Signed(view) => self.forward(&view),
                    PaymentOutcome::ReplayedStored(view) => {
                        Ok(json!({"status":"stored", "txid":view.txid.to_string(),
                        "state":wallet.journal().tx_state(&view.txid).map(|s| s.as_str())}))
                    }
                }
            }
            "inject" => {
                if !self.active || wallet.journal().barrier_active() {
                    return Err(invalid("traffic_not_active"));
                }
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Injection {
                    driver_id: String,
                    raw_tx_hex: String,
                }
                let injection: Injection = serde_json::from_value(payload)?;
                let command = command.ok_or_else(|| invalid("durable_command_id_required"))?;
                if command.starts_with(crate::schedule::COMMAND_PREFIX)
                    || injection.driver_id != self.config.driver_id
                    || !self
                        .config
                        .allocation
                        .actors
                        .iter()
                        .any(|a| a.driver_id == self.config.driver_id && a.hosts_bootstrap_funding)
                {
                    return Err(invalid("traffic_injection_host_invalid"));
                }
                let facts = crate::admit::admit_declared_tx(
                    wallet,
                    command,
                    &injection.raw_tx_hex,
                    self.config.max_vbytes,
                )
                .map_err(|e| invalid(&e))?;
                if facts.state == crate::journal::TxState::Signed {
                    self.send_facts(&facts, None)
                } else {
                    Ok(
                        json!({"status":"stored", "txid":bitcoin::consensus::deserialize::<bitcoin::Transaction>(&hex::decode(&facts.raw_hex).map_err(|_|invalid("journal raw hex"))?).map_err(|_|invalid("journal raw transaction"))?.compute_txid().to_string()}),
                    )
                }
            }
            _ => Err(invalid("unsupported_actor_operation")),
        }
    }

    fn forward(&mut self, signed: &SignedPaymentView) -> io::Result<Value> {
        let facts = self
            .wallet
            .as_ref()
            .unwrap()
            .journal()
            .tx_facts(&signed.txid)
            .ok_or_else(|| invalid("signed facts missing"))?;
        self.send_facts(&facts, None)
    }

    pub(super) fn payment_wallet(&self, payload: Value) -> io::Result<Value> {
        let request = serde_json::from_value(payload)?;
        let wallet = self
            .wallet
            .as_ref()
            .ok_or_else(|| invalid("hello_required"))?;
        let reply = crate::payment_view::read_wallet(
            request,
            &self.config.allocation,
            &self.config.driver_id,
            wallet,
            self.index.as_ref(),
            self.payment_observation.as_ref(),
            self.active,
        )?;
        Ok(serde_json::to_value(reply)?)
    }

    fn send_facts(
        &mut self,
        signed: &crate::journal::SignedTxFacts,
        existing_sequence: Option<u64>,
    ) -> io::Result<Value> {
        let selected = self
            .wallet
            .as_ref()
            .unwrap()
            .journal()
            .command(&signed.command_id)
            .is_some_and(|c| c.request.construction_effect.is_some());
        if selected {
            if !self.config.lab_acceptance {
                return Err(invalid("acceptance_construction_disabled"));
            }
            // Exact replay through the existing declared-raw guard cannot
            // create another signature/reservation or change the chosen effect.
            crate::admit::admit_declared_tx(
                self.wallet.as_ref().unwrap(),
                &signed.command_id,
                &signed.raw_hex,
                self.config.max_vbytes,
            )
            .map_err(|e| invalid(&e))?;
        }
        // Explicit acceptance hook observes an ALREADY durable Signed record.
        // It can delay/cancel only, before allocating a send intent. On release
        // the real chain is observed and core maturity/admission runs normally.
        if self.config.lab_acceptance && existing_sequence.is_none() {
            self.acceptance_before_intent(signed)?;
        }
        self.collect_controls()?;
        if !self.queued.is_empty() && existing_sequence.is_none() {
            return Ok(json!({"status":"deferred"}));
        }
        let wallet = self.wallet.as_ref().unwrap();
        if signed.vbytes > self.config.max_vbytes {
            return Err(invalid("traffic_size_cap_exceeded"));
        }
        let sequence = match existing_sequence {
            Some(sequence) => sequence,
            None => wallet
                .note_broadcast_sent(&signed.command_id)
                .map_err(|e| invalid(&e.to_string()))?,
        };
        let txid = wallet
            .journal()
            .command(&signed.command_id)
            .and_then(|c| c.txid)
            .ok_or_else(|| invalid("send txid missing"))?;
        let intent = wallet
            .journal()
            .pending_send_intent(&signed.command_id)
            .map_err(|e| invalid(&e.to_string()))?;
        // This actor loop is the ONLY local chain writer, and cannot commit a
        // rollback between this recheck and the actual pipe write below.
        if let Err(error) = wallet
            .journal()
            .check_send_before_forward(&intent, wallet.journal().last_seq())
        {
            wallet
                .note_unknown(&signed.command_id, "local_send_revalidation_failed")
                .map_err(|e| invalid(&e.to_string()))?;
            return Err(invalid(&error.to_string()));
        }
        let request = RpcRequest {
            node_id: self.config.authority_node.clone(),
            method: "sendrawtransaction".into(),
            params: json!([signed.raw_hex]),
            send_sequence: Some(sequence),
            txid: Some(txid.to_string()),
        };
        let result = self.rpc(request);
        let wallet = self.wallet.as_ref().unwrap();
        let profile = self
            .config
            .node_profile_binding
            .as_ref()
            .and_then(crate::send_outcome::bind);
        if let Ok(outcome) = &result {
            if let Some(code) = crate::send_outcome::definitive_rejection(
                outcome,
                profile.as_ref(),
                &signed.raw_hex,
            ) {
                wallet
                    .note_rejected(&signed.command_id, code)
                    .map_err(|e| invalid(&e.to_string()))?;
                self.emit_tx(
                    &signed.command_id,
                    crate::control::TrafficEventKind::Rejected,
                )?;
                return Ok(json!({"status":"rejected","code":code,"txid":txid.to_string()}));
            }
        }
        match result {
            Ok(RpcOutcome::Ok { result }) if result.as_str() == Some(&txid.to_string()) => {
                wallet
                    .note_accepted_by_rpc(&signed.command_id, &self.config.authority_node)
                    .map_err(|e| invalid(&e.to_string()))?;
                self.emit_tx(
                    &signed.command_id,
                    crate::control::TrafficEventKind::AcceptedByRpc,
                )?;
                Ok(json!({"status":"accepted_by_rpc", "txid":txid.to_string()}))
            }
            _ => {
                wallet
                    .note_unknown(&signed.command_id, "rpc_outcome_uncertain")
                    .map_err(|e| invalid(&e.to_string()))?;
                self.emit_tx(
                    &signed.command_id,
                    crate::control::TrafficEventKind::Unknown,
                )?;
                Ok(json!({"status":"unknown", "txid":txid.to_string()}))
            }
        }
    }

    fn rpc(&mut self, request: RpcRequest) -> io::Result<RpcOutcome> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ResultEnvelope {
            request: RpcRequest,
            result: RpcOutcome,
        }
        let snapshot = self
            .wallet
            .as_ref()
            .map(|w| w.journal().recovery_snapshot());
        let tx = snapshot.as_ref().and_then(|s| {
            request
                .txid
                .as_ref()
                .and_then(|id| bitcoin::Txid::from_str(id).ok())
                .and_then(|id| s.transactions.get(&id))
        });
        let context = crate::control::ForwardContext {
            command_id: tx
                .map(|t| t.facts.command_id.clone())
                .or_else(|| self.current_command.clone())
                .or_else(|| self.bootstrap_window.as_ref().map(|w| w.command_id.clone())),
            journal_seq: snapshot.as_ref().map_or(0, |s| s.committed_seq),
            chain_revision: snapshot.as_ref().and_then(|s| s.last_chain_observed_seq),
            intent_seq: tx.and_then(|t| t.last_send.map(|(_, seq)| seq)),
            raw_sha256: tx.map(|t| {
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(
                    hex::decode(&t.facts.raw_hex).expect("validated journal hex"),
                ))
            }),
        };
        let result = self.exchange(
            "rpc_forward",
            serde_json::to_value(crate::control::ForwardRequest {
                request: request.clone(),
                context,
            })?,
        )?;
        let envelope: ResultEnvelope = serde_json::from_value(result)?;
        if envelope.request != request {
            return Err(invalid("RPC response correlation mismatch"));
        }
        Ok(envelope.result)
    }

    fn acceptance_before_intent(
        &mut self,
        signed: &crate::journal::SignedTxFacts,
    ) -> io::Result<()> {
        use sha2::Digest;
        let raw = hex::decode(&signed.raw_hex).map_err(|_| invalid("signed hex"))?;
        let payload = serde_json::to_value(crate::control::SignedCheckpoint {
            command_id: signed.command_id.clone(),
            raw_sha256: hex::encode(sha2::Sha256::digest(raw)),
            journal_seq: self.wallet.as_ref().unwrap().journal().last_seq(),
        })?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut held = false;
        loop {
            let reply = self.exchange("acceptance_checkpoint", payload.clone())?;
            match reply["status"].as_str() {
                Some("proceed") => {
                    if held {
                        self.observe_authority().map_err(|e| invalid(&e))?;
                    }
                    return Ok(());
                }
                Some("hold") if Instant::now() < deadline => {
                    held = true;
                    self.observe_authority().map_err(|e| invalid(&e))?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                _ => return Err(invalid("acceptance_cancelled_or_unavailable")),
            }
        }
    }

    fn exchange(&mut self, operation: &str, payload: Value) -> io::Result<Value> {
        let id = self.next_rpc;
        self.next_rpc = self
            .next_rpc
            .checked_add(1)
            .ok_or_else(|| invalid("rpc sequence exhausted"))?;
        write_message(
            &mut self.output,
            &Message::Request {
                protocol: PROTOCOL.into(),
                incarnation: self.incarnation.clone(),
                driver_id: self.config.driver_id.clone(),
                request_id: id,
                command_id: None,
                operation: operation.into(),
                payload,
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| invalid("RPC timeout"))?;
            let message = self
                .input
                .recv_timeout(remaining)
                .map_err(|_| invalid("RPC response unavailable"))??;
            message.validate_binding(&self.config.driver_id, &self.incarnation)?;
            match message {
                Message::Response {
                    request_id,
                    command_id: None,
                    result,
                    ..
                } if request_id == id => {
                    return Ok(result);
                }
                message @ Message::Request { .. } if self.queued.len() < 16 => {
                    self.queued.push_back(message)
                }
                _ => return Err(invalid("unexpected RPC response")),
            }
        }
    }
}
