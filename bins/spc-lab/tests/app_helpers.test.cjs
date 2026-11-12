"use strict";
/* Tests for the pure helpers exported by bins/spc-lab/static/app.js.
 * Run with the Node builtin test runner: `node --test bins/spc-lab/tests/`.
 * No npm framework or dependency is used. */

const test = require("node:test");
const assert = require("node:assert");
const helpers = require("../static/app.js");

test("miner rows pause and resume the whole miner", () => {
  const entity = { kind: "miner", id: "miner-1", status: "running" };
  assert.deepStrictEqual(helpers.controlCommand(entity, "pause"), {
    ok: true,
    payload: { type: "pause_workers", miner_id: "miner-1", workers: [] },
  });
  assert.deepStrictEqual(helpers.controlCommand(entity, "resume"), {
    ok: true,
    payload: { type: "resume_workers", miner_id: "miner-1", workers: [] },
  });
});

test("worker rows carry both the miner id and the worker id", () => {
  const entity = { kind: "worker", id: "worker-1", miner_id: "miner-1", status: "running" };
  const pause = helpers.controlCommand(entity, "pause");
  assert.deepStrictEqual(pause.payload, {
    type: "pause_workers",
    miner_id: "miner-1",
    workers: ["worker-1"],
  });
  const budget = helpers.controlCommand(entity, "budget", "5000");
  assert.deepStrictEqual(budget.payload, {
    type: "set_worker_budget",
    miner_id: "miner-1",
    worker_id: "worker-1",
    hashes_per_second: "5000",
    total_hashes: null,
  });
});

test("a worker row without miner_id refuses visibly instead of sending a wrong id", () => {
  const entity = { kind: "worker", id: "worker-1", status: "running" };
  for (const action of ["pause", "resume", "budget"]) {
    const result = helpers.controlCommand(entity, action, "1");
    assert.strictEqual(result.ok, false, `${action} must refuse`);
    assert.match(result.error, /miner_id/);
  }
});

test("budget refuses empty, malformed, and negative values", () => {
  const entity = { kind: "worker", id: "w", miner_id: "m" };
  for (const value of ["", "   ", "abc", "-1", "1.5", "1e3", null, undefined]) {
    const result = helpers.controlCommand(entity, "budget", value);
    assert.strictEqual(result.ok, false, `value ${String(value)} must refuse`);
    assert.match(result.error, /non-negative decimal integer/);
  }
  assert.strictEqual(helpers.controlCommand(entity, "budget", "0").ok, true);
});

test("budget is refused on a miner row and unknown actions are refused", () => {
  const miner = { kind: "miner", id: "m" };
  assert.strictEqual(helpers.controlCommand(miner, "budget", "10").ok, false);
  assert.strictEqual(helpers.controlCommand(miner, "explode").ok, false);
  assert.strictEqual(helpers.controlCommand(null, "pause").ok, false);
  assert.strictEqual(helpers.controlCommand({ kind: "miner" }, "pause").ok, false);
});

test("command envelope keeps the protocol shape with actor browser", () => {
  const kind = { type: "pause_workers", miner_id: "m", workers: [] };
  const envelope = helpers.commandEnvelope("run-1", kind, 1234, "cmd-1");
  assert.deepStrictEqual(envelope, {
    command_id: "cmd-1",
    run_id: "run-1",
    issued_at_wall: 1234,
    actor: "browser",
    kind,
  });
});

test("command ids are unique and non-empty", () => {
  const seen = new Set();
  for (let i = 0; i < 100; i += 1) {
    const id = helpers.newCommandId();
    assert.ok(id.length > 0);
    assert.ok(!seen.has(id));
    seen.add(id);
  }
});

