/* Public, data-free bootstrap. Credentials live only in tab session storage.
 * No cookies, URL credentials, persistent storage or cross-origin requests. */
"use strict";

function decimal(value) {
  return typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value) ? BigInt(value) : null;
}
function sats(value) {
  const n = decimal(value);
  return n === null ? "unknown" : `${n.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",")} sats`;
}

function pendingEnvelopes(value) {
  if (!value) return [];
  const commands = value.schema === "spc-lab-pending/v2" ? value.commands : [value];
  if (!Array.isArray(commands) || commands.length > 16 || new Set(commands.map(c => c?.command_id)).size !== commands.length
    || commands.some(c => !c || typeof c.command_id !== "string" || !c.command_id || typeof c.kind?.type !== "string")) return [];
  return commands;
}

class PendingCommands {
  constructor(value) { this.commands = new Map(pendingEnvelopes(value).map(c => [c.command_id, c])); }
  snapshot() { return { schema: "spc-lab-pending/v2", commands: [...this.commands.values()] }; }
  static funds(kind) { return ["wallet_refresh", "wallet_offboard", "send_user_payment", "bootstrap_fund", "inject_tx"].includes(kind.type); }
  add(command) {
    if (this.commands.has(command.command_id)) throw new Error("Use the retained command for retries.");
    if (this.commands.size >= 16) throw new Error("Resolve a pending command before adding another.");
    if (PendingCommands.funds(command.kind) && [...this.commands.values()].some(c => PendingCommands.funds(c.kind)))
      throw new Error("Resolve the retained funds operation before another funds submission.");
    this.commands.set(command.command_id, command);
  }
  resolve(command, outcome, retry) {
    const pending = ["accepted", "running", "blocked_on_height", "unknown"].includes(outcome.state);
    // An observation/scope/read-only denial is about this request, not proof
    // that an older operation had no effect. Only durable resolution clears it.
    const deniedRetry = retry && (outcome.state === "rejected_conflict" || !(outcome.journal_seq > 0));
    if (!pending && !deniedRetry) this.commands.delete(command.command_id);
    return this.commands.has(command.command_id);
  }
}

/* Incremental WHATWG event-stream framing: CR, LF, CRLF, split UTF-8,
 * comments, multiline data and persistent last-event-id. Bound a frame as
 * well as a line; a hostile endless data frame cannot grow memory forever. */
class SSEParser {
  constructor(onEvent, limit = 1024 * 1024) {
    this.onEvent = onEvent; this.limit = limit; this.line = "";
    this.data = []; this.event = ""; this.id = ""; this.size = 0; this.cr = false;
  }
  push(text) {
    for (const c of text) {
      if (this.cr) { this.cr = false; if (c === "\n") continue; }
      if (c === "\r" || c === "\n") {
        this.consume(); this.cr = c === "\r";
      } else {
        this.line += c;
        if (++this.size > this.limit) throw new Error("event stream frame too large");
      }
    }
  }
  consume() {
    const line = this.line; this.line = "";
    if (line === "") {
      if (this.data.length) this.onEvent({ type: this.event || "message", data: this.data.join("\n"), id: this.id });
      this.data = []; this.event = ""; this.size = 0;
      return;
    }
    if (line[0] === ":") return;
    const colon = line.indexOf(":");
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? "" : line.slice(colon + 1);
    if (value[0] === " ") value = value.slice(1);
    if (field === "data") this.data.push(value);
    if (field === "event") this.event = value;
    if (field === "id" && !value.includes("\0")) this.id = value;
  }
}

