import { type Page, type Locator, expect } from '@playwright/test';
import { BasePage } from './base.page';

export class SearchPage extends BasePage {
  readonly searchInput: Locator;
  readonly submitButton: Locator;
  readonly results: Locator;

  constructor(page: Page) {
    super(page);
    this.searchInput = page.locator('input[name="q"]');
    this.submitButton = page.getByRole('button', { name: 'Search' });
    this.results = page.locator('.prose');
  }

  async goto(query?: string) {
    if (query) {
      await this.page.goto(`/search_page?q=${encodeURIComponent(query)}`);
    } else {
      await this.page.goto('/search_page?q=');
    }
  }

  async search(query: string) {
    await this.searchInput.fill(query);
    await this.submitButton.click();
  }

  async expectResultsVisible() {
    await expect(this.results).toBeVisible();
  }
}
