import { test, expect } from '@playwright/test';
import { TopologyPage } from '../pages/sentinel.page';

// Mock data matching the sentinel system topology
const MOCK_AGENTS = [
  { id: '00000000-0000-0000-0000-000000000001', name: 'git_inspector', alias: 'inspector', lifecycle_state: 'healthy', uptime_ms: 14000, status: 'running' },
  { id: '00000000-0000-0000-0000-000000000002', name: 'analyst', alias: 'analyst', lifecycle_state: 'active', uptime_ms: 13500, status: 'running' },
];

const MOCK_AGENT_DEEP = {
  id: '00000000-0000-0000-0000-000000000001',
  name: 'git_inspector',
  alias: 'inspector',
  lifecycle_state: 'healthy',
  uptime_ms: 14000,
  status: 'running',
  memory: { scan_count: { value: { Number: 5 }, confidence: 1.0 }, last_health: { value: { Text: 'healthy' }, confidence: 0.9 }, commit_velocity: { value: { Text: '12' }, confidence: 0.85 } },
  timers: {},
  stuck: false,
  hallucinating: false,
  event_count: 8,
  escalation_count: 0,
  knowledge_count: 0,
};

const MOCK_WARDENS = [
  { name: 'sentinel_supervisor', managed_agents: ['git_inspector', 'analyst'], degraded_agents: [], retry_counts: {}, circuit_breaker_tripped: false },
];

function setupMocks(page: import('@playwright/test').Page) {
  return Promise.all([
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

test.describe('Sentinel Topology — Force-Directed Graph (issue #141)', () => {
  test('topology page loads with D3 graph', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectNavVisible();
    await topo.expectGraphLoaded();
  });

  test('graph contains system, warden, and agent nodes', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    expect(await topo.nodeCount('system')).toBe(1);
    expect(await topo.nodeCount('warden')).toBe(1);
    expect(await topo.nodeCount('agent')).toBe(2);
  });

  test('nodes have correct labels', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    const svg = topo.topoSvg;
    await expect(svg.locator('.node-label', { hasText: 'repo_sentinel' })).toBeVisible();
    await expect(svg.locator('.node-label', { hasText: 'sentinel_supervisor' })).toBeVisible();
    await expect(svg.locator('.node-label', { hasText: 'inspector' })).toBeVisible();
    await expect(svg.locator('.node-label', { hasText: 'analyst' })).toBeVisible();
  });

  test('graph has supervision and wiring edges', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    const types = await topo.linkTypes();
    expect(types).toContain('supervises');
    expect(types).toContain('wired');
  });

  test('wired edges have arrow markers', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    const wiredLink = topo.topoSvg.locator('.topo-link.wired').first();
    const markerEnd = await wiredLink.getAttribute('marker-end');
    expect(markerEnd).toBe('url(#arrow)');
  });

  test('SSE connection establishes', async ({ page }) => {
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectSSEConnected();
    await expect(topo.sseStatus).toHaveText('Live');
  });

  test('clicking agent node shows detail panel', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    await topo.clickNode('agent:git_inspector');
    await topo.expectDetailPanel();

    const text = await topo.detailContent.textContent();
    expect(text).toContain('Lifecycle');
    expect(text).toContain('Uptime');
    expect(text).toContain('scan_count');
  });

  test('clicking warden node shows supervision data', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    await topo.clickNode('warden:sentinel_supervisor');
    await topo.expectDetailPanel();

    const text = await topo.detailContent.textContent();
    expect(text).toContain('Managed agents');
    expect(text).toContain('git_inspector');
    expect(text).toContain('analyst');
  });

  test('clicking system node shows composition summary', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    await topo.clickNode('system:repo_sentinel');
    await topo.expectDetailPanel();

    const text = await topo.detailContent.textContent();
    expect(text).toContain('Agents');
    expect(text).toContain('Wardens');
  });

  test('detail panel closes on X button', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    await topo.clickNode('agent:git_inspector');
    await topo.expectDetailPanel();

    await topo.detailClose.click();
    await expect(topo.detailContent).toContainText('Click a node to inspect');
  });

  test('scan button is present and enabled', async ({ page }) => {
    const topo = new TopologyPage(page);
    await topo.goto();
    await expect(topo.scanButton).toBeVisible();
    await expect(topo.scanButton).toBeEnabled();
  });

  test('topology nav link is active', async ({ page }) => {
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectNavVisible();
    await expect(topo.topologyLink).toHaveClass(/btn-primary/);
  });

  test('warden health badge shows ok', async ({ page }) => {
    await setupMocks(page);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    const healthDot = topo.topoSvg.locator('.health-dot.ok');
    await expect(healthDot).toBeVisible();
  });

  test('skeleton loading when APIs are slow', async ({ page }) => {
    await page.route('**/__forge/inspect/**', async (route) => {
      await new Promise((r) => setTimeout(r, 3000));
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    });

    const topo = new TopologyPage(page);
    await topo.goto();

    // SVG should be present but empty (no nodes yet)
    await expect(topo.topoSvg).toBeVisible();
    const nodeCount = await topo.nodeCount();
    expect(nodeCount).toBe(0);
  });

  test('theme toggle works on topology page', async ({ page }) => {
    const topo = new TopologyPage(page);
    await topo.goto();

    const initial = await topo.getTheme();
    await topo.toggleTheme();
    const toggled = await topo.getTheme();
    expect(toggled).not.toBe(initial);
  });
});
