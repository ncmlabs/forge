/**
 * ForgeDetail — shared detail panel module for FORGE Observer.
 *
 * Renders agent, warden, and system details into a container element.
 * Used by both the tree view and topology view.
 *
 * Depends on: ForgeAPI (api.js)
 */
var ForgeDetail = (function () {
  'use strict';

  /**
   * Create a detail panel instance bound to the given DOM elements.
   *
   * @param {HTMLElement} contentEl  — the element to render detail HTML into
   * @param {HTMLElement} [closeBtn] — optional close button to wire up
   * @returns {{ showAgent, showWarden, showSystem, close, getSelectedId }}
   */
  function create(contentEl, closeBtn) {
    var selectedId = null;

    // ── Helpers ──────────────────────────────────────────────────

    function detailField(key, value) {
      return '<div class="detail-field">'
        + '<span class="detail-key">' + ForgeAPI.escapeHtml(key) + '</span>'
        + '<span class="detail-value">' + ForgeAPI.escapeHtml(String(value)) + '</span>'
        + '</div>';
    }

    function flagBadge(flagValue) {
      return '<span class="flag-badge ' + (flagValue ? 'warn' : 'ok') + '">'
        + (flagValue ? 'YES' : 'no') + '</span>';
    }

    function formatMemoryValue(v) {
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
      if (display.length > 80) {
        display = display.substring(0, 77) + '...';
      }
      return display;
    }

    // ── Agent detail ────────────────────────────────────────────

    function showAgent(id) {
      selectedId = id;
      ForgeAPI.fetchJSON('/__forge/inspect/agents/' + id)
        .then(function (data) {
          var html = '';

          // Header
          html += '<div class="flex items-center gap-2 mb-3">'
            + '<span class="state-dot ' + (data.status === 'running' ? 'running' : 'idle') + '"></span>'
            + '<span class="font-bold">' + ForgeAPI.escapeHtml(data.name) + '</span>';
          if (data.alias) {
            html += '<span class="opacity-50 text-sm">(' + ForgeAPI.escapeHtml(data.alias) + ')</span>';
          }
          html += '</div>';

          // Status section
          html += '<div class="detail-section">Status</div>';
          html += detailField('Lifecycle', data.lifecycle_state || 'n/a');
          html += detailField('Uptime', ForgeAPI.formatUptime(data.uptime_ms));
          html += detailField('Events emitted', data.event_count || 0);
          html += detailField('Escalations', data.escalation_count || 0);
          html += detailField('Knowledge entries', data.knowledge_count || 0);

          // Flags section
          html += '<div class="detail-section">Flags</div>';
          html += '<div class="detail-field"><span class="detail-key">Stuck</span>'
            + flagBadge(data.stuck) + '</div>';
          html += '<div class="detail-field"><span class="detail-key">Hallucinating</span>'
            + flagBadge(data.hallucinating) + '</div>';

          // Memory section
          if (data.memory && Object.keys(data.memory).length > 0) {
            html += '<div class="detail-section">Memory</div>';
            Object.keys(data.memory).forEach(function (k) {
              html += detailField(k, formatMemoryValue(data.memory[k]));
            });
          }

          // Timers section
          if (data.timers && Object.keys(data.timers).length > 0) {
            html += '<div class="detail-section">Timers</div>';
            Object.keys(data.timers).forEach(function (k) {
              html += detailField(k, data.timers[k]);
            });
          }

          contentEl.innerHTML = html;
        })
        .catch(function () {
          contentEl.innerHTML = '<p class="text-sm opacity-40 py-4 text-center">'
            + 'Failed to load agent details</p>';
        });
    }

    // ── Warden detail ───────────────────────────────────────────

    function showWarden(w) {
      selectedId = null;
      var dotCls = w.circuit_breaker_tripped ? 'degraded' : 'running';
      var html = '<div class="flex items-center gap-2 mb-3">'
        + '<span class="state-dot ' + dotCls + '"></span>'
        + '<span class="font-bold">' + ForgeAPI.escapeHtml(w.name) + '</span>'
        + '<span class="node-type">warden</span></div>';

      html += '<div class="detail-section">Supervision</div>';
      html += detailField('Managed agents', (w.managed_agents || []).join(', '));
      html += detailField('Degraded agents',
        (w.degraded_agents || []).length > 0
          ? w.degraded_agents.join(', ')
          : 'none');
      html += detailField('Circuit breaker',
        w.circuit_breaker_tripped ? 'TRIPPED' : 'ok');

      // Retry counts
      var retries = w.retry_counts || {};
      var retryKeys = Object.keys(retries);
      if (retryKeys.length > 0) {
        html += '<div class="detail-section">Retries</div>';
        retryKeys.forEach(function (k) {
          html += detailField(k, retries[k]);
        });
      }

      contentEl.innerHTML = html;
    }

    // ── System detail ───────────────────────────────────────────

    function showSystem(name, nodeData, linkData) {
      selectedId = null;
      var html = '<div class="flex items-center gap-2 mb-3">'
        + '<span class="state-dot running"></span>'
        + '<span class="font-bold">' + ForgeAPI.escapeHtml(name) + '</span>'
        + '<span class="node-type">system</span></div>';

      html += '<div class="detail-section">Composition</div>';
      if (nodeData) {
        html += detailField('Agents',
          nodeData.filter(function (n) { return n.type === 'agent'; }).length);
        html += detailField('Wardens',
          nodeData.filter(function (n) { return n.type === 'warden'; }).length);
      }
      if (linkData) {
        html += detailField('Data flow edges',
          linkData.filter(function (l) { return l.type === 'wired'; }).length);
      }

      contentEl.innerHTML = html;
    }

    // ── Close / reset ───────────────────────────────────────────

    function close() {
      selectedId = null;
      contentEl.innerHTML = '<p class="text-sm opacity-40 py-8 text-center">'
        + 'Click a node to inspect</p>';
    }

    function getSelectedId() {
      return selectedId;
    }

    // Wire close button if provided
    if (closeBtn) {
      closeBtn.addEventListener('click', close);
    }

    // Initialize with placeholder
    close();

    return {
      showAgent: showAgent,
      showWarden: showWarden,
      showSystem: showSystem,
      close: close,
      getSelectedId: getSelectedId
    };
  }

  return { create: create };
})();
