// FORGE Sentinel — client-side interactivity

// Dark mode toggle
function toggleTheme() {
  var html = document.documentElement;
  var current = html.getAttribute('data-theme');
  var next = current === 'light' ? 'dark' : 'light';
  html.setAttribute('data-theme', next);
  localStorage.setItem('forge-sentinel-theme', next);
}

// Restore saved theme
(function () {
  var saved = localStorage.getItem('forge-sentinel-theme');
  if (saved) {
    document.documentElement.setAttribute('data-theme', saved);
  }
})();

// ── Auto-refresh ───────────────────────────────────────────────
// Poll /api_health every 30s and update the badge if present.

var REFRESH_INTERVAL = 30000;

function refreshHealth() {
  var badge = document.getElementById('health-badge');
  if (!badge) return;

  fetch('/api_health')
    .then(function (r) { return r.text(); })
    .then(function (text) {
      var trimmed = text.trim().toLowerCase();
      badge.textContent = trimmed;
      badge.className = 'badge badge-lg font-bold health-' + trimmed;
    })
    .catch(function () { /* silent — next poll will retry */ });
}

setInterval(refreshHealth, REFRESH_INTERVAL);

// ── FORGE Activity Log ─────────────────────────────────────────

function createActivityLog(steps) {
  var log = document.createElement('div');
  log.className = 'forge-activity';
  log.innerHTML = '<div class="log-title">FORGE Runtime</div>';

  var startTime = Date.now();
  var entries = [];

  steps.forEach(function (step, i) {
    var entry = document.createElement('div');
    entry.className = 'log-entry';
    entry.style.animationDelay = (i * 0.6) + 's';
    entry.innerHTML =
      '<span class="step-icon">○</span> ' +
      '<span class="step-label">' + step.label + '</span> ' +
      '<span class="step-detail">' + step.detail + '</span>' +
      '<span class="step-time"></span>';
    log.appendChild(entry);
    entries.push({ el: entry, delay: i * 600 });
  });

  entries.forEach(function (e) {
    setTimeout(function () {
      e.el.classList.add('active');
      e.el.querySelector('.step-icon').textContent = '\u25CC';
    }, e.delay);
  });

  function markDone(index) {
    if (index >= entries.length) return;
    var e = entries[index];
    setTimeout(function () {
      e.el.classList.remove('active');
      e.el.classList.add('done');
      e.el.querySelector('.step-icon').textContent = '\u2713';
      var elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
      e.el.querySelector('.step-time').textContent = elapsed + 's';
      markDone(index + 1);
    }, e.delay + 400);
  }
  markDone(0);

  return log;
}

var SCAN_STEPS = [
  { label: 'exec', detail: '\u2192 gathering git data...' },
  { label: 'exec', detail: '\u2192 measuring code metrics...' },
  { label: 'reason', detail: '\u2192 analyzing patterns...' },
  { label: 'classify', detail: '\u2192 scoring health...' },
  { label: 'pool', detail: '\u2192 consensus vote (3 workers)...' },
  { label: 'data.store', detail: '\u2192 publishing results...' }
];

// ── Scan button interceptor ────────────────────────────────────

document.addEventListener('DOMContentLoaded', function () {
  var scanBtn = document.getElementById('scan-trigger');
  if (scanBtn) {
    scanBtn.addEventListener('click', function (e) {
      e.preventDefault();
      var container = document.getElementById('scan-results');
      if (!container) return;

      container.innerHTML = '';
      container.appendChild(createActivityLog(SCAN_STEPS));
      scanBtn.classList.add('loading');
      scanBtn.disabled = true;

      fetch('/scan_now')
        .then(function (r) { return r.text(); })
        .then(function (html) {
          var parser = new DOMParser();
          var doc = parser.parseFromString(html, 'text/html');
          var newResults = doc.getElementById('scan-results');
          if (newResults) {
            container.innerHTML = newResults.innerHTML;
          } else {
            container.innerHTML = html;
          }
          scanBtn.classList.remove('loading');
          scanBtn.disabled = false;
          refreshHealth();
        })
        .catch(function (err) {
          container.innerHTML = '<div class="alert alert-error">' + escapeHtml(err.message) + '</div>';
          scanBtn.classList.remove('loading');
          scanBtn.disabled = false;
        });
    });
  }
});

function escapeHtml(text) {
  var div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}
