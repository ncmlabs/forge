import { test, expect, type Page } from '@playwright/test';
import { ObserverPage } from '../pages/observer.page';

const MOCK_TOPOLOGY = {
  system_name: 'test_system',
  bindings: [['inspector', 'git_inspector'], ['analyst', 'analyst']],
  wiring: [['inspector', 'analyst']],
  subscribers: [],
  routes: {},
};

const MOCK_AGENTS = [
  { id: '00000000-0000-0000-0000-000000000001', name: 'git_inspector', alias: 'inspector', lifecycle_state: 'active', uptime_ms: 12000, status: 'running' },
  { id: '00000000-0000-0000-0000-000000000002', name: 'analyst', alias: 'analyst', lifecycle_state: 'active', uptime_ms: 11500, status: 'running' },
];

const MOCK_AGENT_DEEP = {
  id: '00000000-0000-0000-0000-000000000001', name: 'git_inspector', alias: 'inspector',
  lifecycle_state: 'active', uptime_ms: 12000, status: 'running',
  memory: { scan_count: 3, last_health: 'healthy' }, timers: {},
  stuck: false, hallucinating: false, event_count: 5, escalation_count: 0, knowledge_count: 0,
};

const MOCK_WARDENS = [
  { name: 'test_supervisor', managed_agents: ['git_inspector', 'analyst'], degraded_agents: [], retry_counts: {}, circuit_breaker_tripped: false },
];

const MOCK_COSTS = {
  totals: { calls: 5, tokens_in: 1200, tokens_out: 800, cost_usd: 0.0042 },
  by_operation: { completion: { calls: 5, tokens_in: 1200, tokens_out: 800, cost_usd: 0.0042, avg_confidence: 0.85 } },
  by_agent: { git_inspector: { calls: 3, tokens_in: 700, tokens_out: 500, cost_usd: 0.0025, avg_confidence: 0.9 } },
  by_provider_model: { 'anthropic/claude-3': { calls: 5, tokens_in: 1200, tokens_out: 800, cost_usd: 0.0042 } },
  confidence_histogram: [0, 0, 0, 0, 0, 1, 1, 1, 1, 1],
  uptime_secs: 60,
  tokens_per_sec: 13.3,
};

const MOCK_MASTERY = {
  specialists: ['planner', 'implementer', 'tester', 'reviewer', 'release_manager'],
  projects: ['forge-playground'],
  mastery: {
    'planner::forge-playground': {
      specialist: 'planner',
      project: 'forge-playground',
      current_level: 'apprentice',
      current_score: 55,
      clean_count: 2,
      regress_count: 0,
      total: 2,
      transitions: [
        { at: '2026-04-15T10:00:00Z', level: 'novice', score: 50, clean_count: 1, regress_count: 0, total: 1, last_task: 'T1' },
        { at: '2026-04-16T10:00:00Z', level: 'apprentice', score: 55, clean_count: 2, regress_count: 0, total: 2, last_task: 'T2' },
      ],
    },
  },
  tasks: {
    total_tasks: 3,
    projects: ['forge-playground'],
    uptime_secs: 42,
    tasks_by_project: {
      'forge-playground': [
        { task_id: 'T1', repo: 'forge-playground', outcome: 'merged', review_rounds: 3, ci_passed_first_try: false, time_to_merge: 3600, reverted_within_7d: false, completed_at: '2026-04-15T10:00:00Z' },
        { task_id: 'T2', repo: 'forge-playground', outcome: 'merged', review_rounds: 2, ci_passed_first_try: true,  time_to_merge: 1800, reverted_within_7d: false, completed_at: '2026-04-16T10:00:00Z' },
        { task_id: 'T3', repo: 'forge-playground', outcome: 'merged', review_rounds: 1, ci_passed_first_try: true,  time_to_merge: 900,  reverted_within_7d: false, completed_at: '2026-04-17T10:00:00Z' },
      ],
    },
  },
};

async function setupMocks(page: Page) {
  // Mock all /__forge/* endpoints regardless of the server URL prefix
  await page.route('**/__forge/inspect/topology', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_TOPOLOGY) })
  );
  await page.route('**/__forge/inspect/agents/00000000-*', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_AGENT_DEEP) })
  );
  await page.route('**/__forge/inspect/agents', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_AGENTS) })
  );
  await page.route('**/__forge/inspect/wardens', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_WARDENS) })
  );
  await page.route('**/__forge/inspect/costs', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_COSTS) })
  );
  await page.route('**/__forge/inspect/mastery', route =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_MASTERY) })
  );
  // SSE events endpoint — fulfill with empty stream that stays open
  await page.route('**/__forge/events', route =>
    route.fulfill({ status: 200, contentType: 'text/event-stream', body: 'data: {"event":"say","text":"test","ts_ms":0}\n\n' })
  );
}

