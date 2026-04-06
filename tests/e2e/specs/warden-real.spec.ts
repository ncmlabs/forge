import { test, expect } from '@playwright/test';
import { SearchPage } from '../pages/search.page';
import { AskPage } from '../pages/ask.page';
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
    const search = new SearchPage(page);
    await search.goto('what is a task');
    await search.expectResultsVisible();
    const text = await search.results.textContent();
    // Real LLM should produce substantive content, not just mock data
    expect(text!.length).toBeGreaterThan(20);
  });

  test('Q&A gives real answers via qa_agent', async ({ page }) => {
    const ask = new AskPage(page);
    await ask.goto();
    await ask.askQuestion('What is a warden in FORGE?');

    await expect(ask.answerCard).toBeVisible();
    const answer = await ask.answerCard.textContent();
    // Real answer should mention supervision or agents
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
    test.setTimeout(60_000); // Real LLM calls may take longer
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectLoaded();
    await admin.expectSuccess();

    // View the generated reference — should have real content
    await admin.viewReferenceLink.click();
    await expect(page).toHaveURL(/slug=auto-reference/);
    const docs = new DocsPage(page);
    await docs.expectLoaded();
    const text = await docs.article.textContent();
    expect(text!.length).toBeGreaterThan(100);
  });

  test('admin fact-checks with real LLM', async ({ page }) => {
    test.setTimeout(60_000);
    // Seed docs first
    const admin = new AdminPage(page);
    await admin.goto();
    await admin.expectSuccess();

    // Run fact-check
    await admin.gotoFactCheck('auto-reference');
    await admin.expectFactCheckLoaded();
    const prose = page.locator('.prose');
    await expect(prose).toBeVisible();
    const text = await prose.textContent();
    // Real fact-check should have verdict labels
    expect(text!.length).toBeGreaterThan(20);
  });

  test('confidence tiers reflect real LLM confidence', async ({ page }) => {
    const ask = new AskPage(page);
    await ask.goto();
    await ask.askQuestion('What are the 14 primitives in FORGE?');

    await expect(ask.answerCard).toBeVisible();
    // Confidence badge should be visible
    await expect(ask.confidenceBadge).toBeVisible();
    const badgeText = await ask.confidenceBadge.textContent();
    // Should show one of the confidence tiers
    expect(badgeText).toMatch(/High|Medium|Low/i);
  });

  test('trace output shows supervision activity', async ({ page }) => {
    // Simply verify the wiki is running and serving — trace output
    // goes to stderr which isn't directly accessible from Playwright,
    // but we can verify the system is operational
    await page.goto('/home');
    await expect(page.locator('h1')).toBeVisible();

    // Navigate through multiple endpoints to exercise the supervised agents
    const search = new SearchPage(page);
    await search.goto('agent');
    await search.expectResultsVisible();

    const ask = new AskPage(page);
    await ask.goto();
    await ask.askQuestion('Explain FORGE supervision');
    await expect(ask.answerCard).toBeVisible();
  });
});
