/**
 * ForgeTopology — D3 force-directed agent graph for FORGE Observer.
 *
 * Renders system, warden, and agent nodes with supervision and wiring edges.
 * Supports live visual effects via SSE events and periodic data refresh.
 *
 * Depends on: ForgeAPI (api.js), ForgeDetail (detail.js), ForgeEvents (events.js), D3.js v7
 */
var ForgeTopology = (function () {
  'use strict';

  // ── State ──────────────────────────────────────────────────────

  var svg, container, detailPanel;
  var detail = null;
  var width = 0;
  var height = 0;
  var selectedNodeId = null;
  var simulation = null;
  var nodeData = [];
  var linkData = [];
  var nodeElements = null;
  var linkElements = null;
  var linkGroup = null;
  var nodeGroup = null;
  var refreshInterval = null;
  var unsubscribeEvents = null;

  // ── Helpers ────────────────────────────────────────────────────

  function nodeRadius(d) {
    if (d.type === 'system') return 30;
    if (d.type === 'warden') return 24;
    return 20;
  }

  // ── Graph Data Transform ───────────────────────────────────────

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
      if (w.circuit_breaker_tripped) {
        health = 'critical';
      } else if ((w.degraded_agents || []).length > 0) {
        health = 'degraded';
      } else {
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

  // ── D3 Force Simulation ────────────────────────────────────────

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

  // ── Render Graph ───────────────────────────────────────────────

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

  // ── Drag Behavior ──────────────────────────────────────────────

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

  // ── Node Selection & Detail Panel ──────────────────────────────

  function showDetailPanel() {
    if (detailPanel) {
      detailPanel.style.display = 'block';
      // Allow display change to take effect before transform
      requestAnimationFrame(function () {
        detailPanel.style.transform = 'translateX(0)';
      });
    }
  }

  function hideDetailPanel() {
    if (detailPanel) {
      detailPanel.style.transform = 'translateX(100%)';
      // Hide after transition completes
      setTimeout(function () {
        if (detailPanel.style.transform === 'translateX(100%)') {
          detailPanel.style.display = 'none';
        }
      }, 220);
    }
  }

  function selectNode(d) {
    selectedNodeId = d.id;
    nodeElements.classed('selected', false);
    nodeGroup.select('[data-node-id="' + d.id + '"]').classed('selected', true);

    showDetailPanel();

    if (d.type === 'agent' && d.agentUuid) {
      detail.showAgent(d.agentUuid);
    } else if (d.type === 'warden' && d.wardenData) {
      detail.showWarden(d.wardenData);
    } else if (d.type === 'system') {
      detail.showSystem(d.label, nodeData, linkData);
    }
  }

  function deselectNode() {
    selectedNodeId = null;
    if (nodeElements) {
      nodeElements.classed('selected', false);
    }
    if (detail) {
      detail.close();
    }
    hideDetailPanel();
  }

  // ── Visual Effects ─────────────────────────────────────────────

  function pulseNode(nodeId) {
    if (!nodeId || !nodeGroup) return;
    var node = nodeGroup.select('[data-node-id="' + nodeId + '"]');
    if (node.empty()) return;

    var ring = node.select('.pulse-ring');
    var r = ring.attr('r') || 20;

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
    if (!linkGroup) return;
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
    if (!nodeGroup) return;
    nodeGroup.selectAll('.topo-node').each(function (d) {
      if (d.type === 'agent') {
        d3.select(this).classed('thinking', active);
      }
    });
  }

  function flashAllAgents() {
    if (!nodeGroup) return;
    nodeGroup.selectAll('.topo-node').each(function (d) {
      if (d.type === 'agent') {
        pulseNode(d.id);
      }
    });
  }

  // ── SSE Event Handler ──────────────────────────────────────────

  function handleTraceEvent(evt) {
    var type = evt.event;

    // LLM thinking indicator
    if (type === 'llm_request') {
      setThinking(true);
    } else if (type === 'llm_response') {
      setThinking(false);
    }

    // Event emission: pulse source and animate edges
    if (type === 'event_emit' && evt.source_agent) {
      pulseNode('agent:' + evt.source_agent);
      animateEdgesFrom('agent:' + evt.source_agent);
    }

    // Event delivery: pulse target
    if (type === 'event_delivered' && evt.target_agent) {
      pulseNode('agent:' + evt.target_agent);
    }

    // Warden action: pulse warden node
    if (type === 'ward_action' && evt.warden) {
      pulseNode('warden:' + evt.warden);
    }

    // Flow start: pulse system node
    if (type === 'flow_start') {
      var sysNode = nodeData.length > 0 && nodeData[0].type === 'system' ? nodeData[0].id : null;
      pulseNode(sysNode);
    }

    // Flow/HTTP completion: refresh data
    if (type === 'flow_complete' || type === 'http_response') {
      refreshData();
    }

    // Task events: flash all agents
    if (type === 'task_call' || type === 'task_return') {
      flashAllAgents();
    }
  }

  // ── Data Refresh ───────────────────────────────────────────────

  function refreshData() {
    Promise.all([
      ForgeAPI.fetchJSON('/__forge/inspect/topology'),
      ForgeAPI.fetchJSON('/__forge/inspect/agents'),
      ForgeAPI.fetchJSON('/__forge/inspect/wardens')
    ]).then(function (results) {
      var graph = buildGraphData(results[0], results[1], results[2]);
      mergeGraph(graph);

      // Refresh detail panel if an agent is selected
      if (selectedNodeId) {
        var selNode = nodeData.find(function (n) { return n.id === selectedNodeId; });
        if (selNode && selNode.type === 'agent' && selNode.agentUuid) {
          detail.showAgent(selNode.agentUuid);
        }
      }
    }).catch(function () { /* retry next interval */ });
  }

  function mergeGraph(graph) {
    // Preserve positions for existing nodes
    var posMap = {};
    nodeData.forEach(function (n) {
      if (n.x !== undefined) {
        posMap[n.id] = { x: n.x, y: n.y, vx: n.vx, vy: n.vy };
      }
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

  // ── SVG Setup ──────────────────────────────────────────────────

  function setupSVG() {
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

    linkGroup = svg.append('g').attr('class', 'links');
    nodeGroup = svg.append('g').attr('class', 'nodes');

    // Click SVG background to deselect
    svg.on('click', function () {
      deselectNode();
    });
  }

  // ── Public: init ───────────────────────────────────────────────

  function init() {
    svg = d3.select('#topo-svg');
    if (svg.empty()) return;

    container = document.getElementById('topo-container');
    detailPanel = document.getElementById('topo-detail-panel');

    var detailContent = document.getElementById('topo-detail-content');
    var detailClose = document.getElementById('topo-detail-close');

    // Create the detail panel helper
    detail = ForgeDetail.create(detailContent, detailClose);

    // Wire close button to also hide the panel
    if (detailClose) {
      detailClose.addEventListener('click', function () {
        deselectNode();
      });
    }

    // Calculate dimensions
    width = container.clientWidth;
    height = container.clientHeight;
    svg.attr('viewBox', [0, 0, width, height]);

    // Set up SVG elements
    setupSVG();

    // Fetch initial data and render
    Promise.all([
      ForgeAPI.fetchJSON('/__forge/inspect/topology'),
      ForgeAPI.fetchJSON('/__forge/inspect/agents'),
      ForgeAPI.fetchJSON('/__forge/inspect/wardens')
    ]).then(function (results) {
      var graph = buildGraphData(results[0], results[1], results[2]);
      renderGraph(graph);
    }).catch(function () {
      container.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">Failed to load topology data</p>';
    });

    // Connect to SSE events for visual effects
    unsubscribeEvents = ForgeEvents.onEvent(handleTraceEvent);

    // Periodic data refresh
    refreshInterval = setInterval(refreshData, 5000);
  }

  // ── Public: destroy ────────────────────────────────────────────

  function destroy() {
    if (refreshInterval) {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
    if (unsubscribeEvents) {
      unsubscribeEvents();
      unsubscribeEvents = null;
    }
    if (simulation) {
      simulation.stop();
      simulation = null;
    }
    selectedNodeId = null;
    nodeData = [];
    linkData = [];
    nodeElements = null;
    linkElements = null;
    linkGroup = null;
    nodeGroup = null;
    detail = null;
  }

  // ── Public: resize ─────────────────────────────────────────────

  function resize() {
    if (!container || !svg) return;

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
  }

  // ── Public API ─────────────────────────────────────────────────

  return {
    init: init,
    destroy: destroy,
    resize: resize
  };
})();
