"use strict";
/* Run with `node --test bins/spc-lab/tests/*.test.cjs`; no npm dependencies. */
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const helpers = require("../static/lab.js");

test("inventory URL is scoped to the actual run", () => {
  assert.strictEqual(helpers.inventoryUrl("run id/1"), "/api/lab/v1/runs/run%20id%2F1/inventory");
});

test("only existing local UI and pool API/metrics routes are linkable", () => {
  for (const url of ["/", "/lab", "http://127.0.0.1:18448/api/pool", "http://127.0.0.1:18448/metrics"]) {
    assert.strictEqual(helpers.safeActualUrl(url), url);
  }
  for (const url of [
    "http://10.0.0.8:18448/api/pool",
    "http://127.0.0.1:18448/",
    "http://127.0.0.1:18448/api/pool/window",
    "http://127.0.0.1:18448/api/pool?proxy=1",
    "http://user:pass@127.0.0.1:18448/api/pool",
    "https://127.0.0.1:18448/api/pool",
    "javascript:alert(1)",
    "/proxy?url=node",
  ]) assert.strictEqual(helpers.safeActualUrl(url), null, url);
});

test("typed operator_ui interfaces may link only their known loopback root URL", () => {
  assert.strictEqual(helpers.safeActualUrl("http://127.0.0.1:18448/", "operator_ui"), "http://127.0.0.1:18448/");
  assert.strictEqual(helpers.safeActualUrl("http://[::1]:18448/", "operator_ui"), "http://[::1]:18448/");
  // Any other kind keeps rejecting the operator root.
  for (const kind of [undefined, "ui", "api_json", "metrics", "native_protocol"]) {
    assert.strictEqual(helpers.safeActualUrl("http://127.0.0.1:18448/", kind), null, String(kind));
  }
  // The operator kind opens no other path, and no query, credential or scheme.
  for (const url of [
    "http://127.0.0.1:18448/operator",
    "http://127.0.0.1:18448/api/pool/observation",
    "http://127.0.0.1:18448/?proxy=1",
    "http://127.0.0.1:18448/#x",
    "http://user:pass@127.0.0.1:18448/",
    "https://127.0.0.1:18448/",
    "http://10.0.0.8:18448/",
  ]) assert.strictEqual(helpers.safeActualUrl(url, "operator_ui"), null, url);
});

test("operator interface links when live and is unlinked with no stale URL when recorded", () => {
  const item = {
    label: "Pool operator page", kind: "operator_ui", url: "http://127.0.0.1:18449/", note: "Approved read-only operator page.",
  };
  const live = helpers.interfaceMarkup(item, false);
  assert.ok(live.includes('href="http://127.0.0.1:18449/"'));
  assert.ok(live.includes("operator_ui"));
  const recorded = helpers.interfaceMarkup(item, true);
  assert.ok(!recorded.includes("<a "));
  assert.ok(!recorded.includes("18449"));
  assert.ok(recorded.includes("historical listener URL"));
});

test("namespaced and authenticated interfaces render without a clickable URL", () => {
  const html = helpers.interfaceMarkup({
    label: "Pool API JSON",
    kind: "api_json",
    url: null,
    note: "Internal namespace transport; no host-browser link.",
  });
  assert.ok(!html.includes("<a "));
  assert.ok(html.includes("Pool API JSON"));
  assert.ok(html.includes("api_json"));
  assert.ok(html.includes("Internal namespace"));
});

test("pool API is explicitly JSON rather than a claimed operator UI", () => {
  const html = helpers.interfaceMarkup({
    label: "Pool API JSON", kind: "api_json", url: "http://127.0.0.1:18448/api/pool", note: "JSON",
  });
  assert.ok(html.includes("href=\"http://127.0.0.1:18448/api/pool\""));
  assert.ok(html.includes("api_json"));
  assert.ok(!html.includes(">ui<"));
});

test("recorded mode labels the monitor and unlinks stale external pool endpoints", () => {
  assert.strictEqual(helpers.monitorLinkText(true), "Open recorded monitor (read-only)");
  assert.strictEqual(helpers.monitorLinkText(false), "Open live controls");
  const item = {
    label: "Pool API JSON", kind: "api_json", url: "http://127.0.0.1:45753/api/pool", note: "Actual API route.",
  };
  assert.ok(helpers.interfaceMarkup(item, false).includes("<a href="));
  const recorded = helpers.interfaceMarkup(item, true);
  assert.ok(!recorded.includes("<a "));
  assert.ok(!recorded.includes("45753"));
  assert.ok(recorded.includes("historical listener URL"));
  const local = helpers.interfaceMarkup({ label: "Open live controls", kind: "ui", url: "/", note: "Monitor." }, true);
  assert.ok(local.includes("href=\"/\""));
  assert.ok(local.includes("Open recorded monitor (read-only)"));
});

test("row disclosure state is carried into replacement group markup", () => {
  const group = {
    id: "controller", title: "Controller", purpose: "Test",
    rows: [{
      id: "spc-lab", process: "spc-lab", purpose: "Controller", declared_role: "controller",
      logical_roles: [], recorded_state: "stopped", observation: null, interfaces: [], dependencies: [], details: [],
    }],
  };
  const open = helpers.groupMarkup(group, true, true, new Set(["row-spc-lab"]));
  assert.ok(open.includes('data-key="row-spc-lab" open'));
  const closed = helpers.groupMarkup(group, true, true, new Set());
  assert.ok(!closed.includes('data-key="row-spc-lab" open'));
});

test("required Ark and wallet gaps are never rendered as running", () => {
  assert.strictEqual(
    helpers.integrationStatus({ current_status: "not_running_planned_required" }, "host"),
    "not running in this experiment / planned required integration",
  );
  assert.strictEqual(helpers.integrationStatus({ current_status: "not_running" }, "host"), "not running in this experiment");
  assert.strictEqual(helpers.integrationStatus({ current_status: "conditional_on_topology" }, "namespaces"), "enabled by --topology");
});

test("observations distinguish unknown availability from recorded evidence", () => {
  assert.strictEqual(helpers.observationText({ availability: "unknown" }), "availability unknown");
  assert.strictEqual(helpers.observationText({ availability: "reachable_when_observed" }), "reachable when observed");
  assert.strictEqual(helpers.formatAge(null), "time unavailable");
  assert.strictEqual(helpers.formatAge(2000), "2s ago");
});

test("role data is escaped before insertion", () => {
  assert.strictEqual(helpers.escapeHtml("<img src=x>"), "&lt;img src=x&gt;");
});

test("lab page links the existing monitor and contains no command controls", () => {
  const html = fs.readFileSync(path.join(__dirname, "../static/lab.html"), "utf8");
  const js = fs.readFileSync(path.join(__dirname, "../static/lab.js"), "utf8");
  assert.ok(html.includes("href=\"/\""));
  assert.ok(html.includes("Open run monitor"));
  assert.ok(!html.includes(">Open live controls</a>"));
  assert.ok(html.includes("Advanced run monitor"));
  assert.ok(js.includes("Advanced legacy monitor and miner controls"));
  assert.strictEqual((html.match(/<button\b/g) || []).length, 2, "only sign-in and logout buttons");
  assert.ok(html.includes('id="auth-form"'));
  assert.ok(html.includes('id="private-view" hidden'));
  assert.ok(!html.includes("id=\"role-start\""));
  assert.ok(!js.includes("/commands"));
  assert.ok(!js.includes("method: \"POST\""));
});
