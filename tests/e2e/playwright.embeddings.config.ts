import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './specs',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  retries: 0,
  reporter: 'list',

  use: {
    baseURL: 'http://localhost:3000',
  },

  projects: [
    {
      name: 'embeddings',
      use: { browserName: 'chromium' },
      testMatch: '**/embeddings.spec.ts',
    },
  ],

  webServer: {
    command: 'cargo run -- serve examples/wiki/server.forge -s examples/wiki/shared.forge',
    cwd: '../../',
    url: 'http://localhost:3000/home',
    reuseExistingServer: true,
    timeout: 120_000,
    env: {
      FORGE_MOCK: '1',
    },
  },
});
