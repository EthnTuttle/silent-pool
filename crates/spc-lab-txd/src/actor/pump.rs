//! Background work is multiplexed with the same bounded control pipe. There
//! is one wallet owner; observation commits and actual sends cannot interleave.
use super::*;
use crate::builder::{DestinationSpec, PaymentCategory};
use crate::control::{TrafficEventKind, TrafficObservation};
use crate::journal::{CommandPhase, RequestRecord, TxState};
use crate::observer::{CanonicalIndex, RawRpc};
use std::sync::mpsc::TryRecvError;

impl<W: Write> Actor<W> {
    pub(super) fn collect_controls(&mut self) -> io::Result<()> {
        while matches!(self.queued.front(),Some(Message::Request {operation,..}) if matches!(operation.as_str(),"snapshot"|"payment_wallet"))
        {
            let request = self.queued.pop_front().unwrap();
            self.answer_snapshot(request)?;
        }
        loop {
            match self.input.try_recv() {
                Ok(Ok(message @ Message::Request { .. })) if self.queued.len() < 16 => {
                    message.validate_binding(&self.config.driver_id, &self.incarnation)?;
                    self.queued.push_back(message);
                    while matches!(self.queued.front(),Some(Message::Request {operation,..}) if matches!(operation.as_str(),"snapshot"|"payment_wallet"))
                    {
                        let request = self.queued.pop_front().unwrap();
                        self.answer_snapshot(request)?;
                    }
                }
                Ok(Ok(_)) => return Err(invalid("unsolicited control response or queue overflow")),
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Err(invalid("controller pipe closed")),
            }
        }
    }

