import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { CATALOG } from '../src/catalog.js';
import { SagatError, remediationOf, retryableByDefault, codeOfHttpStatus, type ErrorCode } from '../src/errors.js';

const here = dirname(fileURLToPath(import.meta.url));
// dist/test -> package root -> repository root
const pkg = join(here, '..', '..');
const root = join(pkg, '..', '..');

test('the embedded error catalog is the shared one', () => {
  const shared = JSON.parse(readFileSync(join(root, 'sdks', 'error-catalog.json'), 'utf8')) as { errors: unknown[] };
  assert.deepEqual(JSON.parse(JSON.stringify(CATALOG)), shared.errors, 'run `npm run sync`');
});

test('the worker protocol shipped with the package is the one of the contracts', () => {
  const contract = readFileSync(join(root, 'specs', '001-platform-foundation', 'contracts', 'worker.proto'));
  const shipped = readFileSync(join(pkg, 'proto', 'worker.proto'));
  assert.ok(contract.equals(shipped), 'run `npm run sync`');
});

test('every error has a remediation, a default and a code of its own', () => {
  const codes = new Set(CATALOG.map((entry) => entry.code));
  assert.equal(codes.size, CATALOG.length);
  for (const entry of CATALOG) {
    assert.ok(entry.remediation.length > 20, entry.code);
    assert.equal(remediationOf(entry.code as ErrorCode), entry.remediation);
    assert.equal(retryableByDefault(entry.code as ErrorCode), entry.retryable);
  }
});

test('an error carries the code, the trace and the remediation and says nothing else', () => {
  const error = new SagatError({ code: 'RATE_LIMITED', message: 'slow down', status: 429, traceId: 'a'.repeat(32), reason: 'TENANT', retryAfterMs: 2000 });
  assert.equal(error.retryable, true);
  assert.equal(error.remediation, remediationOf('RATE_LIMITED'));
  assert.deepEqual(Object.keys(error.toJSON()).sort(), ['code', 'message', 'reason', 'remediation', 'retryAfterMs', 'retryable', 'status', 'traceId']);
});

test('a status without an envelope is classified by the catalog', () => {
  assert.equal(codeOfHttpStatus(401), 'UNAUTHENTICATED');
  assert.equal(codeOfHttpStatus(404), 'NOT_FOUND');
  assert.equal(codeOfHttpStatus(409), 'CONFLICT');
  assert.equal(codeOfHttpStatus(422), 'VALIDATION_FAILED');
  assert.equal(codeOfHttpStatus(503), 'UNAVAILABLE');
  assert.equal(codeOfHttpStatus(500), 'INTERNAL');
  assert.equal(codeOfHttpStatus(418), 'BAD_REQUEST');
});
