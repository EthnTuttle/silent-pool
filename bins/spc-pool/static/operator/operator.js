/* spc-pool operator page (EXP-OP).
 *
 * Read-only: fetches only its own same-origin operator route
 * (api/pool/observation) on a poll. Every rendered value goes through
 * textContent - there is no innerHTML, eval, window.open, or any href/src
 * built from data, so every DTO string is inert text. Monetary values are
 * exact decimal strings parsed with BigInt for validation and formatted
 * by string grouping; they never pass through Number or parseInt, and a
 * sats string with a leading '-' is an invalid value, not a negative
 * amount (the server emits u64 only).
 *
 * Poll discipline (Fable R3 rework): at most one observation request is
 * in flight (a tick while a request pends is skipped), completions carry
 * a monotonic sequence so an obsolete response or failure can never reset
 * newer state, and lastGoodAt advances only on a payload with no closed
 * errors - an HTTP 200 whose fields are null is an outage, not
 * freshness. Staleness is bounded (F5): after three poll intervals
 * without good data the main facts read unknown (observation
 * unreachable) instead of showing old values indefinitely.
 */
'use strict';

(function () {
  var POLL_MS = 10000;
  var POLL_SKEW_MS = 500;
  /* F5: after this long without a good payload, transport-failure
   * staleness stops showing old values and the main facts read unknown. */
  var STALE_AFTER_MS = 3 * POLL_MS;

  function $(id) { return document.getElementById(id); }

  var els = {
    staleBanner: $('stale-banner'),
    network: $('v-network'),
    latestBlock: $('v-latest-block'),
    latestBlockNote: $('v-latest-block-note'),
    liability: $('v-liability'),
    liabilityAsof: $('v-liability-asof'),
    arkDue: $('v-ark-due'),
    float: $('v-float'),
    details: $('details'),
    blocksBody: $('blocks-body'),
    blocksNote: $('blocks-note'),
    payouts: $('payouts'),
    workProfile: $('work-profile'),
    freshness: $('freshness'),
    errorList: $('error-list')
  };

  var lastPayloadText = null;
  var lastGoodAt = null;
  var lastGoodDto = null;

  /* Poll discipline state (R3): at most one request in flight; responses
   * and failures carry the sequence of the request that produced them, and
   * anything older than the last applied sequence is dropped. */
  var inFlight = false;
  var requestSeq = 0;
  var lastRenderedSeq = 0;

  /* --- exact money formatting (OP-2) --------------------------------- */

  /* Parse a decimal string with BigInt (validation only), then group the
   * original digits. Never Number, never parseInt: an amount above
   * 2^53 must survive byte-exact. A leading '-' is rejected outright
   * (F6): sats are u64 on the server, so a negative string is an invalid
   * value, never a negative amount to render. */
  function formatSats(value) {
    if (typeof value !== 'string') { return null; }
    if (value.charCodeAt(0) === 45) { return null; /* '-' */ }
    if (value.length === 0 || !/^[0-9]+$/.test(value)) { return null; }
    try {
      BigInt(value); /* throws on anything but an exact integer */
    } catch (err) {
      return null;
    }
    var groups = [];
    var digits = value;
    while (digits.length > 3) {
      groups.unshift(digits.slice(-3));
      digits = digits.slice(0, -3);
    }
    groups.unshift(digits);
    return groups.join(' ') + ' sats';
  }

  /* --- inert DOM helpers ---------------------------------------------- */

  function setText(el, text) {
    if (el && el.textContent !== text) { el.textContent = text; }
  }

  function setUnknown(el, reason) {
    el.classList.add('unknown');
    setText(el, reason ? 'unknown (' + reason + ')' : 'unknown');
  }

  function setMoney(el, value, reason) {
    el.classList.remove('unknown');
    var formatted = formatSats(value);
    if (formatted === null) {
      setUnknown(el, reason || 'invalid value');
      return;
    }
    setText(el, formatted);
  }

  function makeRow(cells) {
    var tr = document.createElement('tr');
    for (var i = 0; i < cells.length; i++) {
      var td = document.createElement('td');
      if (cells[i].cls) { td.className = cells[i].cls; }
      td.textContent = cells[i].text;
      tr.appendChild(td);
    }
    return tr;
  }

  function errorFor(dto, field) {
    var errors = dto.errors || [];
    for (var i = 0; i < errors.length; i++) {
      if (errors[i] && errors[i].field === field) { return errors[i]; }
    }
    return null;
  }

  function reason(err) {
    if (!err) { return null; }
    var parts = [];
    if (err.code) { parts.push(String(err.code)); }
    if (err.source) { parts.push(String(err.source)); }
    return parts.length ? parts.join(', ') : 'unknown';
  }

  /* --- rendering ------------------------------------------------------- */

  function renderNetwork(dto) {
    var profile = (dto.work_profile && typeof dto.work_profile === 'object')
      ? dto.work_profile : {};
    var version = profile.work_unit_version;
    var rule = profile.window_rule;
    var windowN = (typeof dto.window_n === 'number' && isFinite(dto.window_n))
      ? dto.window_n : null;
    var bits = [];
    if (typeof dto.network === 'string' && dto.network) { bits.push(dto.network); }
    if (version !== undefined && version !== null) {
      bits.push('work profile v' + String(version));
    }
    if (typeof rule === 'string' && rule) { bits.push(String(rule)); }
    if (windowN !== null) { bits.push('window N=' + windowN); }
    els.network.classList.remove('unknown');
    setText(els.network, bits.length ? bits.join(' \u00b7 ') : 'unknown');
  }

  function renderLatestBlock(dto) {
    var err = errorFor(dto, 'recent_blocks');
    if (err) {
      setUnknown(els.latestBlock, reason(err));
      setText(els.latestBlockNote, 'Block list unavailable; not "no blocks".');
      return;
    }
    var blocks = dto.recent_blocks;
    if (!blocks || !Array.isArray(blocks.rows)) {
      setUnknown(els.latestBlock, null);
      setText(els.latestBlockNote, '');
      return;
    }
    var rows = blocks.rows;
    if (rows.length === 0) {
      els.latestBlock.classList.remove('unknown');
      setText(els.latestBlock, 'none observed yet');
      setText(els.latestBlockNote,
        'The block table is empty (a successful read of zero rows).');
      return;
    }
    var first = rows[0] || {};
    els.latestBlock.classList.remove('unknown');
    var height = (first.height === undefined || first.height === null)
      ? 'unknown height' : 'height ' + String(first.height);
    var status = (typeof first.status === 'string' && first.status)
      ? first.status : 'unknown status';
    setText(els.latestBlock, height + ' \u00b7 ' + status);
    setText(els.latestBlockNote,
      'Derived from the first of the ten most recent rows' +
      (typeof first.settled_at === 'string' && first.settled_at
        ? '; settled ' + shortTime(first.settled_at) : '') + '.');
  }

  function renderLiability(dto) {
    var err = errorFor(dto, 'pending_liability');
    if (err) {
      setUnknown(els.liability, reason(err));
      setText(els.liabilityAsof, '');
      setUnknown(els.arkDue, reason(err));
      return;
    }
    var liab = dto.pending_liability;
    if (!liab) {
      setUnknown(els.liability, null);
      setUnknown(els.arkDue, null);
      setText(els.liabilityAsof, '');
      return;
    }
    setMoney(els.liability, liab.total_sats, 'invalid value');
    setMoney(els.arkDue, liab.ark_due_sats, 'invalid value');
    setText(els.liabilityAsof,
      typeof liab.as_of === 'string'
        ? 'As of ' + shortTime(liab.as_of) + '.' : '');
  }

  function renderFloat(dto) {
    var float = dto.float || {};
    var state = (typeof float.state === 'string' && float.state)
      ? float.state : 'unknown';
    var why = (typeof float.reason === 'string' && float.reason)
      ? ' (' + float.reason + ')' : '';
    els.float.classList.toggle('unknown', state === 'not_configured' || state === 'unknown');
    setText(els.float, state + why);
  }

  /* Rows are rebuilt only when the payload changed; table cells carry no
   * tabindex, so a keyboard user's focus (summary, body) is never inside
   * the rebuilt nodes and poll refreshes preserve it. */
  function rebuildRows(rows) {
    while (els.blocksBody.firstChild) {
      els.blocksBody.removeChild(els.blocksBody.firstChild);
    }
    for (var i = 0; i < rows.length; i++) {
      var row = rows[i] || {};
      els.blocksBody.appendChild(makeRow([
        {
          text: (row.height === undefined || row.height === null)
            ? '' : String(row.height)
        },
        { text: typeof row.status === 'string' ? row.status : '' },
        {
          text: typeof row.settled_at === 'string' && row.settled_at
            ? shortTime(row.settled_at) : ''
        },
        {
          text: typeof row.block_hash === 'string' ? row.block_hash : '',
          cls: 'hash'
        }
      ]));
    }
  }

  function renderDetails(dto) {
    var blocksErr = errorFor(dto, 'recent_blocks');
    var blocks = dto.recent_blocks;
    if (blocksErr) {
      setText(els.blocksNote,
        'Unavailable (' + reason(blocksErr) + '); not an empty table.');
      rebuildRows([]);
    } else if (blocks && Array.isArray(blocks.rows)) {
      rebuildRows(blocks.rows);
      setText(els.blocksNote,
        'Read at ' + shortTime(blocks.observed_at) +
        ' (a separate read from the funds snapshot).' +
        (blocks.rows.length === 0 ? ' The table is empty.' : ''));
    }

    var payoutErr = errorFor(dto, 'ark_payouts_by_status');
    while (els.payouts.firstChild) {
      els.payouts.removeChild(els.payouts.firstChild);
    }
    if (payoutErr) {
      appendDl(els.payouts, 'status counts', 'unknown (' + reason(payoutErr) + ')');
    } else if (dto.ark_payouts_by_status && typeof dto.ark_payouts_by_status === 'object') {
      var statuses = ['scheduled', 'sent', 'failed_to_pending'];
      for (var i = 0; i < statuses.length; i++) {
        var n = dto.ark_payouts_by_status[statuses[i]];
        appendDl(els.payouts, statuses[i],
          (typeof n === 'number' && isFinite(n)) ? String(n) : 'unknown');
      }
    }

    while (els.workProfile.firstChild) {
      els.workProfile.removeChild(els.workProfile.firstChild);
    }
    if (dto.work_profile && typeof dto.work_profile === 'object') {
      var keys = Object.keys(dto.work_profile).sort();
      for (var k = 0; k < keys.length; k++) {
        var v = dto.work_profile[keys[k]];
        var shown = (v === null || v === undefined)
          ? 'none' : (typeof v === 'object') ? JSON.stringify(v) : String(v);
        appendDl(els.workProfile, keys[k], shown);
      }
    }

    while (els.freshness.firstChild) {
      els.freshness.removeChild(els.freshness.firstChild);
    }
    appendDl(els.freshness, 'schema',
      typeof dto.schema === 'string' ? dto.schema : 'unknown');
    appendDl(els.freshness, 'generated at',
      typeof dto.generated_at === 'string' ? shortTime(dto.generated_at) : 'unknown');
    var liab = dto.pending_liability;
    appendDl(els.freshness, 'funds as of',
      liab && typeof liab.as_of === 'string' ? shortTime(liab.as_of) : 'unknown');
    appendDl(els.freshness, 'blocks observed at',
      dto.recent_blocks && typeof dto.recent_blocks.observed_at === 'string'
        ? shortTime(dto.recent_blocks.observed_at) : 'unknown');
    appendDl(els.freshness, 'sources',
      dto.sources && dto.sources.liability && dto.sources.blocks
        ? 'liability: ' + String(dto.sources.liability) +
          ', blocks: ' + String(dto.sources.blocks)
        : 'unknown');

    while (els.errorList.firstChild) {
      els.errorList.removeChild(els.errorList.firstChild);
    }
    var errors = dto.errors || [];
    for (var e = 0; e < errors.length; e++) {
      var li = document.createElement('li');
      li.textContent = String(errors[e].field) + ': ' +
        String(errors[e].code) + ' (source: ' + String(errors[e].source) + ')';
      els.errorList.appendChild(li);
    }
  }

  function appendDl(dl, term, def) {
    var dt = document.createElement('dt');
    dt.textContent = term;
    var dd = document.createElement('dd');
    dd.textContent = def;
    dl.appendChild(dt);
    dl.appendChild(dd);
  }

  function shortTime(iso) {
    if (typeof iso !== 'string' || !iso) { return 'unknown'; }
    var t = Date.parse(iso);
    if (isNaN(t)) { return iso; }
    var d = new Date(t);
    var two = function (n) { return (n < 10 ? '0' : '') + n; };
    return d.getFullYear() + '-' + two(d.getMonth() + 1) + '-' + two(d.getDate()) +
      ' ' + two(d.getHours()) + ':' + two(d.getMinutes()) + ':' + two(d.getSeconds()) +
      ' UTC' + (d.getTimezoneOffset() === 0 ? '' :
        (d.getTimezoneOffset() > 0 ? '-' : '+') +
        two(Math.floor(Math.abs(d.getTimezoneOffset()) / 60)) + ':' +
        two(Math.abs(d.getTimezoneOffset()) % 60));
  }

  /* --- stale handling --------------------------------------------------- */

  function markStale(message) {
    els.staleBanner.hidden = false;
    els.staleBanner.textContent = message;
    var facts = document.querySelectorAll('.fact');
    for (var i = 0; i < facts.length; i++) { facts[i].classList.add('stale'); }
  }

  function clearStale() {
    els.staleBanner.hidden = true;
    els.staleBanner.textContent = '';
    var facts = document.querySelectorAll('.fact');
    for (var i = 0; i < facts.length; i++) { facts[i].classList.remove('stale'); }
  }

  /* A good payload: parsed, right schema, and no closed-code errors. A
   * 200 response whose fields are null (store outage) is NOT good: it
   * renders the server's unknowns, but it must not advance lastGoodAt or
   * reset the failure machinery (R3: no pretending an error payload is
   * freshness). */
  function isGoodPayload(dto) {
    return !!(dto && Array.isArray(dto.errors) && dto.errors.length === 0);
  }

  function applyPayload(dto, payloadText) {
    var changed = payloadText !== lastPayloadText;
    lastPayloadText = payloadText;
    if (isGoodPayload(dto)) {
      lastGoodDto = dto;
      lastGoodAt = Date.now();
    }
    clearStale();
    renderNetwork(dto);
    renderLatestBlock(dto);
    renderLiability(dto);
    renderFloat(dto);
    if (changed) {
      renderDetails(dto);
    }
  }

  function renderUnknownAll(message) {
    els.staleBanner.hidden = false;
    els.staleBanner.textContent = message;
    setUnknown(els.network, null);
    setUnknown(els.latestBlock, null);
    setUnknown(els.liability, null);
    setUnknown(els.arkDue, null);
    setUnknown(els.float, null);
    setText(els.liabilityAsof, '');
    setText(els.latestBlockNote, '');
  }

  /* F5: bounded staleness. Once the observation endpoint has been
   * unreachable for longer than STALE_AFTER_MS, the five main facts stop
   * repeating the last good payload and read unknown with the transport
   * reason (never a zero, never the old values as current). Detail rows
   * keep their observed-at labels, so anything historical stays
   * explicitly labeled. */
  function renderUnreachable(message) {
    els.staleBanner.hidden = false;
    els.staleBanner.textContent = message;
    setUnknown(els.network, 'observation unreachable');
    setUnknown(els.latestBlock, 'observation unreachable');
    setUnknown(els.liability, 'observation unreachable');
    setUnknown(els.arkDue, 'observation unreachable');
    setUnknown(els.float, 'observation unreachable');
    setText(els.liabilityAsof, '');
    setText(els.latestBlockNote,
      'The operator endpoint could not be reached; values are unknown.');
  }

  /* --- poll -------------------------------------------------------------- */

  function errText(err) {
    return String(err && err.message ? err.message : err);
  }

  function poll() {
    /* R3: at most one observation request in flight. A tick that fires
     * while a slow (or timed-out) response is pending is skipped, so the
     * page can never stack concurrent requests against the server. */
    if (inFlight) { return; }
    inFlight = true;
    var seq = ++requestSeq;
    fetch('api/pool/observation', { cache: 'no-store' })
      .then(function (res) {
        if (!res.ok) { throw new Error('observation fetch failed: ' + res.status); }
        return res.text();
      })
      .then(function (text) {
        var dto;
        try {
          dto = JSON.parse(text);
        } catch (err) {
          throw new Error('observation is not JSON');
        }
        if (!dto || dto.schema !== 'spc-pool-observation/v1') {
          throw new Error('unexpected observation schema');
        }
        inFlight = false;
        /* R3: a completion older than the last applied one (possible only
         * if the guard above is ever relaxed) must not reset newer
         * state. */
        if (seq < lastRenderedSeq) { return; }
        lastRenderedSeq = seq;
        applyPayload(dto, text);
      })
      .catch(function (err) {
        inFlight = false;
        /* R3: an obsolete failure is as ignorable as an obsolete
         * success. */
        if (seq < lastRenderedSeq) { return; }
        lastRenderedSeq = seq;
        if (lastGoodDto === null) {
          renderUnknownAll('No observation yet (' + errText(err) + ').');
          return;
        }
        var age = Date.now() - lastGoodAt;
        var seconds = Math.max(0, Math.round(age / 1000));
        if (age > STALE_AFTER_MS) {
          renderUnreachable('Observation unreachable (' + errText(err) +
            '); no good data for ' + seconds +
            's. Main facts show unknown, not old values.');
        } else {
          markStale('Observation update failed (' + errText(err) +
            '); showing data as of ' + seconds + 's ago. Values may be stale.');
        }
      });
  }

  poll();
  setInterval(poll, POLL_MS + Math.floor(Math.random() * POLL_SKEW_MS));
}());
