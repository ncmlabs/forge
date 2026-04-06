import { test, expect } from '@playwright/test';
import { SearchPage } from '../pages/search.page';
import { AskPage } from '../pages/ask.page';
import { DocsPage } from '../pages/docs.page';
import { AdminPage } from '../pages/admin.page';

test.describe('Warden Supervision (issue #64)', () => {
  test('wiki loads with warden supervision active', async ({ page }) => {
    await page.goto('/home');
    await expect(page.locator('h1')).toBeVisible();
    await expect(page.locator('.navbar')).toBeVisible();
  });

  test('search works under supervision', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto('task');
    await search.expectResultsVisible();
    const text = await search.results.textContent();
    expect(text!.length).toBeGreaterThan(0);
  });

  test('Q&A works under supervision', async ({ page }) => {
    // Use GET with query param (same as existing ask.spec.ts pattern)
    await page.goto('/ask?question=What+is+a+warden');
    const answerCard = page.locator('.card .card-body');
    await expect(answerCard).toBeVisible();
    const answer = await answerCard.textContent();
    expect(answer!.length).toBeGreaterThan(0);
  });

  test('docs endpoint works under supervision', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');
    await docs.expectLoaded();
    const text = await docs.article.textContent();
    expect(text!.length).toBeGreaterThan(10);
  });

  test('admin doc generation works under supervision', async ({ page }) => {
    test.setTimeout(120_000); // generate_docs flow makes multiple LLM calls
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectLoaded();
    await admin.expectSuccess();
  });

  test('admin fact-check works under supervision', async ({ page }) => {
    test.setTimeout(120_000); // requires doc generation + fact-checking
    // Seed docs first
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    // Run fact-check
    await admin.gotoFactCheck('auto-reference');
    await admin.expectFactCheckLoaded();
    const prose = page.locator('.prose');
    await expect(prose).toBeVisible();
  });
});
