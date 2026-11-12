/* Thin browser over the same snapshot/SSE contract as the headless runner.
 *
 * The pure helpers at the top are exported for the Node builtin test runner
 * (`node --test`); the bootstrap only runs when a DOM exists, so requiring
 * this file from a test is side-effect free. There is no npm dependency.
 */

/* --- pure helpers (no DOM) --- */

function escapeHtml(value) {
  const entity = { 38: "amp", 60: "lt", 62: "gt", 34: "quot", 39: "#39" };
  return String(value).replace(/[&<>"']/g, (c) => "&" + entity[c.charCodeAt(0)] + ";");
}

function eventsUrl(runId, cursor) {
  return `/api/lab/v1/runs/${encodeURIComponent(runId)}/events?after=${cursor}`;
}

function snapshotUrl(runId, cursor) {
  const base = `/api/lab/v1/runs/${encodeURIComponent(runId)}/snapshot`;
  return cursor === null || cursor === undefined ? base : `${base}?cursor=${cursor}`;
}

function newCommandId() {
  const source = globalThis.crypto;
  if (source && typeof source.randomUUID === "function") return source.randomUUID();
  return `cmd-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function commandEnvelope(runId, kind, nowMs, commandId, actor = "browser") {
  return {
    command_id: commandId || newCommandId(),
    run_id: runId,
    issued_at_wall: typeof nowMs === "number" ? nowMs : Date.now(),
    actor,
    kind,
  };
}

/* Map a rendered entity row plus a button action to the typed command
 * payload. Miner rows are the miner itself (`workers: []` pauses/resumes all
 * of its workers); worker rows must carry `miner_id`. When the runner does
 * not provide `miner_id` on a worker row the control refuses visibly instead
 * of sending a wrong id (a command aimed at the worker id as miner_id would
 * silently target nothing). */
function controlCommand(entity, action, budgetValue) {
  if (!entity || typeof entity.id !== "string" || entity.id === "") {
    return { ok: false, error: "entity row has no id" };
  }
  const isWorker = entity.kind === "worker";
  if (isWorker && typeof entity.miner_id !== "string") {
    return {
      ok: false,
      error: `worker ${entity.id} row has no miner_id; control disabled to avoid sending a wrong id`,
    };
  }
  const minerId = isWorker ? entity.miner_id : entity.id;
  if (action === "pause" || action === "resume") {
    return {
      ok: true,
      payload: {
        type: action === "pause" ? "pause_workers" : "resume_workers",
        miner_id: minerId,
        workers: isWorker ? [entity.id] : [],
      },
    };
  }
  if (action === "budget") {
    if (!isWorker) {
      return { ok: false, error: "budget applies to a worker row, not a miner" };
    }
    const value = String(budgetValue === null || budgetValue === undefined ? "" : budgetValue).trim();
    if (!/^[0-9]+$/.test(value)) {
      return {
        ok: false,
        error: `budget for worker ${entity.id} must be a non-negative decimal integer`,
      };
    }
    return {
      ok: true,
      payload: {
        type: "set_worker_budget",
        miner_id: minerId,
        worker_id: entity.id,
        hashes_per_second: value,
        total_hashes: null,
      },
    };
  }
  return { ok: false, error: `unknown control action ${String(action)}` };
}

/* A replayed event is new only when its id is strictly greater than anything
 * already delivered. Non-numeric ids never advance a numeric cursor. */
function isNewEvent(lastEventId, eventId) {
  return typeof eventId === "number" && Number.isFinite(eventId) && eventId > lastEventId;
}

function advanceCursor(cursor, eventId) {
  return isNewEvent(cursor, eventId) ? eventId : cursor;
}

function catalogUrl(runId) {
  return `/api/lab/v1/runs/${encodeURIComponent(runId)}/catalog`;
}

function inventoryUrl(runId) {
  return `/api/lab/v1/runs/${encodeURIComponent(runId)}/inventory`;
}

function validMinerId(value) {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(value);
}

/* Build the durable add_miner command for the browser form. The identity is
 * one of the persistent SP identities declared by the runtime; an add never
 * generates or rewrites identity material. */
function addMinerCommand(catalog, spec) {
  if (!catalog || !Array.isArray(catalog.fronts) || !Array.isArray(catalog.identities)) {
    return { ok: false, error: "runtime catalog is unavailable" };
  }
  const minerId = String(spec && spec.minerId || "").trim();
  if (!validMinerId(minerId)) {
    return { ok: false, error: "miner id must be 1-128 ASCII alphanumeric, '.', '_', ':' or '-' characters" };
  }
  const front = catalog.fronts.find((candidate) => candidate.id === spec.frontId);
  if (!front) return { ok: false, error: "choose a declared front/pool" };
  const identity = catalog.identities.find((candidate) => candidate.id === spec.identityId);
  if (!identity) return { ok: false, error: "choose a declared SP identity" };
  if (identity.used) return { ok: false, error: `identity ${identity.id} is already used by a miner` };
  const budget = String(spec.hashesPerSecond === null || spec.hashesPerSecond === undefined ? "" : spec.hashesPerSecond).trim();
  if (!/^[0-9]+$/.test(budget) || budget === "0") {
    return { ok: false, error: "hashes per second must be a positive decimal integer" };
  }
  const connectionId = `${minerId}-conn`;
  const workerId = `${minerId}-worker`;
  return {
    ok: true,
    payload: {
      type: "add_miner",
      miner_id: minerId,
      implementation: (catalog.defaults && catalog.defaults.miner_implementation) || "spc-cpu-miner",
      connections: [{ id: connectionId, front_id: front.id, identity_id: identity.id }],
      workers: [{
        id: workerId,
        identity_id: identity.id,
        connection_id: connectionId,
        budget: { hashes_per_second: budget },
        nonce_start: null,
        nonce_len: null,
      }],
    },
  };
}

/* Move a live miner to another declared front/pool via the existing typed
 * switch_pool command. The connection id and identity come from the catalog,
 * never from a guessed value. */
function switchPoolCommand(catalog, minerId, frontId) {
  const miner = catalog && Array.isArray(catalog.miners)
    ? catalog.miners.find((candidate) => candidate.id === minerId)
    : undefined;
  if (!miner) return { ok: false, error: `miner ${minerId} is not in the catalog` };
  const connection = miner.connections && miner.connections[0];
  if (!connection) return { ok: false, error: `miner ${minerId} has no connection to switch` };
  const front = catalog.fronts.find((candidate) => candidate.id === frontId);
  if (!front) return { ok: false, error: "choose a declared front/pool" };
  return {
    ok: true,
    payload: {
      type: "switch_pool",
      miner_id: minerId,
      connection_id: connection.id,
      front_id: front.id,
      identity_id: connection.identity_id,
    },
  };
}

/* T1's real wire contract uses JSON u64, not strings. Refuse values outside
 * the exact JS integer range rather than silently rounding a payment/fee. */
function paymentCommand(user, destination, amount, feeRate) {
  const exact = value => {
    if (!/^[1-9][0-9]*$/.test(value)) throw new Error("Use a positive integer amount and fee rate.");
    const n = BigInt(value);
    if (n > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error("Payment input exceeds this form's exact integer bound.");
    return Number(n);
  };
  if (typeof destination !== "string" || !/^[mn][1-9A-HJ-NP-Za-km-z]{25,34}$/.test(destination)) throw new Error("T1 requires a regtest P2PKH destination; the signer verifies its checksum and network.");
  return { type: "send_user_payment", user, destination_address: destination, amount_sats: exact(amount), fee_rate_sats_vb: exact(feeRate) };
}

function walletCommand(identity, action, ids, address) {
  if (!Array.isArray(ids) || ids.length < 1 || ids.length > 64 || new Set(ids.map(id => id.toLowerCase())).size !== ids.length || ids.some(id => !/^[a-fA-F0-9]{72}$/.test(id))) throw new Error("Select 1-64 available wallet inputs.");
  if (action === "refresh") return { type: "wallet_refresh", identity_id: identity, vtxo_ids: ids };
  if (action !== "offboard" || typeof address !== "string" || !/^bcrt1[ac-hj-np-z02-9]{6,90}$/.test(address)) throw new Error("Use a native regtest witness offboard address; the controller verifies checksum and policy.");
  return { type: "wallet_offboard", identity_id: identity, vtxo_ids: ids, address };
}

function trafficCommand(driverId, action) {
  if (typeof driverId !== "string" || !driverId || !["pause", "resume"].includes(action)) throw new Error("Choose a declared traffic driver.");
  return { type: action === "pause" ? "pause_tx_driver" : "resume_tx_driver", driver_id: driverId };
}

/* T1 provider join: values stay in their own account. This code is embedded
 * into app.js by the small source-bound patch, not served as a new asset. */
const T1_MAX_AGE_MS = 15000;
function t1Id(value) {
  return typeof value === "string" && /^[A-Za-z0-9._:/@-]{1,128}$/.test(value);
}
function t1Amount(value) {
  return typeof value === "string" && /^(0|[1-9][0-9]{0,19})$/.test(value)
    && BigInt(value) <= 18446744073709551615n;
}
function t1Stamp(value) { return Number.isSafeInteger(value) && value >= 0; }
function t1Operator(context) { return Array.isArray(context?.may) && context.may.includes("operator"); }
function t1OwnRow(row, context) {
  return t1Id(row?.identity_id) && (t1Operator(context)
    ? row.pool_id === null || t1Id(row.pool_id)
    : t1Id(context?.user_id) && context.user_id === context.identity_id
      && row.identity_id === context.identity_id && t1Id(context.pool_id) && row.pool_id === context.pool_id);
}
function t1ViewValid(payment, user) {
  const fields = ["schema", "user_id", "state", "source", "stage", "spendable_sats", "unconfirmed_sats",
    "pending_sats", "receive_address", "source_observed_at_ms", "source_age_ms", "reason"];
  return payment && typeof payment === "object" && !Array.isArray(payment)
    && Object.keys(payment).length === fields.length && fields.every(key => Object.hasOwn(payment, key))
    && payment.schema === "spc-t1-payment-wallet/v1" && t1Id(payment.user_id) && payment.user_id === user
    && payment.source === "T1 test-payment funds" && ["available", "not_available", "stale"].includes(payment.state)
    && ["active", "not_active"].includes(payment.stage)
    && (payment.spendable_sats === null || t1Amount(payment.spendable_sats))
    && payment.unconfirmed_sats === null && payment.pending_sats === null
    && (payment.receive_address === null || (typeof payment.receive_address === "string" && /^[mn][1-9A-HJ-NP-Za-km-z]{25,34}$/.test(payment.receive_address)))
    && (payment.source_observed_at_ms === null || t1Stamp(payment.source_observed_at_ms))
    && (payment.source_age_ms === null || t1Stamp(payment.source_age_ms))
    && (payment.reason === null || (typeof payment.reason === "string" && payment.reason.length <= 256 && !/[\x00-\x1f\x7f]/.test(payment.reason)));
}
function joinT1Wallets(projection, inventory, context, epoch, receivedAt = Date.now()) {
  const invalid = () => { throw new Error("Payment/worker projection is invalid or outside this session."); };
  if (!context || !Array.isArray(context.may) || !t1Id(context.run_id) || !t1Id(context.principal)
    || !t1Stamp(epoch) || !t1Stamp(receivedAt)) invalid();
  const operator = t1Operator(context);
  if (!operator && (!t1Id(context.pool_id) || context.user_id !== context.identity_id)) invalid();
  if (operator !== Array.isArray(projection)) invalid();
  const projections = operator ? projection : [projection], users = new Map();
  for (const p of projections) {
    if (!p || p.schema !== "spc-user-projection/v1" || !t1Id(p.user_id) || users.has(p.user_id)
      || (!operator && (p.user_id !== context.user_id || p.user_id !== context.identity_id))
      || !Array.isArray(p.workers)) invalid();
    const workers = new Set();
    for (const worker of p.workers) {
      if (!t1Id(worker?.worker_id) || workers.has(worker.worker_id)
        || (!operator && (!Array.isArray(context.worker_ids) || !context.worker_ids.includes(worker.worker_id)))) invalid();
      workers.add(worker.worker_id);
    }
    if (p.payment_wallet !== undefined && !t1ViewValid(p.payment_wallet, p.user_id)) invalid();
    users.set(p.user_id, p);
  }
  if (inventory?.schema !== "spc-lab-wallets/v1" || !Array.isArray(inventory.wallets)) invalid();
  const ark = new Map();
  for (const row of inventory.wallets) {
    if (!t1OwnRow(row, context) || !t1Id(row.pool_id) || !users.has(row.identity_id) || ark.has(row.identity_id)) invalid();
    ark.set(row.identity_id, row);
  }
  const wallets = projections.map(p => ({
    ...(ark.get(p.user_id) || { identity_id: p.user_id, pool_id: operator ? null : context.pool_id,
      source: "SP/Ark agent not configured", status: null, inputs: null, inputs_updated_at_ms: null,
      inputs_error: "no configured SP/Ark agent input source" }),
    // Always replace any stale/misplaced wallet-row payment data with the
    // independently authenticated projection's exact, validated T1 object.
    payment_wallet: p.payment_wallet || null,
    payment_context: { run_id: context.run_id, principal: context.principal, epoch, received_at_ms: receivedAt },
  }));
  return { wallets, projections };
}
function t1PaymentState(wallet, context, epoch, now = Date.now()) {
  const no = reason => ({ fresh: false, enabled: false, reason, payment: null, age_ms: null, expires_at_ms: null });
  if (!wallet) return no("T1 provider unavailable");
  if (!Array.isArray(context?.may)) return no("session capabilities unavailable");
  const binding = wallet?.payment_context, payment = wallet?.payment_wallet;
  if (!t1OwnRow(wallet, context) || !binding || binding.run_id !== context.run_id
    || binding.principal !== context.principal || binding.epoch !== epoch) return no("session or identity mismatch");
  if (!t1ViewValid(payment, wallet.identity_id)) return no("T1 provider unavailable or invalid");
  if (payment.state !== "available" || payment.stage !== "active") return no(payment.reason || `provider ${payment.state} / ${payment.stage}`);
  if (!t1Stamp(now) || !t1Stamp(binding.received_at_ms) || now < binding.received_at_ms
    || !t1Stamp(payment.source_observed_at_ms) || payment.source_observed_at_ms > now
    || !t1Stamp(payment.source_age_ms)) return no("source observation time unknown or future");
  // Receipt time can only shorten the server's remaining lifetime; it never
  // replaces or refreshes the hard wall-clock source-observation deadline.
  const age = Math.max(now - payment.source_observed_at_ms, payment.source_age_ms + now - binding.received_at_ms);
  if (age > T1_MAX_AGE_MS) return no("source observation stale");
  if (!t1Amount(payment.spendable_sats) || payment.receive_address === null) return no("confirmed funds or receive address unknown");
  const grant = t1Operator(context) || context.may.includes("send_user_payment");
  const funded = BigInt(payment.spendable_sats) > 0n;
  return { fresh: true, enabled: grant && funded, payment, age_ms: age,
    expires_at_ms: Math.min(payment.source_observed_at_ms + T1_MAX_AGE_MS,
      binding.received_at_ms + T1_MAX_AGE_MS - payment.source_age_ms),
    reason: !grant ? "payment capability unavailable" : !funded ? "no spendable confirmed T1 funds" : "source available" };
}
function t1TrafficDrivers(catalog, context) {
  const controls = catalog?.traffic_controls;
  if (!t1Operator(context) || controls?.available !== true || !Array.isArray(controls.drivers)) return [];
  const ids = new Set();
  for (const row of controls.drivers) {
    if (!t1Id(row?.driver_id) || ids.has(row.driver_id)) return [];
    ids.add(row.driver_id);
  }
  return controls.drivers;
}

const appHelpers = {
  escapeHtml,
  eventsUrl,
  snapshotUrl,
  newCommandId,
  commandEnvelope,
  controlCommand,
  catalogUrl,
  inventoryUrl,
  addMinerCommand,
  switchPoolCommand,
  isNewEvent,
  advanceCursor,
  paymentCommand,
  walletCommand,
  trafficCommand,
  joinT1Wallets,
  t1PaymentState,
  t1TrafficDrivers,
};

if (typeof module !== "undefined" && module.exports) module.exports = appHelpers;

/* --- browser bootstrap (DOM only) --- */

function bootstrap() {
  let access, me, pollTimer;
  let pending = new LabAccess.PendingCommands(), posting = new Set();
  let walletRows = [], polling = false, paymentExpiryTimer = null;
  const state = {
    cursor: 0,
    lastEventId: 0,
    runId: null,
    snapshot: null,
    catalog: null,
    readOnly: false,
    reconnectDelayMs: 0,
    reconnectTimer: null,
  };
  const $ = (id) => document.getElementById(id);
  const setPill = (text, cls) => { const el = $("connection"); el.textContent = text; el.className = "pill " + (cls || ""); };
  const noteError = (message) => { const el = $("control-error"); if (el) el.textContent = message || ""; };

  async function getJson(url) {
    return access.json(url);
  }

  function disableControls() {
    document.querySelectorAll("#miners button, #miners input, #add-miner button, #add-miner input, #add-miner select").forEach((el) => { el.disabled = true; });
    if (me && $("payment-send")) renderPayment();
  }

  function applyInventoryMode(inventory) {
    const recorded = inventory && inventory.view === "recorded_read_only";
    const banner = $("recorded-banner");
    if (banner) banner.hidden = !recorded;
    const mode = $("view-mode");
    if (mode) mode.textContent = recorded ? "recorded / read-only" : "live controller";
    if (recorded) {
      state.readOnly = true;
      disableControls();
      noteError("recorded-run view is read-only; miner controls are disabled");
    }
  }

  async function postCommand(kind, retry = false) {
    if (!state.runId || !me || (state.readOnly && !retry)) return;
    const command = retry ? pending.commands.get(pendingSelect.value) : commandEnvelope(state.runId, kind, undefined, undefined, me.actor);
    if (!command) return;
    if (posting.has(command.command_id)) return;
    if (!retry) {
      try { pending.add(command); } catch (error) { noteError(error.message); return; }
    }
    posting.add(command.command_id);
    savePending();
    const epoch = access.epoch;
    $("command-state").textContent = `${command.command_id}: submitted; awaiting controller result`;
    let outcome;
    try {
      outcome = await access.json(`/api/lab/v1/runs/${encodeURIComponent(state.runId)}/commands`, {
        method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(command),
      });
      if (outcome.command_id !== command.command_id) throw new Error("command result mismatch");
    } catch (_) {
      if (epoch !== access.epoch) return;
      posting.delete(command.command_id);
      $("command-state").textContent = `${command.command_id}: unknown; retained immutable request, no new payment sent`;
      savePending(); return;
    }
    if (epoch !== access.epoch) return;
    posting.delete(command.command_id);
    $("command-state").textContent = `${outcome.command_id}: ${outcome.state} - ${outcome.detail || ""}`;
    // Unknown is not failure and is never automatically retried under a new id.
    const unresolved = pending.resolve(command, outcome, retry);
    if (retry && unresolved && !["accepted", "running", "blocked_on_height", "unknown"].includes(outcome.state)) {
      $("command-state").textContent += " - request denied; original operation remains unresolved, retain its ID";
    }
    savePending();
    appendTimeline({ event_id: state.cursor, occurred_at_wall: Date.now(), payload: { kind: "command_result", data: outcome } });
    if (outcome && outcome.state === "rejected_precondition" && /read-only/.test(String(outcome.detail || ""))) {
      state.readOnly = true;
      disableControls();
      noteError("recorded-run view is read-only; miner controls are disabled");
    }
    return outcome;
  }

  const pendingSelect = document.createElement("select");
  pendingSelect.id = "pending-command"; pendingSelect.setAttribute("aria-label", "Pending command to check");
  $("retry-command").before(pendingSelect);
  function savePending() {
    if (access) access.savePending(pending.commands.size ? pending.snapshot() : null);
    const selected = pendingSelect.value;
    pendingSelect.replaceChildren(...[...pending.commands.values()].map(command => {
      const option = document.createElement("option"); option.value = command.command_id;
      option.textContent = `${command.kind.type}: ${command.command_id}`; return option;
    }));
    if (pending.commands.has(selected)) pendingSelect.value = selected;
    pendingSelect.hidden = !pending.commands.size;
    $("retry-command").hidden = !pending.commands.size;
  }

  function appendTimeline(event) {
    const list = $("timeline");
    const item = document.createElement("li");
    const kind = event.payload && event.payload.kind || event.kind || "event";
    const time = event.occurred_at_wall ? new Date(event.occurred_at_wall).toISOString().slice(11, 23) : "";
    item.textContent = `${kind} ${time} #${event.event_id}`;
    list.prepend(item);
    while (list.children.length > 250) list.lastChild.remove();
  }

  function renderAddMiner() {
    const holder = $("add-miner");
    if (!holder) return;
    if (me && !me.may.includes("operator")) { holder.replaceChildren(); return; }
    if (!state.catalog) {
      holder.innerHTML = "<span class='hint'>runtime catalog unavailable; add-miner disabled</span>";
      return;
    }
    const disabled = state.readOnly ? " disabled" : "";
    const unused = state.catalog.identities.filter((identity) => !identity.used);
    holder.innerHTML = `<div class="row">
      <input id="new-miner-id" placeholder="miner id" value="miner-c"${disabled}>
      <select id="new-miner-front"${disabled}>${state.catalog.fronts.map((front) =>
        `<option value="${escapeHtml(front.id)}">${escapeHtml(front.id)} (${escapeHtml(front.pool_id)})</option>`).join("")}</select>
      <select id="new-miner-identity"${disabled}>${unused.map((identity) =>
        `<option value="${escapeHtml(identity.id)}">${escapeHtml(identity.id)}</option>`).join("") ||
        "<option value=''>no unused persistent identity</option>"}</select>
      <input id="new-miner-budget" type="number" min="1" step="1" value="${escapeHtml((state.catalog.defaults && state.catalog.defaults.hashes_per_second) || "100000")}"${disabled}>
      <button data-action="add-miner"${disabled}>add miner</button>
    </div><p class="hint">The added miner reuses a declared persistent SP identity and goes through the same durable command log.</p>`;
    const traffic = document.createElement("details"); traffic.id = "traffic-details";
    const summary = document.createElement("summary"); summary.textContent = "Background traffic controls"; traffic.append(summary);
    const drivers = t1TrafficDrivers(state.catalog, me);
    if (drivers.length) {
      for (const driver of drivers) {
        if (typeof driver.driver_id !== "string") continue;
        const row = document.createElement("div"); row.className = "row";
        const label = document.createElement("span"); label.textContent = driver.driver_id; row.append(label);
        for (const action of ["pause", "resume"]) {
          const button = document.createElement("button"); button.textContent = action;
          button.disabled = state.readOnly; button.dataset.driver = driver.driver_id; button.dataset.traffic = action;
          row.append(button);
        }
        traffic.append(row);
      }
    } else {
      const reason = document.createElement("p"); reason.textContent = "Unavailable: T1 declared driver capability source not connected."; traffic.append(reason);
    }
    holder.append(traffic);
  }

  function render(snapshot) {
    const saved = saveControls();
    if (snapshot.filtered) snapshot = { ...snapshot, entities: snapshot.workers || [], watermarks: [] };
    state.snapshot = snapshot;
    state.cursor = snapshot.snapshot_cursor || 0;
    state.lastEventId = Math.max(state.lastEventId, state.cursor);
    state.runId = snapshot.run_id;
    $("run-id").textContent = snapshot.run_id;
    $("run-state").textContent = `cursor ${snapshot.snapshot_cursor}; retained from ${snapshot.retained_from}; gaps ${snapshot.evidence.gap_count}`;
    const rows = snapshot.entities.map((entity) =>
      `<tr><td>${escapeHtml(entity.kind)}</td><td class="mono">${escapeHtml(entity.id)}</td><td>${escapeHtml(entity.status)}</td><td class="mono">${escapeHtml(entity.detail)}</td></tr>`
    ).join("");
    $("entities").innerHTML = `<table><thead><tr><th>kind</th><th>id</th><th>status</th><th>detail</th></tr></thead><tbody>${rows}</tbody></table>`;
    const miners = snapshot.entities.filter((entity) => entity.kind === "miner" || entity.kind === "worker");
    $("miners").innerHTML = miners.map((entity) => {
      const isWorker = entity.kind === "worker";
      const id = escapeHtml(entity.id);
      const paused = entity.status === "paused";
      const minerId = isWorker ? entity.miner_id : entity.id;
      // Worker rows without miner_id render disabled controls with a visible
      // reason instead of buttons that would send a wrong id.
      const unusable = isWorker && typeof minerId !== "string";
      const rowDisabled = unusable || state.readOnly ? " disabled" : "";
      const title = unusable ? ` title="worker row has no miner_id; control disabled"` : "";
      const budget = isWorker
        ? `<input type="number" min="0" step="1" placeholder="h/s" data-budget="${id}"${rowDisabled}${title}>
        <button data-action="budget" data-id="${id}"${rowDisabled}${title}>set</button>`
        : "";
      const switchSelect = !isWorker && state.catalog
        ? `<select data-switch-miner="${id}"${rowDisabled}${title}>` +
          state.catalog.fronts.map((front) =>
            `<option value="${escapeHtml(front.id)}">${escapeHtml(front.id)} (${escapeHtml(front.pool_id)})</option>`
          ).join("") + "</select>"
        : "";
      return `<div class="row"><span class="mono">${escapeHtml(entity.kind)} ${id}</span>
        <span>${escapeHtml(entity.status)}</span>
        <button data-action="${paused ? "resume" : "pause"}" data-id="${id}"${rowDisabled}${title}>${paused ? "resume" : "pause"}</button>${budget}${switchSelect}</div>`;
    }).join("") || "<span class='hint'>no miners</span>";
    renderAddMiner();
    $("watermarks").innerHTML = snapshot.watermarks.map((watermark) =>
      `<div class="row mono">${escapeHtml(watermark.source)} -> #${watermark.last_event_id} ${escapeHtml(watermark.last_event_kind)}</div>`
    ).join("") || "<span class='hint'>no source watermarks</span>";
    $("gaps").textContent = snapshot.gaps.length ? `${snapshot.gaps.length} explicit evidence gap(s); affected assertions are inconclusive.` : "";
    restoreControls(saved);
  }

  function controlKey(el) { return el.id || [el.tagName, el.dataset.budget, el.dataset.id, ["pause", "resume"].includes(el.dataset.action) ? "toggle" : el.dataset.action, el.dataset.switchMiner, el.dataset.driver, el.dataset.traffic].join(":"); }
  function saveControls() {
    return Array.from(document.querySelectorAll("#miners input, #miners select, #miners button, #add-miner input, #add-miner select, #add-miner button, #add-miner details, #add-miner summary")).map(el => ({ key: controlKey(el), value: el.value, open: el.open, focused: el === document.activeElement, start: el.selectionStart, end: el.selectionEnd }));
  }
  function restoreControls(saved) {
    for (const el of document.querySelectorAll("#miners input, #miners select, #miners button, #add-miner input, #add-miner select, #add-miner button, #add-miner details, #add-miner summary")) {
      const old = saved.find(item => item.key === controlKey(el));
      if (!old) continue;
      if (el.matches("input,select")) el.value = old.value;
      if (el.matches("details")) el.open = old.open;
      if (old.focused) { el.focus({ preventScroll: true }); if (old.start !== null && el.setSelectionRange) el.setSelectionRange(old.start, old.end); }
    }
  }

  function age(timestamp) {
    const ms = typeof timestamp === "number" ? timestamp : Date.parse(timestamp);
    if (!Number.isFinite(ms) || ms > Date.now()) return "source time unknown";
    const seconds = Math.floor((Date.now() - ms) / 1000);
    return `${seconds > 15 ? "stale / " : ""}${seconds}s old`;
  }

  function mountWallet() {
    if ($("wallet-choice")) return;
    // Static markup only. Every source value below uses textContent.
    $("wallet").innerHTML = `<label>Wallet <select id="wallet-choice"></select></label>
      <p id="wallet-registration"></p><p id="worker-source"></p>
      <p id="watch-only"></p><p id="wallet-balance"></p><p id="payment-balance"></p>
      <p id="payment-receive"></p><p id="payment-source"></p>
      <details id="wallet-details"><summary>Details and wallet actions</summary>
        <p id="wallet-source"></p><div id="wallet-inputs" class="card"></div>
        <p id="wallet-unavailable"></p><button id="wallet-refresh">Refresh selected inputs</button>
        <label>Offboard destination <input id="offboard-address" type="text" autocomplete="off"></label>
        <button id="wallet-offboard">Offboard selected inputs (one rail)</button>
        <p>SPc coinbase outputs are watch-only. The T1 test-payment wallet is a separate account from SP/Ark earnings and background traffic.</p>
        <label>Test-payment destination <input id="payment-address" type="text" autocomplete="off"></label>
        <label>Amount (sats) <input id="payment-amount" inputmode="numeric" type="text"></label>
        <label>Fee rate (sats/vB) <input id="payment-fee" inputmode="numeric" type="text"></label>
        <button id="payment-send">Send test payment</button>
      </details>`;
    $("wallet-choice").addEventListener("change", () => { $("wallet-inputs").replaceChildren(); renderWallet(); });
    for (const action of ["refresh", "offboard"]) $("wallet-" + action).addEventListener("click", () => {
      try {
        const wallet = walletRows.find(w => w.identity_id === $("wallet-choice").value);
        const ids = Array.from($("wallet-inputs").querySelectorAll("input:checked")).map(el => el.value);
        postCommand(walletCommand(wallet.identity_id, action, ids, $("offboard-address").value));
      } catch (error) { noteError(error.message); }
    });
    $("payment-send").addEventListener("click", () => {
      try {
        const view = renderPayment();
        if (!view.enabled) { noteError(view.reason); return; }
        postCommand(paymentCommand(view.payment.user_id, $("payment-address").value, $("payment-amount").value, $("payment-fee").value));
      }
      catch (error) { noteError(error.message); }
    });
  }

  function renderWallet() {
    mountWallet();
    const select = $("wallet-choice"), previous = select.value;
    const ids = walletRows.map(w => w.identity_id);
    if (JSON.stringify(Array.from(select.options).map(o => o.value)) !== JSON.stringify(ids)) {
      select.replaceChildren(...ids.map(id => { const option = document.createElement("option"); option.value = id; option.textContent = id; return option; }));
      if (ids.includes(previous)) select.value = previous;
    }
    const wallet = walletRows.find(w => w.identity_id === select.value);
    const status = wallet?.status;
    $("wallet-registration").textContent = status ? `Registration: ${status.registered ? "registered" : "not registered"}; ${status.state} (${age(status.updated_at)})` : "Registration: unavailable (agent provider not connected)";
    $("watch-only").textContent = "SPc watch-only: unavailable (canonical mature/immature scan summary not connected)";
    $("wallet-balance").textContent = status ? `SP/Ark funds: ${LabAccess.sats(status.spendable_sats)} spendable; ${LabAccess.sats(status.pending_sats)} pending; ${LabAccess.sats(status.needs_refresh_sats)} needs refresh` : "SP/Ark funds: unknown (agent provider unavailable)";
    renderPayment();
    $("wallet-source").textContent = `Source: ${wallet?.source || "not connected"}; status ${age(status?.updated_at)}; inputs ${age(wallet?.inputs_updated_at_ms)}. Next refresh: ${status?.next_refresh_height ?? "unknown"}; expiry: ${status?.min_expiry_height ?? "unknown"}.`;
    const fresh = wallet && Number.isFinite(wallet.inputs_updated_at_ms) && Date.now() - wallet.inputs_updated_at_ms <= 15000 && Date.now() >= wallet.inputs_updated_at_ms;
    const rows = fresh && Array.isArray(wallet.inputs) ? wallet.inputs : [];
    const holder = $("wallet-inputs");
    // Keyed rows preserve live checkbox focus/selection through polling.
    for (const child of Array.from(holder.children)) if (!rows.some(row => row.id === child.dataset.id)) child.remove();
    for (const row of rows) {
      let label = Array.from(holder.children).find(el => el.dataset.id === row.id);
      if (!label) { label = document.createElement("label"); label.className = "row"; label.dataset.id = row.id;
        const input = document.createElement("input"); input.type = "checkbox"; input.value = row.id;
        label.append(input, document.createElement("span")); holder.append(label); }
      label.querySelector("span").textContent = `${row.id} / ${LabAccess.sats(row.amount_sats)} / ${row.state} / ${row.rail} / expiry ${row.expiry_height ?? "unknown"}`;
      label.querySelector("input").disabled = row.state !== "spendable";
      if (row.state !== "spendable") label.querySelector("input").checked = false;
    }
    const usable = rows.some(row => row.state === "spendable");
    $("wallet-unavailable").textContent = usable ? "Select explicit owned inputs; submission is not completion." : `Actions unavailable: ${wallet?.inputs_error || "no spendable inputs from the private agent provider"}.`;
    const may = capability => me && (me.may.includes("operator") || me.may.includes(capability));
    $("wallet-refresh").disabled = !usable || state.readOnly || !may("wallet_refresh");
    $("wallet-offboard").disabled = !usable || state.readOnly || !may("wallet_offboard");

  }
  function renderPayment() {
    clearTimeout(paymentExpiryTimer); paymentExpiryTimer = null;
    const wallet = walletRows.find(row => row.identity_id === $("wallet-choice")?.value);
    const view = t1PaymentState(wallet, me, access?.epoch);
    if (state.readOnly) { view.enabled = false; view.reason = "recorded-run view is read-only"; }
    const payment = view.payment;
    $("payment-balance").textContent = view.fresh
      ? `T1 test-payment funds: ${LabAccess.sats(payment.spendable_sats)} spendable; ${LabAccess.sats(payment.pending_sats)} pending; ${LabAccess.sats(payment.unconfirmed_sats)} unconfirmed`
      : `T1 test-payment funds: unknown / unavailable (${view.reason})`;
    $("payment-receive").textContent = `T1 receive address: ${view.fresh ? payment.receive_address : "unavailable"}`;
    $("payment-source").textContent = view.fresh
      ? `Source: T1 test-payment funds; active; observation ${view.age_ms} ms old. ${view.reason}. Separate from SP/Ark earnings.`
      : `Source: T1 test-payment funds; ${view.reason}. Separate from SP/Ark earnings.`;
    $("payment-send").disabled = !view.enabled;
    if (view.fresh) {
      const epoch = access.epoch;
      paymentExpiryTimer = setTimeout(() => {
        paymentExpiryTimer = null;
        if (me && epoch === access.epoch && $("payment-send")) renderPayment();
      }, Math.max(1, view.expires_at_ms - Date.now() + 1));
    }
    return view;
  }
  document.addEventListener("visibilitychange", () => {
    if (me && $("payment-send")) renderPayment();
  });

  async function pollUser() {
    if (!me || polling) return;
    polling = true; const epoch = access.epoch;
    try {
      const prefix = `/api/lab/v1/runs/${encodeURIComponent(me.run_id)}`;
      const [projection, wallets] = await Promise.all([getJson(prefix + "/projection"), getJson(prefix + "/wallets")]);
      if (epoch !== access.epoch) return;
      const joined = joinT1Wallets(projection, wallets, me, epoch);
      walletRows = joined.wallets; renderWallet();
      const projections = joined.projections;
      $("worker-source").textContent = projections.flatMap(p => (p.workers || []).map(w =>
        `${w.worker_id}: requested ${w.requested_hps ?? "unknown"} H/s, achieved ${w.achieved_hps ?? "unknown"} H/s; job ${w.active_job_id ?? "unknown"}; ${w.error ? "unavailable / " : ""}${age(w.observed_at)}`)).join("; ") || "Workers: unavailable (no observation)";
      const snapshot = await getJson(snapshotUrl(me.run_id, null));
      if (epoch === access.epoch) render(snapshot);
    } catch (_) {
      if (epoch === access.epoch && me) { mountWallet(); $("worker-source").textContent = "Source unavailable; previous observations may be stale"; walletRows = []; renderWallet(); }
    } finally { if (epoch === access.epoch) polling = false; }
  }

  function deliver(rawData) {
    let data;
    try {
      data = JSON.parse(rawData);
    } catch (_) {
      return;
    }
    // Dedupe: a replay that overlaps what we already rendered must not
    // duplicate timeline rows or move the cursor backwards.
    if (!isNewEvent(state.lastEventId, data.event_id)) return;
    state.lastEventId = data.event_id;
    state.cursor = advanceCursor(state.cursor, data.event_id);
    appendTimeline(data);
  }

  function openStream() {
    if (!me) return;
    const epoch = access.epoch;
    setPill("attaching...", "");
    access.stream(eventsUrl(state.runId, state.cursor), state.cursor, event => {
      if (event.type === "message") deliver(event.data);
      if (event.type === "resync") {
        const snapshot = JSON.parse(event.data);
        // Resync is authoritative, including a cursor rollback after restart.
        state.lastEventId = snapshot.snapshot_cursor;
        render(snapshot);
      }
    }, () => {
      state.reconnectDelayMs = 0;
      setPill("live", "live");
    }).catch(() => {}).finally(() => {
      if (epoch === access.epoch && me) { setPill("stream unavailable; reconnecting", "dead"); scheduleReconnect(); }
    });
  }

  function scheduleReconnect(delayOverride) {
    if (state.reconnectTimer) return;
    const delay = delayOverride !== undefined
      ? delayOverride
      : (state.reconnectDelayMs ? Math.min(state.reconnectDelayMs * 2, 5000) : 1000);
    state.reconnectDelayMs = delay;
    state.reconnectTimer = setTimeout(() => {
      state.reconnectTimer = null;
      openStream();
    }, delay);
  }

  function attach() {
    const epoch = access.epoch;
    const load = getJson(snapshotUrl(state.runId, null));
    load.then((snapshot) => {
      if (epoch !== access.epoch) return;
      render(snapshot);
      if (me.may.includes("operator")) {
      getJson(inventoryUrl(snapshot.run_id)).then(applyInventoryMode).catch(() => {});
      getJson(catalogUrl(snapshot.run_id)).then((catalog) => {
        state.catalog = catalog;
        renderAddMiner();
        render(state.snapshot);
      }).catch(() => {});
      }
      if (!me.legacy) { pollUser(); pollTimer = setInterval(pollUser, 2000); }
      openStream();
    }).catch((error) => {
      if (epoch !== access.epoch) return;
      setPill(`no run: ${error.message}`, "dead");
      // No run yet, or the snapshot fetch failed: retry discovery with the
      // same bounded backoff instead of hammering the server.
      if (state.reconnectTimer) return;
      state.reconnectDelayMs = state.reconnectDelayMs
        ? Math.min(state.reconnectDelayMs * 2, 5000)
        : 1000;
      const delay = state.reconnectDelayMs;
      state.reconnectTimer = setTimeout(() => {
        state.reconnectTimer = null;
        if (me) attach();
      }, delay);
    });
  }

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLButtonElement) || target.disabled) return;
    if (target.dataset.traffic) {
      try {
        if (!t1TrafficDrivers(state.catalog, me).some(row => row.driver_id === target.dataset.driver)) {
          noteError("Traffic driver capability unavailable."); return;
        }
        postCommand(trafficCommand(target.dataset.driver, target.dataset.traffic));
      }
      catch (error) { noteError(error.message); }
      return;
    }
    const action = target.dataset.action;
    if (!action) return;
    if (action === "add-miner") {
      const result = addMinerCommand(state.catalog, {
        minerId: $("new-miner-id") && $("new-miner-id").value,
        frontId: $("new-miner-front") && $("new-miner-front").value,
        identityId: $("new-miner-identity") && $("new-miner-identity").value,
        hashesPerSecond: $("new-miner-budget") && $("new-miner-budget").value,
      });
      if (!result.ok) {
        noteError(result.error);
        return;
      }
      noteError("");
      postCommand(result.payload);
      return;
    }
    const id = target.dataset.id;
    const entity = (state.snapshot && state.snapshot.entities || []).find((candidate) => candidate.id === id);
    if (!entity) {
      noteError(`no entity ${id} in the current snapshot`);
      return;
    }
    const input = document.querySelector(`input[data-budget="${CSS.escape(id)}"]`);
    const result = controlCommand(entity, action, input && input.value);
    if (!result.ok) {
      noteError(result.error);
      return;
    }
    noteError("");
    postCommand(result.payload);
  });

  document.addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLSelectElement)) return;
    const minerId = target.dataset.switchMiner;
    if (!minerId) return;
    const result = switchPoolCommand(state.catalog, minerId, target.value);
    if (!result.ok) {
      noteError(result.error);
      return;
    }
    noteError("");
    postCommand(result.payload);
  });

  $("retry-command").addEventListener("click", () => postCommand(null, true));
  LabAccess.mountAccess((session, context) => {
    access = session; me = context; state.runId = me.run_id;
    pending = new LabAccess.PendingCommands(session.loadPending());
    savePending();
    if (pending.commands.size) {
      $("command-state").textContent = "retained unknown/pending commands; check their existing outcomes";
    }
    attach();
  }, () => {
    clearTimeout(state.reconnectTimer); clearInterval(pollTimer);
    clearTimeout(paymentExpiryTimer); paymentExpiryTimer = null;
    state.reconnectTimer = null; state.snapshot = null; state.catalog = null;
    state.runId = null; state.cursor = 0; state.lastEventId = 0; state.readOnly = false;
    me = null; pending = new LabAccess.PendingCommands(); posting = new Set();
    walletRows = []; polling = false;
    for (const id of ["run-id", "run-state", "miners", "add-miner", "entities", "watermarks", "timeline", "gaps", "wallet", "command-state", "control-error"]) $(id).replaceChildren();
    $("retry-command").hidden = true; setPill("signed out", "");
    pendingSelect.replaceChildren(); pendingSelect.hidden = true;
  });
}

if (typeof window !== "undefined" && typeof document !== "undefined") bootstrap();
