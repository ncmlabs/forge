import { test, expect } from '@playwright/test';
import { SearchPage } from '../pages/search.page';
import { DocsPage } from '../pages/docs.page';
import { AdminPage } from '../pages/admin.page';

// These tests require a real ANTHROPIC_API_KEY and hit the actual Claude API.
// They are skipped when the key is not set.
test.beforeEach(async () => {
  test.skip(!process.env.ANTHROPIC_API_KEY, 'requires ANTHROPIC_API_KEY');
});

test.describe('Warden Supervision — Real API (issue #64)', () => {
  test('homepage loads with real LLM backend', async ({ page }) => {
    await page.goto('/home');
    await expect(page.locator('h1')).toContainText('FORGE');
    await expect(page.locator('.navbar')).toBeVisible();
  });

  test('search returns real LLM-powered results', async ({ page }) => {
    test.setTimeout(60_000);
    const search = new SearchPage(page);
    await search.goto('what is a task');
    await search.expectResultsVisible();
    const text = await search.results.textContent();
    // Real LLM should produce substantive content, not just mock data
    expect(text!.length).toBeGreaterThan(20);
  });

  test('Q&A gives real answers', async ({ page }) => {
    test.setTimeout(60_000);
    // Use GET with query param (form POST sends wrong Content-Type)
    await page.goto('/ask?question=What+is+a+warden+in+FORGE');
    const answerCard = page.locator('.card .card-body');
    await expect(answerCard).toBeVisible({ timeout: 45_000 });
    const answer = await answerCard.textContent();
    // Real answer should have substantive content
    expect(answer!.length).toBeGreaterThan(50);
  });

  test('docs render with real content', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');
    await docs.expectLoaded();
    const text = await docs.article.textContent();
    expect(text!.length).toBeGreaterThan(50);
  });

  test('admin generates real documentation', async ({ page }) => {
    test.setTimeout(360_000); // Multi-stage flow: 5 stages, 6+ LLM calls
    await page.goto('/admin_generate_docs', { timeout: 300_000 });
    await expect(page.locator('h1')).toContainText('Doc Generation');
    await expect(page.locator('.alert-success')).toBeVisible({ timeout: 10_000 });

    // View the generated reference — should have real content
    await page.getByRole('link', { name: 'View Generated Reference' }).click();
    await expect(page).toHaveURL(/slug=auto-reference/);
    const docs = new DocsPage(page);
    await docs.expectLoaded();
    const text = await docs.article.textContent();
    expect(text!.length).toBeGreaterThan(100);
  });

  test('admin fact-checks with real LLM', async ({ page }) => {
    test.setTimeout(420_000); // Doc generation + fact-checking, 9+ LLM calls
    // Seed docs first
    await page.goto('/admin_generate_docs', { timeout: 300_000 });
    await expect(page.locator('.alert-success')).toBeVisible({ timeout: 10_000 });

    // Run fact-check
    await page.goto('/admin_fact_check?slug=auto-reference', { timeout: 300_000 });
    await expect(page.locator('h1')).toContainText('Fact-Check Report');
    const prose = page.locator('.prose');
    await expect(prose).toBeVisible();
    const text = await prose.textContent();
    // Real fact-check should have verdict labels
    expect(text!.length).toBeGreaterThan(20);
  });

  test('confidence tiers reflect real LLM confidence', async ({ page }) => {
    test.setTimeout(60_000);
    // Use GET with query param
    await page.goto('/ask?question=What+are+the+14+primitives+in+FORGE');
    const answerCard = page.locator('.card .card-body');
    await expect(answerCard).toBeVisible({ timeout: 45_000 });
    // Confidence badge should be visible
    const badge = page.locator('.badge');
    await expect(badge).toBeVisible();
    const badgeText = await badge.textContent();
    // Should show one of the confidence tiers
    expect(badgeText).toMatch(/confidence/i);
  });

  test('multiple endpoints work under real supervision', async ({ page }) => {
    test.setTimeout(90_000);
    // Verify the wiki is running and serving under supervision
    await page.goto('/home');
    await expect(page.locator('h1')).toBeVisible();

    // Exercise the supervised agents via multiple endpoints
    const search = new SearchPage(page);
    await search.goto('agent');
    await search.expectResultsVisible();

    // Use GET for Q&A — wait for server-side LLM call
    await page.goto('/ask?question=Explain+FORGE+supervision', { timeout: 60_000 });
    const answerCard = page.locator('.card .card-body');
    await expect(answerCard).toBeVisible({ timeout: 10_000 });
  });
});
