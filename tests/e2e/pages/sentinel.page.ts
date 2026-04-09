import { type Page, type Locator, expect } from '@playwright/test';

export class SentinelBasePage {
  readonly page: Page;
  readonly navbar: Locator;
  readonly dashboardLink: Locator;
  readonly insightsLink: Locator;
  readonly observerLink: Locator;
  readonly topologyLink: Locator;
  readonly apiLink: Locator;
  readonly themeToggle: Locator;

  constructor(page: Page) {
    this.page = page;
    this.navbar = page.locator('.navbar');
    this.dashboardLink = this.navbar.getByRole('link', { name: 'Dashboard' });
    this.insightsLink = this.navbar.getByRole('link', { name: 'Insights' });
    this.observerLink = this.navbar.getByRole('link', { name: 'Observer' });
    this.topologyLink = this.navbar.getByRole('link', { name: 'Topology' });
    this.apiLink = this.navbar.getByRole('link', { name: 'API' });
    this.themeToggle = this.navbar.locator('button');
  }

  async expectNavVisible() {
    await expect(this.navbar).toBeVisible();
    await expect(this.dashboardLink).toBeVisible();
    await expect(this.insightsLink).toBeVisible();
    await expect(this.observerLink).toBeVisible();
    await expect(this.topologyLink).toBeVisible();
    await expect(this.apiLink).toBeVisible();
  }

  async getTheme(): Promise<string | null> {
    return this.page.locator('html').getAttribute('data-theme');
  }

  async toggleTheme() {
    await this.themeToggle.click();
  }
}

export class ObserverPage extends SentinelBasePage {
  readonly treeRoot: Locator;
  readonly eventLog: Locator;
  readonly sseStatus: Locator;
  readonly agentDetail: Locator;
  readonly detailContent: Locator;
  readonly detailClose: Locator;
  readonly treeNodes: Locator;
  readonly scanButton: Locator;
  readonly wardenPanel: Locator;

  constructor(page: Page) {
    super(page);
    this.treeRoot = page.locator('#tree-root');
    this.eventLog = page.locator('#event-log');
    this.sseStatus = page.locator('#sse-status');
    this.agentDetail = page.locator('#agent-detail');
    this.detailContent = page.locator('#detail-content');
    this.detailClose = page.locator('#detail-close');
    this.treeNodes = page.locator('.tree-node-card');
    this.scanButton = page.locator('#scan-trigger-obs');
    this.wardenPanel = page.locator('#warden-panel');
  }

  async goto() {
    await this.page.goto('/observer');
  }

  async expectTreeLoaded() {
    // Wait for skeleton to be replaced with actual tree nodes
    await expect(this.treeRoot.locator('ul').first()).toBeVisible({ timeout: 15_000 });
    await expect(this.treeNodes.first()).toBeVisible({ timeout: 15_000 });
  }

  async expectSSEConnected() {
    await expect(this.sseStatus).toHaveClass(/connected/, { timeout: 10_000 });
  }

  async clickAgent(name: string) {
    const node = this.treeRoot.locator(`.tree-node-card[data-agent-name="${name}"]`);
    await node.click();
  }

  async expectDetailPanel() {
    await expect(this.detailContent.locator('.detail-field').first()).toBeVisible({ timeout: 10_000 });
  }

  async treeNodeCount() {
    return this.treeNodes.count();
  }
}

export class TopologyPage extends SentinelBasePage {
  readonly svgContainer: Locator;
  readonly topoSvg: Locator;
  readonly detailContent: Locator;
  readonly detailClose: Locator;
  readonly sseStatus: Locator;
  readonly eventLog: Locator;
  readonly scanButton: Locator;

  constructor(page: Page) {
    super(page);
    this.svgContainer = page.locator('#topo-container');
    this.topoSvg = page.locator('#topo-svg');
    this.detailContent = page.locator('#topo-detail-content');
    this.detailClose = page.locator('#topo-detail-close');
    this.sseStatus = page.locator('#topo-sse-status');
    this.eventLog = page.locator('#topo-event-log');
    this.scanButton = page.locator('#topo-scan-trigger');
  }

  async goto() {
    await this.page.goto('/topology');
  }

  async expectGraphLoaded() {
    await expect(this.topoSvg.locator('g.nodes .topo-node').first()).toBeVisible({ timeout: 15_000 });
  }

  async expectSSEConnected() {
    await expect(this.sseStatus).toHaveClass(/connected/, { timeout: 10_000 });
  }

  /** Count D3 nodes of a given type (system, warden, agent). */
  async nodeCount(type?: string) {
    const selector = type
      ? `.topo-node[data-node-id^="${type}:"]`
      : '.topo-node';
    return this.topoSvg.locator(selector).count();
  }

  /** Click a node by its data-node-id prefix (e.g. 'agent:git_inspector'). */
  async clickNode(nodeId: string) {
    // D3 attaches click listeners on the <g> via addEventListener.
    // Playwright's force:true doesn't reliably bubble through SVG layers,
    // so we dispatch a native click event directly on the <g> element.
    await this.page.evaluate((id) => {
      const el = document.querySelector(`[data-node-id="${id}"]`);
      if (el) el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    }, nodeId);
  }

  async expectDetailPanel() {
    await expect(this.detailContent.locator('.detail-field').first()).toBeVisible({ timeout: 10_000 });
  }

  /** Returns the number of log entries in the mini event log. */
  async eventLogCount() {
    return this.eventLog.locator('.log-entry').count();
  }

  /** Wait for at least N events to appear in the event log. */
  async expectMinEvents(n: number, timeout = 60_000) {
    await expect(async () => {
      const count = await this.eventLogCount();
      expect(count).toBeGreaterThanOrEqual(n);
    }).toPass({ timeout });
  }

  /** Check that a specific node has the 'thinking' class (LLM in progress). */
  async isNodeThinking(nodeId: string) {
    return this.topoSvg.locator(`[data-node-id="${nodeId}"].thinking`).isVisible();
  }

  /** Get all link types currently in the SVG. */
  async linkTypes() {
    const links = this.topoSvg.locator('.topo-link');
    const count = await links.count();
    const types: string[] = [];
    for (let i = 0; i < count; i++) {
      const cls = await links.nth(i).getAttribute('class');
      if (cls?.includes('supervises')) types.push('supervises');
      else if (cls?.includes('wired')) types.push('wired');
      else types.push('unknown');
    }
    return types;
  }
}

export class DashboardPage extends SentinelBasePage {
  readonly healthBadge: Locator;
  readonly metricCards: Locator;

  constructor(page: Page) {
    super(page);
    this.healthBadge = page.locator('#health-badge');
    this.metricCards = page.locator('.metric-card');
  }

  async goto() {
    await this.page.goto('/dashboard');
  }

  async expectLoaded() {
    await expect(this.healthBadge).toBeVisible({ timeout: 30_000 });
  }
}
