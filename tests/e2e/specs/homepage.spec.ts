import { test, expect } from '@playwright/test';
import { HomePage } from '../pages/home.page';

test.describe('Homepage', () => {
  test('loads with hero section and feature cards', async ({ page }) => {
    const home = new HomePage(page);
    await home.goto();

    await home.expectLoaded();
    await home.expectNavVisible();

    await expect(home.heroTitle).toContainText('FORGE');
    await expect(home.getStartedButton).toBeVisible();
    await expect(home.featureCards).toHaveCount(6);
  });

  test('root path redirects to /home', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveURL(/\/home/);
  });

  test('navigation links work', async ({ page }) => {
    const home = new HomePage(page);
    await home.goto();

    await home.docsLink.click();
    await expect(page).toHaveURL(/\/docs/);

    await page.goBack();
    await home.searchLink.click();
    await expect(page).toHaveURL(/\/search/);

    await page.goBack();
    await home.askLink.click();
    await expect(page).toHaveURL(/\/ask_form/);
  });
});
