import { defineConfig, devices } from '@playwright/test';

/**
 * The website's screenshots, not a test suite: `shots.spec.ts` renders the
 * real interface from payloads the real backend produced (`dump.sh`) and
 * writes PNGs. Separate from `../playwright.config.ts` so the suite never
 * runs it and it never gates anything. WebKit, the engine closest to the
 * WebKitGTK the Linux app draws with.
 */
export default defineConfig({
  testDir: '.',
  timeout: 60_000,
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:5199',
    timezoneId: 'Europe/Berlin',
    locale: 'en-GB',
    viewport: { width: 1920, height: 1200 },
    deviceScaleFactor: 1,
  },
  webServer: {
    command: 'npx vite --port 5199 --strictPort',
    cwd: '..',
    url: 'http://localhost:5199/tests/harness/index.html',
    reuseExistingServer: true,
  },
  projects: [{ name: 'webkit', use: { ...devices['Desktop Safari'], viewport: { width: 1920, height: 1200 }, deviceScaleFactor: 1 } }],
});
