/**
 * FORGE syntax highlighting for Prism.js
 *
 * Registers a custom "forge" language grammar so FORGE code blocks
 * get proper syntax highlighting in the wiki and any FORGE web app.
 *
 * Usage (after loading Prism.js):
 *   <script src="/static/js/forge-highlight.js"></script>
 *   <pre><code class="language-forge">...</code></pre>
 */

if (typeof Prism !== 'undefined') {
  Prism.languages.forge = {
    'comment': {
      pattern: /\/\/.*|\/\*[\s\S]*?\*\//,
      greedy: true
    },
    'string': {
      pattern: /"(?:[^"\\]|\\.)*"/,
      greedy: true
    },
    'template-interpolation': {
      pattern: /\{!?[^}]+\}/,
      inside: {
        'punctuation': /^\{!?|\}$/,
        'expression': /[\s\S]+/
      }
    },
    'directive': {
      pattern: /^#!.*$/m,
      alias: 'comment'
    },
    'keyword': /\b(?:task|pure|flow|endpoint|pool|agent|needs|gives|give|do|use|reason|classify|into|search|match|when|try|or|for|in|if|else|emit|on|every|escalate|to|human|spawn|retire|find|learn|with|requires|boundary|status|content_type)\b/,
    'type': /\b(?:Text|Number|Bool|Unit|Html|Results|Report|Intent|Classification|Summary|Failure|Conversation|Profile|Request|Response|Headers|Embedding|Duration)\b/,
    'builtin': /\b(?:asset|html\.layout|html\.escape|llm\.reason|llm\.classify|web\.search|data\.store|data\.embed)\b/,
    'confidence': {
      pattern: /\b(?:uncertain|confident|hallucinated)\b/,
      alias: 'important'
    },
    'boundary-marker': {
      pattern: /\b(?:server|client|worker)\b/,
      alias: 'tag'
    },
    'operator': />>|->|=/,
    'punctuation': /[(),:]/,
    'number': /\b\d+(?:\.\d+)?\b/,
    'boolean': /\b(?:true|false)\b/
  };
}
