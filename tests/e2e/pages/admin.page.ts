import { type Page, type Locator, expect } from '@playwright/test';
import { BasePage } from './base.page';

export class AdminPage extends BasePage {
  readonly heading: Locator;
  readonly successAlert: Locator;
  readonly viewReferenceLink: Locator;

  constructor(page: Page) {
    super(page);
    this.heading = page.locator('h1');
    this.successAlert = page.locator('.alert-success');
    this.viewReferenceLink = page.getByRole('link', { name: 'View Generated Reference' });
  }

  async goto() {
    await this.page.goto('/admin_generate_docs');
  }

  async expectLoaded() {
    await expect(this.heading).toContainText('Doc Generation');
  }

  async expectSuccess() {
    await expect(this.successAlert).toBeVisible();
    await expect(this.successAlert).toContainText('generated');
  }
}
