// FORGE Sentinel — Topology: D3.js force-directed agent graph
// Issue #141: live graph with tap-to-inspect

(function () {
  var svg = d3.select('#topo-svg');
  if (svg.empty()) return;

  var container = document.getElementById('topo-container');
  var detailContent = document.getElementById('topo-detail-content');
  var detailClose = document.getElementById('topo-detail-close');
  var eventLog = document.getElementById('topo-event-log');
  var eventLogEmpty = document.getElementById('topo-event-log-empty');
  var sseStatus = document.getElementById('topo-sse-status');
  var scanTrigger = document.getElementById('topo-scan-trigger');

  var width = container.clientWidth;
  var height = container.clientHeight;
  var selectedNodeId = null;
  var simulation = null;
  var nodeData = [];
  var linkData = [];
  var nodeElements, linkElements;
  var pageStart = Date.now();
  var scanRunning = false;
  var staleTimer = null;

  svg.attr('viewBox', [0, 0, width, height]);

  // ── SVG Setup ────────────────────────────────────────────────

  // Arrow marker for directed edges
  svg.append('defs').append('marker')
    .attr('id', 'arrow')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 32)
    .attr('refY', 0)
    .attr('markerWidth', 8)
    .attr('markerHeight', 8)
    .attr('orient', 'auto')
    .append('path')
    .attr('d', 'M0,-4L10,0L0,4')
    .attr('class', 'topo-arrow');

  var linkGroup = svg.append('g').attr('class', 'links');
  var nodeGroup = svg.append('g').attr('class', 'nodes');

  // ── Helpers ──────────────────────────────────────────────────

  function escapeHtml(text) {
    var div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  }

  function formatUptime(ms) {
    if (ms < 1000) return ms + 'ms';
    var s = Math.floor(ms / 1000);
    if (s < 60) return s + 's';
    var m = Math.floor(s / 60);
    s = s % 60;
    return m + 'm ' + s + 's';
  }

  function relativeTime() {
    return ((Date.now() - pageStart) / 1000).toFixed(1) + 's';
  }

  function fetchJSON(url) {
    return fetch(url).then(function (r) { return r.json(); });
  }

  // ── Graph Data Transform ─────────────────────────────────────

  function buildGraphData(topology, agents, wardens) {
    var nodes = [];
    var links = [];
    var nodeIds = {};

    var systemName = topology.system_name || 'system';
    var bindings = topology.bindings || [];
    var wiring = topology.wiring || [];
    var routes = topology.routes || {};

    // Alias lookup: alias -> agent name
    var aliasToName = {};
    var nameToAlias = {};
    bindings.forEach(function (b) {
      aliasToName[b[0]] = b[1];
      nameToAlias[b[1]] = b[0];
    });

    // Agent lookup by name
    var agentByName = {};
    agents.forEach(function (a) { agentByName[a.name] = a; });

    // 1. System node
    var sysId = 'system:' + systemName;
    nodes.push({ id: sysId, type: 'system', label: systemName });
    nodeIds[sysId] = true;

    // 2. Warden nodes
    wardens.forEach(function (w) {
      var wId = 'warden:' + w.name;
      var health = 'ok';
      if (w.circuit_breaker_tripped) health = 'critical';
      else if ((w.degraded_agents || []).length > 0) health = 'degraded';
      else {
        var retries = w.retry_counts || {};
        var totalRetries = 0;
        Object.keys(retries).forEach(function (k) { totalRetries += retries[k]; });
        if (totalRetries > 0) health = 'degraded';
      }
      nodes.push({ id: wId, type: 'warden', label: w.name, health: health, wardenData: w });
      nodeIds[wId] = true;

      // System -> warden edge
      links.push({ source: sysId, target: wId, type: 'supervises' });

      // Warden -> managed agents
      (w.managed_agents || []).forEach(function (agentName) {
        var aId = 'agent:' + agentName;
        if (!nodeIds[aId]) {
          var agent = agentByName[agentName];
          nodes.push({
            id: aId, type: 'agent', label: agentName,
            alias: nameToAlias[agentName] || null,
            agentUuid: agent ? agent.id : null,
            status: agent ? agent.status : 'unknown',
            lifecycleState: agent ? (agent.lifecycle_state || 'idle') : 'idle',
            uptimeMs: agent ? (agent.uptime_ms || 0) : 0
          });
          nodeIds[aId] = true;
        }
        links.push({ source: wId, target: aId, type: 'supervises' });
      });
    });

    // 3. Agents from bindings not yet added (no warden case)
    bindings.forEach(function (b) {
      var agentName = b[1];
      var aId = 'agent:' + agentName;
      if (!nodeIds[aId]) {
        var agent = agentByName[agentName];
        nodes.push({
          id: aId, type: 'agent', label: agentName,
          alias: b[0] !== agentName ? b[0] : null,
          agentUuid: agent ? agent.id : null,
          status: agent ? agent.status : 'unknown',
          lifecycleState: agent ? (agent.lifecycle_state || 'idle') : 'idle',
          uptimeMs: agent ? (agent.uptime_ms || 0) : 0
        });
        nodeIds[aId] = true;
        // Link to system if no warden
        links.push({ source: sysId, target: aId, type: 'supervises' });
      }
    });

    // 4. Wiring edges (alias-based compose chains)
    var wiringSet = {};
    wiring.forEach(function (chain) {
      for (var i = 0; i < chain.length - 1; i++) {
        var fromName = aliasToName[chain[i]] || chain[i];
        var toName = aliasToName[chain[i + 1]] || chain[i + 1];
        var key = fromName + '>>' + toName;
        if (!wiringSet[key]) {
          wiringSet[key] = true;
          links.push({
            source: 'agent:' + fromName,
            target: 'agent:' + toName,
            type: 'wired'
          });
        }
      }
    });

    // 5. Route edges (deduplicate against wiring)
    Object.keys(routes).forEach(function (from) {
      var targets = routes[from];
      if (!Array.isArray(targets)) return;
      targets.forEach(function (to) {
        var key = from + '>>' + to;
        if (!wiringSet[key]) {
          links.push({
            source: 'agent:' + from,
            target: 'agent:' + to,
            type: 'wired'
          });
        }
      });
    });

    return { nodes: nodes, links: links };
  }

  // ── Node Radius ──────────────────────────────────────────────

  function nodeRadius(d) {
    if (d.type === 'system') return 30;
    if (d.type === 'warden') return 24;
    return 20;
  }

  // ── D3 Force Simulation ──────────────────────────────────────

  function createSimulation(nodes, links) {
    return d3.forceSimulation(nodes)
      .force('link', d3.forceLink(links).id(function (d) { return d.id; })
        .distance(function (d) {
          return d.type === 'supervises' ? 120 : 180;
        })
        .strength(0.8))
      .force('charge', d3.forceManyBody().strength(-400).distanceMax(500))
      .force('center', d3.forceCenter(width / 2, height / 2).strength(0.05))
      .force('collide', d3.forceCollide().radius(function (d) {
        return nodeRadius(d) + 15;
      }).strength(0.7))
      .force('y', d3.forceY().y(function (d) {
        if (d.type === 'system') return height * 0.18;
        if (d.type === 'warden') return height * 0.45;
        return height * 0.75;
      }).strength(0.15))
      .alphaDecay(0.02);
  }

  // ── Render Graph ─────────────────────────────────────────────

  function renderGraph(graph) {
    nodeData = graph.nodes;
    linkData = graph.links;

    // Links
    linkElements = linkGroup.selectAll('line').data(linkData, function (d) {
      var s = typeof d.source === 'object' ? d.source.id : d.source;
      var t = typeof d.target === 'object' ? d.target.id : d.target;
      return s + '|' + t;
    });
    linkElements.exit().remove();
    var linkEnter = linkElements.enter().append('line')
      .attr('class', function (d) { return 'topo-link ' + d.type; })
      .attr('data-source', function (d) { return typeof d.source === 'object' ? d.source.id : d.source; })
      .attr('data-target', function (d) { return typeof d.target === 'object' ? d.target.id : d.target; })
      .attr('marker-end', function (d) { return d.type === 'wired' ? 'url(#arrow)' : null; });
    linkElements = linkEnter.merge(linkElements);

    // Nodes
    nodeElements = nodeGroup.selectAll('g.topo-node').data(nodeData, function (d) { return d.id; });
    nodeElements.exit().remove();

    var nodeEnter = nodeElements.enter().append('g')
      .attr('class', 'topo-node')
      .attr('data-node-id', function (d) { return d.id; })
      .call(d3.drag()
        .on('start', dragStarted)
        .on('drag', dragged)
        .on('end', dragEnded))
      .on('click', function (event, d) {
        event.stopPropagation();
        selectNode(d);
      });

    // Pulse ring (behind main circle)
    nodeEnter.append('circle')
      .attr('class', function (d) { return 'pulse-ring ' + d.type; })
      .attr('r', function (d) { return nodeRadius(d); });

    // Main circle
    nodeEnter.append('circle')
      .attr('class', function (d) { return 'node-circle ' + d.type; })
      .attr('r', function (d) { return nodeRadius(d); });

    // Health badge (wardens only)
    nodeEnter.filter(function (d) { return d.type === 'warden'; })
      .append('circle')
      .attr('class', function (d) { return 'health-dot ' + (d.health || 'ok'); })
      .attr('r', 5)
      .attr('cx', function (d) { return nodeRadius(d) - 4; })
      .attr('cy', function (d) { return -(nodeRadius(d) - 4); });

    // Type label above
    nodeEnter.append('text')
      .attr('class', 'node-type-label')
      .attr('dy', function (d) { return -(nodeRadius(d) + 8); })
      .text(function (d) { return d.type; });

    // Name label below
    nodeEnter.append('text')
      .attr('class', 'node-label')
      .attr('dy', function (d) { return nodeRadius(d) + 16; })
      .text(function (d) { return d.alias || d.label; });

    nodeElements = nodeEnter.merge(nodeElements);

    // Update existing node states
    nodeElements.select('.node-circle')
      .attr('class', function (d) { return 'node-circle ' + d.type; });
    nodeElements.select('.health-dot')
      .attr('class', function (d) { return 'health-dot ' + (d.health || 'ok'); });

    // Simulation
    if (simulation) simulation.stop();
    simulation = createSimulation(nodeData, linkData);
    simulation.on('tick', ticked);
  }

  function ticked() {
    linkElements
      .attr('x1', function (d) { return d.source.x; })
      .attr('y1', function (d) { return d.source.y; })
      .attr('x2', function (d) { return d.target.x; })
      .attr('y2', function (d) { return d.target.y; });

    nodeElements.attr('transform', function (d) {
      return 'translate(' + d.x + ',' + d.y + ')';
    });
  }

  // ── Drag Behavior ────────────────────────────────────────────

  function dragStarted(event, d) {
    if (!event.active) simulation.alphaTarget(0.3).restart();
    d.fx = d.x;
    d.fy = d.y;
  }

  function dragged(event, d) {
    d.fx = event.x;
    d.fy = event.y;
  }

  function dragEnded(event, d) {
    if (!event.active) simulation.alphaTarget(0);
    d.fx = null;
    d.fy = null;
  }

  // ── Node Selection & Detail Panel ────────────────────────────

  function selectNode(d) {
    selectedNodeId = d.id;
    nodeElements.classed('selected', false);
    nodeGroup.select('[data-node-id="' + d.id + '"]').classed('selected', true);

    if (d.type === 'agent' && d.agentUuid) {
      showAgentDetail(d.agentUuid, d.label);
    } else if (d.type === 'warden' && d.wardenData) {
      showWardenDetail(d.wardenData);
    } else if (d.type === 'system') {
      showSystemDetail(d.label);
    }
  }

  function showAgentDetail(uuid, name) {
    fetchJSON('/__forge/inspect/agents/' + uuid)
      .then(function (data) {
        var html = '<div class="flex items-center gap-2 mb-3">'
          + '<span class="state-dot ' + (data.status === 'running' ? 'running' : 'idle') + '"></span>'
          + '<span class="font-bold">' + escapeHtml(data.name) + '</span>';
        if (data.alias) html += '<span class="opacity-50 text-sm">(' + escapeHtml(data.alias) + ')</span>';
        html += '</div>';

        html += '<div class="detail-section">Status</div>';
        html += detailField('Lifecycle', data.lifecycle_state || 'n/a');
        html += detailField('Uptime', formatUptime(data.uptime_ms));
        html += detailField('Events emitted', data.event_count || 0);
        html += detailField('Escalations', data.escalation_count || 0);
        html += detailField('Knowledge entries', data.knowledge_count || 0);

        html += '<div class="detail-section">Flags</div>';
        html += '<div class="detail-field"><span class="detail-key">Stuck</span>'
          + '<span class="flag-badge ' + (data.stuck ? 'warn' : 'ok') + '">'
          + (data.stuck ? 'YES' : 'no') + '</span></div>';
        html += '<div class="detail-field"><span class="detail-key">Hallucinating</span>'
          + '<span class="flag-badge ' + (data.hallucinating ? 'warn' : 'ok') + '">'
          + (data.hallucinating ? 'YES' : 'no') + '</span></div>';

        if (data.memory && Object.keys(data.memory).length > 0) {
          html += '<div class="detail-section">Memory</div>';
          Object.keys(data.memory).forEach(function (k) {
            var v = data.memory[k];
            var display;
            if (v && typeof v === 'object' && 'value' in v) {
              var inner = v.value;
              if (inner && typeof inner === 'object') {
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

  function showWardenDetail(w) {
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
  }

  function showSystemDetail(name) {
    var html = '<div class="flex items-center gap-2 mb-3">'
      + '<span class="state-dot running"></span>'
      + '<span class="font-bold">' + escapeHtml(name) + '</span>'
      + '<span class="node-type">system</span></div>';

    html += '<div class="detail-section">Composition</div>';
    html += detailField('Agents', nodeData.filter(function (n) { return n.type === 'agent'; }).length);
    html += detailField('Wardens', nodeData.filter(function (n) { return n.type === 'warden'; }).length);
    html += detailField('Data flow edges', linkData.filter(function (l) { return l.type === 'wired'; }).length);

    detailContent.innerHTML = html;
  }

  function detailField(key, value) {
    return '<div class="detail-field">'
      + '<span class="detail-key">' + escapeHtml(key) + '</span>'
      + '<span class="detail-value">' + escapeHtml(String(value)) + '</span>'
      + '</div>';
  }

  if (detailClose) {
    detailClose.addEventListener('click', function () {
      selectedNodeId = null;
      nodeElements.classed('selected', false);
      detailContent.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">Click a node to inspect</p>';
    });
  }

  // Click canvas to deselect
  svg.on('click', function () {
    selectedNodeId = null;
    nodeElements.classed('selected', false);
    detailContent.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">Click a node to inspect</p>';
  });

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

  function resetStaleTimer() {
    clearTimeout(staleTimer);
    staleTimer = setTimeout(function () {
      // Dim nodes slightly when stale
    }, 10000);
  }

  // ── SSE Event Handlers ───────────────────────────────────────

  var EVENT_LABELS = {
    exec_call:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'exec', detail: d.command || '' }; },
    exec_return:    function (d) { return { cls: 'exec', icon: '\u2713', label: 'exec', detail: 'Done (' + (d.duration_ms / 1000).toFixed(1) + 's)' }; },
    llm_request:    function (d) { return { cls: 'llm', icon: '\u25CC', label: 'reason', detail: d.operation + ' (' + d.prompt_len + ' chars)' }; },
    llm_response:   function (d) { return { cls: 'llm', icon: '\u2713', label: 'reason', detail: d.operation + ' \u2192 ' + (d.tokens_used || '?') + ' tok' }; },
    task_call:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'task', detail: d.task }; },
    task_return:    function (d) { return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'task', detail: d.task }; },
    flow_start:     function (d) { return { cls: 'exec', icon: '\u25B6', label: 'flow', detail: d.flow + ' (' + d.waves + ' waves)' }; },
    flow_complete:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'flow', detail: d.flow + ' complete' }; },
    stage_start:    function (d) { return { cls: 'exec', icon: '\u25CC', label: 'stage', detail: d.stage }; },
    stage_complete: function (d) { return { cls: 'exec', icon: '\u2713', label: 'stage', detail: d.stage }; },
    wave_start:     function (d) { return { cls: 'exec', icon: '\u25B6', label: 'wave', detail: 'Wave ' + d.wave }; },
    wave_complete:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'wave', detail: 'Wave ' + d.wave }; },
    pool_send:      function (d) { return { cls: 'exec', icon: '\u25B6', label: 'pool', detail: d.pool + ' \u2192 ' + d.workers + ' workers' }; },
    pool_resolved:  function (d) { return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'pool', detail: d.pool + ' resolved' }; },
    event_emit:     function (d) { return { cls: 'event', icon: '\u2192', label: 'emit', detail: d.source_agent + ' \u2192 ' + d.event }; },
    event_delivered: function (d) { return { cls: 'event', icon: '\u2713', label: 'deliver', detail: d.event + ' \u2192 ' + d.target_agent }; },
    ward_action:    function (d) { return { cls: 'warden', icon: '\u26A0', label: 'warden', detail: d.warden + ': ' + d.action + ' ' + d.agent }; },
    say:            function (d) { return { cls: 'exec', icon: '\u00B7', label: 'say', detail: d.text || '' }; },
    http_request:   function (d) { return { cls: 'exec', icon: '\u25B6', label: 'http', detail: d.method + ' ' + d.path }; },
    http_response:  function (d) { return { cls: 'exec', icon: '\u2713', label: 'http', detail: d.endpoint + ' ' + d.status }; },
  };

  function handleTraceEvent(evt) {
    var type = evt.event;

    // 1. Append to mini event log
    var labelFn = EVENT_LABELS[type];
    if (labelFn) {
      var info = labelFn(evt);
      appendLogEntry(info.cls, info.icon, info.label, info.detail);
    }

    // 2. Graph visual effects
    if (type === 'llm_request') {
      setThinking(true);
    } else if (type === 'llm_response') {
      setThinking(false);
    }

    if (type === 'event_emit' && evt.source_agent) {
      pulseNode('agent:' + evt.source_agent);
      animateEdgesFrom('agent:' + evt.source_agent);
    }

    if (type === 'event_delivered' && evt.target_agent) {
      pulseNode('agent:' + evt.target_agent);
    }

    if (type === 'ward_action' && evt.warden) {
      pulseNode('warden:' + evt.warden);
    }

    if (type === 'flow_start') {
      pulseNode(nodeData.length > 0 && nodeData[0].type === 'system' ? nodeData[0].id : null);
    }

    if (type === 'flow_complete') {
      finishScan();
      refreshData();
    }

    if (type === 'http_response') {
      refreshData();
    }

    if (type === 'task_call' || type === 'task_return') {
      flashAllAgents();
    }
  }

  // ── Visual Effects ───────────────────────────────────────────

  function pulseNode(nodeId) {
    if (!nodeId) return;
    var node = nodeGroup.select('[data-node-id="' + nodeId + '"]');
    if (node.empty()) return;

    var ring = node.select('.pulse-ring');
    var r = ring.attr('r') || 20;

    // Trigger pulse animation via D3 transition
    ring.interrupt()
      .attr('r', +r)
      .attr('opacity', 0.7)
      .attr('stroke-width', 3)
      .transition()
      .duration(800)
      .ease(d3.easeQuadOut)
      .attr('r', +r * 2)
      .attr('opacity', 0)
      .attr('stroke-width', 0);
  }

  function animateEdgesFrom(sourceId) {
    linkGroup.selectAll('line[data-source="' + sourceId + '"]')
      .classed('edge-active', true)
      .classed('edge-flowing', true);

    setTimeout(function () {
      linkGroup.selectAll('line[data-source="' + sourceId + '"]')
        .classed('edge-active', false)
        .classed('edge-flowing', false);
    }, 1500);
  }

  function setThinking(active) {
    nodeGroup.selectAll('.topo-node').each(function (d) {
      if (d.type === 'agent') {
        d3.select(this).classed('thinking', active);
      }
    });
  }

  function flashAllAgents() {
    nodeGroup.selectAll('.topo-node').each(function (d) {
      if (d.type === 'agent') {
        pulseNode(d.id);
      }
    });
  }

  // ── Mini Event Log ───────────────────────────────────────────

  function appendLogEntry(cls, icon, label, detail) {
    if (eventLogEmpty) eventLogEmpty.style.display = 'none';

    var entry = document.createElement('div');
    entry.className = 'log-entry ' + cls;
    entry.style.animationDelay = '0s';
    entry.innerHTML =
      '<span class="step-icon">' + icon + '</span> '
      + '<span class="step-label">' + escapeHtml(label) + '</span> '
      + '<span class="step-detail">' + escapeHtml(detail) + '</span>'
      + '<span class="event-elapsed">' + relativeTime() + '</span>';

    eventLog.appendChild(entry);

    if (!eventLog.matches(':hover')) {
      eventLog.scrollTop = eventLog.scrollHeight;
    }

    var entries = eventLog.querySelectorAll('.log-entry');
    if (entries.length > 100) entries[0].remove();
  }

  // ── Scan Trigger ─────────────────────────────────────────────

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
      var graph = buildGraphData(results[0], results[1], results[2]);
      mergeGraph(graph);

      // Refresh detail if agent selected
      if (selectedNodeId) {
        var selNode = nodeData.find(function (n) { return n.id === selectedNodeId; });
        if (selNode && selNode.type === 'agent' && selNode.agentUuid) {
          showAgentDetail(selNode.agentUuid, selNode.label);
        }
      }
    }).catch(function () { /* retry next interval */ });
  }

  function mergeGraph(graph) {
    // Preserve positions for existing nodes
    var posMap = {};
    nodeData.forEach(function (n) {
      if (n.x !== undefined) posMap[n.id] = { x: n.x, y: n.y, vx: n.vx, vy: n.vy };
    });

    graph.nodes.forEach(function (n) {
      if (posMap[n.id]) {
        n.x = posMap[n.id].x;
        n.y = posMap[n.id].y;
        n.vx = posMap[n.id].vx;
        n.vy = posMap[n.id].vy;
      }
    });

    renderGraph(graph);

    // Only gently reheat if positions were preserved
    if (Object.keys(posMap).length > 0) {
      simulation.alpha(0.1).restart();
    }
  }

  // ── Resize Handler ───────────────────────────────────────────

  window.addEventListener('resize', function () {
    width = container.clientWidth;
    height = container.clientHeight;
    svg.attr('viewBox', [0, 0, width, height]);
    if (simulation) {
      simulation.force('center', d3.forceCenter(width / 2, height / 2).strength(0.05));
      simulation.force('y', d3.forceY().y(function (d) {
        if (d.type === 'system') return height * 0.18;
        if (d.type === 'warden') return height * 0.45;
        return height * 0.75;
      }).strength(0.15));
      simulation.alpha(0.3).restart();
    }
  });

  // ── Initialize ───────────────────────────────────────────────

  Promise.all([
    fetchJSON('/__forge/inspect/topology'),
    fetchJSON('/__forge/inspect/agents'),
    fetchJSON('/__forge/inspect/wardens')
  ]).then(function (results) {
    var graph = buildGraphData(results[0], results[1], results[2]);
    renderGraph(graph);
    connectSSE();
  }).catch(function (err) {
    container.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">Failed to load topology data</p>';
  });

  // Periodic refresh
  setInterval(refreshData, 5000);

  // Auto-trigger scan if redirected with ?scan=1
  if (window.location.search.indexOf('scan=1') !== -1) {
    history.replaceState(null, '', '/topology');
    setTimeout(function () {
      if (scanTrigger) scanTrigger.click();
    }, 1000);
  }

})();
