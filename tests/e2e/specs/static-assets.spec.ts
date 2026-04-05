import { test, expect } from '@playwright/test';

test.describe('Static assets', () => {
  test('CSS loads successfully', async ({ page }) => {
    const response = await page.goto('/static/css/style.css');
    expect(response!.status()).toBe(200);
  });

  test('app.js loads successfully', async ({ page }) => {
    const response = await page.goto('/static/js/app.js');
    expect(response!.status()).toBe(200);
  });

  test('forge-highlight.js loads successfully', async ({ page }) => {
    const response = await page.goto('/static/js/forge-highlight.js');
    expect(response!.status()).toBe(200);
  });

  test('homepage loads all static resources without errors', async ({ page }) => {
    const failedRequests: string[] = [];
    page.on('requestfailed', (req) => failedRequests.push(req.url()));

    await page.goto('/home', { waitUntil: 'networkidle' });
    const staticFailures = failedRequests.filter((u) => u.includes('/static/'));
    expect(staticFailures).toHaveLength(0);
  });
});
