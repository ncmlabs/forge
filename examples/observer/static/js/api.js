/* FORGE Observer — centralized API client */

var ForgeAPI = (function () {
  'use strict';

  var serverUrl = '';
  var currentSSE = null;
  var statusCallbacks = [];

  // ── Server URL ──────────────────────────────────────────────

  function setServer(url) {
    serverUrl = url.replace(/\/$/, '');
  }

  function getServer() {
    return serverUrl;
  }

  function isConnected() {
    return !!currentSSE;
  }

  // ── HTTP helpers ────────────────────────────────────────────

  function fetchJSON(path) {
    return fetch(serverUrl + path).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    });
  }

  function postJSON(path, body) {
    return fetch(serverUrl + path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body || {})
    }).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    });
  }

  // ── SSE connection ─────────────────────────────────────────

  function connectSSE(path, onMessage) {
    disconnectSSE();
    var source = new EventSource(serverUrl + path);
    currentSSE = source;

    source.onopen = function () {
      notifyStatus('connected');
    };

    source.onmessage = function (e) {
      try {
        onMessage(JSON.parse(e.data));
      } catch (err) {
        // ignore malformed messages
      }
    };

    source.onerror = function () {
      notifyStatus('reconnecting');
    };

    return source;
  }

  function disconnectSSE() {
    if (currentSSE) {
      currentSSE.close();
      currentSSE = null;
    }
  }

  // ── Status callbacks ───────────────────────────────────────

  function onStatus(cb) {
    statusCallbacks.push(cb);
  }

  function notifyStatus(status) {
    statusCallbacks.forEach(function (cb) {
      cb(status);
    });
  }

  // ── Shared helpers ─────────────────────────────────────────

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
    if (m < 60) return m + 'm ' + s + 's';
    var h = Math.floor(m / 60);
    m = m % 60;
    return h + 'h ' + m + 'm';
  }

  function formatBytes(b) {
    if (b < 1024) return b + ' B';
    if (b < 1048576) return (b / 1024).toFixed(1) + ' KB';
    return (b / 1048576).toFixed(1) + ' MB';
  }

  function formatNumber(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
    return String(n);
  }

  function formatCost(usd) {
    if (usd < 0.01) return '$' + usd.toFixed(4);
    if (usd < 1) return '$' + usd.toFixed(3);
    return '$' + usd.toFixed(2);
  }

  // ── Public API ─────────────────────────────────────────────

  return {
    setServer: setServer,
    getServer: getServer,
    isConnected: isConnected,
    fetchJSON: fetchJSON,
    postJSON: postJSON,
    connectSSE: connectSSE,
    disconnectSSE: disconnectSSE,
    onStatus: onStatus,
    escapeHtml: escapeHtml,
    formatUptime: formatUptime,
    formatBytes: formatBytes,
    formatNumber: formatNumber,
    formatCost: formatCost
  };
})();