class AccessSession {
  constructor({ fetcher = globalThis.fetch.bind(globalThis), storage = globalThis.sessionStorage, onClear = () => {} } = {}) {
    this.fetcher = fetcher; this.storage = storage; this.onClear = onClear;
    this.context = null; this.token = ""; this.epoch = 0; this.controllers = new Set();
  }
  clear(message = "Signed out") {
    ++this.epoch;
    for (const c of this.controllers) c.abort();
    this.controllers.clear(); this.token = ""; this.context = null;
    try {
      for (const key of Object.keys(this.storage)) if (key.startsWith("spc-lab-token:") || key.startsWith("spc-lab-pending:") || key === "spc-lab-session") this.storage.removeItem(key);
    } catch (_) { /* Session storage may be disabled; memory-only is safe. */ }
    this.onClear(message);
  }
  async request(path, init = {}) {
    // Paths are controller-owned, never supplied by a status DTO.
    if (typeof path !== "string" || !path.startsWith("/api/lab/v1/") || path.includes("\\") || path.includes("#")) throw new Error("invalid controller path");
    const epoch = this.epoch, controller = new AbortController();
    this.controllers.add(controller);
    const headers = new Headers(init.headers);
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    try {
      const response = await this.fetcher(path, { ...init, headers, signal: controller.signal, cache: "no-store", credentials: "omit", redirect: "error" });
      if (epoch !== this.epoch) throw new Error("session changed");
      if (response.status === 401) { this.clear("Authentication required. Enter a current run token."); throw new Error("authentication required"); }
      if (!response.ok) throw new Error(response.status === 403 ? "This view is outside your access scope." : `Controller unavailable (${response.status})`);
      // The controller remains owned until the body has been consumed.
      return { response, controller, epoch };
    } catch (error) { this.controllers.delete(controller); throw error; }
  }
  async json(path, init) {
    const r = await this.request(path, init);
    try {
      const value = await r.response.json();
      if (r.epoch !== this.epoch) throw new Error("session changed");
      return value;
    } finally { this.controllers.delete(r.controller); }
  }
  async login(token, expected = null) {
    this.clear("Connecting...");
    if (token && !/^[a-fA-F0-9]{64}$/.test(token)) throw new Error("Enter the 64-character run token.");
    this.token = token;
    const health = await this.json("/api/lab/v1/health");
    if (typeof health.run_id !== "string" || !health.run_id) throw new Error("Invalid run context");
    const context = token
      ? await this.json(`/api/lab/v1/runs/${encodeURIComponent(health.run_id)}/me`)
      : { run_id: health.run_id, principal: "legacy", actor: "browser", may: ["operator"], legacy: true };
    if (context.run_id !== health.run_id || typeof context.principal !== "string" || typeof context.actor !== "string" || !Array.isArray(context.may)
      || (expected && (context.run_id !== expected.run_id || context.principal !== expected.principal))) {
      this.clear("Run or principal changed. Enter its token explicitly.");
      throw new Error("Run or principal changed");
    }
    this.context = context;
    if (token) try {
      this.storage.setItem(`spc-lab-token:${context.run_id}:${context.principal}`, token);
      this.storage.setItem("spc-lab-session", JSON.stringify({ run_id: context.run_id, principal: context.principal }));
    } catch (_) { /* No persistent-storage fallback. */ }
    return context;
  }
  async restore() {
    let expected, token, pending;
    try {
      expected = JSON.parse(this.storage.getItem("spc-lab-session"));
      if (expected) {
        token = this.storage.getItem(`spc-lab-token:${expected.run_id}:${expected.principal}`);
        pending = this.storage.getItem(`spc-lab-pending:${expected.run_id}:${expected.principal}`);
      }
    } catch (_) { /* Treat damaged storage as signed out. */ }
    const context = await this.login(token || "", token ? expected : null);
    if (token && pending) try {
      const value = JSON.parse(pending), commands = pendingEnvelopes(value);
      if (commands.length && commands.every(c => c.run_id === context.run_id && c.actor === context.actor)) this.savePending(value);
    } catch (_) { /* Invalid pending metadata cannot authorize an effect. */ }
    return context;
  }
  savePending(command) {
    if (!this.context) return;
    try {
      const key = `spc-lab-pending:${this.context.run_id}:${this.context.principal}`;
      if (command) this.storage.setItem(key, JSON.stringify(command));
      else this.storage.removeItem(key);
    } catch (_) { /* Memory still retains the immutable pending request. */ }
  }
  loadPending() {
    if (!this.context) return null;
    try { return JSON.parse(this.storage.getItem(`spc-lab-pending:${this.context.run_id}:${this.context.principal}`)); }
    catch (_) { return null; }
  }
  async stream(path, cursor, onEvent, onOpen = () => {}) {
    const headers = { Accept: "text/event-stream" };
    if (/^[0-9]+$/.test(String(cursor))) headers["Last-Event-ID"] = String(cursor);
    const r = await this.request(path, { headers });
    try {
      if (!r.response.headers.get("content-type")?.startsWith("text/event-stream")) throw new Error("Invalid event stream");
      const parser = new SSEParser(event => { if (r.epoch === this.epoch) onEvent(event); });
      const reader = r.response.body.getReader(), decoder = new TextDecoder();
      onOpen();
      try {
        while (r.epoch === this.epoch) {
          const chunk = await reader.read();
          if (chunk.done) break;
          parser.push(decoder.decode(chunk.value, { stream: true }));
        }
        parser.push(decoder.decode());
      } finally { await reader.cancel().catch(() => {}); }
    } finally { this.controllers.delete(r.controller); }
  }
}

/* Shared login UI is static. Private nodes are cleared synchronously before
 * awaiting a switch; delayed requests cannot re-populate the previous view. */
function mountAccess(onReady, onClear) {
  const $ = id => document.getElementById(id);
  const session = new AccessSession({ onClear: message => {
    $("private-view").hidden = true; $("auth-form").hidden = false;
    $("logout").hidden = true; $("auth-message").textContent = message;
    $("auth-token").value = ""; onClear();
  } });
  const ready = async context => {
    $("auth-form").hidden = true; $("logout").hidden = !!context.legacy;
    $("auth-message").textContent = `${context.principal} / ${context.run_id}`;
    $("private-view").hidden = false;
    await onReady(session, context);
  };
  const authenticate = async action => {
    // login/restore synchronously starts a new epoch before its first await.
    // A cancelled older attempt must not clear a newer in-flight login.
    const pending = action(), epoch = session.epoch;
    try {
      const context = await pending;
      if (epoch === session.epoch) await ready(context);
    } catch (error) {
      if (epoch === session.epoch && !session.context) session.clear(error.message);
    }
  };
  $("auth-form").addEventListener("submit", async event => {
    event.preventDefault(); const token = $("auth-token").value.trim(); $("auth-token").value = "";
    await authenticate(() => session.login(token));
  });
  $("logout").addEventListener("click", () => { session.clear(); $("auth-token").focus(); });
  authenticate(() => session.restore());
  // BFCache must never revive a private DOM or a dead stream without auth.
  window.addEventListener("pagehide", () => {
    for (const c of session.controllers) c.abort();
    $("private-view").hidden = true; onClear();
  });
  window.addEventListener("pageshow", event => { if (event.persisted) authenticate(() => session.restore()); });
  return session;
}

const accessHelpers = { decimal, sats, SSEParser, AccessSession, mountAccess, pendingEnvelopes, PendingCommands };
if (typeof module !== "undefined" && module.exports) module.exports = accessHelpers;
if (typeof window !== "undefined") window.LabAccess = accessHelpers;
