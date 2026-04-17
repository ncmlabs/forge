/**
 * ForgeTaskDag — Task DAG view (#299 T4.1).
 *
 * The clone-dev mastermind stores its DAG in `memory.task_graph: TaskNode[]`,
 * persisted via `memory persistent` (#57). This tile fetches that memory
 * via `/__forge/inspect/agents/{id}` and renders the current graph.
 * SSE `event_emit` frames for the five graph events act as refresh cues —
 * the memory endpoint is the source of truth, the SSE stream just tells us
 * when to poll again. A 3s fallback poll covers events the tracer dropped
 * under backpressure.
 *
 * Depends on: ForgeAPI, ForgeEvents.
 */
var ForgeTaskDag = (function () {
  'use strict';

  // ── State ─────────────────────────────────────────────────────

  var listEl = null;
  var emptyEl = null;
  var svgEl = null;
  var statusEl = null;
  var unsubscribe = null;
  var fallbackTimer = null;
  var inFlight = false;
  var mastermindId = null;
  var graph = [];

  var POLL_INTERVAL_MS = 3000;
  // We listen for `HandlerCompleted` frames from the mastermind rather than
  // raw `event_emit` cues. HandlerCompleted fires AFTER the handler returns
  // and the agent's memory is persisted — polling on event_emit races the
  // handler and reads stale state. (Race observed in real-time smoke.)
  var REFRESH_HANDLERS = [
    'ClonedevTaskInbound', 'SeedTask', 'TaskBlocked', 'TaskCompleted'
  ];

  var STATUS_COLOR = {
    in_flight: '#fbbf24',       // yellow
    blocked: '#ef4444',         // red
    done: '#22c55e',            // green
    cycle_rejected: '#6b7280'   // grey
  };

  // ── Polling ───────────────────────────────────────────────────

  function findMastermind() {
    return ForgeAPI.fetchJSON('/__forge/inspect/agents').then(function (agents) {
      if (!Array.isArray(agents)) return null;
      var mm = agents.find(function (a) { return a.name === 'mastermind'; });
      return mm ? mm.id : null;
    });
  }

  function refresh() {
    if (inFlight) return;
    inFlight = true;

    var idPromise = mastermindId
      ? Promise.resolve(mastermindId)
      : findMastermind().then(function (id) { mastermindId = id; return id; });

    idPromise
      .then(function (id) {
        if (!id) return null;
        return ForgeAPI.fetchJSON('/__forge/inspect/agents/' + id);
      })
      .then(function (info) {
        inFlight = false;
        graph = extractGraph(info);
        render();
        setStatus(graph.length === 0 ? 'Connected · no tasks yet' : 'Connected · ' + graph.length + ' tasks');
      })
      .catch(function (err) {
        inFlight = false;
        // Reset cached id so we rediscover on next tick (mastermind may have restarted).
        mastermindId = null;
        setStatus('Fetch failed: ' + (err && err.message ? err.message : err));
      });
  }

  function setStatus(text) {
    if (statusEl) statusEl.textContent = text;
  }

  // ── Event plumbing ────────────────────────────────────────────

  function onSse(evt) {
    // HandlerCompleted frames: { event: "HandlerCompleted", agent, handler, ... }
    if (!evt || evt.event !== 'HandlerCompleted') return;
    if (evt.agent !== 'mastermind') return;
    if (REFRESH_HANDLERS.indexOf(evt.handler) === -1) return;
    refresh();
  }

  // ── Rendering ────────────────────────────────────────────────

  function render() {
    if (!listEl) return;

    if (!graph || graph.length === 0) {
      if (emptyEl) emptyEl.style.display = '';
      listEl.innerHTML = '';
      if (svgEl) svgEl.innerHTML = '';
      return;
    }
    if (emptyEl) emptyEl.style.display = 'none';
    renderTable();
    renderSvg();
  }

  // ConfidentValue-aware unwrap. The inspect endpoint wraps every memory
  // value in `{ confidence, source, value: { TypeTag: rawValue } }`, and
  // custom-type records add an extra `_type`/`_value` layer (see memory.to_json
  // in src/runtime/memory.rs). Arrays come through as `{ value: { Array: [...] }}`,
  // primitives as `{ value: { Text: "s" | Number: 1.0 | Bool: true } }`.
  function unwrap(v) {
    if (v === null || v === undefined) return v;
    if (typeof v !== 'object') return v;
    if ('_value' in v) return unwrap(v._value);
    if ('value' in v && ('confidence' in v || 'source' in v)) {
      return unwrap(v.value);
    }
    if ('Text' in v) return v.Text;
    if ('Number' in v) return v.Number;
    if ('Bool' in v) return v.Bool;
    if ('Array' in v) return v.Array.map(unwrap);
    if ('Record' in v) {
      // Custom types serialize as `Record { _type, _value }` where _value
      // is itself a ConfidentValue wrapping the real record.
      if (v.Record._value !== undefined) return unwrap(v.Record._value);
      var out = {};
      Object.keys(v.Record).forEach(function (k) {
        out[k] = unwrap(v.Record[k]);
      });
      return out;
    }
    return v;
  }

  function extractGraph(info) {
    if (!info || !info.memory || !info.memory.task_graph) return [];
    var tg = unwrap(info.memory.task_graph);
    if (!Array.isArray(tg)) return [];
    return tg.filter(function (n) { return n && typeof n === 'object'; });
  }

  function normalizeNode(raw) {
    // After `unwrap`, raw is a plain JS object with string/array fields.
    return {
      task_id: String(raw.task_id || ''),
      status: String(raw.status || 'in_flight'),
      blocked_on: Array.isArray(raw.blocked_on)
        ? raw.blocked_on.map(String)
        : [],
      specialist: String(raw.specialist || ''),
      project: String(raw.project || '')
    };
  }

  function renderTable() {
    var rows = graph.map(normalizeNode).map(function (node) {
      var color = STATUS_COLOR[node.status] || '#94a3b8';
      var blockers = node.blocked_on.length ? node.blocked_on.join(', ') : '\u2014';
      var specialist = node.specialist || '\u2014';
      return ''
        + '<tr>'
        + '<td class="font-mono">' + ForgeAPI.escapeHtml(node.task_id) + '</td>'
        + '<td><span class="badge" style="background:' + color + ';color:#0f172a">'
        + ForgeAPI.escapeHtml(node.status) + '</span></td>'
        + '<td class="font-mono opacity-80">' + ForgeAPI.escapeHtml(blockers) + '</td>'
        + '<td class="opacity-80">' + ForgeAPI.escapeHtml(specialist) + '</td>'
        + '</tr>';
    });

    listEl.innerHTML = ''
      + '<table class="table table-xs">'
      + '<thead><tr><th>Task</th><th>Status</th><th>Blocked on</th><th>Specialist</th></tr></thead>'
      + '<tbody>' + rows.join('') + '</tbody>'
      + '</table>';
  }

  function renderSvg() {
    if (!svgEl || typeof d3 === 'undefined') return;

    var normalized = graph.map(normalizeNode);
    var ids = new Set(normalized.map(function (n) { return n.task_id; }));

    var nodes = normalized.map(function (n) {
      return { id: n.task_id, status: n.status, specialist: n.specialist };
    });
    var links = [];
    normalized.forEach(function (node) {
      node.blocked_on.forEach(function (blocker) {
        if (ids.has(blocker)) {
          links.push({ source: blocker, target: node.task_id });
        }
      });
    });

    var width = svgEl.clientWidth || 600;
    var height = svgEl.clientHeight || 260;

    var svg = d3.select(svgEl);
    svg.selectAll('*').remove();

    svg.append('defs').append('marker')
      .attr('id', 'taskdag-arrow')
      .attr('viewBox', '0 -5 10 10')
      .attr('refX', 22)
      .attr('refY', 0)
      .attr('markerWidth', 6)
      .attr('markerHeight', 6)
      .attr('orient', 'auto')
      .append('path')
      .attr('d', 'M0,-5L10,0L0,5')
      .attr('fill', '#94a3b8');

    var sim = d3.forceSimulation(nodes)
      .force('link', d3.forceLink(links).id(function (d) { return d.id; }).distance(80))
      .force('charge', d3.forceManyBody().strength(-200))
      .force('center', d3.forceCenter(width / 2, height / 2))
      .force('collide', d3.forceCollide(28));

    var link = svg.append('g').attr('stroke', '#94a3b8').attr('stroke-width', 1.5)
      .selectAll('line').data(links).enter().append('line')
      .attr('marker-end', 'url(#taskdag-arrow)');

    var node = svg.append('g')
      .selectAll('g').data(nodes).enter().append('g');

    node.append('circle')
      .attr('r', 20)
      .attr('fill', function (d) { return STATUS_COLOR[d.status] || '#94a3b8'; })
      .attr('stroke', '#0f172a')
      .attr('stroke-width', 1.5);

    node.append('text')
      .attr('text-anchor', 'middle')
      .attr('dy', 4)
      .attr('font-size', '10px')
      .attr('fill', '#0f172a')
      .attr('font-weight', '600')
      .text(function (d) { return d.id; });

    node.append('title').text(function (d) {
      return d.id + ' [' + d.status + ']' + (d.specialist ? ' ' + d.specialist : '');
    });

    sim.on('tick', function () {
      link
        .attr('x1', function (d) { return d.source.x; })
        .attr('y1', function (d) { return d.source.y; })
        .attr('x2', function (d) { return d.target.x; })
        .attr('y2', function (d) { return d.target.y; });
      node.attr('transform', function (d) { return 'translate(' + d.x + ',' + d.y + ')'; });
    });
  }

  // ── Public API ────────────────────────────────────────────────

  function init() {
    listEl = document.getElementById('taskdag-list');
    emptyEl = document.getElementById('taskdag-empty');
    svgEl = document.getElementById('taskdag-svg');
    statusEl = document.getElementById('taskdag-status');
    graph = [];
    mastermindId = null;

    if (typeof ForgeEvents !== 'undefined' && ForgeEvents.onEvent) {
      if (unsubscribe) unsubscribe();
      unsubscribe = ForgeEvents.onEvent(onSse);
    }

    if (fallbackTimer) clearInterval(fallbackTimer);
    fallbackTimer = setInterval(refresh, POLL_INTERVAL_MS);

    refresh();
    render();
  }

  function destroy() {
    if (unsubscribe) { unsubscribe(); unsubscribe = null; }
    if (fallbackTimer) { clearInterval(fallbackTimer); fallbackTimer = null; }
    graph = [];
    mastermindId = null;
    if (listEl) listEl.innerHTML = '';
    if (svgEl) svgEl.innerHTML = '';
    if (emptyEl) emptyEl.style.display = '';
    setStatus('Disconnected');
  }

  function resize() {
    renderSvg();
  }

  return {
    init: init,
    destroy: destroy,
    resize: resize,
    refresh: refresh
  };
})();
