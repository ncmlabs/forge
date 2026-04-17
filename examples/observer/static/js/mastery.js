/* FORGE Observer — Mastery Tile (#304 T5.3)
 *
 * Renders per-(specialist, project) mastery progression and the
 * `approval_asks_per_task` trend that is the clone-developer track's
 * proof point (#292): "the 10th task needs fewer approval asks than
 * the 1st."
 *
 * Data source: /__forge/inspect/mastery
 *   - mastery: level transition timelines per (specialist, project)
 *   - tasks.tasks_by_project: per-task records with `review_rounds`
 *
 * Live updates: refetch snapshot on SSE `event_emit` whose payload
 * name is `TaskCompleted` or `MasteryUpdated`. We don't depend on
 * payload fields being carried on SSE — refetch is authoritative.
 */

var ForgeMastery = (function () {
  'use strict';

  var SPECIALISTS = ['planner', 'implementer', 'tester', 'reviewer', 'release_manager'];
  var SPECIALIST_COLORS = {
    planner: 'oklch(0.72 0.15 240)',
    implementer: 'oklch(0.70 0.16 160)',
    tester: 'oklch(0.75 0.17 80)',
    reviewer: 'oklch(0.70 0.17 320)',
    release_manager: 'oklch(0.68 0.18 20)'
  };
  var ALL_PROJECTS_KEY = '__all__';

  // ── State ──────────────────────────────────────────────────────
  var state = {
    specialists: SPECIALISTS.slice(),
    projects: [],
    mastery: {},          // key: "{specialist}::{project}" -> tuple snapshot
    tasks_by_project: {}, // key: project -> Array<TaskRecord>
    total_tasks: 0,
    selected_project: ALL_PROJECTS_KEY
  };

  // ── DOM refs ───────────────────────────────────────────────────
  var projectFilter, statusEl;
  var totalTasksEl, projectCountEl, avgAsksEl, topSpecialistEl;
  var scoreChartEl, asksChartEl, tbodyEl;
  var unsubscribeSSE = null;
  var refetchTimer = null;

  // ── Helpers ────────────────────────────────────────────────────

  function cssColorFor(specialist) {
    return SPECIALIST_COLORS[specialist] || 'currentColor';
  }

  function selectedProjects() {
    if (state.selected_project === ALL_PROJECTS_KEY) {
      return state.projects.slice();
    }
    return [state.selected_project];
  }

  function tuplesForSelected() {
    var projects = selectedProjects();
    var tuples = [];
    projects.forEach(function (p) {
      SPECIALISTS.forEach(function (s) {
        var key = s + '::' + p;
        if (state.mastery[key]) tuples.push(state.mastery[key]);
      });
    });
    return tuples;
  }

  function tasksForSelected() {
    var projects = selectedProjects();
    var combined = [];
    projects.forEach(function (p) {
      var arr = state.tasks_by_project[p];
      if (!arr) return;
      arr.forEach(function (t, idx) { combined.push({ task: t, project: p, ordinal: idx + 1 }); });
    });
    return combined;
  }

  // ── Render: summary banner ─────────────────────────────────────

  function renderTotals() {
    if (totalTasksEl) totalTasksEl.textContent = String(state.total_tasks);
    if (projectCountEl) projectCountEl.textContent = String(state.projects.length);

    var tasks = tasksForSelected();
    if (tasks.length === 0) {
      if (avgAsksEl) avgAsksEl.textContent = '—';
    } else {
      var sum = 0;
      tasks.forEach(function (entry) { sum += (entry.task.review_rounds || 0); });
      var avg = sum / tasks.length;
      if (avgAsksEl) avgAsksEl.textContent = avg.toFixed(2);
    }

    // Top specialist within the selected project scope (by current score).
    var tuples = tuplesForSelected();
    if (tuples.length === 0) {
      if (topSpecialistEl) topSpecialistEl.textContent = '—';
    } else {
      var top = tuples.slice().sort(function (a, b) {
        return (b.current_score || 0) - (a.current_score || 0);
      })[0];
      if (topSpecialistEl) {
        topSpecialistEl.textContent = top.specialist + ' (' + (top.current_level || 'novice') + ')';
      }
    }
  }

  // ── Render: summary table ──────────────────────────────────────

  function renderTable() {
    if (!tbodyEl) return;
    var tuples = tuplesForSelected();
    if (tuples.length === 0) {
      tbodyEl.innerHTML = '<tr><td colspan="7" class="text-center opacity-40 py-4">No mastery data yet. Run a dev-cycle task to seed.</td></tr>';
      return;
    }
    tuples.sort(function (a, b) {
      if (a.project !== b.project) return a.project < b.project ? -1 : 1;
      return SPECIALISTS.indexOf(a.specialist) - SPECIALISTS.indexOf(b.specialist);
    });
    var rows = tuples.map(function (t) {
      var level = t.current_level || 'novice';
      var badgeCls = 'badge badge-sm ';
      if (level === 'expert') badgeCls += 'badge-success';
      else if (level === 'journeyman') badgeCls += 'badge-info';
      else if (level === 'apprentice') badgeCls += 'badge-warning';
      else badgeCls += 'badge-ghost';
      return '<tr>' +
        '<td class="font-mono text-sm">' + t.specialist + '</td>' +
        '<td class="font-mono text-sm">' + t.project + '</td>' +
        '<td><span class="' + badgeCls + '">' + level + '</span></td>' +
        '<td class="text-right font-mono">' + (t.current_score || 0).toFixed(1) + '</td>' +
        '<td class="text-right">' + (t.clean_count || 0) + '</td>' +
        '<td class="text-right">' + (t.regress_count || 0) + '</td>' +
        '<td class="text-right">' + (t.total || 0) + '</td>' +
        '</tr>';
    });
    tbodyEl.innerHTML = rows.join('');
  }

  // ── Render: mastery-score line chart ───────────────────────────

  function renderScoreChart() {
    if (!scoreChartEl) return;
    scoreChartEl.innerHTML = '';

    var tuples = tuplesForSelected();
    var series = tuples
      .filter(function (t) { return (t.transitions || []).length > 0; })
      .map(function (t) {
        return {
          label: t.specialist + (state.selected_project === ALL_PROJECTS_KEY ? ' · ' + t.project : ''),
          specialist: t.specialist,
          points: (t.transitions || []).map(function (tr) {
            return { at: new Date(tr.at).getTime(), score: Number(tr.score) || 0, level: tr.level };
          })
        };
      });

    var width = scoreChartEl.clientWidth || 500;
    var height = 240;
    var margin = { top: 10, right: 110, bottom: 24, left: 32 };

    var svg = d3.select(scoreChartEl)
      .append('svg')
      .attr('width', '100%')
      .attr('height', height);

    var plotW = Math.max(50, width - margin.left - margin.right);
    var plotH = Math.max(50, height - margin.top - margin.bottom);
    var g = svg.append('g').attr('transform', 'translate(' + margin.left + ',' + margin.top + ')');

    // Level bands (visual thresholds 40/70/90)
    var bands = [
      { y: 0, h: 40, fill: 'oklch(0.30 0.05 15 / 0.10)' },
      { y: 40, h: 30, fill: 'oklch(0.35 0.05 80 / 0.10)' },
      { y: 70, h: 20, fill: 'oklch(0.40 0.05 220 / 0.10)' },
      { y: 90, h: 10, fill: 'oklch(0.40 0.06 145 / 0.12)' }
    ];
    var yScale = function (v) { return plotH - (v / 100) * plotH; };
    bands.forEach(function (b) {
      g.append('rect')
        .attr('x', 0).attr('y', yScale(b.y + b.h))
        .attr('width', plotW).attr('height', plotH - yScale(b.h))
        .attr('fill', b.fill);
    });

    // Y-axis (0..100)
    [0, 40, 70, 90, 100].forEach(function (v) {
      g.append('line')
        .attr('x1', 0).attr('x2', plotW)
        .attr('y1', yScale(v)).attr('y2', yScale(v))
        .attr('stroke', 'currentColor').attr('stroke-opacity', v === 0 ? 0.4 : 0.12)
        .attr('stroke-dasharray', v === 0 ? null : '2,3');
      g.append('text')
        .attr('x', -6).attr('y', yScale(v) + 3)
        .attr('text-anchor', 'end')
        .attr('fill', 'currentColor').attr('font-size', '10px').attr('opacity', 0.6)
        .text(String(v));
    });

    if (series.length === 0) {
      g.append('text')
        .attr('x', plotW / 2).attr('y', plotH / 2)
        .attr('text-anchor', 'middle')
        .attr('fill', 'currentColor').attr('opacity', 0.4)
        .text('No mastery transitions yet.');
      return;
    }

    // X domain from earliest to latest transition across selected tuples.
    var allTs = [];
    series.forEach(function (s) { s.points.forEach(function (p) { allTs.push(p.at); }); });
    var minT = Math.min.apply(null, allTs);
    var maxT = Math.max.apply(null, allTs);
    if (minT === maxT) { maxT = minT + 1000; }
    var xScale = function (t) {
      return ((t - minT) / (maxT - minT)) * plotW;
    };

    // Draw each series as a polyline.
    var lineGen = d3.line()
      .x(function (p) { return xScale(p.at); })
      .y(function (p) { return yScale(p.score); })
      .curve(d3.curveMonotoneX);

    series.forEach(function (s, i) {
      g.append('path')
        .datum(s.points)
        .attr('fill', 'none')
        .attr('stroke', cssColorFor(s.specialist))
        .attr('stroke-width', 2)
        .attr('opacity', 0.9)
        .attr('d', lineGen);
      g.selectAll('.mastery-pt-' + i)
        .data(s.points)
        .enter()
        .append('circle')
        .attr('cx', function (p) { return xScale(p.at); })
        .attr('cy', function (p) { return yScale(p.score); })
        .attr('r', 3)
        .attr('fill', cssColorFor(s.specialist))
        .append('title')
        .text(function (p) {
          return s.label + ' → ' + p.level + ' (score ' + p.score.toFixed(1) + ') at ' + new Date(p.at).toLocaleString();
        });
    });

    // Legend
    var legend = svg.append('g').attr('transform', 'translate(' + (margin.left + plotW + 10) + ',' + margin.top + ')');
    series.forEach(function (s, i) {
      var row = legend.append('g').attr('transform', 'translate(0,' + (i * 16) + ')');
      row.append('rect').attr('width', 10).attr('height', 10).attr('fill', cssColorFor(s.specialist));
      row.append('text').attr('x', 14).attr('y', 9)
        .attr('fill', 'currentColor').attr('font-size', '10px')
        .text(s.label);
    });
  }

  // ── Render: approval-asks trend chart ──────────────────────────

  function renderAsksChart() {
    if (!asksChartEl) return;
    asksChartEl.innerHTML = '';

    var width = asksChartEl.clientWidth || 500;
    var height = 240;
    var margin = { top: 10, right: 20, bottom: 28, left: 32 };

    var svg = d3.select(asksChartEl)
      .append('svg')
      .attr('width', '100%')
      .attr('height', height);

    var plotW = Math.max(50, width - margin.left - margin.right);
    var plotH = Math.max(50, height - margin.top - margin.bottom);
    var g = svg.append('g').attr('transform', 'translate(' + margin.left + ',' + margin.top + ')');

    var tasks = tasksForSelected();
    if (tasks.length === 0) {
      g.append('text')
        .attr('x', plotW / 2).attr('y', plotH / 2)
        .attr('text-anchor', 'middle')
        .attr('fill', 'currentColor').attr('opacity', 0.4)
        .text('No completed tasks yet.');
      return;
    }

    // Order tasks by completion time for the trend view.
    tasks.sort(function (a, b) {
      return new Date(a.task.completed_at).getTime() - new Date(b.task.completed_at).getTime();
    });

    var maxY = tasks.reduce(function (m, e) {
      return Math.max(m, e.task.review_rounds || 0);
    }, 1);
    maxY = Math.max(1, maxY);
    var xStep = tasks.length > 1 ? plotW / (tasks.length - 1) : plotW;
    var yScale = function (v) { return plotH - (v / maxY) * plotH; };

    // Y gridlines
    [0, Math.ceil(maxY / 2), maxY].forEach(function (v) {
      g.append('line')
        .attr('x1', 0).attr('x2', plotW)
        .attr('y1', yScale(v)).attr('y2', yScale(v))
        .attr('stroke', 'currentColor').attr('stroke-opacity', v === 0 ? 0.4 : 0.12);
      g.append('text')
        .attr('x', -6).attr('y', yScale(v) + 3)
        .attr('text-anchor', 'end')
        .attr('fill', 'currentColor').attr('font-size', '10px').attr('opacity', 0.6)
        .text(String(v));
    });

    var barW = Math.max(4, Math.min(24, xStep - 4));
    g.selectAll('rect.ask-bar')
      .data(tasks)
      .enter()
      .append('rect')
      .attr('class', 'ask-bar')
      .attr('x', function (_d, i) { return i * xStep - barW / 2 + (xStep / 2); })
      .attr('y', function (d) { return yScale(d.task.review_rounds || 0); })
      .attr('width', barW)
      .attr('height', function (d) { return plotH - yScale(d.task.review_rounds || 0); })
      .attr('fill', function (d) {
        if (d.task.outcome === 'merged') return 'oklch(0.70 0.15 145)';
        if (d.task.outcome === 'rejected') return 'oklch(0.65 0.17 25)';
        return 'oklch(0.65 0.10 60)';
      })
      .attr('opacity', 0.85)
      .append('title')
      .text(function (d) {
        return 'task=' + d.task.task_id + ' · outcome=' + d.task.outcome +
          ' · review_rounds=' + d.task.review_rounds +
          ' · completed=' + new Date(d.task.completed_at).toLocaleString();
      });

    // X-axis: task ordinal labels (1, mid, N)
    var labelIndices = tasks.length <= 10
      ? tasks.map(function (_d, i) { return i; })
      : [0, Math.floor(tasks.length / 2), tasks.length - 1];
    labelIndices.forEach(function (i) {
      g.append('text')
        .attr('x', i * xStep + (xStep / 2))
        .attr('y', plotH + 14)
        .attr('text-anchor', 'middle')
        .attr('fill', 'currentColor').attr('font-size', '10px').attr('opacity', 0.6)
        .text('#' + (i + 1));
    });

    // Trend-line (linear regression over (ordinal, review_rounds))
    if (tasks.length >= 2) {
      var n = tasks.length;
      var sx = 0, sy = 0, sxy = 0, sxx = 0;
      for (var i = 0; i < n; i++) {
        var x = i;
        var y = tasks[i].task.review_rounds || 0;
        sx += x; sy += y; sxy += x * y; sxx += x * x;
      }
      var denom = n * sxx - sx * sx;
      if (denom !== 0) {
        var slope = (n * sxy - sx * sy) / denom;
        var intercept = (sy - slope * sx) / n;
        var x0 = 0, x1 = n - 1;
        var y0 = intercept;
        var y1 = slope * x1 + intercept;
        g.append('line')
          .attr('x1', x0 * xStep + (xStep / 2))
          .attr('y1', yScale(Math.max(0, y0)))
          .attr('x2', x1 * xStep + (xStep / 2))
          .attr('y2', yScale(Math.max(0, y1)))
          .attr('stroke', 'oklch(0.65 0.13 280)')
          .attr('stroke-width', 1.5)
          .attr('stroke-dasharray', '4,4')
          .attr('opacity', 0.8);
      }
    }
  }

  // ── Render orchestration ───────────────────────────────────────

  function renderAll() {
    renderTotals();
    renderTable();
    renderScoreChart();
    renderAsksChart();
  }

  // ── Project filter ─────────────────────────────────────────────

  function syncProjectFilter() {
    if (!projectFilter) return;
    var current = projectFilter.value || ALL_PROJECTS_KEY;
    var options = ['<option value="' + ALL_PROJECTS_KEY + '">All projects</option>'];
    state.projects.slice().sort().forEach(function (p) {
      options.push('<option value="' + p + '">' + p + '</option>');
    });
    projectFilter.innerHTML = options.join('');
    if (state.projects.indexOf(current) !== -1 || current === ALL_PROJECTS_KEY) {
      projectFilter.value = current;
    } else {
      projectFilter.value = ALL_PROJECTS_KEY;
    }
    state.selected_project = projectFilter.value;
  }

  // ── Data fetch ─────────────────────────────────────────────────

  function loadSnapshot() {
    return ForgeAPI.fetchJSON('/__forge/inspect/mastery')
      .then(function (data) {
        state.specialists = Array.isArray(data.specialists) && data.specialists.length > 0
          ? data.specialists : SPECIALISTS.slice();
        state.projects = Array.isArray(data.projects) ? data.projects : [];
        state.mastery = (data.mastery && typeof data.mastery === 'object') ? data.mastery : {};
        var tasksPayload = data.tasks || {};
        state.tasks_by_project = tasksPayload.tasks_by_project || {};
        state.total_tasks = Number(tasksPayload.total_tasks || 0);
        syncProjectFilter();
        if (statusEl) {
          statusEl.textContent = state.total_tasks > 0
            ? 'Live · ' + state.total_tasks + ' tasks'
            : 'Streaming (no tasks yet)';
        }
        renderAll();
      })
      .catch(function (err) {
        if (statusEl) statusEl.textContent = 'Failed to load mastery snapshot';
        console.warn('[ForgeMastery] snapshot failed:', err);
      });
  }

  function scheduleRefetch() {
    if (refetchTimer) return;
    refetchTimer = setTimeout(function () {
      refetchTimer = null;
      loadSnapshot();
    }, 250); // debounce bursts of events
  }

  // ── SSE handler ────────────────────────────────────────────────

  function handleEvent(evt) {
    // The tracer's `event_emit` trace shape has the FORGE event name under
    // `evt.event` (it overrides the trace-type label in tracer.emit's merge).
    // We refetch on TaskCompleted and MasteryUpdated — their full fields
    // aren't carried on SSE, but the REST snapshot is authoritative.
    if (!evt || (evt.event !== 'TaskCompleted' && evt.event !== 'MasteryUpdated')) return;
    scheduleRefetch();
  }

  // ── Public API ─────────────────────────────────────────────────

  function init() {
    var root = document.getElementById('view-mastery');
    if (!root) return;

    projectFilter = document.getElementById('mastery-project-filter');
    statusEl = document.getElementById('mastery-status');
    totalTasksEl = document.getElementById('mastery-total-tasks');
    projectCountEl = document.getElementById('mastery-project-count');
    avgAsksEl = document.getElementById('mastery-avg-asks');
    topSpecialistEl = document.getElementById('mastery-top-specialist');
    scoreChartEl = document.getElementById('mastery-score-chart');
    asksChartEl = document.getElementById('mastery-asks-chart');
    tbodyEl = document.getElementById('mastery-tbody');

    if (projectFilter) {
      projectFilter.addEventListener('change', function () {
        state.selected_project = projectFilter.value;
        renderAll();
      });
    }

    loadSnapshot();

    if (typeof ForgeEvents !== 'undefined' && ForgeEvents.onEvent) {
      unsubscribeSSE = ForgeEvents.onEvent(handleEvent);
    }
  }

  function destroy() {
    if (unsubscribeSSE) { unsubscribeSSE(); unsubscribeSSE = null; }
    if (refetchTimer) { clearTimeout(refetchTimer); refetchTimer = null; }
  }

  function refresh() { return loadSnapshot(); }

  return {
    init: init,
    destroy: destroy,
    refresh: refresh
  };
})();
