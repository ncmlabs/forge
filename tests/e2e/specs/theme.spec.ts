import { test, expect } from '@playwright/test';
import { HomePage } from '../pages/home.page';

test.describe('Dark mode', () => {
  test('toggles between light and dark', async ({ page }) => {
    const home = new HomePage(page);
    await home.goto();

    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');

    await home.toggleTheme();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

    await home.toggleTheme();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  });

  test('persists theme across navigation', async ({ page }) => {
    const home = new HomePage(page);
    await home.goto();

    await home.toggleTheme();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

    await home.docsLink.click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  });

  test('saves theme to localStorage', async ({ page }) => {
    const home = new HomePage(page);
    await home.goto();

    await home.toggleTheme();
    const stored = await page.evaluate(() => localStorage.getItem('forge-wiki-theme'));
    expect(stored).toBe('dark');
  });
});
