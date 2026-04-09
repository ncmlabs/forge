import { test, expect } from '@playwright/test';
import { DocsPage } from '../pages/docs.page';

test.describe('Documentation', () => {
  test('renders getting-started with sidebar and content', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    await docs.expectLoaded();
    await docs.expectSidebarVisible();
    await expect(docs.article).toContainText('Installation');
  });

  test('sidebar contains key navigation links', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    await expect(docs.sidebarLinks.filter({ hasText: 'Quick Start' })).toBeVisible();
    await expect(docs.sidebarLinks.filter({ hasText: 'task' })).toBeVisible();
    await expect(docs.sidebarLinks.filter({ hasText: 'agent' })).toBeVisible();
    await expect(docs.sidebarLinks.filter({ hasText: 'Roadmap' })).toBeVisible();
  });

  test('sidebar links navigate between pages', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    await docs.sidebarLinks.filter({ hasText: 'Roadmap' }).click();
    await expect(page).toHaveURL(/slug=roadmap/);
    await docs.expectLoaded();
  });

  test('principles page renders Nine First Principles', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('principles');

    await docs.expectLoaded();
    await expect(docs.article).toContainText('Nine First Principles');
    await expect(docs.article).toContainText('Honesty');
  });
});

test.describe('Content seeding from files', () => {
  const slugs = ['getting-started', 'principles', 'roadmap', 'task', 'agent', 'flow', 'pool'];

  for (const slug of slugs) {
    test(`${slug} page has real content`, async ({ page }) => {
      const docs = new DocsPage(page);
      await docs.goto(slug);
      await docs.expectLoaded();

      const text = await docs.article.textContent();
      expect(text!.length).toBeGreaterThan(20);
    });
  }
});
