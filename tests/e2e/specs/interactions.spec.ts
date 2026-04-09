import { test, expect } from '@playwright/test';
import { DocsPage } from '../pages/docs.page';

test.describe('Code block copy buttons', () => {
  test('copy buttons are injected into code blocks', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    const codeBlockCount = await docs.codeBlocks.count();
    expect(codeBlockCount).toBeGreaterThan(0);

    const copyButtons = page.locator('pre button', { hasText: 'Copy' });
    await expect(copyButtons.first()).toBeVisible();
    expect(await copyButtons.count()).toBe(codeBlockCount);
  });

  test('copy button is clickable and positioned correctly', async ({ page }) => {
    const docs = new DocsPage(page);
    await docs.goto('getting-started');

    const firstCopyBtn = page.locator('pre button', { hasText: 'Copy' }).first();
    await expect(firstCopyBtn).toBeVisible();

    // Verify the button is positioned inside a pre with position: relative
    const prePosition = await firstCopyBtn.locator('..').evaluate(
      (el) => getComputedStyle(el).position
    );
    expect(prePosition).toBe('relative');
  });
});
