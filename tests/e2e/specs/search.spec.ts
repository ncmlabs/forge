import { test, expect } from '@playwright/test';
import { SearchPage } from '../pages/search.page';

test.describe('Search', () => {
  test('page loads with search input', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto();

    await expect(search.searchInput).toBeVisible();
    await expect(search.submitButton).toBeVisible();
  });

  test('submitting a query returns results', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto();
    await search.search('task');

    // Form uses AJAX fetch, so URL doesn't change — wait for results to appear
    await expect(search.results).toHaveText(/.+/, { timeout: 15_000 });
  });

  test('URL query param pre-fills input', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto('agent');

    await expect(search.searchInput).toHaveValue('agent');
    await search.expectResultsVisible();
  });

  test('search results contain content', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto('agent');

    const results = await search.results.textContent();
    expect(results!.length).toBeGreaterThan(10);
  });

  test('Cmd+K shortcut navigates to search', async ({ page }) => {
    await page.goto('/home');
    await page.keyboard.press('Meta+k');
    await expect(page).toHaveURL(/\/search_page/);
  });
});
