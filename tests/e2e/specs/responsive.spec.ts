import { test, expect } from '@playwright/test';
import { DocsPage } from '../pages/docs.page';

test.describe('Mobile responsive', () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test('sidebar is hidden on mobile viewport', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    await docs.expectLoaded();
    await docs.expectSidebarHidden();
  });

  test('navbar is still visible on mobile', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');
    await docs.expectNavVisible();
  });
});
