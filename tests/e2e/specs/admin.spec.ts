import { test, expect } from '@playwright/test';
import { AdminPage } from '../pages/admin.page';
import { DocsPage } from '../pages/docs.page';

test.describe('Doc Generation (issue #62)', () => {
  // generate_docs flow makes multiple LLM calls with full doc context
  test.beforeEach(async () => { test.setTimeout(120_000); });

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

test.describe('Fact-Checking Pool (issue #63)', () => {
  // fact-checking requires doc generation + per-claim verification
  test.beforeEach(async () => { test.setTimeout(180_000); });

  test('admin_generate_docs shows fact-check report link', async ({ page }) => {
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectLoaded();
    await admin.expectSuccess();
    await expect(admin.viewFactCheckLink).toBeVisible();
  });

  test('fact-check report link navigates to report page', async ({ page }) => {
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    await admin.viewFactCheckLink.click();
    await expect(page).toHaveURL(/slug=fact-check-report/);
  });

  test('admin_fact_check endpoint renders verification report', async ({ page }) => {
    test.setTimeout(300_000); // doc gen + fact-check with full doc context
    // Seed a page first via generate_docs
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    // Navigate to fact-check for auto-reference
    await admin.gotoFactCheck('auto-reference');
    await admin.expectFactCheckLoaded();

    // Report should have content
    const prose = page.locator('.prose');
    await expect(prose).toBeVisible();
    const text = await prose.textContent();
    expect(text!.length).toBeGreaterThan(0);
  });

  test('sidebar includes fact-check report link', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');
    await docs.expectSidebarVisible();

    const factCheckLink = page.getByRole('link', { name: 'Fact-Check Report' });
    await expect(factCheckLink).toBeVisible();
  });

  test('fact-check report contains per-claim breakdown', async ({ page }) => {
    // Seed docs first
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    // View fact-check report via docs
    const docs = new DocsPage(page);
    await docs.goto('fact-check-report');
    await docs.expectLoaded();

    const text = await docs.article.textContent();
    // Report should contain verdict labels (PASS, NEEDS_REVIEW, or FAIL)
    // and individual verdicts
    expect(text!.length).toBeGreaterThan(10);
    // With mock provider, claims should have verdicts
    expect(text).toMatch(/PASS|NEEDS_REVIEW|FAIL|Verdicts/);
  });
});
