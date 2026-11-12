//! Native tx-driver pipe control and controller-owned RPC gate. All outgoing
//! effects share the gate mutex through RPC completion; pause closes it before
//! asking the actor to commit its drained durable barrier.
use super::consumer::{validate_request, validate_result, MethodClass};
use crate::adapters::RpcNodeControl;
use crate::command::traffic_audit::{self, Event as AuditEvent, HookEvent, HookKey, Session};
use crate::ids::{ActorId, CommandId, NodeId};
use serde_json::{json, Value};
use spc_lab_txd::control::{
    read_message, write_message, Message, RequestSequence, RpcOutcome, RpcRequest, UnknownClass,
    PROTOCOL,
};
use std::collections::BTreeMap;
use std::io::BufReader;
use std::process::{ChildStdin, ChildStdout};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

pub struct RpcGate {
    pub nodes: BTreeMap<NodeId, Arc<RpcNodeControl>>,
    closed: bool,
    bootstrap_open: bool,
    bootstrap_paused: bool,
    last_send: u64,
    budget: Option<(Arc<crate::command::CommandLog>, CommandId, String)>,
    audit: Option<(Arc<crate::command::CommandLog>, Session)>,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // Gate fixtures remain beside its private DTO.
mod acceptance_tests {
    use super::*;
    use crate::command::CommandLog;
    use spc_lab_txd::control::ForwardContext;
    fn setup(port: u16) -> (RpcGate, Arc<CommandLog>, std::path::PathBuf) {
        let path = std::path::PathBuf::from("/tmp/opencode")
            .join(format!("traffic-forward-{}.jsonl", CommandId::generate()));
        let log = Arc::new(CommandLog::open_traffic(&path).unwrap());
        let nodes = BTreeMap::from([(
            NodeId::trusted("node"),
            Arc::new(RpcNodeControl::new(
                NodeId::trusted("node"),
                crate::resource::Endpoint {
                    host: "127.0.0.1".into(),
                    port,
                },
                None,
            )),
        )]);
        let gate = RpcGate::new(nodes, 0)
            .with_audit(
                log.clone(),
                Session {
                    run_id: "run".into(),
                    driver_id: "driver".into(),
                    incarnation: "epoch".into(),
                    controller_sha256: "aa".repeat(32),
                    actor_sha256: "bb".repeat(32),
                    acceptance_enabled: true,
                },
            )
            .unwrap();
        (gate, log, path)
    }
    fn read_request() -> RpcRequest {
        RpcRequest {
            node_id: "node".into(),
            method: "getmempoolinfo".into(),
            params: json!([]),
            send_sequence: None,
            txid: None,
        }
    }
    fn context() -> ForwardContext {
        ForwardContext {
            command_id: Some("command".into()),
            journal_seq: 2,
            chain_revision: Some(2),
            intent_seq: None,
            raw_sha256: None,
        }
    }
    #[test]
    fn failed_connect_is_durably_attempted_and_completed() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let (mut gate, log, path) = setup(port);
        assert!(matches!(
            gate.call(1, &read_request(), &context()),
            RpcOutcome::Unknown { .. }
        ));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("forward_attempt"));
        assert!(text.contains("forward_result"));
        assert!(text.contains("watermark"));
        drop(log);
        CommandLog::open_traffic(&path).unwrap();
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn evidence_failure_prevents_connection_before_dispatch() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let (mut gate, log, path) = setup(socket.local_addr().unwrap().port());
        log.fail_audit_writes();
        assert!(matches!(
            gate.call(1, &read_request(), &context()),
            RpcOutcome::Unknown { .. }
        ));
        assert!(matches!(socket.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
        assert!(gate.closed);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn actual_socket_result_loss_is_once_and_closed_barrier_has_no_new_attempt() {
        use std::io::{BufRead, Read, Write};
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut gate, log, path) = setup(socket.local_addr().unwrap().port());
        let tx = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest).txdata[0]
            .clone();
        let raw = bitcoin::consensus::serialize(&tx);
        let raw_sha = traffic_audit::hash(&raw);
        let mut request = RpcRequest {
            node_id: "node".into(),
            method: "sendrawtransaction".into(),
            params: json!([hex::encode(raw)]),
            send_sequence: Some(1),
            txid: Some(tx.compute_txid().to_string()),
        };
        let mut context = ForwardContext {
            command_id: Some("payment".into()),
            journal_seq: 3,
            chain_revision: Some(1),
            intent_seq: Some(3),
            raw_sha256: Some(raw_sha.clone()),
        };
        let key = HookKey {
            driver_id: "driver".into(),
            incarnation: "epoch".into(),
            command_id: "payment".into(),
        };
        log.acceptance_action(
            key.clone(),
            traffic_audit::HookAction::Arm {
                mode: traffic_audit::HookMode::LoseOneResponse,
                timeout_ms: 120000,
            },
        )
        .unwrap();
        log.acceptance_checkpoint(
            key,
            &spc_lab_txd::control::SignedCheckpoint {
                command_id: "payment".into(),
                raw_sha256: raw_sha,
                journal_seq: 2,
            },
        )
        .unwrap();
        let evidence = path.clone();
        let txid = tx.compute_txid().to_string();
        let server = std::thread::spawn(move || {
            for count in 1..=2 {
                let (mut stream, _) = socket.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length: ") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let value: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    std::fs::read_to_string(&evidence)
                        .unwrap()
                        .matches("\"event\":\"forward_attempt\"")
                        .count(),
                    count
                );
                let response = json!({"id":value["id"],"result":txid,"error":null}).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
            }
        });
        assert_eq!(
            gate.call(1, &request, &context),
            RpcOutcome::unknown(UnknownClass::Transport)
        );
        request.send_sequence = Some(2);
        context.intent_seq = Some(4);
        context.journal_seq = 4;
        assert!(matches!(
            gate.call(2, &request, &context),
            RpcOutcome::Ok { .. }
        ));
        server.join().unwrap();
        gate.watermark(true, Some("pause".into())).unwrap();
        let before = std::fs::read(&path).unwrap();
        request.send_sequence = Some(3);
        assert_eq!(gate.call(3, &request, &context), RpcOutcome::GateClosed);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        CommandLog::open_traffic(&path).unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
impl RpcGate {
    pub fn new(nodes: BTreeMap<NodeId, Arc<RpcNodeControl>>, last_send: u64) -> Self {
        Self {
            nodes,
            closed: false,
            bootstrap_open: false,
            bootstrap_paused: false,
            last_send,
            budget: None,
            audit: None,
        }
    }
    pub fn with_audit(
        mut self,
        log: Arc<crate::command::CommandLog>,
        binding: Session,
    ) -> std::io::Result<Self> {
        log.audit(AuditEvent::Session {
            binding: binding.clone(),
        })?;
        self.audit = Some((log, binding));
        Ok(self)
    }
    fn watermark(&mut self, closed: bool, command: Option<String>) -> Result<(), String> {
        self.closed = true;
        let (log, binding) = self.audit.as_ref().ok_or("traffic_audit_unbound")?;
        log.audit_watermark(&binding.incarnation, closed, command)
            .map_err(|e| e.to_string())?;
        self.closed = closed;
        Ok(())
    }
    fn call(
        &mut self,
        id: u64,
        request: &RpcRequest,
        context: &spc_lab_txd::control::ForwardContext,
    ) -> RpcOutcome {
        let class = match validate_request(request, self.bootstrap_open) {
            Ok(class) => class,
            Err(_) => return RpcOutcome::unknown(UnknownClass::Shape),
        };
        if class != MethodClass::Read && self.closed {
            return RpcOutcome::GateClosed;
        }
        let node_id = match NodeId::new(&request.node_id) {
            Ok(id) => id,
            Err(_) => return RpcOutcome::unknown(UnknownClass::Shape),
        };
        let Some(node) = self.nodes.get(&node_id) else {
            return RpcOutcome::unknown(UnknownClass::Shape);
        };
        if class == MethodClass::Send {
            let allowed = (|| -> std::io::Result<bool> {
                let (log, binding) = self
                    .audit
                    .as_ref()
                    .ok_or_else(|| spc_lab_txd::control::invalid("audit missing"))?;
                let command_id = context
                    .command_id
                    .clone()
                    .ok_or_else(|| spc_lab_txd::control::invalid("send command missing"))?;
                log.acceptance_forward_allowed(
                    &HookKey {
                        driver_id: binding.driver_id.clone(),
                        incarnation: binding.incarnation.clone(),
                        command_id,
                    },
                    context.raw_sha256.as_deref(),
                )
            })();
            if !matches!(allowed, Ok(true)) {
                return RpcOutcome::GateClosed;
            }
            let raw = request
                .params
                .get(0)
                .and_then(Value::as_str)
                .and_then(|s| hex::decode(s).ok());
            if raw.as_ref().is_none_or(|raw| {
                context.raw_sha256.as_deref() != Some(traffic_audit::hash(raw).as_str())
            }) {
                return RpcOutcome::unknown(UnknownClass::Protocol);
            }
            let sequence = request.send_sequence.unwrap();
            if sequence <= self.last_send {
                return RpcOutcome::unknown(UnknownClass::Protocol);
            }
            // Gaps can represent journaled-but-withheld sends, but duplicate
            // wire requests never forward twice within this incarnation.
            self.last_send = sequence;
        }
        let Some((log, binding)) = &self.audit else {
            self.closed = true;
            return RpcOutcome::unknown(UnknownClass::Protocol);
        };
        let append = || -> std::io::Result<u64> {
            let params = serde_json::to_vec(&request.params)?;
            log.audit(AuditEvent::ForwardAttempt {
                attempt: traffic_audit::Attempt {
                    incarnation: binding.incarnation.clone(),
                    rpc_request_id: id,
                    node_id: request.node_id.clone(),
                    method: request.method.clone(),
                    send_sequence: request.send_sequence,
                    payload_sha256: traffic_audit::hash(&params),
                    context: context.clone(),
                    permit_revision: log.audit_revision(&binding.incarnation)?,
                },
            })
        };
        let attempt = match append() {
            Ok(seq) => seq,
            Err(_) => {
                self.closed = true;
                return RpcOutcome::unknown(UnknownClass::Protocol);
            }
        };
        // No asynchronous evidence queue: the attempt is fsynced before the
        // actual host/broker dispatch, including connect/write failures.
        let actual = node.call_traffic(id, &request.method, request.params.clone());
        let key = context.command_id.as_ref().map(|command_id| HookKey {
            driver_id: binding.driver_id.clone(),
            incarnation: binding.incarnation.clone(),
            command_id: command_id.clone(),
        });
        let lose = class == MethodClass::Send
            && key.as_ref().is_some_and(|key| {
                log.acceptance_lose_response(key, context.raw_sha256.as_deref())
            });
        let logged = (|| -> std::io::Result<()> {
            let outcome = match &actual {
                RpcOutcome::Ok { .. } => "ok",
                RpcOutcome::NodeRejected { .. } => "node_rejected",
                RpcOutcome::Unknown { .. } => "unknown",
                RpcOutcome::GateClosed => "gate_closed",
            };
            log.audit(AuditEvent::ForwardResult {
                attempt_seq: attempt,
                outcome: outcome.into(),
                response_sha256: traffic_audit::hash(&serde_json::to_vec(&actual)?),
                response_suppressed: lose,
            })?;
            if lose {
                log.audit(AuditEvent::Hook {
                    key: key.unwrap(),
                    detail: HookEvent::ResponseDiscarded {
                        attempt_seq: attempt,
                    },
                })?;
            }
            log.audit_watermark(&binding.incarnation, self.closed, None)?;
            Ok(())
        })();
        if logged.is_err() {
            self.closed = true;
            return RpcOutcome::unknown(UnknownClass::Protocol);
        }
        if lose {
            return RpcOutcome::unknown(UnknownClass::Transport);
        }
        match actual {
            RpcOutcome::Ok { result } if !validate_result(&request.method, &result) => {
                RpcOutcome::unknown(UnknownClass::Shape)
            }
            result => result,
        }
    }

