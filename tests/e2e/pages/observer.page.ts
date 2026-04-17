import { type Page, type Locator, expect } from '@playwright/test';

export class ObserverPage {
  readonly page: Page;
  // Connection bar
  readonly serverUrl: Locator;
  readonly connectBtn: Locator;
  readonly disconnectBtn: Locator;
  readonly connectionStatus: Locator;
  readonly themeToggle: Locator;
  // Tabs
  readonly tabTree: Locator;
  readonly tabTopology: Locator;
  readonly tabCosts: Locator;
  readonly tabMastery: Locator;
  readonly tabTimeline: Locator;
  // Tree view
  readonly treeRoot: Locator;
  readonly eventLog: Locator;
  readonly detailContent: Locator;
  readonly detailClose: Locator;
  // Topology view
  readonly topoContainer: Locator;
  readonly topoSvg: Locator;
  readonly topoDetailContent: Locator;
  // Costs view
  readonly costTotals: Locator;
  readonly costUsd: Locator;
  readonly costCalls: Locator;
  // Mastery view (#304 T5.3)
  readonly masteryTotalTasks: Locator;
  readonly masteryProjectCount: Locator;
  readonly masteryAvgAsks: Locator;
  readonly masteryTopSpecialist: Locator;
  readonly masteryProjectFilter: Locator;
  readonly masteryScoreChart: Locator;
  readonly masteryAsksChart: Locator;
  readonly masteryTbody: Locator;
  // Timeline view
  readonly timelineContainer: Locator;
  readonly timelineFilters: Locator;

  constructor(page: Page) {
    this.page = page;
    // Connection bar
    this.serverUrl = page.locator('#server-url');
    this.connectBtn = page.locator('#connect-btn');
    this.disconnectBtn = page.locator('#disconnect-btn');
    this.connectionStatus = page.locator('#connection-status');
    this.themeToggle = page.locator('#theme-toggle');
    // Tabs
    this.tabTree = page.locator('#tab-tree');
    this.tabTopology = page.locator('#tab-topology');
    this.tabCosts = page.locator('#tab-costs');
    this.tabMastery = page.locator('#tab-mastery');
    this.tabTimeline = page.locator('#tab-timeline');
    // Tree view
    this.treeRoot = page.locator('#tree-root');
    this.eventLog = page.locator('#event-log');
    this.detailContent = page.locator('#detail-content');
    this.detailClose = page.locator('#detail-close');
    // Topology view
    this.topoContainer = page.locator('#topo-container');
    this.topoSvg = page.locator('#topo-svg');
    this.topoDetailContent = page.locator('#topo-detail-content');
    // Costs view
    this.costTotals = page.locator('#cost-totals');
    this.costUsd = page.locator('#cost-usd');
    this.costCalls = page.locator('#cost-calls');
    // Mastery view (#304 T5.3)
    this.masteryTotalTasks = page.locator('#mastery-total-tasks');
    this.masteryProjectCount = page.locator('#mastery-project-count');
    this.masteryAvgAsks = page.locator('#mastery-avg-asks');
    this.masteryTopSpecialist = page.locator('#mastery-top-specialist');
    this.masteryProjectFilter = page.locator('#mastery-project-filter');
    this.masteryScoreChart = page.locator('#mastery-score-chart');
    this.masteryAsksChart = page.locator('#mastery-asks-chart');
    this.masteryTbody = page.locator('#mastery-tbody');
    // Timeline view
    this.timelineContainer = page.locator('#timeline-container');
    this.timelineFilters = page.locator('#timeline-filters');
  }

  async goto() {
    await this.page.goto('/static/index.html');
  }

  async gotoWithServer(serverUrl: string) {
    await this.page.goto('/static/index.html?server=' + encodeURIComponent(serverUrl));
  }

  async connect(serverUrl?: string) {
    if (serverUrl) {
      await this.serverUrl.fill(serverUrl);
    }
    await this.connectBtn.click();
  }

  async expectConnected() {
    await expect(this.connectionStatus).toHaveText('Connected', { timeout: 10000 });
  }

  async expectDisconnected() {
    await expect(this.connectionStatus).toContainText('Disconnected');
  }

  async switchTab(name: 'tree' | 'topology' | 'costs' | 'mastery' | 'timeline') {
    const tabMap: Record<string, Locator> = {
      tree: this.tabTree,
      topology: this.tabTopology,
      costs: this.tabCosts,
      mastery: this.tabMastery,
      timeline: this.tabTimeline,
    };
    await tabMap[name].click();
  }

  async getTheme() {
    return this.page.locator('html').getAttribute('data-theme');
  }

  async toggleTheme() {
    await this.themeToggle.click();
  }
}
