import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}', 'test/**/*.test.{ts,tsx}'],
    css: false,
    // Bound jsdom contention on shared developer and continuous integration machines.
    maxWorkers: 4,
    // A test that types through the design system's inputs costs more per keystroke in jsdom than one
    // through a bare input (Architecture decision 0101). Measured on 2026-11-21: the Iceberg form tests
    // take about 1.1 seconds on a developer machine and passed 5 seconds on the continuous integration
    // runner, which is several times slower. The limit is raised for every test; no assertion is changed.
    testTimeout: 20_000,
  },
});
