import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  use: { baseURL: 'http://127.0.0.1:1421', channel: 'msedge', viewport: { width: 1440, height: 1000 }, headless: true },
  reporter: [['list'], ['json', { outputFile: 'test-results/e2e-results.json' }]],
  webServer: { command: 'node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1421 --strictPort', url: 'http://127.0.0.1:1421', reuseExistingServer: false, timeout: 20_000 },
});
