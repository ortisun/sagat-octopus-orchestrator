// A worker: an activity, its compensation and a reconciler, served until the process is told to stop.
import { ActivityFailure, WorkerHost, reconcile } from '../src/index.js';

// Stands for the payment provider. What matters is that it is given the idempotency key.
const provider = {
  async charge(amount: string, options: { idempotencyKey: string; signal: AbortSignal }): Promise<string> {
    void options;
    return `ch-${amount}`;
  },
  async refund(chargeKey: string): Promise<void> {
    void chargeKey;
  },
  async find(key: string): Promise<string | undefined> {
    void key;
    return undefined;
  },
};

const host = new WorkerHost({
  grpcUrl: 'https://octopus.example.com:443',
  namespace: 'payments',
  token: () => process.env['OCTOPUS_TOKEN'] ?? '',
  workerName: 'payments-worker-1',
  maxInflight: 16,
});

host.registerActivity<{ orderId: string; amount: string }, { chargeId: string }>(
  'payments.charge',
  async (input, ctx) => {
    // Stable across the attempts of this step: the provider answers the same charge to a repeat.
    const chargeId = await provider.charge(input.amount, { idempotencyKey: ctx.idempotencyKey, signal: ctx.signal });
    if (chargeId === '') throw ActivityFailure.permanent('CARD_DECLINED', 'the card was declined');
    return { chargeId };
  },
);

// The undo of a step: it is idempotent, and it is told which effect it undoes.
host.registerCompensation('payments.refund', async (_input, ctx) => {
  await provider.refund(ctx.targetIdempotencyKey);
  return { refunded: true };
});

// Whether an effect whose outcome is unknown happened: the answer is exactly a verdict.
host.registerReconciler('payments.find-charge', async (_input, ctx) => {
  const found = await provider.find(ctx.targetIdempotencyKey);
  return found === undefined ? reconcile.notApplied() : reconcile.applied({ chargeId: found });
});

await host.start();
console.log('serving at protocol', host.protocolVersion);

// Stop polling, let the tasks in flight finish (up to 30 s by default), then end.
process.on('SIGTERM', () => void host.stop());
