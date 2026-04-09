// FORGE Sentinel — client-side interactivity

// Dark mode toggle
function toggleTheme() {
  var html = document.documentElement;
  var current = html.getAttribute('data-theme');
  var next = current === 'light' ? 'dark' : 'light';
  html.setAttribute('data-theme', next);
  localStorage.setItem('forge-sentinel-theme', next);
}

// Restore saved theme
(function () {
  var saved = localStorage.getItem('forge-sentinel-theme');
  if (saved) {
    document.documentElement.setAttribute('data-theme', saved);
  }
})();

// ── Auto-refresh ───────────────────────────────────────────────
// Poll /api_health every 30s and update the badge if present.

var REFRESH_INTERVAL = 30000;

function refreshHealth() {
  var badge = document.getElementById('health-badge');
  if (!badge) return;

  fetch('/api_health')
    .then(function (r) { return r.text(); })
    .then(function (text) {
      var trimmed = text.trim().toLowerCase();
      badge.textContent = trimmed;
      badge.className = 'badge badge-lg font-bold health-' + trimmed;
    })
    .catch(function () { /* silent — next poll will retry */ });
}

setInterval(refreshHealth, REFRESH_INTERVAL);

// ── FORGE Activity Log ─────────────────────────────────────────

function createActivityLog(steps) {
  var log = document.createElement('div');
  log.className = 'forge-activity';
  log.innerHTML = '<div class="log-title">FORGE Runtime</div>';

  var startTime = Date.now();
  var entries = [];

  steps.forEach(function (step, i) {
    var entry = document.createElement('div');
    entry.className = 'log-entry';
    entry.style.animationDelay = (i * 0.6) + 's';
    entry.innerHTML =
      '<span class="step-icon">○</span> ' +
      '<span class="step-label">' + step.label + '</span> ' +
      '<span class="step-detail">' + step.detail + '</span>' +
      '<span class="step-time"></span>';
    log.appendChild(entry);
    entries.push({ el: entry, delay: i * 600 });
  });

  entries.forEach(function (e) {
    setTimeout(function () {
      e.el.classList.add('active');
      e.el.querySelector('.step-icon').textContent = '\u25CC';
    }, e.delay);
  });

  function markDone(index) {
    if (index >= entries.length) return;
    var e = entries[index];
    setTimeout(function () {
      e.el.classList.remove('active');
      e.el.classList.add('done');
      e.el.querySelector('.step-icon').textContent = '\u2713';
      var elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
      e.el.querySelector('.step-time').textContent = elapsed + 's';
      markDone(index + 1);
    }, e.delay + 400);
  }
  markDone(0);

  return log;
}

var SCAN_STEPS = [
  { label: 'exec', detail: '\u2192 gathering git data...' },
  { label: 'exec', detail: '\u2192 measuring code metrics...' },
  { label: 'reason', detail: '\u2192 analyzing patterns...' },
  { label: 'classify', detail: '\u2192 scoring health...' },
  { label: 'pool', detail: '\u2192 consensus vote (3 workers)...' },
  { label: 'data.store', detail: '\u2192 publishing results...' }
];

// ── Scan button interceptor ────────────────────────────────────

document.addEventListener('DOMContentLoaded', function () {
  var scanBtn = document.getElementById('scan-trigger');
  if (scanBtn) {
    scanBtn.addEventListener('click', function (e) {
      e.preventDefault();
      // Navigate to observer and trigger scan there for full live experience
      window.location.href = '/observer?scan=1';
    });
  }
});

