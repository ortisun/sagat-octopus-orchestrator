import { defineConfig } from '@playwright/test';

const origin = process.env['STUDIO_ORIGIN'] ?? 'http://127.0.0.1:8080';

export default defineConfig({
  testDir: './tests',
  // One platform, one browser at a time: the journeys share the platform and create what they need with unique names.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: [['line']],
  use: {
    baseURL: `${origin}/studio/`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    // The page is served by the control plane: a strict policy, no inline script. A violation is a failure.
    bypassCSP: false,
    locale: 'en-US',
    timezoneId: 'UTC',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
