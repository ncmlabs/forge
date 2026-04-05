import { type Page, type Locator, expect } from '@playwright/test';
import { BasePage } from './base.page';

export class HomePage extends BasePage {
  readonly hero: Locator;
  readonly heroTitle: Locator;
  readonly featureCards: Locator;
  readonly getStartedButton: Locator;

  constructor(page: Page) {
    super(page);
    this.hero = page.locator('.hero').first();
    this.heroTitle = this.hero.locator('h1');
    this.featureCards = page.locator('.card.shadow-xl');
    this.getStartedButton = this.hero.getByRole('link', { name: 'Get Started' });
  }

  async goto() {
    await this.page.goto('/home');
  }

  async expectLoaded() {
    await expect(this.heroTitle).toBeVisible();
    await expect(this.featureCards.first()).toBeVisible();
  }
}
