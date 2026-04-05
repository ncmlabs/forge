import { test, expect } from '@playwright/test';
import { AdminPage } from '../pages/admin.page';
import { DocsPage } from '../pages/docs.page';

test.describe('Doc Generation (issue #62)', () => {
  test('admin endpoint triggers flow and shows success', async ({ page }) => {
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectLoaded();
    await admin.expectSuccess();
  });

  test('generated reference is viewable via docs endpoint', async ({ page }) => {
    // Trigger generation first
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    // Follow link to view generated docs
    await admin.viewReferenceLink.click();
    await expect(page).toHaveURL(/slug=auto-reference/);

    const docs = new DocsPage(page);
    await docs.expectLoaded();
    const text = await docs.article.textContent();
    expect(text!.length).toBeGreaterThan(10);
  });

  test('view reference link navigates correctly', async ({ page }) => {
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.viewReferenceLink.click();
    await expect(page).toHaveURL(/slug=auto-reference/);
  });
});
