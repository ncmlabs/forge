import { type Page, type Locator, expect } from '@playwright/test';

export class BasePage {
  readonly page: Page;
  readonly navbar: Locator;
  readonly homeLink: Locator;
  readonly docsLink: Locator;
  readonly searchLink: Locator;
  readonly askLink: Locator;
  readonly themeToggle: Locator;

  constructor(page: Page) {
    this.page = page;
    this.navbar = page.locator('.navbar');
    this.homeLink = this.navbar.getByRole('link', { name: 'Home' });
    this.docsLink = this.navbar.getByRole('link', { name: 'Docs' });
    this.searchLink = this.navbar.getByRole('link', { name: 'Search' });
    this.askLink = this.navbar.getByRole('link', { name: 'Ask' });
    this.themeToggle = this.navbar.locator('button');
  }

  async expectNavVisible() {
    await expect(this.navbar).toBeVisible();
    await expect(this.homeLink).toBeVisible();
    await expect(this.docsLink).toBeVisible();
    await expect(this.searchLink).toBeVisible();
    await expect(this.askLink).toBeVisible();
  }

  async getTheme(): Promise<string | null> {
    return this.page.locator('html').getAttribute('data-theme');
  }

  async toggleTheme() {
    await this.themeToggle.click();
  }
}
