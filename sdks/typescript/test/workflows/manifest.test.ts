import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { ManifestMismatch, SDK_VERSION, codeDigest, manifestOf, runtimeOf, verifyManifest } from '../../src/workflows/index.js';

const BUNDLE = '(() => { /* a bundle */ })();\n';

test('the code digest is the SHA-256 of the canonical JSON of the six fields', () => {
  const content = createHash('sha256').update(BUNDLE).digest('hex');
  const canonical = `{"content":"${content}","language":"typescript","runtime":"node 22.11.0 icu 75.1","sdkName":"@sagat-octopus-orchestrator/sdk","sdkVersion":"0.2.0","workflowType":"order"}`;
  const expected = `sha256:${createHash('sha256').update(canonical).digest('hex')}`;
  assert.equal(codeDigest({ runtime: 'node 22.11.0 icu 75.1', sdkVersion: '0.2.0', workflowType: 'order', content }), expected);
  assert.match(expected, /^sha256:[0-9a-f]{64}$/);
});

test('the runtime names Node and its locale data', () => {
  assert.equal(runtimeOf({ node: '22.11.0', icu: '75.1' }), 'node 22.11.0 icu 75.1');
  assert.equal(runtimeOf({ node: '22.11.0' }), 'node 22.11.0');
});

test('a worker verifies the manifest against the bundle it loads, its Node and its SDK', () => {
  const manifest = manifestOf(BUNDLE, 'workflows.bundle.js', ['order', 'shipping'], 'node 22.11.0 icu 75.1');
  assert.equal(manifest.sdkVersion, SDK_VERSION);
  assert.deepEqual(verifyManifest(manifest, BUNDLE, 'node 22.11.0 icu 75.1').map((workflow) => workflow.workflowType), ['order', 'shipping']);
  const refused = (change: () => void, reason: string): void => {
    assert.throws(change, (error: unknown) => error instanceof ManifestMismatch && error.reason === reason, reason);
  };
  refused(() => verifyManifest(manifest, `${BUNDLE} `, 'node 22.11.0 icu 75.1'), 'CONTENT');
  refused(() => verifyManifest(manifest, BUNDLE, 'node 22.12.0 icu 75.1'), 'RUNTIME');
  refused(() => verifyManifest({ ...manifest, sdkVersion: '9.9.9' }, BUNDLE, 'node 22.11.0 icu 75.1'), 'SDK');
  refused(() => verifyManifest({ ...manifest, workflows: [{ workflowType: 'order', digest: `sha256:${'0'.repeat(64)}` }] }, BUNDLE, 'node 22.11.0 icu 75.1'), 'DIGEST');
  refused(() => verifyManifest({ ...manifest, manifestFormat: 2 }, BUNDLE, 'node 22.11.0 icu 75.1'), 'FORMAT');
});

test('a workflow type outside the pattern of Requirement 044-001 has no manifest', () => {
  assert.throws(() => manifestOf(BUNDLE, 'b.js', ['_private']), (error: unknown) => error instanceof ManifestMismatch && error.reason === 'WORKFLOW_TYPE');
  assert.throws(() => manifestOf(BUNDLE, 'b.js', ['x'.repeat(49)]), ManifestMismatch);
  assert.doesNotThrow(() => manifestOf(BUNDLE, 'b.js', ['x'.repeat(48)]));
});