    fn hash_request(
        &mut self,
        operation: &str,
        payload: Value,
        incarnation: &str,
    ) -> Result<Value, String> {
        if operation == "hash_reserve" && self.bootstrap_paused {
            return Err("bootstrap_paused".into());
        }
        let (log, id, driver) = self.budget.as_ref().ok_or("bootstrap_window_closed")?;
        if !self.bootstrap_open || self.closed {
            return Err("bootstrap_window_closed".into());
        }
        match operation {
            "hash_status" => {
                serde_json::to_value(log.hash_budget_snapshot(id).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())
            }
            "hash_reserve" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Reserve {
                    operation_id: CommandId,
                    batch_id: u64,
                    attempts: u64,
                }
                let request: Reserve =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                if request.operation_id != *id {
                    return Err("hash_operation_mismatch".into());
                }
                let state = log.hash_budget_snapshot(id).map_err(|e| e.to_string())?;
                if state.operation.driver_id != *driver {
                    return Err("hash_driver_mismatch".into());
                }
                let status = log
                    .reserve_hashes(
                        id,
                        incarnation,
                        request.batch_id,
                        request.attempts,
                        &spc_lab_txd::bootstrap::boot_id().map_err(|e| e.to_string())?,
                        spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                serde_json::to_value(status).map_err(|e| e.to_string())
            }
            "hash_reconcile" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Reconcile {
                    authorization: crate::command::traffic_budget::HashAuthorization,
                    measured_attempts: Option<u64>,
                }
                let request: Reconcile =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                if request.authorization.operation_id != *id
                    || request.authorization.driver_incarnation != incarnation
                {
                    return Err("hash_reconcile_binding_mismatch".into());
                }
                log.reconcile_hashes(&request.authorization, request.measured_attempts)
                    .map_err(|e| e.to_string())?;
                Ok(json!({"status":"reconciled"}))
            }
            _ => Err("unsupported_hash_operation".into()),
        }
    }
}

