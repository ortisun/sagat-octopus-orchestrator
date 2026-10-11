// Start a run, wait for it, and handle the canonical error.
import { SagatClient, SagatError } from '../src/index.js';

const client = new SagatClient({
  apiUrl: 'https://octopus.example.com/v1',
  namespace: 'payments',
  // A function is called before every request, so a token that rotates is always fresh.
  token: () => process.env['OCTOPUS_TOKEN'] ?? '',
});

try {
  // The key makes the call safe to repeat: the SDK retries a lost answer with the same key and the
  // platform answers the run it already created.
  const run = await client.start('charge-card', { orderId: 'o-1', amount: '19.90' }, {
    idempotencyKey: 'order-o-1-start',
    correlationId: 'order-o-1',
  });
  console.log(run.runId, run.status, run.traceId);

  const done = await client.waitForCompletion(run.runId, { timeoutMs: 60_000 });
  console.log(done.status);
} catch (error) {
  if (error instanceof SagatError) {
    // code, status, retryable, traceId, reason and a remediation sentence; never the payload or a token.
    console.error(error.code, error.traceId, error.remediation);
  } else {
    throw error;
  }
}
