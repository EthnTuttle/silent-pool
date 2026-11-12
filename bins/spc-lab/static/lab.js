/* Read-only lab hub. No command endpoint is called from this file. */
"use strict";

function escapeHtml(value) {
  const entity = { 38: "amp", 60: "lt", 62: "gt", 34: "quot", 39: "#39" };
  return String(value).replace(/[&<>"']/g, (c) => "&" + entity[c.charCodeAt(0)] + ";");
}

function inventoryUrl(runId) {
  return `/api/lab/v1/runs/${encodeURIComponent(runId)}/inventory`;
}

/* Only known local routes can become links. Internal namespace addresses,
 * authenticated RPC and storage endpoints deliberately have no URL at all.
 * A typed operator_ui interface may link exactly its known loopback root
 * URL; no other path, query, credential or fragment is ever permitted. */
function safeActualUrl(value, kind) {
  if (value === "/" || value === "/lab") return value;
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    const loopback = url.hostname === "127.0.0.1" || url.hostname === "[::1]";
    const route = url.pathname === "/api/pool" || url.pathname === "/metrics";
    const operatorRoot = kind === "operator_ui" && url.pathname === "/";
    if (url.protocol !== "http:" || !loopback || (!route && !operatorRoot) || url.username || url.password || url.search || url.hash) return null;
    return url.href;
  } catch (_) {
    return null;
  }
}

function formatAge(ageMs) {
  if (typeof ageMs !== "number") return "time unavailable";
  if (ageMs < 1000) return "just now";
  const seconds = Math.floor(ageMs / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}

function observationText(observation) {
  if (!observation || observation.availability === "unknown") return "availability unknown";
  return String(observation.availability || "unknown").replaceAll("_", " ");
}

function observationMarkup(observation) {
  const obs = observation || {};
  const time = typeof obs.observed_at_ms === "number"
    ? `${formatAge(obs.age_ms)} / ${new Date(obs.observed_at_ms).toISOString()}`
    : "no collected availability observation";
  return `<span class="observation">${escapeHtml(observationText(obs))}<small>${escapeHtml(time)}${obs.freshness ? ` / ${escapeHtml(obs.freshness)}` : ""}</small><small>${escapeHtml(obs.source || "source unknown")}</small></span>`;
}

function monitorLinkText(recorded) {
  return recorded ? "Open recorded monitor (read-only)" : "Open live controls";
}

function interfaceMarkup(item, recorded = false) {
  const suppliedUrl = safeActualUrl(item.url, item.kind);
  const staleRecordedEndpoint = recorded && suppliedUrl && suppliedUrl !== "/" && suppliedUrl !== "/lab";
  const url = staleRecordedEndpoint ? null : suppliedUrl;
  const labelText = recorded && suppliedUrl === "/" ? monitorLinkText(true) : (item.label || "interface");
  const label = escapeHtml(labelText);
  const shown = url ? `<a href="${escapeHtml(url)}">${label}</a>` : `<span>${label}</span>`;
  const note = staleRecordedEndpoint
    ? `${item.note || ""} Recorded serve removes the historical listener URL; no current endpoint or reachability is implied.`
    : (item.note || "");
  return `<div>${shown} <span class="tag">${escapeHtml(item.kind || "interface")}</span><div class="interface-note">${escapeHtml(note)}</div></div>`;
}

function integrationStatus(item, topology) {
  switch (item.current_status) {
    case "not_running_planned_required": return "not running in this experiment / planned required integration";
    case "not_running": return "not running in this experiment";
    case "declared_in_scenario": return "declared by this scenario; see process facts";
    case "conditional_on_topology": return topology === "namespaces" ? "enabled by --topology" : "not enabled in this experiment";
    default: return "status unavailable";
  }
}

function groupMarkup(group, defaultOpen, recorded, open) {
  const rows = (group.rows || []).map((row) => {
    const roles = (row.logical_roles || []).map((role) => `<span class="role">${escapeHtml(role)}</span>`).join("");
    const interfaces = (row.interfaces || []).map((item) => interfaceMarkup(item, recorded)).join("") || "<span class='subtle'>No browser UI</span>";
    const dependencies = (row.dependencies || []).map((dependency) => `<li>${escapeHtml(dependency)}</li>`).join("") || "<li>none declared</li>";
    const details = (row.details || []).map((line) => `<li>${escapeHtml(line)}</li>`).join("");
    return `<tr>
      <td><div class="process">${escapeHtml(row.process)}</div><div class="purpose">${escapeHtml(row.purpose)}</div><div class="subtle mono">${escapeHtml(row.declared_role)}</div>
        <details class="row-detail" data-key="row-${escapeHtml(row.id)}"${open && open.has(`row-${row.id}`) ? " open" : ""}><summary>Dependencies &amp; notes</summary><div><strong>Dependencies</strong><ul>${dependencies}</ul>${details ? `<strong>Role notes</strong><ul>${details}</ul>` : ""}</div></details></td>
      <td><div class="roles">${roles}</div></td>
      <td><span class="state">${escapeHtml(row.recorded_state)}</span><div class="subtle">last recorded process state</div></td>
      <td>${observationMarkup(row.observation)}</td>
      <td><div class="interfaces">${interfaces}</div></td>
    </tr>`;
  }).join("");
  return `<details class="group" data-key="group-${escapeHtml(group.id)}"${defaultOpen ? " open" : ""}>
    <summary><strong>${escapeHtml(group.title)}</strong><span class="group-purpose">${escapeHtml(group.purpose)}</span></summary>
    <div class="table-wrap"><table><thead><tr><th>Process / declared role</th><th>Co-located logical roles</th><th>Process record</th><th>Observed availability</th><th>Interface / actual access</th></tr></thead><tbody>${rows}</tbody></table></div>
  </details>`;
}

function integrationMarkup(item, topology, open) {
  const details = (item.details || []).map((line) => `<li>${escapeHtml(line)}</li>`).join("");
  return `<details class="integration" data-key="integration-${escapeHtml(item.id)}"${open ? " open" : ""}>
    <summary><strong>${escapeHtml(item.title)}</strong><span class="integration-status">${escapeHtml(integrationStatus(item, topology))}</span></summary>
    <div><p><strong>UI:</strong> ${escapeHtml(item.ui || "No role-native browser UI")}</p>${item.required_next_profile ? "<p><strong>Required next profile:</strong> expanded live lab integration; not a claim about this run.</p>" : ""}<ul>${details}</ul></div>
  </details>`;
}

function disclosureFocusToken() {
  const active = document.activeElement;
  const details = active && active.closest("details[data-key]");
  if (!details) return null;
  if (active.matches("summary")) return { key: details.dataset.key, summary: true };
  if (active.matches("a[href]")) {
    const index = Array.from(details.querySelectorAll("a[href]")).indexOf(active);
    if (index >= 0) return { key: details.dataset.key, linkIndex: index };
  }
  return null;
}

function restoreDisclosureFocus(token) {
  if (!token) return;
  const details = Array.from(document.querySelectorAll("details[data-key]")).find((item) => item.dataset.key === token.key);
  if (!details) return;
  const target = token.summary
    ? details.querySelector(":scope > summary")
    : details.querySelectorAll("a[href]")[token.linkIndex];
  if (target) target.focus({ preventScroll: true });
}

function renderInventory(inventory, previousOpen, firstRender) {
  const open = previousOpen || new Set();
  const focusToken = disclosureFocusToken();
  document.getElementById("run-id").textContent = inventory.run_id;
  document.getElementById("lifecycle").textContent = inventory.lifecycle;
  document.getElementById("actor-count").textContent = `${inventory.recorded_live_actor_count} live / ${inventory.recorded_actor_count} recorded / ${inventory.declared_actor_count} declared`;
  document.getElementById("evidence-age").textContent = `${formatAge(inventory.latest_evidence.age_ms)} (${inventory.latest_evidence.freshness})`;
  document.getElementById("topology").textContent = `${inventory.topology} topology`;
  const recorded = inventory.view === "recorded_read_only";
  const banner = document.getElementById("view-banner");
  banner.className = `banner${recorded ? " recorded" : ""}`;
  banner.textContent = recorded
    ? "Recorded-run serve: this hub and the linked monitor are read-only. Process states and observations are historical records, not live health."
    : "Live controller view: read-only role inventory. Use Open live controls for the existing durable miner controls.";
  document.querySelector("a.primary-link").textContent = monitorLinkText(recorded);
  document.getElementById("monitor-link").textContent = recorded
    ? "Advanced recorded monitor (read-only)"
    : "Advanced legacy monitor and miner controls";
  const groups = document.getElementById("groups");
  groups.innerHTML = (inventory.groups || []).map((group) => groupMarkup(group, firstRender || open.has(`group-${group.id}`), recorded, open)).join("");
  const integrations = document.getElementById("integrations");
  integrations.innerHTML = (inventory.integrations || []).map((item) => integrationMarkup(item, inventory.topology, open.has(`integration-${item.id}`))).join("");
  restoreDisclosureFocus(focusToken);
}

const labHelpers = {
  escapeHtml,
  inventoryUrl,
  safeActualUrl,
  formatAge,
  observationText,
  monitorLinkText,
  interfaceMarkup,
  integrationStatus,
  groupMarkup,
};

if (typeof module !== "undefined" && module.exports) module.exports = labHelpers;

if (typeof window !== "undefined" && typeof document !== "undefined") {
  let firstRender = true;
  let loading = false;
  let session, context, timer;
  async function refresh() {
    if (loading || !context) return;
    loading = true;
    const epoch = session.epoch;
    try {
      const inventory = await session.json(inventoryUrl(context.run_id));
      if (epoch !== session.epoch) return;
      // Capture interaction state after the final await, immediately before
      // replacing nodes, so an in-flight response cannot undo a newer toggle.
      const open = new Set(Array.from(document.querySelectorAll("details[data-key][open]")).map((item) => item.dataset.key));
      renderInventory(inventory, open, firstRender);
      firstRender = false;
    } catch (error) {
      if (epoch !== session.epoch) return;
      const banner = document.getElementById("view-banner");
      banner.className = "banner error";
      banner.textContent = `Inventory unavailable: ${error.message}`;
    } finally {
      if (epoch === session.epoch) loading = false;
    }
  }
  LabAccess.mountAccess((access, me) => {
    session = access; context = me;
    if (!me.may.includes("operator")) {
      // Never fetch the whole Inventory with a user claim or display a 403
      // response as raw JSON. The root uses the typed own-user projection.
      window.location.replace("/"); return;
    }
    refresh(); timer = setInterval(refresh, 2000);
  }, () => {
    clearInterval(timer); context = null; loading = false; firstRender = true;
    for (const id of ["groups", "integrations", "run-id", "lifecycle", "actor-count", "evidence-age", "topology"]) document.getElementById(id).replaceChildren();
  });
}
