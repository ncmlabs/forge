import { type Page, type Locator, expect } from '@playwright/test';

export class SentinelBasePage {
  readonly page: Page;
  readonly navbar: Locator;
  readonly dashboardLink: Locator;
  readonly scanLink: Locator;
  readonly insightsLink: Locator;
  readonly observerLink: Locator;
  readonly apiLink: Locator;
  readonly themeToggle: Locator;

  constructor(page: Page) {
    this.page = page;
    this.navbar = page.locator('.navbar');
    this.dashboardLink = this.navbar.getByRole('link', { name: 'Dashboard' });
    this.scanLink = this.navbar.getByRole('link', { name: 'Scan' });
    this.insightsLink = this.navbar.getByRole('link', { name: 'Insights' });
    this.observerLink = this.navbar.getByRole('link', { name: 'Observer' });
    this.apiLink = this.navbar.getByRole('link', { name: 'API' });
    this.themeToggle = this.navbar.locator('button');
  }

  async expectNavVisible() {
    await expect(this.navbar).toBeVisible();
    await expect(this.dashboardLink).toBeVisible();
    await expect(this.scanLink).toBeVisible();
    await expect(this.insightsLink).toBeVisible();
    await expect(this.observerLink).toBeVisible();
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
