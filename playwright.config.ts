import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/e2e', outputDir: './temp/playwright-results',
  timeout: 30_000, fullyParallel: false,
  use: { baseURL: 'http://127.0.0.1:5173', browserName: 'chromium', channel: 'msedge', viewport: { width: 1280, height: 800 }, headless: true, trace: 'retain-on-failure' },
  webServer: { command: 'npm run dev', url: 'http://127.0.0.1:5173', reuseExistingServer: true },
});
