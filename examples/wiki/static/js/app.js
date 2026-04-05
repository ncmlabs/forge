// FORGE Wiki — client-side interactivity

// Dark mode toggle
function toggleTheme() {
  const html = document.documentElement;
  const current = html.getAttribute('data-theme');
  const next = current === 'light' ? 'dark' : 'light';
  html.setAttribute('data-theme', next);
  localStorage.setItem('forge-wiki-theme', next);
}

// Restore saved theme
(function () {
  const saved = localStorage.getItem('forge-wiki-theme');
  if (saved) {
    document.documentElement.setAttribute('data-theme', saved);
  }
})();

// Code block copy buttons
document.addEventListener('DOMContentLoaded', function () {
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
});

// Cmd+K search shortcut
document.addEventListener('keydown', function (e) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
    e.preventDefault();
    window.location.href = '/search';
  }
});
