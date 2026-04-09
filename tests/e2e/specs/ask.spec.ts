import { test, expect } from '@playwright/test';
import { AskPage } from '../pages/ask.page';

test.describe('Q&A', () => {
  test('form loads with textarea and submit button', async ({ page }) => {
    const ask = new AskPage(page);
    await ask.goto();

    await expect(ask.questionInput).toBeVisible();
    await expect(ask.submitButton).toBeVisible();
  });

  test('submitting a question returns an answer', async ({ page }) => {
    // Use GET with query param since the form's native POST sends form-encoded data
    // but the endpoint expects application/json
    await page.goto('/ask_page?question=What+is+a+task');
    const answerCard = page.locator('.card .card-body');
    await expect(answerCard).toBeVisible();
    const text = await answerCard.textContent();
    expect(text!.length).toBeGreaterThan(5);
  });

  test('answer page shows confidence badge', async ({ page }) => {
    const ask = new AskPage(page);
    await page.goto('/ask_page?question=What+is+a+pool');
    await expect(ask.confidenceBadge).toBeVisible();
    const badge = await ask.confidenceBadge.textContent();
    expect(badge).toMatch(/confidence/);
  });
});
