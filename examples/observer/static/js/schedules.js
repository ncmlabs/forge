/**
 * ForgeSchedules — live schedule introspection panel (issue #336).
 *
 * Fetches `/__forge/inspect/schedules` on tab open, subscribes to SSE to
 * refresh whenever any `schedule_*` or `webhook_received` event arrives, and
 * renders a live `next_run_at - now` countdown per row.
 *
 * Depends on: ForgeAPI (api.js), ForgeEvents (events.js)
 */
var ForgeSchedules = (function () {
  'use strict';

  var listEl = null;
  var metaEl = null;
  var statusEl = null;
  var unsubscribeSSE = null;
  var countdownTimer = null;
  var lastData = null;

  function init() {
    listEl = document.getElementById('schedules-list');
    metaEl = document.getElementById('schedules-meta');
    statusEl = document.getElementById('schedules-status');
  }

  function start() {
    if (!listEl) return;
    refresh();
    if (unsubscribeSSE) unsubscribeSSE();
    unsubscribeSSE = ForgeEvents.onEvent(function (evt) {
      if (!evt || !evt.event) return;
      if (evt.event.indexOf('schedule_') === 0 ||
          evt.event === 'webhook_received' ||
          evt.event.indexOf('correlation_') === 0) {
        refresh();
      }
    });
    if (countdownTimer) clearInterval(countdownTimer);
    countdownTimer = setInterval(tick, 1000);
    if (statusEl) statusEl.textContent = 'Live.';
  }

  function stop() {
    if (unsubscribeSSE) { unsubscribeSSE(); unsubscribeSSE = null; }
    if (countdownTimer) { clearInterval(countdownTimer); countdownTimer = null; }
    if (statusEl) statusEl.textContent = 'Not streaming.';
  }

  function refresh() {
    ForgeAPI.fetchJSON('/__forge/inspect/schedules')
      .then(function (data) {
        lastData = data;
        render();
      })
      .catch(function () {
        if (listEl) {
          listEl.innerHTML = '<div class="text-sm opacity-40 py-4">'
            + 'Failed to load schedules.</div>';
        }
      });
  }

  function fmtCountdown(ms) {
    if (ms === null || ms === undefined) return '—';
    if (ms <= 0) return 'due';
    var sec = Math.floor(ms / 1000);
    if (sec < 60) return sec + 's';
    var min = Math.floor(sec / 60);
    if (min < 60) return min + 'm ' + (sec % 60) + 's';
    var hr = Math.floor(min / 60);
    return hr + 'h ' + (min % 60) + 'm';
  }

  function fmtTimestamp(ms) {
    if (!ms) return '—';
    return new Date(ms).toLocaleTimeString();
  }

  function statusBadge(status) {
    var cls = 'badge-neutral';
    if (status === 'success') cls = 'badge-success';
    else if (status === 'error' || status === 'halted') cls = 'badge-error';
    else if (status === 'skippedbudget' || status === 'skippedconcurrent') cls = 'badge-warning';
    else if (status === 'not_registered') cls = 'badge-ghost';
    return '<span class="badge badge-sm ' + cls + '">' + ForgeAPI.escapeHtml(status) + '</span>';
  }

  function render() {
    if (!listEl || !lastData) return;

    var schedules = lastData.schedules || [];
    if (schedules.length === 0) {
      listEl.innerHTML = '<div class="text-sm opacity-40 py-4">'
        + 'No schedules declared.</div>';
    } else {
      var rows = schedules.map(function (s) {
        var decl = s.declaration || {};
        var when = decl.when || '(no when)';
        var mode = decl.mode || '?';
        var nextIn = s.next_run_at_ms !== null && s.next_run_at_ms !== undefined
          ? fmtCountdown(s.next_run_at_ms - Date.now())
          : '—';
        return '<tr data-next="' + (s.next_run_at_ms || '') + '">'
          + '<td class="font-mono text-sm">' + ForgeAPI.escapeHtml(s.agent) + '.'
            + ForgeAPI.escapeHtml(s.schedule) + '</td>'
          + '<td class="text-xs opacity-70">' + ForgeAPI.escapeHtml(when) + '</td>'
          + '<td class="text-xs">' + ForgeAPI.escapeHtml(mode) + '</td>'
          + '<td class="text-xs font-mono next-in">' + nextIn + '</td>'
          + '<td class="text-xs opacity-70">' + fmtTimestamp(s.last_run_at_ms) + '</td>'
          + '<td>' + statusBadge(String(s.last_status || 'unknown').toLowerCase().replace(/[^a-z_]/g, '')) + '</td>'
          + '<td class="text-xs text-right">' + (s.consecutive_errors || 0) + '</td>'
          + '</tr>';
      }).join('');

      listEl.innerHTML = '<table class="table table-sm"><thead>'
        + '<tr><th>Schedule</th><th>When</th><th>Mode</th><th class="text-right">Next</th>'
        + '<th>Last run</th><th>Status</th><th class="text-right">Errors</th></tr>'
        + '</thead><tbody>' + rows + '</tbody></table>';
    }

    if (metaEl) {
      var hooks = (lastData.webhooks || []).map(function (w) {
        var sig = w.signed
          ? '<span class="badge badge-xs badge-success">HMAC</span>'
          : '<span class="badge badge-xs badge-ghost">unsigned</span>';
        return '<tr><td class="font-mono text-sm">/webhook/' + ForgeAPI.escapeHtml(w.endpoint)
          + '</td><td>' + sig + '</td></tr>';
      }).join('');
      var corrLive = (lastData.correlations_live || []).map(function (c) {
        return '<tr><td class="font-mono text-sm">' + ForgeAPI.escapeHtml(c.agent) + '.'
          + ForgeAPI.escapeHtml(c.field) + '</td><td class="text-xs text-right">'
          + c.value_count + '</td></tr>';
      }).join('');
      var corrDecl = (lastData.correlations_declared || []).map(function (c) {
        return '<tr><td class="font-mono text-sm">' + ForgeAPI.escapeHtml(c.agent) + '</td>'
          + '<td class="text-xs">' + ForgeAPI.escapeHtml(c.event_type) + '.'
          + ForgeAPI.escapeHtml(c.field) + '</td>'
          + '<td class="text-xs">' + ForgeAPI.escapeHtml(c.mode || '?') + '</td></tr>';
      }).join('');

      metaEl.innerHTML = ''
        + '<div class="text-xs uppercase opacity-50 font-semibold mb-1">Webhooks</div>'
        + (hooks
            ? '<table class="table table-xs mb-3"><tbody>' + hooks + '</tbody></table>'
            : '<div class="text-xs opacity-40 mb-3">No endpoints registered.</div>')
        + '<div class="text-xs uppercase opacity-50 font-semibold mb-1">Declared correlations</div>'
        + (corrDecl
            ? '<table class="table table-xs mb-3"><thead><tr><th>Agent</th><th>Event.field</th><th>Mode</th></tr></thead><tbody>' + corrDecl + '</tbody></table>'
            : '<div class="text-xs opacity-40 mb-3">None.</div>')
        + '<div class="text-xs uppercase opacity-50 font-semibold mb-1">Live correlation keys</div>'
        + (corrLive
            ? '<table class="table table-xs"><thead><tr><th>Agent.field</th><th class="text-right">Values</th></tr></thead><tbody>' + corrLive + '</tbody></table>'
            : '<div class="text-xs opacity-40">None yet.</div>');
    }
  }

  function tick() {
    if (!listEl) return;
    var rows = listEl.querySelectorAll('tr[data-next]');
    var now = Date.now();
    rows.forEach(function (r) {
      var next = parseInt(r.getAttribute('data-next'), 10);
      if (!next) return;
      var cell = r.querySelector('.next-in');
      if (cell) cell.textContent = fmtCountdown(next - now);
    });
  }

  return {
    init: init,
    start: start,
    stop: stop,
    refresh: refresh
  };
})();
