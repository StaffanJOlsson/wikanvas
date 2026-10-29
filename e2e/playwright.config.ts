// Playwright configuration for Wikanvas (docs/ARCHITECTURE.md §4.3, §14). Run it only through
// `scripts/e2e.sh <frame|smoke>`, which takes the build lock and runs Playwright from the repository root inside
// mcr.microsoft.com/playwright:v1.64.1-noble under the build caps (4 CPUs, 6 GiB).
//   frame: the Inkboard frame page (PLAN W5), served statically from web/dist-inkboard by serve-static.mjs.
//   smoke: the production image in the compose e2e project (PLAN W7), reached directly over plain HTTP on the project
//          network as http://wikanvas:8080; Caddy is not part of e2e runs.
import { defineConfig, devices } from '@playwright/test';

// WebGL2 without a GPU, for Inkboard's canvas in headless Chromium.
const chromium = {
  ...devices['Desktop Chrome'],
  launchOptions: { args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] },
};

export default defineConfig({
  testDir: 'tests',
  outputDir: 'test-results',
  workers: 2,
  retries: 0,
  forbidOnly: true,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report', open: 'never' }]],
  use: { trace: 'retain-on-failure' },
  projects: [
    {
      name: 'frame',
      testMatch: ['frame.spec.ts', 'frame/**/*.spec.ts'],
      use: { ...chromium, baseURL: 'http://127.0.0.1:4173' },
    },
    {
      name: 'smoke',
      testMatch: ['smoke.spec.ts', 'smoke/**/*.spec.ts'],
      use: { ...chromium, baseURL: 'http://wikanvas:8080' },
    },
  ],
  // Started for every project; serve-static answers 404 while web/dist-inkboard does not exist (smoke runs).
  webServer: {
    command: 'node e2e/serve-static.mjs web/dist-inkboard 4173',
    cwd: '..', // the repository root (Playwright resolves cwd against this file's directory)
    port: 4173,
    reuseExistingServer: false,
    stdout: 'pipe',
  },
});