function escapeHtml(text) {
  var div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// ── Observer: Agent Tree + SSE Live Events ─────────────────────
//
// Only activates on the /observer page (presence of #tree-root).

(function () {
  var treeRoot = document.getElementById('tree-root');
  if (!treeRoot) return;

  var eventLog = document.getElementById('event-log');
  var eventLogEmpty = document.getElementById('event-log-empty');
  var detailContent = document.getElementById('detail-content');
  var detailClose = document.getElementById('detail-close');
  var wardenPanel = document.getElementById('warden-panel');
  var sseStatus = document.getElementById('sse-status');
  var scanTrigger = document.getElementById('scan-trigger-obs');

  var selectedAgentId = null;
  var agentMap = {};       // id -> agent data
  var lastEventTime = 0;
  var staleTimer = null;
  var thinkingAgents = {}; // track llm_request -> llm_response

  // ── Formatting helpers ───────────────────────────────────────

  function formatUptime(ms) {
    if (ms < 1000) return ms + 'ms';
    var s = Math.floor(ms / 1000);
    if (s < 60) return s + 's';
    var m = Math.floor(s / 60);
    s = s % 60;
    return m + 'm ' + s + 's';
  }

  function formatBytes(b) {
    if (b < 1024) return b + ' B';
    return (b / 1024).toFixed(1) + ' KB';
  }

  function relativeTime() {
    return ((Date.now() - pageStart) / 1000).toFixed(1) + 's';
  }

  var pageStart = Date.now();

  // ── Fetch helpers ────────────────────────────────────────────

  function fetchJSON(url) {
    return fetch(url).then(function (r) { return r.json(); });
  }

  // ── Tree Building ────────────────────────────────────────────

  function buildTree(topology, agents, wardens) {
    var html = '<ul>';
    var systemName = topology.system_name || 'system';
    var bindings = topology.bindings || [];
    var wiring = topology.wiring || [];

    // System root node
    html += '<li><div class="tree-node-card" data-node-type="system">'
      + '<span class="state-dot running"></span>'
      + '<span class="node-type">system</span>'
      + '<span class="node-name">' + escapeHtml(systemName) + '</span>'
      + '</div>';

    // Collect all children into a single <ul>
    var children = '';

    // Wardens with their managed agents
    if (wardens && wardens.length > 0) {
      wardens.forEach(function (w) {
        var tripped = w.circuit_breaker_tripped;
        var dotCls = tripped ? 'degraded' : 'running';
        var wardenIdx = wardens.indexOf(w);
        children += '<li><div class="tree-node-card" data-node-type="warden"'
          + ' data-warden-idx="' + wardenIdx + '"'
          + ' onclick="window._selectWarden(' + wardenIdx + ')">'
          + '<span class="state-dot ' + dotCls + '"></span>'
          + '<span class="node-type">warden</span>'
          + '<span class="node-name">' + escapeHtml(w.name) + '</span>';
        if (tripped) {
          children += ' <span class="flag-badge warn">circuit open</span>';
        }
        children += '</div>';

        var managed = w.managed_agents || [];
        if (managed.length > 0) {
          children += '<ul>';
          managed.forEach(function (agentName) {
            var agent = findAgentByName(agents, agentName);
            children += buildAgentNode(agent, agentName, bindings);
          });
          children += '</ul>';
        }
        children += '</li>';
      });
    } else if (agents && agents.length > 0) {
      // No wardens — show running agents directly
      agents.forEach(function (a) {
        children += buildAgentNode(a, a.name, bindings);
      });
    } else if (bindings.length > 0) {
      // No live agents — show static topology from bindings
      bindings.forEach(function (b) {
        children += buildAgentNode(null, b[1], bindings);
      });
    }

    // Wiring labels as children
    wiring.forEach(function (chain) {
      children += '<li><div class="wiring-label" data-node-type="wiring">'
        + chain.join(' >> ') + '</div></li>';
    });

    if (children) {
      html += '<ul>' + children + '</ul>';
    }

    html += '</li></ul>';
    treeRoot.innerHTML = html;
  }

  function findAgentByName(agents, name) {
    for (var i = 0; i < agents.length; i++) {
      if (agents[i].name === name) return agents[i];
    }
    return null;
  }

  function findAlias(bindings, agentName) {
    if (!bindings) return null;
    for (var i = 0; i < bindings.length; i++) {
      if (bindings[i][1] === agentName) return bindings[i][0];
    }
    return null;
  }

  function buildAgentNode(agent, agentName, bindings) {
    var alias = findAlias(bindings, agentName);
    var id = agent ? agent.id : '';
    var state = agent ? (agent.lifecycle_state || 'idle') : 'idle';
    var status = agent ? agent.status : 'unknown';
    var uptime = agent ? agent.uptime_ms : 0;
    var dotCls = status === 'running' ? 'running' : 'idle';

    if (agent) agentMap[id] = agent;

    var html = '<li><div class="tree-node-card" data-node-type="agent"'
      + ' data-agent-id="' + escapeHtml(id) + '"'
      + ' data-agent-name="' + escapeHtml(agentName) + '"'
      + ' data-state="' + escapeHtml(dotCls) + '"'
      + ' onclick="window._selectAgent(\'' + escapeHtml(id) + '\')">'
      + '<span class="state-dot ' + dotCls + '"></span>'
      + '<span class="node-type">agent</span>'
      + '<span class="node-name">' + escapeHtml(agentName) + '</span>';
    if (alias && alias !== agentName) {
      html += '<span class="node-alias">(' + escapeHtml(alias) + ')</span>';
    }
    if (uptime > 0) {
      html += '<span class="node-uptime">' + formatUptime(uptime) + '</span>';
    }
    html += '<span class="thinking-indicator">reasoning...</span>';
    html += '</div></li>';
    return html;
  }

  // ── Agent State Polling ──────────────────────────────────────

  function updateAgentStates(agents) {
    agents.forEach(function (a) {
      agentMap[a.id] = a;
      var card = treeRoot.querySelector('[data-agent-id="' + a.id + '"]');
      if (!card) return;
      var dotCls = a.status === 'running' ? 'running' : 'idle';
      card.setAttribute('data-state', dotCls);
      var dot = card.querySelector('.state-dot');
      if (dot) dot.className = 'state-dot ' + dotCls;
      var uptimeEl = card.querySelector('.node-uptime');
      if (uptimeEl) uptimeEl.textContent = formatUptime(a.uptime_ms);
    });
    // Update detail if open
    if (selectedAgentId) showAgentDetail(selectedAgentId);
  }

  // ── Detail Panel (Agent + Warden) ─────────────────────────────

  var cachedWardens = [];

  window._selectWarden = function (idx) {
    selectedAgentId = null;
    var w = cachedWardens[idx];
    if (!w) return;

    // Mark selected
    var cards = treeRoot.querySelectorAll('.tree-node-card');
    for (var i = 0; i < cards.length; i++) cards[i].classList.remove('selected');
    var sel = treeRoot.querySelector('[data-warden-idx="' + idx + '"]');
    if (sel) sel.classList.add('selected');

    var html = '<div class="flex items-center gap-2 mb-3">'
      + '<span class="state-dot ' + (w.circuit_breaker_tripped ? 'degraded' : 'running') + '"></span>'
      + '<span class="font-bold">' + escapeHtml(w.name) + '</span>'
      + '<span class="node-type">warden</span></div>';

    html += '<div class="detail-section">Supervision</div>';
    html += detailField('Managed agents', (w.managed_agents || []).join(', '));
    html += detailField('Degraded agents', (w.degraded_agents || []).length > 0
      ? w.degraded_agents.join(', ') : 'none');
    html += detailField('Circuit breaker', w.circuit_breaker_tripped ? 'TRIPPED' : 'ok');

    var retries = w.retry_counts || {};
    var retryKeys = Object.keys(retries);
    if (retryKeys.length > 0) {
      html += '<div class="detail-section">Retries</div>';
      retryKeys.forEach(function (k) {
        html += detailField(k, retries[k]);
      });
    }

    detailContent.innerHTML = html;
  };

  window._selectAgent = function (id) {
    selectedAgentId = id;
    // Mark selected
    var cards = treeRoot.querySelectorAll('.tree-node-card');
    for (var i = 0; i < cards.length; i++) {
      cards[i].classList.remove('selected');
    }
    var sel = treeRoot.querySelector('[data-agent-id="' + id + '"]');
    if (sel) sel.classList.add('selected');
    showAgentDetail(id);
  };

  function showAgentDetail(id) {
    if (!id) return;
    fetchJSON('/__forge/inspect/agents/' + id)
      .then(function (data) {
        var html = '';

        // Header
        html += '<div class="flex items-center gap-2 mb-3">'
          + '<span class="state-dot ' + (data.status === 'running' ? 'running' : 'idle') + '"></span>'
          + '<span class="font-bold">' + escapeHtml(data.name) + '</span>';
        if (data.alias) html += '<span class="opacity-50 text-sm">(' + escapeHtml(data.alias) + ')</span>';
        html += '</div>';

        // Status row
        html += '<div class="detail-section">Status</div>';
        html += detailField('Lifecycle', data.lifecycle_state || 'n/a');
        html += detailField('Uptime', formatUptime(data.uptime_ms));
        html += detailField('Events emitted', data.event_count || 0);
        html += detailField('Escalations', data.escalation_count || 0);
        html += detailField('Knowledge entries', data.knowledge_count || 0);

        // Flags
        html += '<div class="detail-section">Flags</div>';
        html += '<div class="detail-field"><span class="detail-key">Stuck</span>'
          + '<span class="flag-badge ' + (data.stuck ? 'warn' : 'ok') + '">'
          + (data.stuck ? 'YES' : 'no') + '</span></div>';
        html += '<div class="detail-field"><span class="detail-key">Hallucinating</span>'
          + '<span class="flag-badge ' + (data.hallucinating ? 'warn' : 'ok') + '">'
          + (data.hallucinating ? 'YES' : 'no') + '</span></div>';

        // Memory
        if (data.memory && Object.keys(data.memory).length > 0) {
          html += '<div class="detail-section">Memory</div>';
          Object.keys(data.memory).forEach(function (k) {
            var v = data.memory[k];
            // Extract value from ConfidentValue envelope if present
            var display;
            if (v && typeof v === 'object' && 'value' in v) {
              var inner = v.value;
              if (inner && typeof inner === 'object') {
                // Value is tagged: {"Text":"hello"} or {"Number":42}
                var keys = Object.keys(inner);
                display = keys.length === 1 ? String(inner[keys[0]]) : JSON.stringify(inner);
              } else {
                display = String(inner);
              }
            } else {
              display = typeof v === 'object' ? JSON.stringify(v) : String(v);
            }
            if (display.length > 80) display = display.substring(0, 77) + '...';
            html += detailField(k, display);
          });
        }

        // Timers
        if (data.timers && Object.keys(data.timers).length > 0) {
          html += '<div class="detail-section">Timers</div>';
          Object.keys(data.timers).forEach(function (k) {
            html += detailField(k, data.timers[k]);
          });
        }

        detailContent.innerHTML = html;
      })
      .catch(function () {
        detailContent.innerHTML = '<p class="text-sm opacity-40 py-4 text-center">Failed to load agent details</p>';
      });
  }

  function detailField(key, value) {
    return '<div class="detail-field">'
      + '<span class="detail-key">' + escapeHtml(key) + '</span>'
      + '<span class="detail-value">' + escapeHtml(String(value)) + '</span>'
      + '</div>';
  }

  if (detailClose) {
    detailClose.addEventListener('click', function () {
      selectedAgentId = null;
      detailContent.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">Click an agent node to inspect</p>';
      var cards = treeRoot.querySelectorAll('.tree-node-card');
      for (var i = 0; i < cards.length; i++) {
        cards[i].classList.remove('selected');
      }
    });
  }

  // ── Warden Panel ─────────────────────────────────────────────

  function renderWardens(wardens) {
    if (!wardenPanel) return;
    if (!wardens || wardens.length === 0) {
      wardenPanel.innerHTML = '<p class="text-sm opacity-40">No wardens active</p>';
      return;
    }
    var html = '';
    wardens.forEach(function (w) {
      html += '<div class="warden-card">';
      html += '<div class="warden-name">' + escapeHtml(w.name);
      if (w.circuit_breaker_tripped) {
        html += ' <span class="flag-badge warn">circuit open</span>';
      } else {
        html += ' <span class="flag-badge ok">ok</span>';
      }
      html += '</div>';
      html += '<div class="warden-agents">Manages: ' + (w.managed_agents || []).join(', ') + '</div>';
      if (w.degraded_agents && w.degraded_agents.length > 0) {
        html += '<div class="warden-agents" style="color:oklch(0.7 0.15 25)">Degraded: ' + w.degraded_agents.join(', ') + '</div>';
      }
      var retries = w.retry_counts || {};
      var retryKeys = Object.keys(retries);
      if (retryKeys.length > 0) {
        html += '<div class="warden-agents">Retries: '
          + retryKeys.map(function (k) { return k + ':' + retries[k]; }).join(', ')
          + '</div>';
      }
      html += '</div>';
    });
    wardenPanel.innerHTML = html;
  }

  // ── SSE Connection ───────────────────────────────────────────

  function connectSSE() {
    if (!sseStatus) return;
    var source = new EventSource('/__forge/events');

    sseStatus.textContent = 'Connecting...';
    sseStatus.className = 'badge badge-sm connecting';

    source.onopen = function () {
      sseStatus.textContent = 'Live';
      sseStatus.className = 'badge badge-sm connected';
      resetStaleTimer();
    };

    source.onmessage = function (e) {
      try {
        var event = JSON.parse(e.data);
        handleTraceEvent(event);
        resetStaleTimer();
      } catch (err) { /* ignore malformed */ }
    };

    source.onerror = function () {
      sseStatus.textContent = 'Reconnecting...';
      sseStatus.className = 'badge badge-sm disconnected';
    };
  }

  // ── Stale detection ──────────────────────────────────────────

  function resetStaleTimer() {
    lastEventTime = Date.now();
    var staleEl = document.getElementById('event-log-stale');
    if (staleEl) staleEl.style.display = 'none';

    clearTimeout(staleTimer);
    staleTimer = setTimeout(function () {
      var staleEl = document.getElementById('event-log-stale');
      if (!staleEl) {
        staleEl = document.createElement('div');
        staleEl.id = 'event-log-stale';
        staleEl.textContent = 'Waiting for activity...';
        eventLog.appendChild(staleEl);
      }
      staleEl.style.display = 'block';
    }, 10000);
  }

  // ── Event Handler ────────────────────────────────────────────

  var EVENT_LABELS = {
    exec_call:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'exec', detail: d.command || '' }; },
    exec_return:    function (d) { return { cls: 'exec', icon: '\u2713', label: 'exec', detail: 'Done (' + (d.duration_ms / 1000).toFixed(1) + 's)' + (d.success ? '' : ' FAILED') }; },
    llm_request:    function (d) { return { cls: 'llm', icon: '\u25CC', label: 'reason', detail: d.operation + ' (' + d.prompt_len + ' chars)' }; },
    llm_response:   function (d) { return { cls: 'llm', icon: '\u2713', label: 'reason', detail: d.operation + ' \u2192 ' + d.provider + '/' + d.model + ' (' + d.tokens_used + ' tok, ' + (d.duration_ms / 1000).toFixed(1) + 's)' }; },
    task_call:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'task', detail: d.task }; },
    task_return:    function (d) { return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'task', detail: d.task + (d.success ? '' : ' FAILED') }; },
    flow_start:     function (d) { return { cls: 'exec', icon: '\u25B6', label: 'flow', detail: d.flow + ' (' + d.waves + ' waves)' }; },
    flow_complete:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'flow', detail: d.flow + ' complete' }; },
    stage_start:    function (d) { return { cls: 'exec', icon: '\u25CC', label: 'stage', detail: d.stage }; },
    stage_complete: function (d) { return { cls: 'exec', icon: '\u2713', label: 'stage', detail: d.stage }; },
    wave_start:     function (d) { return { cls: 'exec', icon: '\u25B6', label: 'wave', detail: 'Wave ' + d.wave + ': [' + (d.stages || []).join(', ') + ']' }; },
    wave_complete:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'wave', detail: 'Wave ' + d.wave }; },
    pool_send:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'pool', detail: d.pool + ' \u2192 ' + d.workers + ' workers (' + d.strategy + ')' }; },
    pool_resolved:  function (d) { return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'pool', detail: d.pool + ' resolved' }; },
    event_emit:     function (d) { return { cls: 'event', icon: '\u2192', label: 'emit', detail: d.source_agent + ' \u2192 ' + d.event + ' (' + d.subscribers + ' subs)' }; },
    event_delivered: function (d) { return { cls: 'event', icon: '\u2713', label: 'deliver', detail: d.event + ' \u2192 ' + d.target_agent }; },
    ward_action:    function (d) { return { cls: 'warden', icon: '\u26A0', label: 'warden', detail: d.warden + ': ' + d.action + ' ' + d.agent + ' (' + d.failure_type + ')' }; },
    say:            function (d) { return { cls: 'exec', icon: '\u00B7', label: 'say', detail: d.text || '' }; },
    when_dispatch:  function (d) { return { cls: 'exec', icon: d.matched ? '\u2713' : '\u00B7', label: 'when', detail: d.level + (d.matched ? ' matched' : ' skipped') }; },
    skill_call:     function (d) { return { cls: 'exec', icon: '\u25B6', label: 'skill', detail: d.skill }; },
    skill_return:   function (d) { return { cls: 'exec', icon: '\u2713', label: 'skill', detail: d.skill + ' (' + (d.duration_ms / 1000).toFixed(1) + 's)' }; },
    http_request:   function (d) { return { cls: 'exec', icon: '\u25B6', label: 'http', detail: d.method + ' ' + d.path }; },
    http_response:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'http', detail: d.endpoint + ' ' + d.status + ' (' + d.duration_ms + 'ms)' }; },
  };

  function handleTraceEvent(evt) {
    var type = evt.event;
    var data = evt;

    // 1. Update event log
    var labelFn = EVENT_LABELS[type];
    if (labelFn) {
      var info = labelFn(data);
      appendLogEntry(info.cls, info.icon, info.label, info.detail, evt.ts_ms);
    }

    // 2. Tree node effects
    if (type === 'llm_request') {
      setThinking(true);
    } else if (type === 'llm_response') {
      setThinking(false);
    }

    // Flash relevant agent nodes on certain events
    if (type === 'task_call' || type === 'task_return' ||
        type === 'event_emit' || type === 'event_delivered' ||
        type === 'ward_action') {
      flashAgentNodes();
    }

    // Animate wiring on event flow
    if (type === 'event_emit' || type === 'event_delivered') {
      var wiringLabels = treeRoot.querySelectorAll('.wiring-label');
      for (var i = 0; i < wiringLabels.length; i++) {
        wiringLabels[i].classList.add('active');
        setTimeout(function (el) { el.classList.remove('active'); }, 1500, wiringLabels[i]);
      }
    }

    // Auto-refresh and re-enable scan button when scan completes
    if (type === 'flow_complete') {
      finishScan();
    }
    if (type === 'http_response') {
      refreshData();
    }
  }

  function appendLogEntry(cls, icon, label, detail, tsMs) {
    if (eventLogEmpty) {
      eventLogEmpty.style.display = 'none';
    }

    var entry = document.createElement('div');
    entry.className = 'log-entry ' + cls;
    entry.style.animationDelay = '0s';

    var elapsed = relativeTime();
    entry.innerHTML =
      '<span class="step-icon">' + icon + '</span> '
      + '<span class="step-label">' + escapeHtml(label) + '</span> '
      + '<span class="step-detail">' + escapeHtml(detail) + '</span>'
      + '<span class="event-elapsed">' + elapsed + '</span>';

    eventLog.appendChild(entry);

    // Auto-scroll unless user is hovering
    if (!eventLog.matches(':hover')) {
      eventLog.scrollTop = eventLog.scrollHeight;
    }

    // Limit to 200 entries
    var entries = eventLog.querySelectorAll('.log-entry');
    if (entries.length > 200) {
      entries[0].remove();
    }
  }

  function setThinking(active) {
    var cards = treeRoot.querySelectorAll('.tree-node-card[data-node-type="agent"]');
    for (var i = 0; i < cards.length; i++) {
      if (active) {
        cards[i].classList.add('thinking');
      } else {
        cards[i].classList.remove('thinking');
      }
    }
  }

  function flashAgentNodes() {
    var cards = treeRoot.querySelectorAll('.tree-node-card[data-node-type="agent"]');
    for (var i = 0; i < cards.length; i++) {
      cards[i].classList.remove('event-flash');
      // Force reflow
      void cards[i].offsetWidth;
      cards[i].classList.add('event-flash');
    }
  }

  // ── Scan Trigger ─────────────────────────────────────────────

  var scanRunning = false;

  function finishScan() {
    scanRunning = false;
    if (scanTrigger) {
      scanTrigger.classList.remove('loading');
      scanTrigger.textContent = 'Run Scan';
      scanTrigger.disabled = false;
    }
    refreshData();
  }

  if (scanTrigger) {
    scanTrigger.addEventListener('click', function (e) {
      e.preventDefault();
      if (scanRunning) return;
      scanRunning = true;
      scanTrigger.classList.add('loading');
      scanTrigger.textContent = 'Scanning...';
      scanTrigger.disabled = true;

      // Fire-and-forget — SSE flow_complete event will re-enable the button
      fetch('/scan_now').then(function () {
        finishScan();
      }).catch(function () {
        finishScan();
      });
    });
  }

  // ── Data Refresh ─────────────────────────────────────────────

  function refreshData() {
    Promise.all([
      fetchJSON('/__forge/inspect/topology'),
      fetchJSON('/__forge/inspect/agents'),
      fetchJSON('/__forge/inspect/wardens')
    ]).then(function (results) {
      var topology = results[0];
      var agents = results[1];
      var wardens = results[2];
      cachedWardens = wardens;
      buildTree(topology, agents, wardens);
      renderWardens(wardens);
    }).catch(function () {
      // Silently retry on next interval
    });
  }

  function refreshAgentsOnly() {
    fetchJSON('/__forge/inspect/agents')
      .then(function (agents) { updateAgentStates(agents); })
      .catch(function () { /* retry next interval */ });
  }

  // ── Initialize ───────────────────────────────────────────────

  refreshData();                        // Immediate tree build
  connectSSE();                         // Start event stream
  setInterval(refreshAgentsOnly, 5000); // Poll agent state every 5s
  setInterval(refreshData, 30000);      // Full topology refresh every 30s

  // Auto-trigger scan if redirected from dashboard with ?scan=1
  if (window.location.search.indexOf('scan=1') !== -1) {
    // Remove param from URL to prevent re-trigger on refresh
    history.replaceState(null, '', '/observer');
    setTimeout(function () {
      if (scanTrigger) scanTrigger.click();
    }, 1000);
  }

})();