test.describe('Standalone Observer (issue #144)', () => {
  test('page loads with connection bar', async ({ page }) => {
    const obs = new ObserverPage(page);
    await obs.goto();
    await expect(obs.serverUrl).toBeVisible();
    await expect(obs.connectBtn).toBeVisible();
    await expect(obs.connectionStatus).toContainText('Disconnected');
  });

  test('connects to mock server', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.goto();
    await obs.connect('http://localhost:3001');
    await obs.expectConnected();
    await expect(obs.disconnectBtn).toBeVisible();
  });

  test('auto-connects with ?server= param', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
  });

  test('tree view renders agents from topology', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();

    // Wait for tree to render
    await expect(obs.treeRoot.locator('.tree-node-card')).toHaveCount(4, { timeout: 5000 });
    // System + warden + 2 agents = 4 nodes (tree-node-cards)
    const systemNode = obs.treeRoot.locator('.tree-node-card[data-node-type="system"]');
    await expect(systemNode).toContainText('test_system');
  });

  test('clicking agent shows detail panel', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await expect(obs.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first()).toBeVisible({ timeout: 5000 });

    await obs.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first().click();
    await expect(obs.detailContent).toContainText('Lifecycle', { timeout: 5000 });
    await expect(obs.detailContent).toContainText('scan_count');
  });

  test('tab switching works', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();

    // Tree is default active
    await expect(page.locator('#view-tree')).toHaveClass(/active/);

    // Switch to costs tab
    await obs.switchTab('costs');
    await expect(page.locator('#view-costs')).toHaveClass(/active/);
    await expect(page.locator('#view-tree')).not.toHaveClass(/active/);
  });

  test('costs view shows data', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await obs.switchTab('costs');

    // Wait for cost data to load
    await expect(obs.costUsd).not.toHaveText('$0.0000', { timeout: 5000 });
  });

  test('mastery view shows proof-point panels', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await obs.switchTab('mastery');

    // Summary banner populated from snapshot
    await expect(obs.masteryTotalTasks).toHaveText('3', { timeout: 5000 });
    await expect(obs.masteryProjectCount).toHaveText('1');
    // Avg review_rounds = (3 + 2 + 1) / 3 = 2.00
    await expect(obs.masteryAvgAsks).toHaveText('2.00');
    await expect(obs.masteryTopSpecialist).toContainText('planner');

    // Project filter populated with the mocked project
    await expect(obs.masteryProjectFilter.locator('option[value="forge-playground"]')).toHaveCount(1);

    // Summary table row for planner::forge-playground
    const plannerRow = obs.masteryTbody.locator('tr', { hasText: 'planner' }).first();
    await expect(plannerRow).toContainText('apprentice');
    await expect(plannerRow).toContainText('forge-playground');

    // Both charts rendered
    await expect(obs.masteryScoreChart.locator('svg')).toBeVisible();
    await expect(obs.masteryAsksChart.locator('svg')).toBeVisible();
    // Three approval-ask bars (one per task)
    await expect(obs.masteryAsksChart.locator('rect.ask-bar')).toHaveCount(3);
  });

  test('topology view renders SVG', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await obs.switchTab('topology');

    // SVG should have topo-node elements
    await expect(obs.topoSvg.locator('.topo-node')).toHaveCount(4, { timeout: 5000 });
  });

  test('timeline view renders with filters', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await obs.switchTab('timeline');

    // Filter checkboxes should be visible
    const filters = obs.timelineFilters.locator('.timeline-filter');
    await expect(filters).toHaveCount(6); // 6 categories
  });

  test('theme toggle works', async ({ page }) => {
    const obs = new ObserverPage(page);
    await obs.goto();
    const initial = await obs.getTheme();
    await obs.toggleTheme();
    const toggled = await obs.getTheme();
    expect(toggled).not.toBe(initial);
  });

  test('disconnect clears state', async ({ page }) => {
    await setupMocks(page);
    const obs = new ObserverPage(page);
    await obs.gotoWithServer('http://localhost:3001');
    await obs.expectConnected();
    await obs.page.locator('#disconnect-btn').click();
    await obs.expectDisconnected();
    await expect(obs.connectBtn).toBeVisible();
  });
});
