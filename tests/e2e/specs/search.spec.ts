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

    await expect(page).toHaveURL(/q=task/);
    await search.expectResultsVisible();
  });

  test('URL query param pre-fills input', async ({ page }) => {
    const search = new SearchPage(page);
    await search.goto('agent');

    await expect(search.searchInput).toHaveValue('agent');
    await search.expectResultsVisible();
  });

  test('Cmd+K shortcut navigates to search', async ({ page }) => {
    await page.goto('/home');
    await page.keyboard.press('Meta+k');
    await expect(page).toHaveURL(/\/search/);
  });
});
