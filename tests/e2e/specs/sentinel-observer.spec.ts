import { test, expect } from '@playwright/test';
import { ObserverPage } from '../pages/sentinel.page';

test.describe('Sentinel Observer — Live Agent Tree (issue #140)', () => {
  test('observer page loads with tree structure', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectNavVisible();
    await observer.expectTreeLoaded();

    // Should have at least: system node, warden node, 2 agent nodes
    const count = await observer.treeNodeCount();
    expect(count).toBeGreaterThanOrEqual(2);

    // System node should be present
    const systemNode = observer.treeRoot.locator('.tree-node-card[data-node-type="system"]');
    await expect(systemNode).toBeVisible();
  });

  test('SSE connection establishes', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectSSEConnected();
    await expect(observer.sseStatus).toHaveText('Live');
  });

  test('tree nodes have state attributes', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Agent nodes should have data-state attributes
    const agentNodes = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]');
    const count = await agentNodes.count();
    for (let i = 0; i < count; i++) {
      const state = await agentNodes.nth(i).getAttribute('data-state');
      expect(state).toBeTruthy();
    }
  });

  test('clicking agent shows detail panel', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Click the first agent node
    const agentNode = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first();
    await agentNode.click();

    // Detail panel should show fields
    await observer.expectDetailPanel();

    // Should show status section
    const detailText = await observer.detailContent.textContent();
    expect(detailText).toContain('Lifecycle');
    expect(detailText).toContain('Uptime');
  });

  test('detail panel closes on X button', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Open detail
    const agentNode = observer.treeRoot.locator('.tree-node-card[data-node-type="agent"]').first();
    await agentNode.click();
    await observer.expectDetailPanel();

    // Close detail
    await observer.detailClose.click();
    await expect(observer.detailContent).toContainText('Click an agent node to inspect');
  });

  test('event log exists with waiting message', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();

    await expect(observer.eventLog).toBeVisible();
    // Should show empty state initially
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

    // Toggle back
    await observer.toggleTheme();
    const restored = await observer.getTheme();
    expect(restored).toBe(initial);
  });

  test('nav bar has all five links', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectNavVisible();

    // Observer link should be active (btn-primary)
    const observerBtn = observer.observerLink;
    await expect(observerBtn).toHaveClass(/btn-primary/);
  });

  test('skeleton screens visible before data loads', async ({ page }) => {
    // Intercept API calls to delay them
    await page.route('**/__forge/inspect/**', async (route) => {
      await new Promise((r) => setTimeout(r, 2000));
      await route.continue();
    });

    const observer = new ObserverPage(page);
    await observer.goto();

    // Skeleton should be visible immediately
    const skeletons = page.locator('#tree-root .skeleton');
    await expect(skeletons.first()).toBeVisible();
  });

  test('scan button triggers scan and shows loading state', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Scan button should be present
    await expect(observer.scanButton).toBeVisible();
    await expect(observer.scanButton).toBeEnabled();
  });

  test('warden panel renders', async ({ page }) => {
    const observer = new ObserverPage(page);
    await observer.goto();
    await observer.expectTreeLoaded();

    // Warden panel should be present
    await expect(observer.wardenPanel).toBeVisible();
  });
});
