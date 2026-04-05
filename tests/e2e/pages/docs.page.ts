import { type Page, type Locator, expect } from '@playwright/test';
import { BasePage } from './base.page';

export class DocsPage extends BasePage {
  readonly sidebar: Locator;
  readonly sidebarLinks: Locator;
  readonly article: Locator;
  readonly codeBlocks: Locator;

  constructor(page: Page) {
    super(page);
    this.sidebar = page.locator('aside');
    this.sidebarLinks = this.sidebar.locator('.menu a');
    this.article = page.locator('article.prose');
    this.codeBlocks = this.article.locator('pre code');
  }

  async goto(slug: string) {
    await this.page.goto(`/docs?slug=${slug}`);
  }

  async expectLoaded() {
    await expect(this.article).toBeVisible();
  }

  async expectSidebarVisible() {
    await expect(this.sidebar).toBeVisible();
  }

  async expectSidebarHidden() {
    await expect(this.sidebar).toBeHidden();
  }
}
