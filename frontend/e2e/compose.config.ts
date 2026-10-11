import { defineConfig } from '@playwright/test';

const origin = process.env['STUDIO_ORIGIN'];
if (origin === undefined || origin === '') throw new Error('STUDIO_ORIGIN is required');

// Playwright otherwise snapshots a failed sign-in's textarea into error-context.md even when
// screenshots, video and traces are disabled. The token may still be in that field on failure.
process.env['PLAYWRIGHT_NO_COPY_PROMPT'] = '1';

export default defineConfig({
  testDir: './compose',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  reporter: [['line']],
  outputDir: process.env['RUNNER_TEMP'] ? `${process.env['RUNNER_TEMP']}/sagat-compose-playwright` : '/tmp/sagat-compose-playwright',
  use: {
    baseURL: origin,
    actionTimeout: 10_000,
    trace: 'off',
    screenshot: 'off',
    video: 'off',
    bypassCSP: false,
    locale: 'en-US',
    timezoneId: 'UTC',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
