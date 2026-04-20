/**
 * ForgeEvents — SSE event stream module for FORGE Observer.
 *
 * Single source of truth for event labels, event handling, and trace buffer.
 * Used by tree.js and topology.js for visual effects and the event log panel.
 *
 * Depends on: ForgeAPI (api.js)
 */
var ForgeEvents = (function () {
  'use strict';

  // ── Constants ───────────────────────────────────────────────

  var MAX_BUFFER_SIZE = 5000;
  var MAX_LOG_ENTRIES = 200;
  var STALE_TIMEOUT_MS = 10000;

  // ── Event label definitions ─────────────────────────────────
  // Each returns { cls, icon, label, detail } for rendering.

  var EVENT_LABELS = {
    exec_call: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'exec', detail: d.command || '' };
    },
    exec_return: function (d) {
      var dur = (d.duration_ms / 1000).toFixed(1) + 's';
      return { cls: 'exec', icon: '\u2713', label: 'exec', detail: 'Done (' + dur + ')' + (d.success ? '' : ' FAILED') };
    },
    llm_request: function (d) {
      return { cls: 'llm', icon: '\u25CC', label: 'reason', detail: d.operation + ' (' + d.prompt_len + ' chars)' };
    },
    llm_response: function (d) {
      var dur = (d.duration_ms / 1000).toFixed(1) + 's';
      return { cls: 'llm', icon: '\u2713', label: 'reason', detail: d.operation + ' \u2192 ' + d.provider + '/' + d.model + ' (' + d.tokens_used + ' tok, ' + dur + ')' };
    },
    task_call: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'task', detail: d.task };
    },
    task_return: function (d) {
      return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'task', detail: d.task + (d.success ? '' : ' FAILED') };
    },
    flow_start: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'flow', detail: d.flow + ' (' + d.waves + ' waves)' };
    },
    flow_complete: function (d) {
      return { cls: 'exec', icon: '\u2713', label: 'flow', detail: d.flow + ' complete' };
    },
    stage_start: function (d) {
      return { cls: 'exec', icon: '\u25CC', label: 'stage', detail: d.stage };
    },
    stage_complete: function (d) {
      return { cls: 'exec', icon: '\u2713', label: 'stage', detail: d.stage };
    },
    wave_start: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'wave', detail: 'Wave ' + d.wave + ': [' + (d.stages || []).join(', ') + ']' };
    },
    wave_complete: function (d) {
      return { cls: 'exec', icon: '\u2713', label: 'wave', detail: 'Wave ' + d.wave };
    },
    pool_send: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'pool', detail: d.pool + ' \u2192 ' + d.workers + ' workers (' + d.strategy + ')' };
    },
    pool_resolved: function (d) {
      return { cls: 'exec', icon: d.success ? '\u2713' : '\u2717', label: 'pool', detail: d.pool + ' resolved' };
    },
    event_emit: function (d) {
      return { cls: 'event', icon: '\u2192', label: 'emit', detail: d.source_agent + ' \u2192 ' + d.event + ' (' + d.subscribers + ' subs)' };
    },
    event_delivered: function (d) {
      return { cls: 'event', icon: '\u2713', label: 'deliver', detail: d.event + ' \u2192 ' + d.target_agent };
    },
    ward_action: function (d) {
      return { cls: 'warden', icon: '\u26A0', label: 'warden', detail: d.warden + ': ' + d.action + ' ' + d.agent + ' (' + d.failure_type + ')' };
    },
    say: function (d) {
      return { cls: 'exec', icon: '\u00B7', label: 'say', detail: d.text || '' };
    },
    when_dispatch: function (d) {
      return { cls: 'exec', icon: d.matched ? '\u2713' : '\u00B7', label: 'when', detail: d.level + (d.matched ? ' matched' : ' skipped') };
    },
    skill_call: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'skill', detail: d.skill };
    },
    skill_return: function (d) {
      var dur = (d.duration_ms / 1000).toFixed(1) + 's';
      return { cls: 'exec', icon: '\u2713', label: 'skill', detail: d.skill + ' (' + dur + ')' };
    },
    http_request: function (d) {
      return { cls: 'exec', icon: '\u25B6', label: 'http', detail: d.method + ' ' + d.path };
    },
    http_response: function (d) {
      return { cls: 'exec', icon: '\u2713', label: 'http', detail: d.endpoint + ' ' + d.status + ' (' + d.duration_ms + 'ms)' };
    },
    // ── Wake surface (issue #336) ─────────────────────────────────
    schedule_fired: function (d) {
      var delta = (d.scheduled_at_ms !== undefined && d.wall_time_ms !== undefined)
        ? ' (\u0394 ' + (d.wall_time_ms - d.scheduled_at_ms) + 'ms)'
        : '';
      return { cls: 'schedule', icon: '\u23F0', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' fired' + delta };
    },
    schedule_skipped_concurrent: function (d) {
      return { cls: 'schedule', icon: '\u23F8', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' skipped (held by ' + (d.held_by || '?') + ')' };
    },
    schedule_skipped_budget: function (d) {
      return { cls: 'schedule', icon: '\uD83D\uDCB0', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' skipped (budget: ' + (d.budget_state || '?') + ')' };
    },
    schedule_errored: function (d) {
      return { cls: 'schedule', icon: '\u274C', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' errored (retry ' + (d.retry_count || 0) + '): ' + (d.error || '') };
    },
    schedule_claim_lost: function (d) {
      return { cls: 'schedule', icon: '\u26A0', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' claim lost to ' + (d.winner || '?') };
    },
    schedule_rehydrated: function (d) {
      var n = (d.memory_keys_restored && d.memory_keys_restored.length) || 0;
      return { cls: 'schedule', icon: '\uD83D\uDCA4', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' rehydrated (' + n + ' memory keys)' };
    },
    session_rehydrate_failed: function (d) {
      return { cls: 'schedule', icon: '\uD83D\uDCA4', label: 'schedule',
        detail: d.agent + '.' + d.schedule + ' rehydrate failed: ' + (d.error || '') };
    },
    webhook_received: function (d) {
      var sig;
      if (d.signature_valid === true) { sig = ' \u2713 signed'; }
      else if (d.signature_valid === false) { sig = ' \u2717 bad sig'; }
      else { sig = ' (unsigned)'; }
      return { cls: 'webhook', icon: '\uD83E\uDE9D', label: 'webhook',
        detail: d.endpoint + sig + ' ' + (d.body_bytes || 0) + 'B' };
    },
    correlation_hit: function (d) {
      return { cls: 'correlate', icon: '\uD83C\uDFAF', label: 'correlate',
        detail: d.event_name + '.' + d.field + ' \u2192 ' + d.target_alias };
    },
    correlation_miss: function (d) {
      return { cls: 'correlate', icon: '\u00B7', label: 'correlate',
        detail: d.event_name + '.' + d.field + ' miss' };
    },
    correlation_registered: function (d) {
      return { cls: 'correlate', icon: '\u2731', label: 'correlate',
        detail: d.agent + '.' + d.field + ' \u2192 ' + d.target_alias };
    }
  };

  // ── State ───────────────────────────────────────────────────

  var eventLogEl = null;
  var eventLogEmptyEl = null;
  var sseCleanup = null;
  var listeners = [];
  var traceBuffer = [];
  var staleTimer = null;
  var pageStart = Date.now();

  // ── Internal helpers ────────────────────────────────────────

  function relativeTime() {
    return ((Date.now() - pageStart) / 1000).toFixed(1) + 's';
  }

  function showStaleMessage() {
    if (!eventLogEl) return;
    // Only show if no entries exist yet or last entry is not already stale
    var existing = eventLogEl.querySelector('.log-entry.stale');
    if (existing) return;

    var entry = document.createElement('div');
    entry.className = 'log-entry stale';
    entry.innerHTML = '<span class="step-icon">\u00B7</span> '
      + '<span class="step-detail opacity-40">Waiting for activity...</span>';
    eventLogEl.appendChild(entry);
  }

  function clearStaleMessage() {
    if (!eventLogEl) return;
    var stale = eventLogEl.querySelectorAll('.log-entry.stale');
    for (var i = 0; i < stale.length; i++) {
      stale[i].remove();
    }
  }

  function resetStaleTimer() {
    if (staleTimer) clearTimeout(staleTimer);
    staleTimer = setTimeout(showStaleMessage, STALE_TIMEOUT_MS);
  }

  function appendLogEntry(cls, icon, label, detail, tsMs) {
    if (!eventLogEl) return;

    // Hide empty placeholder
    if (eventLogEmptyEl) {
      eventLogEmptyEl.style.display = 'none';
    }
    clearStaleMessage();

    var entry = document.createElement('div');
    entry.className = 'log-entry ' + cls;
    entry.style.animationDelay = '0s';

    var elapsed = relativeTime();
    entry.innerHTML =
      '<span class="step-icon">' + icon + '</span> '
      + '<span class="step-label">' + ForgeAPI.escapeHtml(label) + '</span> '
      + '<span class="step-detail">' + ForgeAPI.escapeHtml(detail) + '</span>'
      + '<span class="event-elapsed">' + elapsed + '</span>';

    eventLogEl.appendChild(entry);

    // Auto-scroll unless user is hovering
    if (!eventLogEl.matches(':hover')) {
      eventLogEl.scrollTop = eventLogEl.scrollHeight;
    }

    // Limit visible entries
    var entries = eventLogEl.querySelectorAll('.log-entry');
    if (entries.length > MAX_LOG_ENTRIES) {
      entries[0].remove();
    }
  }

  function handleEvent(evt) {
    var type = evt.event;
    var data = evt;

    // Add to trace buffer
    traceBuffer.push({
      event: type,
      data: data,
      ts: Date.now(),
      ts_ms: evt.ts_ms || Date.now()
    });
    if (traceBuffer.length > MAX_BUFFER_SIZE) {
      traceBuffer.shift();
    }

    // Render into event log
    var labelFn = EVENT_LABELS[type];
    if (labelFn) {
      var info = labelFn(data);
      appendLogEntry(info.cls, info.icon, info.label, info.detail, evt.ts_ms);
    }

    // Reset stale detection
    resetStaleTimer();

    // Notify subscribers
    for (var i = 0; i < listeners.length; i++) {
      try {
        listeners[i](evt);
      } catch (e) {
        console.error('[ForgeEvents] listener error:', e);
      }
    }
  }

  // ── Public API ──────────────────────────────────────────────

  /**
   * Initialize the event module with DOM elements for the event log.
   *
   * @param {HTMLElement} logEl      — scrollable container for log entries
   * @param {HTMLElement} [emptyEl]  — placeholder shown when log is empty
   */
  function init(logEl, emptyEl) {
    eventLogEl = logEl;
    eventLogEmptyEl = emptyEl || null;
    pageStart = Date.now();
    traceBuffer = [];
  }

  /**
   * Connect to the SSE event stream.
   */
  function start() {
    if (sseCleanup) {
      sseCleanup();
      sseCleanup = null;
    }

    resetStaleTimer();

    sseCleanup = ForgeAPI.connectSSE('/__forge/events', function (evt) {
      handleEvent(evt);
    });
  }

  /**
   * Disconnect from the SSE event stream.
   */
  function stop() {
    if (sseCleanup) {
      sseCleanup();
      sseCleanup = null;
    }
    if (staleTimer) {
      clearTimeout(staleTimer);
      staleTimer = null;
    }
  }

  /**
   * Subscribe to incoming events. The callback receives the raw event object.
   * Returns an unsubscribe function.
   *
   * @param {Function} callback
   * @returns {Function} unsubscribe
   */
  function onEvent(callback) {
    listeners.push(callback);
    return function () {
      var idx = listeners.indexOf(callback);
      if (idx !== -1) listeners.splice(idx, 1);
    };
  }

  /**
   * Returns the trace buffer (array of recent events with timestamps).
   *
   * @returns {Array}
   */
  function getBuffer() {
    return traceBuffer;
  }

  /**
   * Clears the trace buffer.
   */
  function clearBuffer() {
    traceBuffer = [];
  }

  /**
   * Returns the page start timestamp (for relative time calculations).
   *
   * @returns {number}
   */
  function getPageStart() {
    return pageStart;
  }

  /**
   * Look up the label function for a given event type.
   *
   * @param {string} type
   * @returns {Function|undefined}
   */
  function getLabelFn(type) {
    return EVENT_LABELS[type];
  }

  return {
    EVENT_LABELS: EVENT_LABELS,
    init: init,
    start: start,
    stop: stop,
    onEvent: onEvent,
    getBuffer: getBuffer,
    clearBuffer: clearBuffer,
    getPageStart: getPageStart,
    getLabelFn: getLabelFn
  };
})();