pub struct TxDriverControl {
    payment_ready: AtomicBool,
    driver_id: ActorId,
    incarnation: String,
    stdin: Arc<Mutex<ChildStdin>>,
    serial: Mutex<u64>,
    pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Message>>>>,
    events: Arc<Mutex<mpsc::Receiver<Message>>>,
    failed: Arc<AtomicBool>,
    gate: Arc<Mutex<RpcGate>>,
}

impl TxDriverControl {
    pub fn spawn(
        driver_id: ActorId,
        incarnation: String,
        stdin: ChildStdin,
        stdout: ChildStdout,
        gate: RpcGate,
    ) -> std::io::Result<Self> {
        let stdin = Arc::new(Mutex::new(stdin));
        let pending = Arc::new(Mutex::new(BTreeMap::<u64, mpsc::Sender<Message>>::new()));
        let (events_tx, events_rx) = mpsc::sync_channel(64);
        let failed = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(Mutex::new(gate));
        let (input, waiting, broken, rpc_gate) =
            (stdin.clone(), pending.clone(), failed.clone(), gate.clone());
        let (driver, epoch) = (driver_id.clone(), incarnation.clone());
        std::thread::Builder::new()
            .name("txd-control-reader".into())
            .spawn(move || {
                let run = || -> std::io::Result<()> {
                    let mut reader = BufReader::new(stdout);
                    let mut requests = RequestSequence::default();
                    let mut event_sequence = RequestSequence::default();
                    while let Some(message) = read_message(&mut reader)? {
                        if broken.load(Ordering::SeqCst) {
                            return Err(spc_lab_txd::control::invalid(
                                "controller acknowledgment lost",
                            ));
                        }
                        message.validate_binding(driver.as_str(), &epoch)?;
                        match message {
                            message @ Message::Response { request_id, .. } => {
                                let sender =
                                    waiting.lock().unwrap().remove(&request_id).ok_or_else(
                                        || {
                                            spc_lab_txd::control::invalid(
                                                "unsolicited/duplicate actor response",
                                            )
                                        },
                                    )?;
                                sender.send(message).map_err(|_| {
                                    spc_lab_txd::control::invalid("response waiter lost")
                                })?;
                            }
                            Message::Request {
                                request_id,
                                command_id: None,
                                operation,
                                payload,
                                ..
                            } if operation == "rpc_forward" => {
                                requests.accept(request_id)?;
                                let forward: spc_lab_txd::control::ForwardRequest =
                                    serde_json::from_value(payload)?;
                                let request = forward.request;
                                let result = rpc_gate.lock().unwrap().call(
                                    request_id,
                                    &request,
                                    &forward.context,
                                );
                                write_message(
                                    &mut *input.lock().unwrap(),
                                    &Message::Response {
                                        protocol: PROTOCOL.into(),
                                        incarnation: epoch.clone(),
                                        driver_id: driver.to_string(),
                                        request_id,
                                        command_id: None,
                                        result: json!({"request":request,"result":result}),
                                    },
                                )?;
                            }
                            Message::Request {
                                request_id,
                                command_id: None,
                                operation,
                                payload,
                                ..
                            } if operation == "acceptance_checkpoint" => {
                                requests.accept(request_id)?;
                                let checkpoint: spc_lab_txd::control::SignedCheckpoint =
                                    serde_json::from_value(payload)?;
                                let gate = rpc_gate.lock().unwrap();
                                let (log, binding) = gate.audit.as_ref().ok_or_else(|| {
                                    spc_lab_txd::control::invalid("audit missing")
                                })?;
                                if !binding.acceptance_enabled {
                                    return Err(spc_lab_txd::control::invalid(
                                        "acceptance disabled",
                                    ));
                                }
                                let key = HookKey {
                                    driver_id: driver.to_string(),
                                    incarnation: epoch.clone(),
                                    command_id: checkpoint.command_id.clone(),
                                };
                                let status = log.acceptance_checkpoint(key, &checkpoint)?;
                                write_message(
                                    &mut *input.lock().unwrap(),
                                    &Message::Response {
                                        protocol: PROTOCOL.into(),
                                        incarnation: epoch.clone(),
                                        driver_id: driver.to_string(),
                                        request_id,
                                        command_id: None,
                                        result: json!({"status":status}),
                                    },
                                )?;
                            }
                            Message::Request {
                                request_id,
                                command_id: None,
                                operation,
                                payload,
                                ..
                            } if operation == "hash_reserve"
                                || operation == "hash_reconcile"
                                || operation == "hash_status" =>
                            {
                                requests.accept(request_id)?;
                                let result = match rpc_gate
                                    .lock()
                                    .unwrap()
                                    .hash_request(&operation, payload, &epoch)
                                {
                                    Ok(value) => json!({"status":"ok","value":value}),
                                    Err(error) => json!({"status":"failed","code":error}),
                                };
                                write_message(
                                    &mut *input.lock().unwrap(),
                                    &Message::Response {
                                        protocol: PROTOCOL.into(),
                                        incarnation: epoch.clone(),
                                        driver_id: driver.to_string(),
                                        request_id,
                                        command_id: None,
                                        result,
                                    },
                                )?;
                            }
                            message @ Message::Event { event_id, .. } => {
                                event_sequence.accept(event_id)?;
                                if let Message::Event { name, payload, .. } = &message {
                                    if name == "tx_traffic" {
                                        let _: spc_lab_txd::control::TrafficObservation =
                                            serde_json::from_value(payload.clone())?;
                                    } else if name != "hello" {
                                        return Err(spc_lab_txd::control::invalid(
                                            "unknown actor event",
                                        ));
                                    }
                                }
                                events_tx.try_send(message).map_err(|_| {
                                    spc_lab_txd::control::invalid("actor event queue overflow")
                                })?;
                            }
                            _ => {
                                return Err(spc_lab_txd::control::invalid(
                                    "unsupported actor message",
                                ))
                            }
                        }
                    }
                    Err(spc_lab_txd::control::invalid("actor pipe closed"))
                };
                let _ = run();
                broken.store(true, Ordering::SeqCst);
                rpc_gate.lock().unwrap().closed = true;
                waiting.lock().unwrap().clear();
            })?;
        Ok(Self {
            payment_ready: AtomicBool::new(false),
            driver_id,
            incarnation,
            stdin,
            serial: Mutex::new(1),
            pending,
            events: Arc::new(Mutex::new(events_rx)),
            failed,
            gate,
        })
    }

