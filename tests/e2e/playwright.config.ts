import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './specs',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [['html', { open: 'never' }]],

  use: {
    baseURL: 'http://localhost:3000',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { browserName: 'chromium' },
      testIgnore: ['**/warden-real.spec.ts', '**/sentinel-*.spec.ts'],
    },
    {
      name: 'real-api',
      use: { browserName: 'chromium' },
      testMatch: '**/warden-real.spec.ts',
    },
    {
      name: 'sentinel',
      use: {
        browserName: 'chromium',
        baseURL: 'http://localhost:3001',
      },
      testMatch: '**/sentinel-*.spec.ts',
      testIgnore: '**/sentinel-*-real.spec.ts',
    },
    {
      name: 'sentinel-real',
      use: {
        browserName: 'chromium',
        baseURL: 'http://localhost:3001',
      },
      testMatch: '**/sentinel-*-real.spec.ts',
    },
    {
      name: 'observer',
      use: {
        browserName: 'chromium',
        baseURL: 'http://localhost:3002',
      },
      testMatch: '**/observer-*.spec.ts',
    },
  ],

  webServer: [
    {
      command: 'cargo run -- serve examples/wiki/server.forge -s examples/wiki/shared.forge',
      cwd: '../../',
      url: 'http://localhost:3000/home',
      reuseExistingServer: true,
      timeout: 120_000,
      env: {
        // Real-LLM opt-in (#288): only forward the API key when the developer
        // has explicitly set FORGE_LLM_LIVE=1. Otherwise force mock mode even
        // if the key is present in the parent env.
        ...(process.env.FORGE_LLM_LIVE === '1' && process.env.ANTHROPIC_API_KEY
          ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
          : { FORGE_MOCK: '1' }),
      },
    },
    {
      command: 'cargo run -- serve examples/sentinel/server.forge -s examples/sentinel/shared.forge',
      cwd: '../../',
      url: 'http://localhost:3001/dashboard',
      reuseExistingServer: true,
      timeout: 120_000,
      env: {
        // Real-LLM opt-in (#288): only forward the API key when the developer
        // has explicitly set FORGE_LLM_LIVE=1. Otherwise force mock mode even
        // if the key is present in the parent env.
        ...(process.env.FORGE_LLM_LIVE === '1' && process.env.ANTHROPIC_API_KEY
          ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
          : { FORGE_MOCK: '1' }),
      },
    },
    {
      command: 'cargo run -- serve examples/observer/server.forge -s examples/observer/shared.forge',
      cwd: '../../',
      url: 'http://localhost:3002/static/index.html',
      reuseExistingServer: true,
      timeout: 120_000,
    },
  ],
});