test("cursor helpers dedupe and never move backwards", () => {
  assert.strictEqual(helpers.advanceCursor(5, 6), 6);
  assert.strictEqual(helpers.advanceCursor(6, 6), 6);
  assert.strictEqual(helpers.advanceCursor(6, 4), 6);
  assert.strictEqual(helpers.isNewEvent(6, 6), false);
  assert.strictEqual(helpers.isNewEvent(6, 7), true);
  // A non-numeric id never advances a numeric cursor.
  assert.strictEqual(helpers.isNewEvent(6, "7"), false);
  assert.strictEqual(helpers.isNewEvent(6, undefined), false);
  assert.strictEqual(helpers.advanceCursor(0, undefined), 0);
});

test("urls encode the run id and carry the cursor", () => {
  assert.strictEqual(
    helpers.eventsUrl("run id/1", 7),
    "/api/lab/v1/runs/run%20id%2F1/events?after=7",
  );
  assert.strictEqual(
    helpers.snapshotUrl("run id/1", null),
    "/api/lab/v1/runs/run%20id%2F1/snapshot",
  );
  assert.strictEqual(
    helpers.snapshotUrl("r", 12),
    "/api/lab/v1/runs/r/snapshot?cursor=12",
  );
});

test("escapeHtml neutralizes markup", () => {
  const input = "<img src=x onerror=\"alert('1')\">&";
  const expected = "&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt;&amp;";
  assert.strictEqual(helpers.escapeHtml(input), expected);
});

const catalog = {
  defaults: { miner_implementation: "spc-cpu-miner", hashes_per_second: "100" },
  fronts: [{ id: "front-a1", pool_id: "pool-a", sv2: "127.0.0.1:1" }],
  identities: [
    { id: "id-alice", address: "tsp1qalice", used: true },
    { id: "id-spare-1", address: "tsp1qspare", used: false },
  ],
  miners: [{ id: "miner-a", connections: [{ id: "conn-a", front_id: "front-a1", identity_id: "id-alice" }] }],
};

test("add miner builds a typed durable command from the runtime catalog", () => {
  const result = helpers.addMinerCommand(catalog, {
    minerId: "miner-c", frontId: "front-a1", identityId: "id-spare-1", hashesPerSecond: "40",
  });
  assert.strictEqual(result.ok, true);
  assert.deepStrictEqual(result.payload, {
    type: "add_miner",
    miner_id: "miner-c",
    implementation: "spc-cpu-miner",
    connections: [{ id: "miner-c-conn", front_id: "front-a1", identity_id: "id-spare-1" }],
    workers: [{
      id: "miner-c-worker",
      identity_id: "id-spare-1",
      connection_id: "miner-c-conn",
      budget: { hashes_per_second: "40" },
      nonce_start: null,
      nonce_len: null,
    }],
  });
});

test("add miner refuses a used identity, a bad id, and a zero budget", () => {
  const base = { minerId: "miner-c", frontId: "front-a1", hashesPerSecond: "10" };
  assert.strictEqual(helpers.addMinerCommand(catalog, { ...base, identityId: "id-alice" }).ok, false);
  assert.strictEqual(helpers.addMinerCommand(catalog, { ...base, identityId: "id-spare-1", minerId: "bad id" }).ok, false);
  assert.strictEqual(helpers.addMinerCommand(catalog, { ...base, identityId: "id-spare-1", hashesPerSecond: "0" }).ok, false);
});

test("switch pool uses the miner's recorded connection and identity", () => {
  const result = helpers.switchPoolCommand(catalog, "miner-a", "front-a1");
  assert.deepStrictEqual(result.payload, {
    type: "switch_pool",
    miner_id: "miner-a",
    connection_id: "conn-a",
    front_id: "front-a1",
    identity_id: "id-alice",
  });
  assert.strictEqual(helpers.switchPoolCommand(catalog, "missing", "front-a1").ok, false);
});

test("catalog url is run-scoped", () => {
  assert.strictEqual(helpers.catalogUrl("run id/1"), "/api/lab/v1/runs/run%20id%2F1/catalog");
});
