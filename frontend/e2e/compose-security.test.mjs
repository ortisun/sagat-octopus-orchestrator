import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const here = dirname(fileURLToPath(import.meta.url));
const sentinel = 'synthetic-onboarding-token-must-never-appear-in-artifacts';

async function filesBelow(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  return (await Promise.all(entries.map((entry) => entry.isDirectory()
    ? filesBelow(join(dir, entry.name)) : [join(dir, entry.name)]))).flat();
}

for (const failure of ['disabled field', 'rejected sign-in', 'failure after sign-in']) {
  test(`compose browser ${failure} keeps the token out of output and artifacts`, async () => {
    const dir = await mkdtemp(join(tmpdir(), 'sagat-browser-redaction-'));
    const server = createServer((_request, response) => {
      response.setHeader('content-type', 'text/html');
      response.end(`<label>Access token<textarea ${failure === 'disabled field' ? 'disabled' : ''}></textarea></label><button>Sign in</button>${failure === 'failure after sign-in' ? '<nav aria-label="Main">Signed in, but the token field was incorrectly retained</nav>' : ''}`);
    });
    await new Promise((resolveListening) => server.listen(0, '127.0.0.1', resolveListening));
    try {
      const address = server.address();
      assert.ok(address !== null && typeof address !== 'string');
      const output = join(dir, 'artifacts');
      const config = join(dir, 'playwright.config.ts');
      await writeFile(config, `
        import config from ${JSON.stringify(join(here, 'compose.config.ts'))};
        export default {
          ...config,
          testDir: ${JSON.stringify(join(here, 'compose'))},
          outputDir: ${JSON.stringify(output)},
          timeout: 10_000,
          expect: { timeout: 250 },
          use: { ...config.use, actionTimeout: 250 },
        };
      `);
      const child = spawn(process.execPath, [resolve(here, '../node_modules/playwright/cli.js'), 'test', '--config', config], {
        cwd: resolve(here, '..'),
        env: {
          ...process.env,
          STUDIO_ORIGIN: `http://127.0.0.1:${address.port}/studio/`,
          STUDIO_RUN_ID: '0199f1f0-0000-7000-8000-000000000001',
          STUDIO_TOKEN: sentinel,
          // The production compose config must opt out itself, even without an inherited setting.
          PLAYWRIGHT_NO_COPY_PROMPT: '',
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        timeout: 20_000,
      });
      let captured = '';
      child.stdout.on('data', (chunk) => { captured += chunk; });
      child.stderr.on('data', (chunk) => { captured += chunk; });
      const code = await new Promise((resolveExit, reject) => {
        child.on('error', reject);
        child.on('close', resolveExit);
      });
      assert.equal(code, 1, 'the login scenario must fail through the real Playwright runner');
      assert.equal(captured.includes(sentinel), false, 'runner output disclosed the synthetic token');
      for (const file of await filesBelow(output)) {
        assert.equal((await readFile(file)).includes(Buffer.from(sentinel)), false,
          `artifact ${file.slice(output.length + 1)} disclosed the synthetic token`);
      }
      const diagnosis = failure === 'failure after sign-in' ? /toHaveCount/ : /sign-in failed/i;
      assert.equal(diagnosis.test(captured), true, 'failure must retain a useful safe diagnosis');
    } finally {
      await new Promise((resolveClosed) => server.close(resolveClosed));
      await rm(dir, { recursive: true, force: true });
    }
  });
}
