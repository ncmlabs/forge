// FORGE Wiki — client-side interactivity

// Dark mode toggle
function toggleTheme() {
  var html = document.documentElement;
  var current = html.getAttribute('data-theme');
  var next = current === 'light' ? 'dark' : 'light';
  html.setAttribute('data-theme', next);
  localStorage.setItem('forge-wiki-theme', next);
}

// Restore saved theme
(function () {
  var saved = localStorage.getItem('forge-wiki-theme');
  if (saved) {
    document.documentElement.setAttribute('data-theme', saved);
  }
})();

// ── FORGE Activity Log ─────────────────────────────────────────
// Surfaces the FORGE runtime mechanics to the user during LLM calls.

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

  // Animate entries becoming active one by one
  entries.forEach(function (e) {
    setTimeout(function () {
      e.el.classList.add('active');
      e.el.querySelector('.step-icon').textContent = '◌';
    }, e.delay);
  });

  // Mark entries as done progressively
  function markDone(index) {
    if (index >= entries.length) return;
    var e = entries[index];
    setTimeout(function () {
      e.el.classList.remove('active');
      e.el.classList.add('done');
      e.el.querySelector('.step-icon').textContent = '✓';
      var elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
      e.el.querySelector('.step-time').textContent = elapsed + 's';
      markDone(index + 1);
    }, e.delay + 400);
  }
  markDone(0);

  return log;
}

var SEARCH_STEPS = [
  { label: 'data.list', detail: '→ loading page index...' },
  { label: 'classify', detail: '→ detecting query intent...' },
  { label: 'reason', detail: '→ searching documentation...' },
  { label: 'when .sure', detail: '→ confidence gating...' }
];

var ASK_STEPS = [
  { label: 'data.list', detail: '→ loading page index...' },
  { label: 'reason', detail: '→ generating answer...' },
  { label: 'when .sure', detail: '→ evaluating confidence...' },
  { label: 'confidence_tier', detail: '→ classifying response...' }
];

// ── Form Interceptors ──────────────────────────────────────────

document.addEventListener('DOMContentLoaded', function () {

  // Code block copy buttons
  document.querySelectorAll('pre code').forEach(function (block) {
    var btn = document.createElement('button');
    btn.className = 'btn btn-xs btn-ghost absolute top-2 right-2 opacity-50 hover:opacity-100';
    btn.textContent = 'Copy';
    btn.addEventListener('click', function () {
      navigator.clipboard.writeText(block.textContent).then(function () {
        btn.textContent = 'Copied!';
        setTimeout(function () { btn.textContent = 'Copy'; }, 1500);
      });
    });
    var pre = block.parentElement;
    pre.style.position = 'relative';
    pre.appendChild(btn);
  });

  // Search form interceptor
  var searchForm = document.getElementById('search-form');
  if (searchForm) {
    searchForm.addEventListener('submit', function (e) {
      var input = searchForm.querySelector('input[name="q"]');
      var query = input ? input.value.trim() : '';
      if (!query) return; // let empty submit go through normally

      e.preventDefault();
      var resultsDiv = document.getElementById('search-results');
      if (!resultsDiv) return;

      // Show activity log
      resultsDiv.innerHTML = '';
      resultsDiv.appendChild(createActivityLog(SEARCH_STEPS));

      // Disable submit button
      var btn = searchForm.querySelector('button[type="submit"]');
      if (btn) {
        btn.classList.add('loading');
        btn.disabled = true;
      }

      fetch('/search?q=' + encodeURIComponent(query))
        .then(function (r) { return r.text(); })
        .then(function (html) {
          // Extract the search-results div content from the response
          var parser = new DOMParser();
          var doc = parser.parseFromString(html, 'text/html');
          var newResults = doc.getElementById('search-results');
          if (newResults) {
            resultsDiv.innerHTML = newResults.innerHTML;
          } else {
            resultsDiv.innerHTML = html;
          }
          // Re-enable button
          if (btn) {
            btn.classList.remove('loading');
            btn.disabled = false;
          }
        })
        .catch(function (err) {
          resultsDiv.innerHTML = '<div class="alert alert-error">Search failed: ' + err.message + '</div>';
          if (btn) {
            btn.classList.remove('loading');
            btn.disabled = false;
          }
        });
    });
  }

  // Ask form interceptor
  var askForm = document.getElementById('ask-form');
  if (askForm) {
    askForm.addEventListener('submit', function (e) {
      var textarea = askForm.querySelector('textarea[name="question"]');
      var question = textarea ? textarea.value.trim() : '';
      if (!question) return;

      e.preventDefault();
      var container = document.getElementById('ask-container');
      if (!container) return;

      // Replace form with activity log
      container.innerHTML =
        '<h1 class="text-3xl font-bold mb-6">Thinking...</h1>' +
        '<p class="mb-4 opacity-70">Asking: <em>' + escapeHtml(question) + '</em></p>';
      container.appendChild(createActivityLog(ASK_STEPS));

      fetch('/ask?question=' + encodeURIComponent(question))
        .then(function (r) { return r.text(); })
        .then(function (html) {
          // Extract the ask-container content from the response
          var parser = new DOMParser();
          var doc = parser.parseFromString(html, 'text/html');
          var newContainer = doc.getElementById('ask-container');
          if (newContainer) {
            container.innerHTML = newContainer.innerHTML;
          } else {
            container.innerHTML = html;
          }
        })
        .catch(function (err) {
          container.innerHTML =
            '<h1 class="text-3xl font-bold mb-6">Error</h1>' +
            '<div class="alert alert-error">' + err.message + '</div>' +
            '<a href="/ask_form" class="btn btn-ghost mt-4">Try again</a>';
        });
    });
  }
});

// Cmd+K search shortcut
document.addEventListener('keydown', function (e) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
    e.preventDefault();
    window.location.href = '/search';
  }
});

function escapeHtml(text) {
  var div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}
