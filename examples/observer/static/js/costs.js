/* FORGE Observer — Token Economy Dashboard
 *
 * Real-time cost and confidence visibility via the shared SSE stream.
 *
 * Depends on:
 *   - ForgeAPI  (api.js)  — fetchJSON(), formatNumber(), formatCost()
 *   - ForgeEvents (events.js) — onEvent() for live SSE updates
 *   - D3.js v7              — confidence histogram
 */

var ForgeCosts = (function () {
  'use strict';

  // ── State ──────────────────────────────────────────────────────
  var state = {
    calls: 0,
    tokens_in: 0,
    tokens_out: 0,
    cost_usd: 0,
    by_operation: {},
    by_agent: {},
    by_provider_model: {},
    by_schedule: [],        // issue #336 — (agent, schedule) cost attribution
    budget_gate_skips: [],  // issue #336 — "saved by budget gate" tallies
    concurrent_skips: [],   // issue #336 — schedule_skipped_concurrent counts
    confidence_histogram: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    uptime_secs: 0,
    tokens_per_sec: 0
  };

  // ── DOM refs ───────────────────────────────────────────────────
  var costUsd, costCalls, tokensIn, tokensOut;
  var throughputIn, throughputOut, uptimeEl, tpsEl;
  var opTable, agentTable, providerTable;
  var scheduleTable, budgetSkipTable;
  var confChart, sseStatus;

  // ── Intervals & subscriptions ──────────────────────────────────
  var uptimeInterval = null;
  var unsubscribeSSE = null;

  // ── Formatting helpers ─────────────────────────────────────────

  function fmtUptime(secs) {
    if (secs < 60) return Math.round(secs) + 's';
    if (secs < 3600) return Math.round(secs / 60) + 'm ' + Math.round(secs % 60) + 's';
    var h = Math.floor(secs / 3600);
    var m = Math.round((secs % 3600) / 60);
    return h + 'h ' + m + 'm';
  }

  function confBadge(avg) {
    if (avg >= 0.8) return '<span class="badge badge-sm badge-success">' + avg.toFixed(2) + '</span>';
    if (avg >= 0.5) return '<span class="badge badge-sm badge-warning">' + avg.toFixed(2) + '</span>';
    return '<span class="badge badge-sm badge-error">' + avg.toFixed(2) + '</span>';
  }

  // ── Render functions ───────────────────────────────────────────

  function renderTotals() {
    costUsd.textContent = ForgeAPI.formatCost(state.cost_usd);
    costCalls.textContent = state.calls + ' calls';
    tokensIn.textContent = ForgeAPI.formatNumber(state.tokens_in);
    tokensOut.textContent = ForgeAPI.formatNumber(state.tokens_out);

    if (state.uptime_secs > 0) {
      var tIn = (state.tokens_in / state.uptime_secs).toFixed(1);
      var tOut = (state.tokens_out / state.uptime_secs).toFixed(1);
      throughputIn.textContent = tIn + ' tok/s';
      throughputOut.textContent = tOut + ' tok/s';
      tpsEl.textContent = tOut + ' tok/s out';
    }
    uptimeEl.textContent = fmtUptime(state.uptime_secs);

    // Flash animation
    costUsd.classList.add('cost-flash');
    setTimeout(function () { costUsd.classList.remove('cost-flash'); }, 400);
  }

  function renderOpTable() {
    var tbody = opTable.querySelector('tbody');
    var ops = Object.keys(state.by_operation);
    if (ops.length === 0) {
      tbody.innerHTML = '<tr><td colspan="6" class="text-center opacity-40 py-4">No data yet</td></tr>';
      return;
    }
    var rows = ops.sort().map(function (op) {
      var s = state.by_operation[op];
      var avg = s.calls > 0 ? s.avg_confidence : 0;
      return '<tr><td class="font-mono text-sm">' + op +
        '</td><td class="text-right">' + s.calls +
        '</td><td class="text-right">' + ForgeAPI.formatNumber(s.tokens_in) +
        '</td><td class="text-right">' + ForgeAPI.formatNumber(s.tokens_out) +
        '</td><td class="text-right font-mono">' + ForgeAPI.formatCost(s.cost_usd) +
        '</td><td class="text-right">' + confBadge(avg) +
        '</td></tr>';
    });
    tbody.innerHTML = rows.join('');
  }

  function renderAgentTable() {
    var tbody = agentTable.querySelector('tbody');
    var agents = Object.keys(state.by_agent);
    if (agents.length === 0) {
      tbody.innerHTML = '<tr><td colspan="5" class="text-center opacity-40 py-4">No data yet</td></tr>';
      return;
    }
    agents.sort(function (a, b) {
      return (state.by_agent[b].cost_usd || 0) - (state.by_agent[a].cost_usd || 0);
    });
    var rows = agents.map(function (agent) {
      var s = state.by_agent[agent];
      var total_tok = (s.tokens_in || 0) + (s.tokens_out || 0);
      var avg = s.calls > 0 ? s.avg_confidence : 0;
      return '<tr><td class="font-mono text-sm">' + agent +
        '</td><td class="text-right">' + s.calls +
        '</td><td class="text-right">' + ForgeAPI.formatNumber(total_tok) +
        '</td><td class="text-right font-mono">' + ForgeAPI.formatCost(s.cost_usd) +
        '</td><td class="text-right">' + confBadge(avg) +
        '</td></tr>';
    });
    tbody.innerHTML = rows.join('');
  }

  function renderProviderTable() {
    var tbody = providerTable.querySelector('tbody');
    var keys = Object.keys(state.by_provider_model);
    if (keys.length === 0) {
      tbody.innerHTML = '<tr><td colspan="4" class="text-center opacity-40 py-4">No data yet</td></tr>';
      return;
    }
    keys.sort(function (a, b) {
      return (state.by_provider_model[b].cost_usd || 0) - (state.by_provider_model[a].cost_usd || 0);
    });
    var rows = keys.map(function (k) {
      var s = state.by_provider_model[k];
      var total_tok = (s.tokens_in || 0) + (s.tokens_out || 0);
      return '<tr><td class="font-mono text-sm">' + k +
        '</td><td class="text-right">' + s.calls +
        '</td><td class="text-right">' + ForgeAPI.formatNumber(total_tok) +
        '</td><td class="text-right font-mono">' + ForgeAPI.formatCost(s.cost_usd) +
        '</td></tr>';
    });
    tbody.innerHTML = rows.join('');
  }

  function renderConfidenceChart() {
    var hist = state.confidence_histogram;
    var max = Math.max.apply(null, hist) || 1;

    confChart.innerHTML = '';

    var svg = d3.select(confChart)
      .append('svg')
      .attr('width', '100%')
      .attr('height', 200);

    var width = confChart.clientWidth || 400;
    var barWidth = Math.floor((width - 40) / 10);
    var labels = ['0.0', '0.1', '0.2', '0.3', '0.4', '0.5', '0.6', '0.7', '0.8', '0.9'];

    var g = svg.append('g').attr('transform', 'translate(20, 10)');

    g.selectAll('rect')
      .data(hist)
      .enter()
      .append('rect')
      .attr('x', function (d, i) { return i * barWidth; })
      .attr('y', function (d) { return 150 - (d / max) * 150; })
      .attr('width', barWidth - 2)
      .attr('height', function (d) { return (d / max) * 150; })
      .attr('rx', 2)
      .attr('fill', function (d, i) {
        if (i >= 8) return 'oklch(0.7 0.15 145)';   // green — sure
        if (i >= 5) return 'oklch(0.75 0.15 55)';    // amber — unsure
        return 'oklch(0.65 0.15 25)';                 // red — unreliable
      })
      .attr('opacity', 0.85);

    // Count labels on bars
    g.selectAll('.bar-label')
      .data(hist)
      .enter()
      .append('text')
      .attr('x', function (d, i) { return i * barWidth + barWidth / 2 - 1; })
      .attr('y', function (d) { return 150 - (d / max) * 150 - 4; })
      .attr('text-anchor', 'middle')
      .attr('fill', 'currentColor')
      .attr('font-size', '10px')
      .attr('opacity', 0.7)
      .text(function (d) { return d > 0 ? d : ''; });

    // X-axis labels
    g.selectAll('.x-label')
      .data(labels)
      .enter()
      .append('text')
      .attr('x', function (d, i) { return i * barWidth + barWidth / 2 - 1; })
      .attr('y', 168)
      .attr('text-anchor', 'middle')
      .attr('fill', 'currentColor')
      .attr('font-size', '10px')
      .attr('opacity', 0.5)
      .text(function (d) { return d; });
  }

  function renderScheduleTable() {
    if (!scheduleTable) return;
    if (!state.by_schedule || state.by_schedule.length === 0) {
      scheduleTable.innerHTML = '<tr><td colspan="3" class="text-center opacity-40 py-4">'
        + 'No schedule-attributed spend yet</td></tr>';
      return;
    }
    var rows = state.by_schedule.slice().sort(function (a, b) {
      return (b.cost_usd || 0) - (a.cost_usd || 0);
    }).map(function (s) {
      return '<tr>'
        + '<td class="font-mono text-sm">' + ForgeAPI.escapeHtml((s.agent || '?') + '.' + (s.schedule || '?')) + '</td>'
        + '<td class="text-right">' + (s.calls || 0) + '</td>'
        + '<td class="text-right font-mono">' + ForgeAPI.formatCost(s.cost_usd || 0) + '</td>'
        + '</tr>';
    });
    scheduleTable.innerHTML = rows.join('');
  }

  function renderBudgetSkipTable() {
    if (!budgetSkipTable) return;
    if (!state.budget_gate_skips || state.budget_gate_skips.length === 0) {
      budgetSkipTable.innerHTML = '<tr><td colspan="2" class="text-center opacity-40 py-4">'
        + 'No budget-gate skips yet</td></tr>';
      return;
    }
    var rows = state.budget_gate_skips.slice().sort(function (a, b) {
      return (b.count || 0) - (a.count || 0);
    }).map(function (s) {
      return '<tr>'
        + '<td class="font-mono text-sm">' + ForgeAPI.escapeHtml((s.agent || '?') + '.' + (s.schedule || '?')) + '</td>'
        + '<td class="text-right">' + (s.count || 0) + '</td>'
        + '</tr>';
    });
    budgetSkipTable.innerHTML = rows.join('');
  }

  function renderAll() {
    renderTotals();
    renderOpTable();
    renderAgentTable();
    renderProviderTable();
    renderScheduleTable();
    renderBudgetSkipTable();
    renderConfidenceChart();
  }

  // ── Load initial snapshot ──────────────────────────────────────

  function loadSnapshot() {
    ForgeAPI.fetchJSON('/__forge/inspect/costs')
      .then(function (data) {
        if (data.totals) {
          state.calls = data.totals.calls || 0;
          state.tokens_in = data.totals.tokens_in || 0;
          state.tokens_out = data.totals.tokens_out || 0;
          state.cost_usd = data.totals.cost_usd || 0;
        }
        state.by_operation = data.by_operation || {};
        state.by_agent = data.by_agent || {};
        state.by_provider_model = data.by_provider_model || {};
        state.by_schedule = data.by_schedule || [];
        state.budget_gate_skips = data.budget_gate_skips || [];
        state.concurrent_skips = data.concurrent_skips || [];
        state.confidence_histogram = data.confidence_histogram || state.confidence_histogram;
        state.uptime_secs = data.uptime_secs || 0;
        state.tokens_per_sec = data.tokens_per_sec || 0;
        renderAll();
      })
      .catch(function (err) {
        console.warn('[ForgeCosts] Failed to load snapshot:', err);
      });
  }

  // ── SSE event handler ─────────────────────────────────────────

  function handleLLMResponse(ev) {
    var ti = ev.tokens_in || 0;
    var to = ev.tokens_out || 0;
    var cost = ev.cost_usd || 0;
    var conf = ev.confidence || 0;
    var op = ev.operation || 'unknown';
    var agent = ev.agent || '(anonymous)';
    var providerModel = (ev.provider || 'unknown') + '/' + (ev.model || 'unknown');

    // Update totals
    state.calls += 1;
    state.tokens_in += ti;
    state.tokens_out += to;
    state.cost_usd += cost;

    // Confidence histogram
    var bucket = Math.min(Math.floor(conf * 10), 9);
    state.confidence_histogram[bucket] += 1;

    // By operation
    if (!state.by_operation[op]) {
      state.by_operation[op] = { calls: 0, tokens_in: 0, tokens_out: 0, cost_usd: 0, avg_confidence: 0, _conf_sum: 0 };
    }
    var opS = state.by_operation[op];
    opS.calls += 1;
    opS.tokens_in += ti;
    opS.tokens_out += to;
    opS.cost_usd += cost;
    opS._conf_sum = (opS._conf_sum || 0) + conf;
    opS.avg_confidence = opS._conf_sum / opS.calls;

    // By agent
    if (!state.by_agent[agent]) {
      state.by_agent[agent] = { calls: 0, tokens_in: 0, tokens_out: 0, cost_usd: 0, avg_confidence: 0, _conf_sum: 0 };
    }
    var agS = state.by_agent[agent];
    agS.calls += 1;
    agS.tokens_in += ti;
    agS.tokens_out += to;
    agS.cost_usd += cost;
    agS._conf_sum = (agS._conf_sum || 0) + conf;
    agS.avg_confidence = agS._conf_sum / agS.calls;

    // By provider/model
    if (!state.by_provider_model[providerModel]) {
      state.by_provider_model[providerModel] = { calls: 0, tokens_in: 0, tokens_out: 0, cost_usd: 0 };
    }
    var pmS = state.by_provider_model[providerModel];
    pmS.calls += 1;
    pmS.tokens_in += ti;
    pmS.tokens_out += to;
    pmS.cost_usd += cost;

    // Update SSE status indicator
    if (sseStatus) {
      sseStatus.textContent = 'Live';
      sseStatus.className = 'badge badge-sm badge-success';
    }

    renderAll();
  }

  // Budget-gate skip — increment the saved-by-budget-gate table live.
  function bumpBudgetSkip(ev) {
    var agent = ev.agent || '?';
    var schedule = ev.schedule || '?';
    var existing = state.budget_gate_skips.find(function (s) {
      return s.agent === agent && s.schedule === schedule;
    });
    if (existing) {
      existing.count = (existing.count || 0) + 1;
    } else {
      state.budget_gate_skips.push({ agent: agent, schedule: schedule, count: 1 });
    }
    renderBudgetSkipTable();
  }

  // ── Public API ─────────────────────────────────────────────────

  function init() {
    // Presence check — only activate on costs view
    var totalsEl = document.getElementById('cost-totals');
    if (!totalsEl) return;

    // Bind DOM refs
    costUsd = document.getElementById('cost-usd');
    costCalls = document.getElementById('cost-calls');
    tokensIn = document.getElementById('cost-tokens-in');
    tokensOut = document.getElementById('cost-tokens-out');
    throughputIn = document.getElementById('cost-throughput-in');
    throughputOut = document.getElementById('cost-throughput-out');
    uptimeEl = document.getElementById('cost-uptime');
    tpsEl = document.getElementById('cost-tps');
    opTable = document.getElementById('cost-by-operation');
    agentTable = document.getElementById('cost-by-agent');
    providerTable = document.getElementById('cost-by-provider');
    scheduleTable = document.getElementById('cost-by-schedule');
    budgetSkipTable = document.getElementById('cost-budget-skips');
    confChart = document.getElementById('confidence-chart');
    sseStatus = document.getElementById('cost-sse-status');

    // Load initial snapshot from inspect API
    loadSnapshot();

    // Subscribe to SSE events via the shared ForgeEvents stream
    unsubscribeSSE = ForgeEvents.onEvent(function (evt) {
      if (evt.event === 'llm_response') {
        handleLLMResponse(evt);
      } else if (evt.event === 'schedule_skipped_budget') {
        bumpBudgetSkip(evt);
      }
    });

    // Mark SSE status based on connection state
    if (sseStatus) {
      sseStatus.textContent = 'Connected';
      sseStatus.className = 'badge badge-sm badge-success';
    }

    // Uptime ticker — increment every second
    uptimeInterval = setInterval(function () {
      state.uptime_secs += 1;
      if (uptimeEl) uptimeEl.textContent = fmtUptime(state.uptime_secs);
      if (tpsEl && state.uptime_secs > 0) {
        tpsEl.textContent = (state.tokens_out / state.uptime_secs).toFixed(1) + ' tok/s out';
      }
    }, 1000);
  }

  function destroy() {
    if (uptimeInterval) {
      clearInterval(uptimeInterval);
      uptimeInterval = null;
    }
    if (unsubscribeSSE) {
      unsubscribeSSE();
      unsubscribeSSE = null;
    }
  }

  return {
    init: init,
    destroy: destroy
  };
})();
