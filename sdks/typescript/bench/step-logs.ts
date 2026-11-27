// The cost of one captured logger call (sdks/SPEC.md §6.9, "The cost of a call"): an 8 KiB message redacted with a
// secret set of 1,024 values, formatted, cut and sized. Run by hand, never in CI:
//   npx tsc -p tsconfig.json && node dist/bench/step-logs.js
// 1,000 warm-up calls, then 10,000 measured on the monotonic clock; the p99 must be under 1 ms.
import { randomBytes } from 'node:crypto';
import { SecretSet, buildRecord, recordSize } from '../src/worker/step-logs.js';

const secrets = new SecretSet();
secrets.add(Array.from({ length: 1024 }, () => Buffer.from(randomBytes(12).toString('base64url'))));
const message = 'x'.repeat(8 * 1024 - 32) + ' an ordinary line of text';
const fields = { order: 'o-123', amount: 1999, currency: 'EUR' };
const call = (sequence: number): number =>
  recordSize(buildRecord(sequence, Date.now(), 'info', message, fields, undefined, secrets));

for (let n = 1; n <= 1000; n++) call(n);
const samples: number[] = [];
for (let n = 1; n <= 10_000; n++) {
  const start = process.hrtime.bigint();
  call(n);
  samples.push(Number(process.hrtime.bigint() - start) / 1e6);
}
samples.sort((a, b) => a - b);
const at = (q: number): string => samples[Math.min(samples.length - 1, Math.ceil(q * samples.length) - 1)]!.toFixed(4);
console.log(`typescript step-log call, 8 KiB message, 1,024 secrets: p50 ${at(0.5)} ms, p99 ${at(0.99)} ms, max ${at(1)} ms`);
