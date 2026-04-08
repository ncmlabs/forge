import { test, expect } from '@playwright/test';
import { ObserverPage } from '../pages/sentinel.page';

// Mock data matching FORGE inspect API shapes
const MOCK_TOPOLOGY = {
  system_name: 'repo_sentinel',
  bindings: [['inspector', 'git_inspector'], ['analyst', 'analyst']],
  wiring: [['inspector', 'analyst']],
  subscribers: [{ event: 'ScanComplete', agent: 'analyst', has_filter: false }],
  routes: { git_inspector: ['analyst'] },
};

const MOCK_AGENTS = [
  { id: '00000000-0000-0000-0000-000000000001', name: 'git_inspector', alias: 'inspector', lifecycle_state: 'active', uptime_ms: 12000, status: 'running' },
  { id: '00000000-0000-0000-0000-000000000002', name: 'analyst', alias: 'analyst', lifecycle_state: 'active', uptime_ms: 11500, status: 'running' },
];

const MOCK_AGENT_DEEP = {
  id: '00000000-0000-0000-0000-000000000001',
  name: 'git_inspector',
  alias: 'inspector',
  lifecycle_state: 'active',
  uptime_ms: 12000,
  status: 'running',
  memory: { scan_count: 3, last_health: 'healthy', commit_velocity: '12' },
  timers: {},
  stuck: false,
  hallucinating: false,
  event_count: 5,
  escalation_count: 0,
  knowledge_count: 0,
};

const MOCK_WARDENS = [
  { name: 'sentinel_supervisor', managed_agents: ['git_inspector', 'analyst'], degraded_agents: [], retry_counts: {}, circuit_breaker_tripped: false },
];

function setupMockAPIs(page: import('@playwright/test').Page) {
  return Promise.all([
    page.route('**/__forge/inspect/topology', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_TOPOLOGY) })
    ),
    page.route('**/__forge/inspect/agents/00000000-*', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_AGENT_DEEP) })
    ),
    page.route('**/__forge/inspect/agents', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_AGENTS) })
    ),
    page.route('**/__forge/inspect/wardens', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(MOCK_WARDENS) })
    ),
  ]);
}

test.describe('Sentinel Observer — Live Agent Tree (issue #140)', () => {
  test('observer page loads with tree structure', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectNavVisible();
    await observer.expectTreeLoaded();

    // System + warden + 2 agents = 4 tree-node-cards
    const count = await observer.treeNodeCount();
    expect(count).toBeGreaterThanOrEqual(4);

    const systemNode = observer.treeRoot.locator('.tree-node-card[data-node-type="system"]');
    await expect(systemNode).toBeVisible();

    const wardenNode = observer.treeRoot.locator('.tree-node-card[data-node-type="warden"]');
    await expect(wardenNode).toBeVisible();
  });

  test('SSE connection establishes', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectSSEConnected();
    await expect(observer.sseStatus).toHaveText('Live');
  });

  test('tree nodes have state attributes', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    const agentNodes = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]');
    const count = await agentNodes.count();
    expect(count).toBe(2);
    for (let i = 0; i < count; i++) {
      const state = await agentNodes.nth(i).getAttribute('data-state');
      expect(state).toBe('running');
    }
  });

  test('clicking agent shows detail panel with memory', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    const agentNode = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first();
    await agentNode.click();

    // Node should get selected class
    await expect(agentNode).toHaveClass(/selected/);

    // Detail panel should show fields from deep inspection
    await observer.expectDetailPanel();
    const detailText = await observer.detailContent.textContent();
    expect(detailText).toContain('Lifecycle');
    expect(detailText).toContain('Uptime');
    expect(detailText).toContain('scan_count');
  });

  test('detail panel closes on X button', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Open detail
    const agentNode = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first();
    await agentNode.click();
    await observer.expectDetailPanel();

    // Close
    await observer.detailClose.click();
    await expect(observer.detailContent).toContainText('Click an agent node to inspect');
  });

  test('event log exists with waiting message', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();

    await expect(observer.eventLog).toBeVisible();
    const logEmpty = page.locator('#event-log-empty');
    await expect(logEmpty).toBeVisible();
  });

  test('theme toggle works on observer page', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();

    const initial = await observer.getTheme();
    await observer.toggleTheme();
    const toggled = await observer.getTheme();
    expect(toggled).not.toBe(initial);

    await observer.toggleTheme();
    const restored = await observer.getTheme();
    expect(restored).toBe(initial);
  });

  test('nav bar has all five links', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectNavVisible();

    await expect(observer.observerLink).toHaveClass(/btn-primary/);
  });

  test('skeleton screens visible before data loads', async ({ page }) => {
    // Delay API responses to observe skeletons
    await page.route('**/__forge/inspect/**', async (route) => {
      await new Promise((r) => setTimeout(r, 2000));
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    });

    const observer = new ObserverPage(page);
    await observer.goto();

    const skeletons = page.locator('#tree-root .skeleton');
    await expect(skeletons.first()).toBeVisible();
  });

  test('scan button is present and enabled', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();

    await expect(observer.scanButton).toBeVisible();
    await expect(observer.scanButton).toBeEnabled();
  });

  test('warden panel renders warden data', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Warden panel should show the warden name
    await expect(observer.wardenPanel).toContainText('sentinel_supervisor');
  });

  test('tree shows wiring labels', async ({ page }) => {
    await setupMockAPIs(page);
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    const wiringLabel = observer.treeRoot.locator('.wiring-label');
    await expect(wiringLabel.first()).toBeVisible();
    await expect(wiringLabel.first()).toContainText('inspector >> analyst');
  });
});
