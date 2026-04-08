import { test, expect } from '@playwright/test';
import { DashboardPage } from '../pages/sentinel.page';

test.describe('Sentinel Dashboard smoke tests (issue #140)', () => {
  test('dashboard loads with health badge', async ({ page }) => {
    const dashboard = new DashboardPage(page);
    await dashboard.goto();
    await dashboard.expectLoaded();

    const badgeText = await dashboard.healthBadge.textContent();
    expect(badgeText!.length).toBeGreaterThan(0);
  });

  test('metric cards render', async ({ page }) => {
    const dashboard = new DashboardPage(page);
    await dashboard.goto();
    await dashboard.expectLoaded();

    const count = await dashboard.metricCards.count();
    expect(count).toBe(3); // velocity, branches, codebase size
  });

  test('nav bar includes Observer link', async ({ page }) => {
    const dashboard = new DashboardPage(page);
    await dashboard.goto();
    await dashboard.expectNavVisible();

    // Observer link should exist and be clickable
    await expect(dashboard.observerLink).toHaveAttribute('href', '/observer');
  });
});
