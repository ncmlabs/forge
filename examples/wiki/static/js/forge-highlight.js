// Prism.js language definition for FORGE
if (typeof Prism !== 'undefined') {
  Prism.languages.forge = {
    'comment': /#.*/,
    'directive': /^#!.*/m,
    'string': {
      pattern: /"(?:[^"\\]|\\.)*"/,
      greedy: true,
      inside: {
        'interpolation': {
          pattern: /\{!?[^}]+\}/,
          inside: {
            'punctuation': /[{}!]/
          }
        }
      }
    },
    'keyword': /\b(?:task|pure|flow|agent|pool|system|warden|event|states|endpoint|fn|type|use|needs|gives|do|give|say|emit|when|match|if|else|for|in|try|or|boundary|requires|on|fail|subscribe|memory|lifecycle|timer|transition|to|spawn|find|retire|learn|recall|start|cancel|reset|escalate|forward|exportable|import|from|as|with|where|stage|workers|strategy|timeout|manages|after|max_retries|per|then|stuck)\b/,
    'builtin': /\b(?:reason|classify|search|data\.store|data\.get|data\.list|data\.delete|web\.fetch|web\.post|html\.layout|html\.escape|markdown\.render|asset)\b/,
    'type': /\b(?:Text|Number|Bool|Html|Unit|Results|Report|Intent|Classification|Summary|Array|Record|Embedding|Duration)\b/,
    'confidence': /\b(?:sure|unsure|unreliable|uncertain)\b/,
    'boolean': /\b(?:true|false)\b/,
    'number': /\b\d+(?:\.\d+)?\b/,
    'operator': />>|->|\|>|!=|==|>=|<=|[+\-*/><!]/,
    'punctuation': /[()[\]{},.:]/
  };
}