    pub(super) fn public_snapshot(&self) -> Value {
        json!({"status":if self.index.is_some(){"observed"}else{"observing"},"scan_complete":self.index.is_some(),
            "tip":self.index.as_ref().map(|i|i.anchor()),"active":self.active,
            "paused":self.wallet.as_ref().map(|w|w.journal().barrier_active()),
            "summary":self.wallet.as_ref().map(|w|w.journal().summary()),"journal_seq":self.wallet.as_ref().map(|w|w.journal().last_seq())})
    }
    fn answer_snapshot(&mut self, message: Message) -> io::Result<()> {
        let Message::Request {
            request_id,
            command_id,
            operation,
            payload,
            ..
        } = message
        else {
            return Err(invalid("snapshot request expected"));
        };
        self.requests.accept(request_id)?;
        let result = if operation == "payment_wallet" {
            match self.payment_wallet(payload) {
                Ok(v) => v,
                Err(_) => json!({"status":"failed","code":"payment_read_unavailable"}),
            }
        } else {
            self.public_snapshot()
        };
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
        )
    }

    pub(super) fn observe_authority(&mut self) -> Result<(), String> {
        let policy = self.config.chain_policy.clone();
        let node = self.config.authority_node.clone();
        self.index = None;
        let index = CanonicalIndex::scan(self, &node, policy.genesis, policy.pow_limit()?)?;
        index.verify_policy(&policy)?;
        let wallet = self.wallet.take().ok_or("wallet not provisioned")?;
        let lookahead = self
            .config
            .allocation
            .actors
            .iter()
            .find(|a| a.driver_id == self.config.driver_id)
            .ok_or("allocation owner missing")?
            .lookahead;
        let result = index.observe(
            &wallet,
            self,
            &node,
            policy.genesis,
            policy.pow_limit()?,
            lookahead,
        );
        self.wallet = Some(wallet);
        result?;
        self.payment_observation = Some(crate::payment_view::ObservationStamp {
            boot_id: crate::bootstrap::boot_id().map_err(|e| e.to_string())?,
            monotonic_ms: crate::bootstrap::monotonic_ms().map_err(|e| e.to_string())?,
            wall_ms: crate::now_unix_ms(),
        });
        self.index = Some(index);
        Ok(())
    }

    pub(super) fn pump(&mut self) -> io::Result<()> {
        if self.wallet.is_none() {
            return Ok(());
        }
        self.collect_controls()?;
        if !self.queued.is_empty() {
            return Ok(());
        }
        if self.active && Instant::now() >= self.next_rate_report {
            self.next_rate_report = Instant::now() + Duration::from_secs(30);
            let snapshot = self.wallet.as_ref().unwrap().journal().recovery_snapshot();
            let count = snapshot
                .transactions
                .values()
                .filter(|tx| {
                    tx.facts
                        .command_id
                        .starts_with(crate::schedule::COMMAND_PREFIX)
                        && tx.last_send.is_some()
                })
                .count() as u64;
            let mut event = TrafficObservation::empty(TrafficEventKind::RateMeasured);
            event.requested_interval_ms = Some(self.config.ordinary_interval_ms);
            event.measured_elapsed_ms = Some(
                crate::bootstrap::monotonic_ms()?
                    .checked_sub(
                        self.measured_start_ms
                            .ok_or_else(|| invalid("measured boundary missing"))?,
                    )
                    .ok_or_else(|| invalid("measured clock moved backwards"))?,
            );
            event.ordinary_broadcast_count = Some(count);
            self.traffic_event(event)?;
        }
        if Instant::now() >= self.next_observation {
            self.next_observation = Instant::now() + Duration::from_secs(1);
            let previous = self.wallet.as_ref().unwrap().journal().recovery_snapshot();
            if self.observe_authority().is_err() {
                if self.queued.is_empty() {
                    self.traffic_event(TrafficObservation::health(false))?;
                }
                return Ok(());
            }
            let after = self.wallet.as_ref().unwrap().journal().recovery_snapshot();
            for (txid, tx) in &after.transactions {
                if previous
                    .transactions
                    .get(txid)
                    .is_none_or(|old| old.facts.state != tx.facts.state)
                {
                    let kind = match tx.facts.state {
                        TxState::Confirmed => TrafficEventKind::Confirmed,
                        TxState::Reorged | TxState::Invalidated => TrafficEventKind::Reorged,
                        _ => continue,
                    };
                    self.emit_tx(&tx.facts.command_id, kind)?;
                }
            }
            self.traffic_event(TrafficObservation::health(true))?;
        }
        let paused = self.wallet.as_ref().unwrap().journal().barrier_active();
        if paused || (!self.active && self.bootstrap_window.is_none()) || self.index.is_none() {
            return Ok(());
        }
        if Instant::now() >= self.next_retry {
            self.next_retry = Instant::now() + Duration::from_secs(2);
            self.recover_one()?;
        }
        if self.active && self.queued.is_empty() {
            let now = crate::bootstrap::monotonic_ms()?;
            if let Some(slot) = self
                .schedule
                .as_mut()
                .map(|s| s.next_slot(now))
                .transpose()?
                .flatten()
            {
                self.ordinary(slot)?;
            }
        }
        Ok(())
    }

    fn recover_one(&mut self) -> io::Result<()> {
        let snapshot = self.wallet.as_ref().unwrap().journal().recovery_snapshot();
        if snapshot.poisoned {
            return Err(invalid("traffic journal poisoned"));
        }
        for command in snapshot.commands.values() {
            if command.txid.is_none()
                && matches!(command.phase, CommandPhase::Began | CommandPhase::Reserved)
            {
                // External declared bytes are not re-signed after an incomplete
                // admission. Ordinary/user/bootstrap requests are fully bound
                // by the existing RequestRecord, including their destinations.
                if command.category == PaymentCategory::Ordinary
                    && !command
                        .command_id
                        .starts_with(crate::schedule::COMMAND_PREFIX)
                {
                    continue;
                }
                if !self.active && command.category != PaymentCategory::BootstrapDistribution {
                    continue;
                }
                let request = Self::restore_request(&command.command_id, &command.request)?;
                if command.request.construction_effect.is_some() && !self.config.lab_acceptance {
                    continue;
                }
                let outcome = self
                    .wallet
                    .as_ref()
                    .unwrap()
                    .sign_payment_with_construction(&request, command.request.construction_effect);
                if let Ok(PaymentOutcome::Signed(view) | PaymentOutcome::ReplayedStored(view)) =
                    outcome
                {
                    self.forward(&view)?;
                } else if command.phase == CommandPhase::Began
                    && command
                        .command_id
                        .starts_with(crate::schedule::COMMAND_PREFIX)
                {
                    self.wallet
                        .as_ref()
                        .unwrap()
                        .journal()
                        .finish_command(&command.command_id, "deferred_no_available_utxo")
                        .map_err(|e| invalid(&e.to_string()))?;
                }
                return Ok(());
            }
        }
        for (txid, tx) in &snapshot.transactions {
            let facts = &tx.facts;
            if !self.active && facts.category != PaymentCategory::BootstrapDistribution {
                continue;
            }
            match facts.state {
                TxState::Signed if facts.vbytes <= self.config.max_vbytes => {
                    if self.send_facts(facts, None).is_err() {
                        self.traffic_event(TrafficObservation::empty(TrafficEventKind::Deferred))?;
                    }
                    return Ok(());
                }
                TxState::Unknown if facts.retry_sends < snapshot.caps.unknown_retry_sends => {
                    // Absence is not rejection. A present same transaction is
                    // left uncertain until canonical confirmation, not resent.
                    let node = self.config.authority_node.clone();
                    let anchor = self.index.as_ref().unwrap().anchor().block_hash;
                    let cap = self.config.max_mempool_backlog_tx as usize;
                    let Ok(pool) = crate::observer::MempoolView::capture(self, &node, anchor, cap)
                    else {
                        return Ok(());
                    };
                    if pool.conflict(*txid, &facts.spent_outpoints).is_some() {
                        self.wallet
                            .as_ref()
                            .unwrap()
                            .note_conflicted(&facts.command_id, "verified_mempool_conflict")
                            .map_err(|e| invalid(&e.to_string()))?;
                        continue;
                    }
                    if let Some(tx) = pool.transactions.get(txid) {
                        if hex::encode(bitcoin::consensus::serialize(tx)) != facts.raw_hex {
                            return Err(invalid("own mempool bytes mismatch"));
                        }
                        continue;
                    }
                    if self.send_facts(facts, None).is_err() {
                        self.traffic_event(TrafficObservation::empty(TrafficEventKind::Deferred))?;
                    }
                    return Ok(());
                }
                TxState::Reorged if facts.reorg_resubmits < snapshot.caps.reorg_resubmits => {
                    let index = self.index.take().unwrap();
                    let node = self.config.authority_node.clone();
                    let backlog = self.config.max_mempool_backlog_tx as usize;
                    let proof =
                        index.prove_unspent(self, &node, *txid, &facts.spent_outpoints, backlog);
                    self.index = Some(index);
                    if !matches!(proof, Ok(true)) {
                        if matches!(proof, Ok(false)) {
                            let anchor = self.index.as_ref().unwrap().anchor().block_hash;
                            if let Ok(pool) =
                                crate::observer::MempoolView::capture(self, &node, anchor, backlog)
                            {
                                if pool.conflict(*txid, &facts.spent_outpoints).is_some() {
                                    self.wallet
                                        .as_ref()
                                        .unwrap()
                                        .note_conflicted(
                                            &facts.command_id,
                                            "verified_mempool_conflict",
                                        )
                                        .map_err(|e| invalid(&e.to_string()))?;
                                }
                            }
                        }
                        continue;
                    }
                    let raw =
                        hex::decode(&facts.raw_hex).map_err(|_| invalid("journal hex invalid"))?;
                    let sequence = match self
                        .wallet
                        .as_ref()
                        .unwrap()
                        .journal()
                        .record_reorg_resubmit(&facts.command_id, &raw, &facts.spent_outpoints)
                    {
                        Ok(sequence) => sequence,
                        Err(_) => continue, // maturity/caps remain authoritative
                    };
                    self.send_facts(facts, Some(sequence))?;
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn restore_request(id: &str, record: &RequestRecord) -> io::Result<PaymentRequest> {
        Ok(PaymentRequest {
            command_id: id.into(),
            payer_role: record.payer_role.clone(),
            category: record.category.clone(),
            destinations: record
                .destinations
                .iter()
                .map(|d| {
                    Ok(DestinationSpec {
                        script_pubkey: bitcoin::ScriptBuf::from_bytes(
                            hex::decode(&d.script_hex)
                                .map_err(|_| invalid("stored destination invalid"))?,
                        ),
                        amount_sats: d.amount_sats,
                        owner: d.owner_role.clone(),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?,
            fee_rate_sats_vb: record.fee_rate_sats_vb,
        })
    }

    fn ordinary(&mut self, slot: crate::schedule::Slot) -> io::Result<()> {
        let wallet = self.wallet.as_ref().unwrap();
        let mut roles: Vec<_> = wallet
            .roles()
            .into_iter()
            .filter(|r| matches!(r, WalletRole::NetworkWallet { .. }))
            .collect();
        roles.sort();
        let n = u64::from_be_bytes(slot.entropy[8..16].try_into().unwrap());
        let payer = roles[(n % roles.len() as u64) as usize].clone();
        let choice =
            u64::from_be_bytes(slot.entropy[16..24].try_into().unwrap()) as f64 / u64::MAX as f64;
        let mut cumulative = 0.0;
        let class = self
            .config
            .size_classes
            .iter()
            .find(|class| {
                cumulative += class.mix;
                choice <= cumulative
            })
            .unwrap_or_else(|| self.config.size_classes.last().unwrap());
        // Nominal classes choose an output count. Actual signed size/fee are
        // always measured by the core, and the hard cap applies afterwards.
        let outputs = ((class.vbytes.saturating_sub(190)) / 34).clamp(1, 64);
        let target_role =
            roles[((n % roles.len() as u64 + 1) % roles.len() as u64) as usize].clone();
        let destination = wallet
            .next_receive_address(&target_role)
            .map_err(|e| invalid(&e.to_string()))?;
        let policy = wallet.fee_policy();
        let rate =
            policy.min_rate_sats_vb + (n % (policy.max_rate_sats_vb - policy.min_rate_sats_vb + 1));
        let request = PaymentRequest {
            command_id: format!("{}{:020}", crate::schedule::COMMAND_PREFIX, slot.sequence),
            payer_role: payer,
            category: PaymentCategory::Ordinary,
            destinations: (0..outputs)
                .map(|_| DestinationSpec {
                    script_pubkey: destination.script_pubkey.clone(),
                    amount_sats: 10_000u64.max(policy.dust_sats),
                    owner: Some(target_role.clone()),
                })
                .collect(),
            fee_rate_sats_vb: rate,
        };
        let class_vbytes = class.vbytes;
        let record = RequestRecord {
            construction_effect: None,
            payer_role: request.payer_role.clone(),
            category: request.category.clone(),
            destinations: request
                .destinations
                .iter()
                .map(|d| crate::journal::DestinationRecord {
                    script_hex: d.script_pubkey.to_hex_string(),
                    amount_sats: d.amount_sats,
                    owner_role: d.owner.clone(),
                })
                .collect(),
            fee_rate_sats_vb: rate,
        };
        wallet
            .journal()
            .begin_command(&request.command_id, record)
            .map_err(|e| invalid(&e.to_string()))?;
        let mut planned = TrafficObservation::empty(TrafficEventKind::Planned);
        planned.planned_monotonic_ms = Some(slot.planned_ms);
        planned.requested_vbytes = Some(class_vbytes);
        planned.schedule_sequence = Some(slot.sequence);
        self.traffic_event(planned)?;
        let node = self.config.authority_node.clone();
        let info = self.read(&node, "getmempoolinfo", json!([]));
        if !matches!(info,Ok(ref value) if value["size"].as_u64().is_some_and(|n|n<self.config.max_mempool_backlog_tx))
        {
            self.defer_ordinary(&request.command_id)?;
            return Ok(());
        }
        match self.wallet.as_ref().unwrap().sign_payment(&request) {
            Ok(PaymentOutcome::Signed(view) | PaymentOutcome::ReplayedStored(view)) => {
                self.emit_tx(&request.command_id, TrafficEventKind::Signed)?;
                if view.vbytes <= self.config.max_vbytes {
                    self.forward(&view)?;
                } else {
                    self.traffic_event(TrafficObservation::empty(TrafficEventKind::SizeRefused))?;
                }
            }
            Err(_) => self.defer_ordinary(&request.command_id)?,
        }
        Ok(())
    }

    fn defer_ordinary(&mut self, id: &str) -> io::Result<()> {
        let journal = self.wallet.as_ref().unwrap().journal();
        if journal.command_phase(id) == Some(CommandPhase::Began) {
            journal
                .finish_command(id, "deferred_no_available_utxo_or_backlog")
                .map_err(|e| invalid(&e.to_string()))?;
        }
        self.traffic_event(TrafficObservation::empty(TrafficEventKind::Deferred))
    }

    pub(super) fn emit_tx(&mut self, command_id: &str, kind: TrafficEventKind) -> io::Result<()> {
        let journal = self.wallet.as_ref().unwrap().journal();
        let command = journal
            .command(command_id)
            .ok_or_else(|| invalid("traffic command missing"))?;
        let txid = command
            .txid
            .ok_or_else(|| invalid("traffic txid missing"))?;
        let facts = journal
            .tx_facts(&txid)
            .ok_or_else(|| invalid("traffic facts missing"))?;
        let mut event = TrafficObservation::empty(kind);
        event.txid = Some(txid.to_string());
        event.vbytes = Some(facts.vbytes);
        event.weight_wu = Some(facts.weight_wu);
        event.fee_sats = Some(facts.fee_sats);
        event.fee_rate_sats_vb = Some(facts.fee_rate_sats_vb);
        event.category = Some(facts.category.as_str());
        event.wallet = Some(command.wallet_role.name());
        self.traffic_event(event)
    }

    fn traffic_event(&mut self, mut event: TrafficObservation) -> io::Result<()> {
        event.journal_seq = self
            .wallet
            .as_ref()
            .map(|w| w.journal().last_seq())
            .unwrap_or(0);
        event.observed_monotonic_ms = crate::bootstrap::monotonic_ms()?;
        event.observed_wall_ms = crate::now_unix_ms();
        self.event("tx_traffic", serde_json::to_value(event)?)
    }
}

impl<W: Write> RawRpc for Actor<W> {
    fn read(&mut self, node: &str, method: &str, params: Value) -> Result<Value, String> {
        self.checkpoint()?;
        let response = self
            .rpc(RpcRequest {
                node_id: node.into(),
                method: method.into(),
                params,
                send_sequence: None,
                txid: None,
            })
            .map_err(|e| e.to_string())?;
        self.checkpoint()?;
        match response {
            RpcOutcome::Ok { result } => Ok(result),
            _ => Err("observer_rpc_unavailable".into()),
        }
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        self.collect_controls().map_err(|e| e.to_string())?;
        if self.queued.is_empty() {
            Ok(())
        } else {
            Err("observer_yield_to_control".into())
        }
    }
}
