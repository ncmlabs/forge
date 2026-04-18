/**
 * ForgeTimeline — trace timeline visualization for FORGE Observer.
 *
 * D3.js swim-lane timeline showing buffered SSE events as colored ticks.
 * Supports brush zoom, event filtering by category, and click-to-inspect.
 *
 * Depends on: ForgeAPI (api.js), ForgeEvents (events.js), D3.js v7
 */
var ForgeTimeline = (function () {
  'use strict';

  // ── Constants ──────────────────────────────────────────────
  var LANE_HEIGHT = 40;
  var TICK_WIDTH = 3;
  var MARGIN = { top: 30, right: 20, bottom: 40, left: 80 };

  var CATEGORIES = [
    { key: 'llm',       label: 'LLM',      color: 'oklch(0.65 0.15 280)', events: ['llm_request', 'llm_response'] },
    { key: 'exec',      label: 'Exec',     color: 'oklch(0.6 0.12 200)',  events: ['exec_call', 'exec_return', 'task_call', 'task_return', 'skill_call', 'skill_return'] },
    { key: 'flow',      label: 'Flow',     color: 'oklch(0.6 0.15 170)',  events: ['flow_start', 'flow_complete', 'stage_start', 'stage_complete', 'wave_start', 'wave_complete', 'pool_send', 'pool_resolved'] },
    { key: 'event',     label: 'Events',   color: 'oklch(0.65 0.15 145)', events: ['event_emit', 'event_delivered'] },
    { key: 'schedule',  label: 'Schedule', color: 'oklch(0.72 0.17 95)',  events: ['schedule_fired', 'schedule_rehydrated', 'schedule_skipped_concurrent', 'schedule_skipped_budget', 'schedule_errored', 'schedule_claim_lost', 'session_rehydrate_failed'] },
    { key: 'warden',    label: 'Warden',   color: 'oklch(0.7 0.15 25)',   events: ['ward_action'] },
    { key: 'http',      label: 'HTTP',     color: 'oklch(0.55 0.1 220)',  events: ['http_request', 'http_response'] }
  ];

  // Build event -> category lookup
  var eventToCategory = {};
  CATEGORIES.forEach(function (cat) {
    cat.events.forEach(function (e) { eventToCategory[e] = cat; });
  });

  // ── State ──────────────────────────────────────────────────
  var containerEl = null;
  var filtersEl = null;
  var svg = null;
  var mainGroup = null;
  var xScale = null;
  var brush = null;
  var brushGroup = null;
  var activeFilters = {};  // category key -> boolean (true = visible)
  var unsubscribeEvents = null;
  var initialized = false;
  var tooltip = null;

  // ── Initialization ─────────────────────────────────────────

  function init() {
    if (initialized) return;
    initialized = true;

    containerEl = document.getElementById('timeline-container');
    filtersEl = document.getElementById('timeline-filters');

    if (!containerEl || !filtersEl) return;

    // Initialize filters (all enabled)
    CATEGORIES.forEach(function (cat) { activeFilters[cat.key] = true; });
    buildFilterControls();

    // Clear container placeholder
    containerEl.innerHTML = '';

    // Create SVG
    var w = containerEl.clientWidth || 800;
    var h = MARGIN.top + (CATEGORIES.length * LANE_HEIGHT) + MARGIN.bottom;
    containerEl.style.height = h + 'px';

    svg = d3.select(containerEl).append('svg')
      .attr('width', w)
      .attr('height', h);

    mainGroup = svg.append('g')
      .attr('transform', 'translate(' + MARGIN.left + ',' + MARGIN.top + ')');

    // Time scale
    var now = Date.now();
    xScale = d3.scaleTime()
      .domain([now - 60000, now])  // default: last 60s
      .range([0, w - MARGIN.left - MARGIN.right]);

    // X axis
    mainGroup.append('g')
      .attr('class', 'x-axis')
      .attr('transform', 'translate(0,' + (CATEGORIES.length * LANE_HEIGHT) + ')')
      .call(d3.axisBottom(xScale).ticks(8).tickFormat(d3.timeFormat('%H:%M:%S')));

    // Swim lane labels
    mainGroup.selectAll('.lane-label')
      .data(CATEGORIES)
      .enter().append('text')
      .attr('class', 'lane-label')
      .attr('x', -10)
      .attr('y', function (d, i) { return i * LANE_HEIGHT + LANE_HEIGHT / 2; })
      .attr('text-anchor', 'end')
      .attr('dominant-baseline', 'central')
      .attr('fill', function (d) { return d.color; })
      .attr('font-size', '11px')
      .attr('font-weight', '600')
      .text(function (d) { return d.label; });

    // Swim lane separators
    mainGroup.selectAll('.lane-separator')
      .data(CATEGORIES)
      .enter().append('line')
      .attr('class', 'lane-separator')
      .attr('x1', 0)
      .attr('x2', w - MARGIN.left - MARGIN.right)
      .attr('y1', function (d, i) { return (i + 1) * LANE_HEIGHT; })
      .attr('y2', function (d, i) { return (i + 1) * LANE_HEIGHT; })
      .attr('stroke', 'currentColor')
      .attr('stroke-opacity', 0.08);

    // Tick group for event marks
    mainGroup.append('g').attr('class', 'ticks');

    // Brush for zoom
    brush = d3.brushX()
      .extent([[0, 0], [w - MARGIN.left - MARGIN.right, CATEGORIES.length * LANE_HEIGHT]])
      .on('end', handleBrush);

    brushGroup = mainGroup.append('g')
      .attr('class', 'brush')
      .call(brush);

    // Tooltip
    tooltip = d3.select(containerEl).append('div')
      .attr('class', 'timeline-tooltip')
      .style('position', 'absolute')
      .style('display', 'none')
      .style('pointer-events', 'none')
      .style('background', 'oklch(var(--b2))')
      .style('border', '1px solid oklch(var(--bc) / 0.15)')
      .style('border-radius', '0.375rem')
      .style('padding', '0.375rem 0.5rem')
      .style('font-size', '0.75rem')
      .style('max-width', '300px')
      .style('z-index', '10');

    // Double-click to reset zoom
    svg.on('dblclick', function () {
      var now = Date.now();
      xScale.domain([now - 60000, now]);
      render();
    });

    // Subscribe to events
    unsubscribeEvents = ForgeEvents.onEvent(handleEvent);

    // Initial render
    render();
  }

  function destroy() {
    if (unsubscribeEvents) { unsubscribeEvents(); unsubscribeEvents = null; }
    if (containerEl) containerEl.innerHTML = '';
    if (filtersEl) {
      // Remove only the filter checkboxes we added
      var labels = filtersEl.querySelectorAll('.timeline-filter');
      for (var i = 0; i < labels.length; i++) { labels[i].remove(); }
    }
    svg = null;
    mainGroup = null;
    xScale = null;
    brush = null;
    tooltip = null;
    initialized = false;
  }

  function resize() {
    if (!svg || !containerEl) return;
    var w = containerEl.clientWidth || 800;
    var h = MARGIN.top + (CATEGORIES.length * LANE_HEIGHT) + MARGIN.bottom;
    svg.attr('width', w).attr('height', h);
    xScale.range([0, w - MARGIN.left - MARGIN.right]);

    // Update separators width
    mainGroup.selectAll('.lane-separator')
      .attr('x2', w - MARGIN.left - MARGIN.right);

    // Update brush extent
    brush.extent([[0, 0], [w - MARGIN.left - MARGIN.right, CATEGORIES.length * LANE_HEIGHT]]);
    brushGroup.call(brush);

    render();
  }

  // ── Filter Controls ────────────────────────────────────────

  function buildFilterControls() {
    CATEGORIES.forEach(function (cat) {
      var label = document.createElement('label');
      label.className = 'timeline-filter flex items-center gap-1 text-xs cursor-pointer';
      var cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = true;
      cb.className = 'checkbox checkbox-xs';
      cb.style.borderColor = cat.color;
      cb.addEventListener('change', function () {
        activeFilters[cat.key] = cb.checked;
        render();
      });
      label.appendChild(cb);
      var span = document.createElement('span');
      span.textContent = cat.label;
      span.style.color = cat.color;
      label.appendChild(span);
      filtersEl.appendChild(label);
    });
  }

  // ── Event Handling ─────────────────────────────────────────

  function handleEvent(evt) {
    // Auto-expand the time domain to include new events
    if (xScale) {
      var domain = xScale.domain();
      var now = Date.now();
      if (now > domain[1].getTime()) {
        // Slide window forward — keep same duration
        var duration = domain[1].getTime() - domain[0].getTime();
        xScale.domain([now - duration, now]);
      }
    }
    render();
  }

  // ── Brush Zoom ─────────────────────────────────────────────

  function handleBrush(event) {
    var selection = event.selection;
    if (!selection) return; // Click, not drag

    var newDomain = [xScale.invert(selection[0]), xScale.invert(selection[1])];
    xScale.domain(newDomain);

    // Clear the brush selection visual
    brushGroup.call(brush.move, null);

    render();
  }

  // ── Rendering ──────────────────────────────────────────────

  function render() {
    if (!mainGroup || !xScale) return;

    var buffer = ForgeEvents.getBuffer();
    var domain = xScale.domain();
    var domainStart = domain[0].getTime();
    var domainEnd = domain[1].getTime();

    // Filter events: within time domain and matching active category filters
    var visible = buffer.filter(function (item) {
      var cat = eventToCategory[item.event];
      if (!cat) return false;
      if (!activeFilters[cat.key]) return false;
      var t = item.ts_ms || item.ts;
      return t >= domainStart && t <= domainEnd;
    });

    // Map to render data
    var tickData = visible.map(function (item) {
      var cat = eventToCategory[item.event];
      var catIdx = CATEGORIES.indexOf(cat);
      return {
        x: xScale(new Date(item.ts_ms || item.ts)),
        y: catIdx * LANE_HEIGHT + 4,
        h: LANE_HEIGHT - 8,
        color: cat.color,
        item: item,
        cat: cat
      };
    });

    // Update x-axis
    mainGroup.select('.x-axis')
      .call(d3.axisBottom(xScale).ticks(8).tickFormat(d3.timeFormat('%H:%M:%S')));

    // Style axis
    mainGroup.selectAll('.x-axis text')
      .attr('fill', 'currentColor')
      .attr('opacity', 0.5)
      .attr('font-size', '10px');
    mainGroup.selectAll('.x-axis line, .x-axis path')
      .attr('stroke', 'currentColor')
      .attr('stroke-opacity', 0.1);

    // Render ticks
    var ticks = mainGroup.select('.ticks').selectAll('.timeline-tick')
      .data(tickData, function (d, i) { return i; });

    ticks.exit().remove();

    var enter = ticks.enter().append('rect')
      .attr('class', 'timeline-tick')
      .attr('rx', 1)
      .attr('cursor', 'pointer')
      .on('mouseover', function (event, d) {
        showTooltip(event, d);
      })
      .on('mouseout', function () {
        if (tooltip) tooltip.style('display', 'none');
      })
      .on('click', function (event, d) {
        showEventDetail(d);
      });

    enter.merge(ticks)
      .attr('x', function (d) { return d.x - TICK_WIDTH / 2; })
      .attr('y', function (d) { return d.y; })
      .attr('width', TICK_WIDTH)
      .attr('height', function (d) { return d.h; })
      .attr('fill', function (d) { return d.color; })
      .attr('opacity', 0.8);
  }

  // ── Tooltip & Detail ───────────────────────────────────────

  function showTooltip(event, d) {
    if (!tooltip) return;
    var labelFn = ForgeEvents.getLabelFn(d.item.event);
    var info = labelFn ? labelFn(d.item.data) : { label: d.item.event, detail: '' };

    var ts = new Date(d.item.ts_ms || d.item.ts);
    var timeStr = ts.toLocaleTimeString();

    tooltip
      .html('<strong>' + ForgeAPI.escapeHtml(info.label) + '</strong> ' +
            '<span style="opacity:0.6">' + ForgeAPI.escapeHtml(info.detail) + '</span>' +
            '<br><span style="opacity:0.4;font-size:0.7em">' + timeStr + '</span>')
      .style('display', 'block')
      .style('left', (event.offsetX + 10) + 'px')
      .style('top', (event.offsetY - 10) + 'px');
  }

  function showEventDetail(d) {
    // Show full event data in the tree detail panel (if visible)
    var detailContent = document.getElementById('detail-content');
    if (!detailContent) return;

    var labelFn = ForgeEvents.getLabelFn(d.item.event);
    var info = labelFn ? labelFn(d.item.data) : { label: d.item.event, detail: '' };
    var ts = new Date(d.item.ts_ms || d.item.ts);

    var html = '<div class="detail-section">Event</div>';
    html += '<div class="detail-field"><span class="detail-key">Type</span>'
      + '<span class="detail-value">' + ForgeAPI.escapeHtml(d.item.event) + '</span></div>';
    html += '<div class="detail-field"><span class="detail-key">Category</span>'
      + '<span class="detail-value" style="color:' + d.cat.color + '">' + ForgeAPI.escapeHtml(d.cat.label) + '</span></div>';
    html += '<div class="detail-field"><span class="detail-key">Time</span>'
      + '<span class="detail-value">' + ts.toLocaleTimeString() + '</span></div>';
    html += '<div class="detail-field"><span class="detail-key">Summary</span>'
      + '<span class="detail-value">' + ForgeAPI.escapeHtml(info.detail) + '</span></div>';

    // Raw event data
    html += '<div class="detail-section">Raw Data</div>';
    var rawKeys = Object.keys(d.item.data).filter(function (k) { return k !== 'event'; });
    rawKeys.forEach(function (k) {
      var val = d.item.data[k];
      var display = typeof val === 'object' ? JSON.stringify(val) : String(val);
      if (display.length > 60) display = display.substring(0, 57) + '...';
      html += '<div class="detail-field"><span class="detail-key">' + ForgeAPI.escapeHtml(k) + '</span>'
        + '<span class="detail-value">' + ForgeAPI.escapeHtml(display) + '</span></div>';
    });

    detailContent.innerHTML = html;
  }

  // ── Public API ─────────────────────────────────────────────

  return {
    init: init,
    destroy: destroy,
    resize: resize
  };
})();
