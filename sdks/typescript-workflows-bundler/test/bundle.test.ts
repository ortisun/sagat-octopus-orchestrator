import { strict as assert } from 'node:assert';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { WorkflowTestEnvironment, codeDigest, runtimeOf, verifyManifest } from '@sagat-octopus-orchestrator/sdk/workflows';
import { WorkflowImportRefused, bundleWorkflows } from '../src/index.js';

const FIXTURES = fileURLToPath(new URL('../../test/fixtures/', import.meta.url));
const fixture = (name: string): string => join(FIXTURES, name);

test('an import of a builtin with or without the node prefix is refused at bundling', async () => {
  for (const [file, specifier] of [['builtin-bare.ts', 'fs'], ['builtin-prefixed.ts', 'node:crypto']]) {
    await assert.rejects(bundleWorkflows({ entry: fixture(file!) }), (error: unknown) => {
      assert.ok(error instanceof WorkflowImportRefused, String(error));
      assert.deepEqual(error.specifiers, [specifier]);
      assert.match(error.message, /may not import the built-in module/);
      return true;
    });
  }
});

test('the activity side of the SDK and the modules the bundle refuses are refused at bundling', async () => {
  await assert.rejects(bundleWorkflows({ entry: fixture('activity-side.ts') }),
    (error: unknown) => error instanceof WorkflowImportRefused && error.specifiers[0] === '@sagat-octopus-orchestrator/sdk');
  await assert.rejects(bundleWorkflows({ entry: fixture('uses-activities.ts'), refuse: ['./activities'] }),
    (error: unknown) => error instanceof WorkflowImportRefused && error.specifiers[0] === './activities/payments.js');
  // Without the refusal the module is ordinary code and bundles.
  await assert.doesNotReject(bundleWorkflows({ entry: fixture('uses-activities.ts') }));
});

test('the bundle runs its workflows with the SDK runtime and replays', async () => {
  const { code, manifest } = await bundleWorkflows({ entry: fixture('order.ts') });
  assert.deepEqual(manifest.workflows.map((workflow) => workflow.workflowType), ['order', 'refund']);
  const env = new WorkflowTestEnvironment({
    bundle: { code }, workflowType: 'order',
    activities: { 'inventory.reserve': () => 'r-1', 'payments.charge': (input) => ({ paid: (input as { amount: number }).amount }) },
  }).start({ sku: 'A-1', quantity: 2 });
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED');
  assert.deepEqual((outcome.output as { charged: unknown }).charged, { paid: 2500 });
  assert.equal((await env.replay()).verdict, 'MATCH');
  const empty = new WorkflowTestEnvironment({ bundle: { code }, workflowType: 'order', activities: { 'inventory.reserve': () => 'r-1' } }).start({ sku: 'A-1', quantity: 0 });
  assert.deepEqual((await empty.runUntilBlocked()).failure, { code: 'EMPTY_ORDER', message: 'nothing to charge' });
});

test('two builds of the same code are the same bytes, and the manifest names their digests', async () => {
  const first = await bundleWorkflows({ entry: fixture('order.ts') });
  const second = await bundleWorkflows({ entry: fixture('order.ts') });
  assert.equal(first.code, second.code);
  assert.equal(first.manifest.runtime, runtimeOf());
  const verified = verifyManifest(first.manifest, second.code);
  for (const workflow of verified) {
    assert.equal(workflow.digest, codeDigest({ runtime: runtimeOf(), sdkVersion: first.manifest.sdkVersion, workflowType: workflow.workflowType, content: first.manifest.content }));
  }
  // The runtime replaces the globals before any workflow module: it is the bundle's first module.
  assert.ok(first.code.indexOf('__sagatInstall') < first.code.indexOf('priceOf'));
  assert.doesNotMatch(first.code, new RegExp(FIXTURES.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), 'no absolute path of the build machine');
});

test('a workflow module that reads the clock at its top level is refused', async () => {
  await assert.rejects(bundleWorkflows({ entry: fixture('toplevel-clock.ts') }), /available only inside a workflow function/);
});

test('the sagat-workflows command writes the bundle and the manifest', () => {
  const out = mkdtempSync(join(tmpdir(), 'sagat-workflows-'));
  try {
    const cli = fileURLToPath(new URL('../src/cli.js', import.meta.url));
    const run = spawnSync(process.execPath, [cli, 'bundle', fixture('order.ts'), '--out', out], { encoding: 'utf8', env: { ...process.env, TZ: 'UTC' } });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /^order sha256:[0-9a-f]{64}$/m);
    const manifest = JSON.parse(readFileSync(join(out, 'workflows.manifest.json'), 'utf8'));
    assert.doesNotThrow(() => verifyManifest(manifest, readFileSync(join(out, 'workflows.bundle.js'))));
    const refused = spawnSync(process.execPath, [cli, 'bundle', fixture('builtin-bare.ts'), '--out', out], { encoding: 'utf8' });
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /may not import the built-in module fs/);
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});
