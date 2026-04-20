/* FORGE Observer — main app controller */

var ForgeObserver = (function () {
  'use strict';

  // ── DOM refs ───────────────────────────────────────────────

  var urlInput = null;
  var connectBtn = null;
  var disconnectBtn = null;
  var statusBadge = null;
  var themeToggle = null;
  var tabs = [];
  var views = {};

  var STORAGE_KEY = 'forge-observer-server';
  var DEFAULT_URL = 'http://localhost:3001';
  var connecting = false;
  var serverConnected = false; // true once topology fetch succeeds

  // ── Initialization ─────────────────────────────────────────

  function init() {
    urlInput = document.getElementById('server-url');
    connectBtn = document.getElementById('connect-btn');
    disconnectBtn = document.getElementById('disconnect-btn');
    statusBadge = document.getElementById('connection-status');
    themeToggle = document.getElementById('theme-toggle');

    tabs = [
      document.getElementById('tab-tree'),
      document.getElementById('tab-topology'),
      document.getElementById('tab-taskdag'),
      document.getElementById('tab-costs'),
      document.getElementById('tab-mastery'),
      document.getElementById('tab-schedules'),
      document.getElementById('tab-timeline')
    ];

    views = {
      tree: document.getElementById('view-tree'),
      topology: document.getElementById('view-topology'),
      taskdag: document.getElementById('view-taskdag'),
      costs: document.getElementById('view-costs'),
      mastery: document.getElementById('view-mastery'),
      schedules: document.getElementById('view-schedules'),
      timeline: document.getElementById('view-timeline')
    };

    // Restore last server URL
    var saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      urlInput.value = saved;
    }

    // Event listeners
    connectBtn.addEventListener('click', handleConnect);
    disconnectBtn.addEventListener('click', handleDisconnect);
    themeToggle.addEventListener('click', toggleTheme);

    tabs.forEach(function (tab) {
      tab.addEventListener('click', function () {
        activateTab(tab.getAttribute('data-view'));
      });
    });

    // Status updates from API layer — only update badge if not already connected
    ForgeAPI.onStatus(function (status) {
      // Once server is confirmed reachable, don't let SSE reconnection downgrade the badge
      if (serverConnected && (status === 'reconnecting' || status === 'connecting')) return;
      updateStatus(status);
    });

    // Check URL params
    var params = new URLSearchParams(window.location.search);
    var serverParam = params.get('server');
    if (serverParam) {
      urlInput.value = serverParam;
      handleConnect();
    }

    // Restore tab from hash
    var hash = window.location.hash.replace('#', '');
    if (hash && views[hash]) {
      activateTab(hash);
    }

    // Listen for hash changes
    window.addEventListener('hashchange', function () {
      var h = window.location.hash.replace('#', '');
      if (h && views[h]) {
        activateTab(h);
      }
    });
  }

  // ── Connection ─────────────────────────────────────────────

  function handleConnect() {
    if (connecting) return;

    var url = urlInput.value.trim();
    if (!url) {
      url = DEFAULT_URL;
      urlInput.value = url;
    }

    connecting = true;
    connectBtn.disabled = true;
    connectBtn.textContent = 'Connecting...';
    updateStatus('connecting');

    ForgeAPI.setServer(url);
    localStorage.setItem(STORAGE_KEY, url);

    // Verify server is reachable by fetching topology
    ForgeAPI.fetchJSON('/__forge/inspect/topology').then(function (topology) {
      connecting = false;
      serverConnected = true;
      connectBtn.classList.add('hidden');
      disconnectBtn.classList.remove('hidden');
      urlInput.disabled = true;
      updateStatus('connected');

      // Initialize all view modules
      initViews(topology);

    }).catch(function (err) {
      connecting = false;
      connectBtn.disabled = false;
      connectBtn.textContent = 'Connect';
      updateStatus('disconnected');
      console.error('Connection failed:', err.message);
    });
  }

  function handleDisconnect() {
    ForgeAPI.disconnectSSE();
    connecting = false;
    serverConnected = false;
    connectBtn.classList.remove('hidden');
    connectBtn.disabled = false;
    connectBtn.textContent = 'Connect';
    disconnectBtn.classList.add('hidden');
    urlInput.disabled = false;
    updateStatus('disconnected');

    clearViews();
  }

  function initViews(topology) {
    // Initialize the event stream (shared SSE connection)
    if (typeof ForgeEvents !== 'undefined') {
      var logEl = document.getElementById('event-log');
      var emptyEl = document.getElementById('event-log-empty');
      ForgeEvents.init(logEl, emptyEl);
      ForgeEvents.start();
    }

    // Initialize tree view
    if (typeof ForgeTree !== 'undefined' && ForgeTree.init) {
      ForgeTree.init(topology);
    }

    // Initialize topology view (lazy — renders on tab switch)
    if (typeof ForgeTopology !== 'undefined' && ForgeTopology.init) {
      ForgeTopology.init();
    }

    // Initialize Task DAG view (#299 T4.1)
    if (typeof ForgeTaskDag !== 'undefined' && ForgeTaskDag.init) {
      ForgeTaskDag.init();
    }

    // Initialize costs view
    if (typeof ForgeCosts !== 'undefined' && ForgeCosts.init) {
      ForgeCosts.init();
    }

    // Initialize mastery view (#304 T5.3)
    if (typeof ForgeMastery !== 'undefined' && ForgeMastery.init) {
      ForgeMastery.init();
    }

    // Initialize schedules view (issue #336)
    if (typeof ForgeSchedules !== 'undefined' && ForgeSchedules.init) {
      ForgeSchedules.init();
      ForgeSchedules.start();
    }

    // Initialize timeline view
    if (typeof ForgeTimeline !== 'undefined' && ForgeTimeline.init) {
      ForgeTimeline.init();
    }
  }

  function clearViews() {
    if (typeof ForgeEvents !== 'undefined' && ForgeEvents.stop) {
      ForgeEvents.stop();
    }
    if (typeof ForgeTree !== 'undefined' && ForgeTree.destroy) {
      ForgeTree.destroy();
    }
    if (typeof ForgeTopology !== 'undefined' && ForgeTopology.destroy) {
      ForgeTopology.destroy();
    }
    if (typeof ForgeTaskDag !== 'undefined' && ForgeTaskDag.destroy) {
      ForgeTaskDag.destroy();
    }
    if (typeof ForgeCosts !== 'undefined' && ForgeCosts.destroy) {
      ForgeCosts.destroy();
    }
    if (typeof ForgeMastery !== 'undefined' && ForgeMastery.destroy) {
      ForgeMastery.destroy();
    }
    if (typeof ForgeSchedules !== 'undefined' && ForgeSchedules.stop) {
      ForgeSchedules.stop();
    }
    if (typeof ForgeTimeline !== 'undefined' && ForgeTimeline.destroy) {
      ForgeTimeline.destroy();
    }
  }

  // ── Tab Routing ────────────────────────────────────────────

  function activateTab(name) {
    // Update tabs
    tabs.forEach(function (tab) {
      if (tab.getAttribute('data-view') === name) {
        tab.classList.add('tab-active');
      } else {
        tab.classList.remove('tab-active');
      }
    });

    // Update views
    Object.keys(views).forEach(function (key) {
      if (key === name) {
        views[key].classList.add('active');
      } else {
        views[key].classList.remove('active');
      }
    });

    // Update URL hash without scrolling
    if (window.location.hash !== '#' + name) {
      history.replaceState(null, '', '#' + name);
    }

    // Trigger resize for D3 views that need dimension recalculation
    if (name === 'topology' && typeof ForgeTopology !== 'undefined' && ForgeTopology.resize) {
      setTimeout(function () { ForgeTopology.resize(); }, 50);
    }
    if (name === 'taskdag' && typeof ForgeTaskDag !== 'undefined' && ForgeTaskDag.resize) {
      setTimeout(function () { ForgeTaskDag.resize(); }, 50);
    }
    if (name === 'timeline' && typeof ForgeTimeline !== 'undefined' && ForgeTimeline.resize) {
      setTimeout(function () { ForgeTimeline.resize(); }, 50);
    }
  }

  // ── Status Badge ───────────────────────────────────────────

  function updateStatus(status) {
    if (!statusBadge) return;

    statusBadge.className = 'badge badge-sm sse-status';

    switch (status) {
      case 'connected':
        statusBadge.classList.add('connected');
        statusBadge.textContent = 'Connected';
        break;
      case 'connecting':
        statusBadge.classList.add('connecting');
        statusBadge.textContent = 'Connecting...';
        break;
      case 'reconnecting':
        statusBadge.classList.add('reconnecting');
        statusBadge.textContent = 'Reconnecting...';
        break;
      case 'disconnected':
      default:
        statusBadge.classList.add('disconnected');
        statusBadge.textContent = 'Disconnected';
        break;
    }
  }

  // ── Theme Toggle ───────────────────────────────────────────

  function toggleTheme() {
    var html = document.documentElement;
    var current = html.getAttribute('data-theme');
    var next = current === 'dark' ? 'light' : 'dark';
    html.setAttribute('data-theme', next);
    localStorage.setItem('forge-observer-theme', next);
  }

  // Restore theme on load
  (function () {
    var saved = localStorage.getItem('forge-observer-theme');
    if (saved) {
      document.documentElement.setAttribute('data-theme', saved);
    }
  })();

  // ── Public API ─────────────────────────────────────────────

  return {
    init: init,
    activateTab: activateTab
  };
})();

// Boot on DOM ready
document.addEventListener('DOMContentLoaded', function () {
  ForgeObserver.init();
});
