/**
 * ForgeCockpit — Orchestration cockpit view (#308 T7).
 *
 * Three stacked sections, populated from existing SSE + inspect endpoints
 * (no new runtime APIs, per the issue). Follows the `ForgeTaskDag` pattern:
 * `/__forge/inspect/agents` is the source of truth, SSE is a refresh cue,
 * and a 2s fallback poll covers dropped frames (DoD: within 2s of change).
 *
 *   1. Pipeline (Kanban) — task cards in dev-cycle stage columns. Stage is
 *      derived from which named specialist (planner/implementer/tester/
 *      reviewer/release_manager) currently holds the issue_id in memory,
 *      plus the release_manager's last_merged for the merged column.
 *   2. Decision queue — any agent whose lifecycle_state begins with
 *      `awaiting_` is a pending human decision. Memory provides the
 *      issue_id / pr_url / channel context.
 *   3. Agent activity strip — compact per-agent state pills pulled from
 *      the summary endpoint, annotated with warden circuit-breaker flag
 *      and the timestamp of the last SSE frame mentioning that agent.
 *
 * Depends on: ForgeAPI, ForgeEvents.
 */
var ForgeCockpit = (function () {
  'use strict';

  // ── DOM refs ──────────────────────────────────────────────────

  var pipelineEl = null;
  var decisionsEl = null;
  var agentsEl = null;
  var statusEl = null;

  // ── State ─────────────────────────────────────────────────────

  var unsubscribe = null;
  var pollTimer = null;
  var inFlight = false;
  var agents = [];
  var agentDetails = {};
  var wardens = [];
  var lastSeenByAgent = {};

  var POLL_INTERVAL_MS = 2000;

  // Dev-cycle specialist → pipeline column.
  var SPECIALIST_STAGE = {
    planner: 'planning',
    implementer: 'implementing',
    tester: 'testing',
    reviewer: 'reviewing',
    release_manager: 'pr_ready'
  };

  var STAGES = [
    { key: 'planning',     label: 'Planning',     specialist: 'planner' },
    { key: 'implementing', label: 'Implementing', specialist: 'implementer' },
    { key: 'testing',      label: 'Testing',      specialist: 'tester' },
    { key: 'reviewing',    label: 'Reviewing',    specialist: 'reviewer' },
    { key: 'pr_ready',     label: 'PR Ready',     specialist: 'release_manager' },
    { key: 'merged',       label: 'Merged',       specialist: null }
  ];

  // HandlerCompleted frames for these handlers warrant an immediate refresh.
  // These are the events that cause a pipeline stage transition or a
  // pending-approval change. Anything else falls back to the 2s poll.
  var REFRESH_HANDLERS = {
    IssueAssigned: true,
    PlanApproved: true,
    ImplementationReady: true,
    TestsFailed: true,
    AcceptanceMet: true,
    PRReady: true,
    PRMerged: true,
    TaskCompleted: true,
    ApprovalResponse: true,
    ClonedevTaskInbound: true,
    TaskRouted: true,
    TaskBlocked: true
  };

  // ── ConfidentValue unwrap (copied from taskdag.js) ────────────
  // The inspect endpoint wraps every memory value in
  // `{ confidence, source, value: { TypeTag: rawValue } }`. Custom types
  // add a `_type`/`_value` layer. Keep this in sync with taskdag.js.

  function unwrap(v) {
    if (v === null || v === undefined) return v;
    if (typeof v !== 'object') return v;
    if ('_value' in v) return unwrap(v._value);
    if ('value' in v && ('confidence' in v || 'source' in v)) {
      return unwrap(v.value);
    }
    if ('Text' in v) return v.Text;
    if ('Number' in v) return v.Number;
    if ('Bool' in v) return v.Bool;
    if ('Array' in v) return v.Array.map(unwrap);
    if ('Record' in v) {
      if (v.Record._value !== undefined) return unwrap(v.Record._value);
      var out = {};
      Object.keys(v.Record).forEach(function (k) {
        out[k] = unwrap(v.Record[k]);
      });
      return out;
    }
    return v;
  }

  function memoryField(info, field) {
    if (!info || !info.memory || !(field in info.memory)) return '';
    var v = unwrap(info.memory[field]);
    return v === null || v === undefined ? '' : String(v);
  }

  function memoryNumber(info, field) {
    if (!info || !info.memory || !(field in info.memory)) return 0;
    var v = unwrap(info.memory[field]);
    var n = Number(v);
    return isNaN(n) ? 0 : n;
  }

  // ── Fetching ──────────────────────────────────────────────────

  function refresh() {
    if (inFlight) return;
    inFlight = true;

    var agentsPromise = ForgeAPI.fetchJSON('/__forge/inspect/agents').catch(function () {
      return [];
    });
    var wardensPromise = ForgeAPI.fetchJSON('/__forge/inspect/wardens').catch(function () {
      return [];
    });

    Promise.all([agentsPromise, wardensPromise])
      .then(function (results) {
        agents = Array.isArray(results[0]) ? results[0] : [];
        wardens = Array.isArray(results[1]) ? results[1] : [];

        // Fetch deep state for the dev-cycle specialists we know about.
        // Only these need memory.issue_id; for the activity strip the
        // summary response is enough, so we don't fetch every agent.
        var wanted = Object.keys(SPECIALIST_STAGE);
        var deepFetches = agents
          .filter(function (a) { return wanted.indexOf(a.name) !== -1; })
          .map(function (a) {
            return ForgeAPI.fetchJSON('/__forge/inspect/agents/' + a.id)
              .then(function (info) { return { id: a.id, info: info }; })
              .catch(function () { return { id: a.id, info: null }; });
          });
        return Promise.all(deepFetches);
      })
      .then(function (deeps) {
        agentDetails = {};
        deeps.forEach(function (d) {
          if (d && d.info) agentDetails[d.id] = d.info;
        });
        inFlight = false;
        render();
      })
      .catch(function (err) {
        inFlight = false;
        setStatus('Fetch failed: ' + (err && err.message ? err.message : err));
      });
  }

  function setStatus(text) {
    if (statusEl) statusEl.textContent = text;
  }

  // ── SSE ───────────────────────────────────────────────────────

  function onSse(evt) {
    if (!evt) return;

    // Track last-seen timestamp per agent for the activity strip. Tracer
    // frames carry the originating agent under different keys (source_agent
    // for event_emit, agent for handler/exec frames); accept either.
    // NOTE: evt.ts_ms is *relative to tracer start* (see src/tracer.rs:80),
    // not wall-clock. Use Date.now() so formatDuration(now - lastSeen) is
    // meaningful.
    var who = evt.source_agent || evt.agent || evt.target_agent;
    if (who) {
      lastSeenByAgent[who] = Date.now();
    }

    // Refresh on HandlerCompleted for the handlers that change pipeline
    // or decision-queue state. Everything else is rendered lazily on the
    // 2s poll tick.
    if (evt.event === 'HandlerCompleted' && REFRESH_HANDLERS[evt.handler]) {
      refresh();
      return;
    }

    // The tracer's `event_emit` trace shape collides the trace-type label
    // with the FORGE event name under `evt.event` (see mastery.js:448).
    // Treat a bare domain event name as a refresh cue too — covers cases
    // where HandlerCompleted was dropped under backpressure.
    if (REFRESH_HANDLERS[evt.event] && evt.source_agent) {
      refresh();
    }
  }

  // Rebuild the per-agent last-seen map from the current SSE buffer so
  // the activity strip has timestamps immediately after a tab switch or
  // page reload (the circular buffer holds up to 5000 recent events).
  // Buffer entries carry `entry.ts = Date.now()` stamped at receive-time
  // in events.js:handleEvent — prefer that over the tracer-relative
  // `entry.ts_ms`.
  function seedLastSeenFromBuffer() {
    if (typeof ForgeEvents === 'undefined' || !ForgeEvents.getBuffer) return;
    var buf = ForgeEvents.getBuffer() || [];
    for (var i = 0; i < buf.length; i++) {
      var entry = buf[i];
      if (!entry) continue;
      var evt = entry.data || entry;
      var who = evt.source_agent || evt.agent || evt.target_agent;
      if (who) {
        lastSeenByAgent[who] = entry.ts || 0;
      }
    }
  }

  // ── Rendering: pipeline ───────────────────────────────────────

  function pipelineCardsByStage() {
    // issue_id → card data.
    var issues = {};

    agents.forEach(function (a) {
      if (!(a.name in SPECIALIST_STAGE)) return;
      var info = agentDetails[a.id];
      if (!info) return;

      var issueId = memoryField(info, 'issue_id');
      if (!issueId) return;

      var stage = SPECIALIST_STAGE[a.name];
      // The release_manager holds issue_id even after PRMerged fires, so
      // classify it as merged when its lifecycle has already been cleared
      // (state machine transitions to open / idle after TaskCompleted).
      // A simpler heuristic: if the release_manager's last_outcome is
      // "merged" and no later specialist is holding the same issue, show
      // it in the merged column.
      if (a.name === 'release_manager' && memoryField(info, 'last_outcome') === 'merged') {
        stage = 'merged';
      }

      issues[issueId] = issues[issueId] || {
        issue_id: issueId,
        stage: stage,
        specialist: a.name,
        repo: memoryField(info, 'repo'),
        branch: memoryField(info, 'branch'),
        channel: memoryField(info, 'channel'),
        pr_url: memoryField(info, 'pr_url'),
        lifecycle: a.lifecycle_state || '',
        review_rounds: memoryNumber(info, 'review_rounds'),
        iteration: memoryNumber(info, 'iteration'),
        updated_at: Date.now() - (a.uptime_ms || 0)
      };

      // If multiple specialists hold the same issue_id, the one furthest
      // along wins. STAGES is ordered planning→merged, so prefer the
      // higher-indexed entry.
      var currentIdx = stageIndex(issues[issueId].stage);
      var candidateIdx = stageIndex(stage);
      if (candidateIdx > currentIdx) {
        issues[issueId].stage = stage;
        issues[issueId].specialist = a.name;
        issues[issueId].repo = memoryField(info, 'repo') || issues[issueId].repo;
        issues[issueId].branch = memoryField(info, 'branch') || issues[issueId].branch;
        issues[issueId].pr_url = memoryField(info, 'pr_url') || issues[issueId].pr_url;
      }
    });

    // Release_manager's last_merged is a terminal signal — include it in
    // the merged column even if the memory.issue_id has rotated.
    agents.forEach(function (a) {
      if (a.name !== 'release_manager') return;
      var info = agentDetails[a.id];
      if (!info) return;
      var lastMerged = memoryField(info, 'last_merged');
      if (!lastMerged || issues[lastMerged]) return;
      issues[lastMerged] = {
        issue_id: lastMerged,
        stage: 'merged',
        specialist: 'release_manager',
        repo: memoryField(info, 'repo'),
        branch: memoryField(info, 'branch'),
        channel: '',
        pr_url: '',
        lifecycle: '',
        review_rounds: 0,
        iteration: 0,
        updated_at: 0
      };
    });

    var byStage = {};
    STAGES.forEach(function (s) { byStage[s.key] = []; });
    Object.keys(issues).forEach(function (id) {
      var card = issues[id];
      if (byStage[card.stage]) byStage[card.stage].push(card);
    });
    return byStage;
  }

  function stageIndex(stageKey) {
    for (var i = 0; i < STAGES.length; i++) {
      if (STAGES[i].key === stageKey) return i;
    }
    return -1;
  }

  function renderPipeline() {
    if (!pipelineEl) return;

    var byStage = pipelineCardsByStage();
    var total = 0;
    Object.keys(byStage).forEach(function (k) { total += byStage[k].length; });

    if (total === 0) {
      pipelineEl.innerHTML = ''
        + '<div class="cockpit-empty">'
        + 'No dev-cycle tasks in flight. Post to <code>/dev_cycle</code> on a running '
        + 'workflow to seed the pipeline.'
        + '</div>';
      return;
    }

    var html = '<div class="cockpit-pipeline-columns">';
    STAGES.forEach(function (stage) {
      var cards = byStage[stage.key] || [];
      html += ''
        + '<div class="cockpit-pipeline-column">'
        + '<div class="cockpit-pipeline-header">'
        + '<span>' + ForgeAPI.escapeHtml(stage.label) + '</span>'
        + '<span class="cockpit-pipeline-count">' + cards.length + '</span>'
        + '</div>'
        + '<div class="cockpit-pipeline-cards">'
        + cards.map(renderPipelineCard).join('')
        + '</div>'
        + '</div>';
    });
    html += '</div>';

    pipelineEl.innerHTML = html;
  }

  function renderPipelineCard(card) {
    var statusClass = 'cockpit-card';
    if (card.stage === 'merged') statusClass += ' cockpit-card--done';
    if (card.lifecycle === 'awaiting_approval') statusClass += ' cockpit-card--waiting';

    var metaParts = [];
    if (card.repo) metaParts.push(ForgeAPI.escapeHtml(card.repo));
    if (card.branch) metaParts.push(ForgeAPI.escapeHtml(card.branch));
    var meta = metaParts.length
      ? '<div class="cockpit-card-meta">' + metaParts.join(' \u00b7 ') + '</div>'
      : '';

    var badges = [];
    if (card.lifecycle === 'awaiting_approval') {
      badges.push('<span class="cockpit-badge cockpit-badge--waiting">waiting-human</span>');
    }
    if (card.iteration && card.iteration > 0) {
      badges.push('<span class="cockpit-badge">iter ' + card.iteration + '</span>');
    }
    if (card.review_rounds && card.review_rounds > 0) {
      badges.push('<span class="cockpit-badge">rounds ' + card.review_rounds + '</span>');
    }
    var badgeRow = badges.length
      ? '<div class="cockpit-card-badges">' + badges.join(' ') + '</div>'
      : '';

    return ''
      + '<div class="' + statusClass + '">'
      + '<div class="cockpit-card-title">#' + ForgeAPI.escapeHtml(card.issue_id) + '</div>'
      + meta
      + '<div class="cockpit-card-agent">' + ForgeAPI.escapeHtml(card.specialist) + '</div>'
      + badgeRow
      + '</div>';
  }

  // ── Rendering: decision queue ────────────────────────────────

  function decisionRows() {
    // Any agent whose lifecycle state begins with `awaiting_` is waiting
    // on a human decision. For the reviewer agent we have rich memory
    // (issue_id, pr_url, channel, review_verdict); for future approval
    // gates we fall back to whatever summary is available.
    var rows = [];
    agents.forEach(function (a) {
      var lc = a.lifecycle_state || '';
      if (lc.indexOf('awaiting_') !== 0) return;

      var info = agentDetails[a.id];
      var issueId = info ? memoryField(info, 'issue_id') : '';
      var prUrl = info ? memoryField(info, 'pr_url') : '';
      var channel = info ? memoryField(info, 'channel') : '';
      var callbackUrl = info ? memoryField(info, 'callback_url') : '';
      var verdict = info ? memoryField(info, 'review_verdict') : '';
      var waitingSince = a.uptime_ms || 0;

      rows.push({
        agent: a.name + (a.alias ? ' · ' + a.alias : ''),
        state: lc,
        issue_id: issueId,
        pr_url: prUrl,
        channel: channel,
        callback_url: callbackUrl,
        verdict: verdict,
        waiting_ms: waitingSince
      });
    });
    return rows;
  }

  function renderDecisions() {
    if (!decisionsEl) return;
    var rows = decisionRows();
    if (rows.length === 0) {
      decisionsEl.innerHTML = '<div class="cockpit-empty-inline">No decisions waiting.</div>';
      return;
    }

    var html = '<div class="cockpit-decision-list">';
    rows.forEach(function (r) {
      var linkHtml = '';
      if (r.pr_url) {
        linkHtml = '<a href="' + ForgeAPI.escapeHtml(r.pr_url) + '" target="_blank" rel="noopener">PR</a>';
      }
      if (r.callback_url) {
        linkHtml += (linkHtml ? ' · ' : '')
          + '<a href="' + ForgeAPI.escapeHtml(r.callback_url) + '" target="_blank" rel="noopener">Slack</a>';
      }
      var meta = [];
      if (r.issue_id) meta.push('#' + ForgeAPI.escapeHtml(r.issue_id));
      if (r.channel) meta.push(ForgeAPI.escapeHtml(r.channel));
      if (r.verdict) meta.push('verdict: ' + ForgeAPI.escapeHtml(r.verdict));

      html += ''
        + '<div class="cockpit-decision-row">'
        + '<div class="cockpit-decision-agent">' + ForgeAPI.escapeHtml(r.agent) + '</div>'
        + '<div class="cockpit-decision-meta">' + meta.join(' \u00b7 ') + '</div>'
        + '<div class="cockpit-decision-state">' + ForgeAPI.escapeHtml(r.state) + '</div>'
        + '<div class="cockpit-decision-wait">' + formatDuration(r.waiting_ms) + '</div>'
        + '<div class="cockpit-decision-links">' + linkHtml + '</div>'
        + '</div>';
    });
    html += '</div>';
    decisionsEl.innerHTML = html;
  }

  // ── Rendering: agent activity strip ──────────────────────────

  function wardenStateByAgent() {
    // WardenSnapshot exposes `managed_agents` and `degraded_agents`
    // (src/runtime/warded.rs:30). Any name present in `degraded_agents`
    // is flagged as degraded; anything else managed is healthy.
    var degraded = {};
    wardens.forEach(function (w) {
      (w.degraded_agents || []).forEach(function (name) { degraded[name] = true; });
    });
    return degraded;
  }

  function renderAgents() {
    if (!agentsEl) return;
    if (agents.length === 0) {
      agentsEl.innerHTML = '<div class="cockpit-empty-inline">No agents running.</div>';
      return;
    }

    var degraded = wardenStateByAgent();
    var now = Date.now();

    var html = '<div class="cockpit-agent-strip">';
    agents.forEach(function (a) {
      var cls = 'cockpit-agent-pill';
      var state = a.lifecycle_state || 'running';
      var isDegraded = !!degraded[a.name] || !!degraded[a.alias];
      var isWaiting = state.indexOf('awaiting_') === 0;
      var lastSeen = lastSeenByAgent[a.name] || lastSeenByAgent[a.alias] || 0;
      var isActive = lastSeen > 0 && (now - lastSeen) < 10000;

      if (isDegraded) cls += ' cockpit-agent-pill--degraded';
      else if (isWaiting) cls += ' cockpit-agent-pill--waiting';
      else if (isActive) cls += ' cockpit-agent-pill--active';
      else cls += ' cockpit-agent-pill--idle';

      var label = a.alias || a.name;
      var lastText = lastSeen ? formatDuration(now - lastSeen) + ' ago' : '—';

      html += ''
        + '<div class="' + cls + '">'
        + '<span class="cockpit-agent-name">' + ForgeAPI.escapeHtml(label) + '</span>'
        + '<span class="cockpit-agent-state">' + ForgeAPI.escapeHtml(state) + '</span>'
        + '<span class="cockpit-agent-last">' + ForgeAPI.escapeHtml(lastText) + '</span>'
        + '</div>';
    });
    html += '</div>';
    agentsEl.innerHTML = html;
  }

  function formatDuration(ms) {
    if (!ms || ms < 0) return '—';
    if (ms < 1000) return '<1s';
    var secs = Math.floor(ms / 1000);
    if (secs < 60) return secs + 's';
    var mins = Math.floor(secs / 60);
    if (mins < 60) return mins + 'm';
    var hours = Math.floor(mins / 60);
    return hours + 'h';
  }

  // ── Orchestration ────────────────────────────────────────────

  function render() {
    renderPipeline();
    renderDecisions();
    renderAgents();
    var total = agents.length;
    var pending = decisionRows().length;
    setStatus('Connected \u00b7 ' + total + ' agents \u00b7 ' + pending + ' decisions pending');
  }

  function init() {
    pipelineEl = document.getElementById('cockpit-pipeline');
    decisionsEl = document.getElementById('cockpit-decisions');
    agentsEl = document.getElementById('cockpit-agents');
    statusEl = document.getElementById('cockpit-status');

    agents = [];
    agentDetails = {};
    wardens = [];
    lastSeenByAgent = {};

    seedLastSeenFromBuffer();

    if (typeof ForgeEvents !== 'undefined' && ForgeEvents.onEvent) {
      if (unsubscribe) unsubscribe();
      unsubscribe = ForgeEvents.onEvent(onSse);
    }

    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(refresh, POLL_INTERVAL_MS);

    refresh();
    render();
  }

  function destroy() {
    if (unsubscribe) { unsubscribe(); unsubscribe = null; }
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    agents = [];
    agentDetails = {};
    wardens = [];
    lastSeenByAgent = {};
    if (pipelineEl) pipelineEl.innerHTML = '';
    if (decisionsEl) decisionsEl.innerHTML = '';
    if (agentsEl) agentsEl.innerHTML = '';
    setStatus('Disconnected');
  }

  function resize() { /* nothing size-sensitive for now */ }

  return {
    init: init,
    destroy: destroy,
    resize: resize,
    refresh: refresh
  };
})();