    pub fn request(
        &self,
        command: Option<&CommandId>,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        // One controller command in flight per actor. RPC servicing is on the
        // reader thread and remains available while the command waits.
        let serial = self.serial.lock().unwrap();
        self.request_locked(command, operation, payload, serial)
    }

    fn request_locked(
        &self,
        command: Option<&CommandId>,
        operation: &str,
        payload: Value,
        mut serial: std::sync::MutexGuard<'_, u64>,
    ) -> Result<Value, String> {
        if self.failed.load(Ordering::SeqCst) {
            return Err("actor_control_lost".into());
        }
        let id = *serial;
        *serial = id
            .checked_add(1)
            .ok_or("actor_request_sequence_exhausted")?;
        let command_id = command.map(ToString::to_string);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(error) = write_message(
            &mut *self.stdin.lock().unwrap(),
            &Message::Request {
                protocol: PROTOCOL.into(),
                incarnation: self.incarnation.clone(),
                driver_id: self.driver_id.to_string(),
                request_id: id,
                command_id: command_id.clone(),
                operation: operation.into(),
                payload,
            },
        ) {
            self.failed.store(true, Ordering::SeqCst);
            return Err(error.to_string());
        }
        let acceptance = self
            .gate
            .lock()
            .unwrap()
            .audit
            .as_ref()
            .is_some_and(|(_, s)| s.acceptance_enabled);
        match rx.recv_timeout(Duration::from_secs(
            if acceptance && operation != "payment_wallet" {
                150
            } else {
                15
            },
        )) {
            Ok(Message::Response {
                request_id,
                command_id: echoed,
                result,
                ..
            }) if request_id == id && echoed == command_id => Ok(result),
            _ => {
                self.failed.store(true, Ordering::SeqCst);
                Err("actor_ack_unknown".into())
            }
        }
    }

    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }
    pub fn payment_ready(&self) -> bool {
        self.payment_ready.load(Ordering::SeqCst) && !self.failed.load(Ordering::SeqCst)
    }
    pub fn read_payment_wallet(
        &self,
        request: spc_lab_txd::payment_view::PaymentRead,
    ) -> Result<Value, String> {
        if !self.payment_ready() {
            return Err("payment_provider_unavailable".into());
        }
        let serial = self
            .serial
            .try_lock()
            .map_err(|_| "payment_provider_busy")?;
        self.request_locked(
            None,
            "payment_wallet",
            serde_json::to_value(request).map_err(|e| e.to_string())?,
            serial,
        )
    }
    pub fn acceptance_control(
        &self,
        key: HookKey,
        action: traffic_audit::HookAction,
    ) -> Result<u64, String> {
        let gate = self.gate.lock().unwrap(); // drains any already forwarded call
        let (log, binding) = gate.audit.as_ref().ok_or("audit missing")?;
        if !binding.acceptance_enabled
            || key.driver_id != binding.driver_id
            || key.incarnation != binding.incarnation
        {
            return Err("acceptance binding/enablement mismatch".into());
        }
        log.acceptance_action(key, action)
            .map_err(|e| e.to_string())
    }
    pub fn event_receiver(&self) -> Arc<Mutex<mpsc::Receiver<Message>>> {
        Arc::clone(&self.events)
    }
    pub fn close_gate(&self) {
        let _ = self.gate.lock().unwrap().watermark(true, None);
    }
    pub fn open_bootstrap(
        &self,
        log: Arc<crate::command::CommandLog>,
        request: &spc_lab_txd::bootstrap::BootstrapRequest,
    ) -> Result<(), String> {
        let id = CommandId::new(&request.command_id).map_err(|e| e.to_string())?;
        let snapshot = log.hash_budget_snapshot(&id).map_err(|e| e.to_string())?;
        if snapshot.operation.driver_id != self.driver_id.as_str()
            || snapshot.operation.boot_id != request.boot_id
            || snapshot.operation.deadline_monotonic_ms != request.deadline_monotonic_ms
        {
            return Err("bootstrap_budget_binding_mismatch".into());
        }
        if let Some(old) = snapshot
            .outstanding
            .filter(|a| a.driver_incarnation != self.incarnation)
        {
            log.reconcile_hashes(&old, None)
                .map_err(|e| e.to_string())?;
        }
        let mut gate = self.gate.lock().unwrap();
        gate.bootstrap_open = true;
        gate.budget = Some((log, id.clone(), self.driver_id.to_string()));
        drop(gate);
        let result = self.request(
            Some(&id),
            "bootstrap_window",
            serde_json::to_value(request).map_err(|e| e.to_string())?,
        )?;
        if result["status"].as_str() != Some("bootstrap_window_open") {
            self.close_bootstrap();
            return Err("bootstrap_actor_window_failed".into());
        }
        Ok(())
    }
    pub fn close_bootstrap(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.bootstrap_open = false;
        gate.budget = None;
    }
    pub fn pause_bootstrap(&self, paused: bool) {
        self.gate.lock().unwrap().bootstrap_paused = paused;
    }

    pub fn hello(&self) -> Result<Value, String> {
        let event = self
            .events
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "actor_hello_timeout")?;
        let Message::Event { name, payload, .. } = event else {
            return Err("actor_hello_invalid".into());
        };
        if name != "hello" {
            return Err("actor_hello_invalid".into());
        }
        let hello: super::protocol::TxControlHello =
            serde_json::from_value(payload).map_err(|e| e.to_string())?;
        hello.validate().map_err(|e| e.to_string())?;
        if hello.driver_id != self.driver_id {
            return Err("actor_hello_driver_mismatch".into());
        }
        let result = self.request(
            None,
            "hello_ack",
            json!({"protocol_name":"spc-lab-tx-control", "protocol_major":1,
            "protocol_minor":0, "accepted":true, "history_version":3, "reason":null}),
        )?;
        let paused = result
            .get("paused")
            .and_then(Value::as_bool)
            .ok_or("actor_pause_restore_missing")?;
        let sequence = result
            .get("last_used_send_sequence")
            .and_then(Value::as_u64)
            .ok_or("actor_sequence_restore_missing")?;
        let mut gate = self.gate.lock().unwrap();
        gate.watermark(paused, None)?;
        gate.last_send = sequence;
        self.payment_ready
            .store(hello.supports("payment_wallet_read_v1"), Ordering::SeqCst);
        Ok(result)
    }

    pub fn pause(&self, command: &CommandId) -> Result<Value, String> {
        self.gate
            .lock()
            .unwrap()
            .watermark(true, Some(command.to_string()))?; // drains calls
        let result = self.request(Some(command), "pause", json!({}))?;
        if result["status"].as_str() != Some("paused") {
            return Err("actor_pause_not_acknowledged".into());
        }
        let sequence = result["last_used_send_sequence"]
            .as_u64()
            .ok_or("actor_pause_sequence_missing")?;
        let last = result.get("last_txid").ok_or("actor_pause_txid_missing")?;
        let txid_valid = if sequence == 0 {
            last.is_null()
        } else {
            last.as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        };
        if !txid_valid
            || result["command_id"].as_str() != Some(command.as_str())
            || result["wall_ms"].as_u64().is_none()
            || result["monotonic_ms"].as_u64().is_none()
            || result["journal_seq"].as_u64().is_none()
        {
            return Err("actor_pause_ack_invalid".into());
        }
        let mut gate = self.gate.lock().unwrap();
        if sequence < gate.last_send {
            return Err("actor_pause_sequence_regressed".into());
        }
        gate.last_send = sequence;
        gate.watermark(true, Some(command.to_string()))?;
        Ok(result)
    }
    pub fn resume(&self, command: &CommandId) -> Result<Value, String> {
        let result = self.request(Some(command), "resume", json!({}))?;
        if result.get("status").and_then(Value::as_str) == Some("resumed") {
            self.gate
                .lock()
                .unwrap()
                .watermark(false, Some(command.to_string()))?;
        }
        Ok(result)
    }
}
