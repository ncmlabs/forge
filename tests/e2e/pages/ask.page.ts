import { type Page, type Locator, expect } from '@playwright/test';
import { BasePage } from './base.page';

export class AskPage extends BasePage {
  readonly questionInput: Locator;
  readonly submitButton: Locator;
  readonly answerCard: Locator;

  constructor(page: Page) {
    super(page);
    this.questionInput = page.locator('textarea[name="question"]');
    this.submitButton = page.getByRole('button', { name: 'Ask' });
    this.answerCard = page.locator('.card .card-body');
  }

  async goto() {
    await this.page.goto('/ask_form');
  }

  async askQuestion(question: string) {
    await this.questionInput.fill(question);
    await this.submitButton.click();
  }
}
