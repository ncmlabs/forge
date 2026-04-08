/**
 * ForgeTree — supervision tree panel for FORGE Observer.
 *
 * Renders the agent hierarchy as a collapsible tree with live state updates,
 * thinking indicators, and event flash animations.
 *
 * Adapted from the sentinel app.js observer code.
 *
 * Depends on:
 *   ForgeAPI   (api.js)   — fetchJSON(), escapeHtml(), formatUptime()
 *   ForgeDetail (detail.js) — ForgeDetail.create() -> { showAgent, showWarden, close, getSelectedId }
 *   ForgeEvents (events.js) — ForgeEvents.onEvent(callback)
 *
 * DOM elements (from index.html):
 *   #tree-root      — container for the agent hierarchy tree
 *   #detail-content — detail panel content area
 *   #detail-close   — close button for detail panel
 */
var ForgeTree = (function () {
  'use strict';

  var treeRoot = null;
  var detail = null;       // ForgeDetail instance
  var agentMap = {};       // id -> agent data
  var cachedWardens = [];
  var initialized = false;
  var pollInterval = null;
  var fullRefreshInterval = null;
  var eventUnsub = null;

  // ── Initialization ─────────────────────────────────────────

  function init(topology) {
    if (initialized) return;
    initialized = true;

    treeRoot = document.getElementById('tree-root');
    var detailContent = document.getElementById('detail-content');
    var detailClose = document.getElementById('detail-close');
    detail = ForgeDetail.create(detailContent, detailClose);

    // Expose click handlers on window for onclick attributes in HTML
    window._selectAgent = function (id) {
      // Mark selected in tree
      var cards = treeRoot.querySelectorAll('.tree-node-card');
      for (var i = 0; i < cards.length; i++) {
        cards[i].classList.remove('selected');
      }
      var sel = treeRoot.querySelector('[data-agent-id="' + id + '"]');
      if (sel) sel.classList.add('selected');

      detail.showAgent(id);
    };

    window._selectWarden = function (idx) {
      // Mark selected in tree
      var cards = treeRoot.querySelectorAll('.tree-node-card');
      for (var i = 0; i < cards.length; i++) {
        cards[i].classList.remove('selected');
      }
      var sel = treeRoot.querySelector('[data-warden-idx="' + idx + '"]');
      if (sel) sel.classList.add('selected');

      var w = cachedWardens[idx];
      if (w) detail.showWarden(w);
    };

    // Initial data load
    refreshData();

    // Subscribe to SSE events for visual effects
    eventUnsub = ForgeEvents.onEvent(handleEvent);

    // Polling intervals
    pollInterval = setInterval(refreshAgentsOnly, 5000);
    fullRefreshInterval = setInterval(refreshData, 30000);
  }

  function destroy() {
    if (pollInterval) { clearInterval(pollInterval); pollInterval = null; }
    if (fullRefreshInterval) { clearInterval(fullRefreshInterval); fullRefreshInterval = null; }
    if (eventUnsub) { eventUnsub(); eventUnsub = null; }
    if (treeRoot) treeRoot.innerHTML = '';
    agentMap = {};
    cachedWardens = [];
    initialized = false;

    // Clean up window handlers
    if (window._selectAgent) { delete window._selectAgent; }
    if (window._selectWarden) { delete window._selectWarden; }
  }

  // ── Tree Building ──────────────────────────────────────────

  function buildTree(topology, agents, wardens) {
    var html = '<ul>';
    var systemName = topology.system_name || 'system';
    var bindings = topology.bindings || [];
    var wiring = topology.wiring || [];

    // System root node
    html += '<li><div class="tree-node-card" data-node-type="system">'
      + '<span class="state-dot running"></span>'
      + '<span class="node-type">system</span>'
      + '<span class="node-name">' + ForgeAPI.escapeHtml(systemName) + '</span>'
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
          + '<span class="node-name">' + ForgeAPI.escapeHtml(w.name) + '</span>';
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
    if (!agents) return null;
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
      + ' data-agent-id="' + ForgeAPI.escapeHtml(id) + '"'
      + ' data-agent-name="' + ForgeAPI.escapeHtml(agentName) + '"'
      + ' data-state="' + ForgeAPI.escapeHtml(dotCls) + '"'
      + ' onclick="window._selectAgent(\'' + ForgeAPI.escapeHtml(id) + '\')">'
      + '<span class="state-dot ' + dotCls + '"></span>'
      + '<span class="node-type">agent</span>'
      + '<span class="node-name">' + ForgeAPI.escapeHtml(agentName) + '</span>';
    if (alias && alias !== agentName) {
      html += '<span class="node-alias">(' + ForgeAPI.escapeHtml(alias) + ')</span>';
    }
    if (uptime > 0) {
      html += '<span class="node-uptime">' + ForgeAPI.formatUptime(uptime) + '</span>';
    }
    html += '<span class="thinking-indicator">reasoning...</span>';
    html += '</div></li>';
    return html;
  }

  // ── Agent State Polling ────────────────────────────────────

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
      if (uptimeEl) {
        uptimeEl.textContent = ForgeAPI.formatUptime(a.uptime_ms);
      } else if (a.uptime_ms > 0) {
        // Create uptime span if it didn't exist before
        var span = document.createElement('span');
        span.className = 'node-uptime';
        span.textContent = ForgeAPI.formatUptime(a.uptime_ms);
        // Insert before the thinking indicator
        var thinking = card.querySelector('.thinking-indicator');
        if (thinking) {
          card.insertBefore(span, thinking);
        } else {
          card.appendChild(span);
        }
      }
    });

    // Refresh detail panel if an agent is currently selected
    var selectedId = detail ? detail.getSelectedId() : null;
    if (selectedId) {
      detail.showAgent(selectedId);
    }
  }

  // ── SSE Event Handling (visual effects) ─────────────────────

  function handleEvent(evt) {
    var type = evt.event;

    // Thinking indicator for LLM operations
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

    // Animate wiring labels on event flow
    if (type === 'event_emit' || type === 'event_delivered') {
      animateWiring();
    }

    // Auto-refresh data on HTTP responses and flow completions
    if (type === 'http_response' || type === 'flow_complete') {
      refreshData();
    }
  }

  function setThinking(active) {
    if (!treeRoot) return;
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
    if (!treeRoot) return;
    var cards = treeRoot.querySelectorAll('.tree-node-card[data-node-type="agent"]');
    for (var i = 0; i < cards.length; i++) {
      cards[i].classList.remove('event-flash');
      // Force reflow to restart animation
      void cards[i].offsetWidth;
      cards[i].classList.add('event-flash');
    }
  }

  function animateWiring() {
    if (!treeRoot) return;
    var wiringLabels = treeRoot.querySelectorAll('.wiring-label');
    for (var i = 0; i < wiringLabels.length; i++) {
      wiringLabels[i].classList.add('active');
      setTimeout(function (el) {
        el.classList.remove('active');
      }, 1500, wiringLabels[i]);
    }
  }

  // ── Data Refresh ─────────────────────────────────────────

  function refreshData() {
    Promise.all([
      ForgeAPI.fetchJSON('/__forge/inspect/topology'),
      ForgeAPI.fetchJSON('/__forge/inspect/agents'),
      ForgeAPI.fetchJSON('/__forge/inspect/wardens')
    ]).then(function (results) {
      var topology = results[0];
      var agents = results[1];
      var wardens = results[2];
      cachedWardens = wardens;
      buildTree(topology, agents, wardens);
    }).catch(function () {
      // Silently retry on next interval
    });
  }

  function refreshAgentsOnly() {
    ForgeAPI.fetchJSON('/__forge/inspect/agents')
      .then(function (agents) { updateAgentStates(agents); })
      .catch(function () { /* retry next interval */ });
  }

  // ── Public API ─────────────────────────────────────────────

  return {
    init: init,
    destroy: destroy
  };
})();
