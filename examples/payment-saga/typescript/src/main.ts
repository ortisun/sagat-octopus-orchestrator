// The payment saga served with the TypeScript SDK, against a payment provider that is a few lines of
// memory (templates/compensation is the workflow). It runs one of three days at the provider:
//
//   ok            the reservation is made and the charge works
//   decline       the card is declined for good: the saga releases the reservation
//   lost-answer   the provider charges but the answer is lost: the platform asks it (a reconciler)
//                 whether it happened, instead of charging again
//
//   SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
//   SAGAT_NAMESPACE=payments SAGAT_TOKEN=... npm start -- [ok|decline|lost-answer]
import { ActivityFailure, SagatClient, SagatError, WorkerHost, reconcile } from '@sagat-octopus-orchestrator/sdk';

type Payment = {
  paymentId: string;
  amount: number;
};

type Day = 'ok' | 'decline' | 'lost-answer';

/** The provider. Every call that makes an effect is keyed, and a repeat of a key does nothing more. */
class Provider {
  readonly reservations = new Map<string, { id: string; released: boolean }>();
  readonly charges = new Map<string, { id: string; refunded: boolean }>();

  constructor(private readonly day: Day) {}

  reserve(key: string, payment: Payment): string {
    const kept = this.reservations.get(key) ?? { id: `r-${payment.paymentId}`, released: false };
    this.reservations.set(key, kept);
    return kept.id;
  }

  release(key: string): void {
    const kept = this.reservations.get(key);
    if (kept !== undefined) kept.released = true;
  }

  charge(key: string, payment: Payment): string {
    const known = this.charges.get(key);
    if (known !== undefined) return known.id;
    if (this.day === 'decline') throw ActivityFailure.permanent('CARD_DECLINED', 'the card was declined');
    const made = { id: `ch-${payment.paymentId}`, refunded: false };
    this.charges.set(key, made);
    if (this.day === 'lost-answer') {
      // The provider has the money and the worker never heard: the outcome is unknown, not a failure.
      console.log(`charge ${payment.paymentId}: taken by the provider, the answer was lost`);
      throw ActivityFailure.outcomeUnknown('no answer from the provider');
    }
    return made.id;
  }

  refund(key: string): void {
    const made = this.charges.get(key);
    if (made !== undefined) made.refunded = true;
  }

  find(key: string): string | undefined {
    return this.charges.get(key)?.id;
  }

  ledger(): string {
    const count = (items: Iterable<{ released?: boolean; refunded?: boolean }>, flag: 'released' | 'refunded') =>
      [...items].filter((i) => i[flag] === true).length;
    return (
      `ledger: reservations=${this.reservations.size} released=${count(this.reservations.values(), 'released')} ` +
      `charges=${this.charges.size} refunded=${count(this.charges.values(), 'refunded')}`
    );
  }
}

/** The status each day is meant to end with. */
const EXPECTED: Record<Day, string> = { ok: 'SUCCEEDED', decline: 'COMPENSATED', 'lost-answer': 'SUCCEEDED' };

async function run(): Promise<void> {
  const apiUrl = setting('SAGAT_SERVER');
  const grpcUrl = setting('SAGAT_GRPC');
  const namespace = setting('SAGAT_NAMESPACE');
  const token = setting('SAGAT_TOKEN');
  const day = (process.argv[2] ?? 'ok') as Day;
  if (!(day in EXPECTED)) {
    console.error('the day at the provider is ok, decline or lost-answer');
    process.exit(2);
  }
  const payment: Payment = { paymentId: 'p-1', amount: 10 };
  const provider = new Provider(day);

  const interrupted = new AbortController();
  process.once('SIGINT', () => interrupted.abort());

  const host = new WorkerHost({ grpcUrl, namespace, token, workerName: 'payment-saga-typescript' });
  // Forward steps. ctx.idempotencyKey is the same for every attempt of a step: it is the key the provider keeps.
  host.registerActivity<Payment, { reservationId: string }>('limit.reserve', (input, ctx) => {
    const reservationId = provider.reserve(ctx.idempotencyKey, input);
    console.log(`reserve ${input.paymentId}: ${reservationId} for ${input.amount}`);
    return { reservationId };
  });
  host.registerActivity<Payment, { chargeId: string }>('payment.charge', (input, ctx) => {
    try {
      const chargeId = provider.charge(ctx.idempotencyKey, input);
      console.log(`charge ${input.paymentId}: ${chargeId}`);
      return { chargeId };
    } catch (error) {
      if (error instanceof ActivityFailure && error.code === 'CARD_DECLINED') {
        console.log(`charge ${input.paymentId}: declined (${error.code})`);
      }
      throw error;
    }
  });
  host.registerActivity<Payment, { notified: boolean }>('customer.notify', (input) => {
    console.log(`notify ${input.paymentId}`);
    return { notified: true };
  });
  // The undo of a step: it is told which effect it undoes (the key of the forward step), and it
  // tolerates an effect that never happened.
  host.registerCompensation('limit.release', (input: Payment, ctx) => {
    provider.release(ctx.targetIdempotencyKey);
    console.log(`release ${input.paymentId}: the reservation is released`);
    return { released: true };
  });
  host.registerCompensation('payment.refund', (input: Payment, ctx) => {
    provider.refund(ctx.targetIdempotencyKey);
    console.log(`refund ${input.paymentId}`);
    return { refunded: true };
  });
  // Whether an effect whose outcome is unknown happened: exactly a verdict.
  host.registerReconciler('payment.find-charge', (input: Payment, ctx) => {
    const chargeId = provider.find(ctx.targetIdempotencyKey);
    console.log(`find-charge ${input.paymentId}: ${chargeId === undefined ? 'not applied' : `applied ${chargeId}`}`);
    return chargeId === undefined ? reconcile.notApplied() : reconcile.applied({ chargeId });
  });
  await host.start();
  try {
    console.log(`serving the payment saga at protocol ${host.protocolVersion}`);
    const api = new SagatClient({ apiUrl, namespace, token });
    const started = await api.start('payment-saga', payment);
    console.log(`run ${started.runId} started`);
    const done = await api.waitForCompletion(started.runId, { timeoutMs: 60_000, signal: interrupted.signal });
    console.log(provider.ledger());
    console.log(`run ${done.runId} ${done.status}`);
    if (done.status !== EXPECTED[day]) throw new Error(`the run ended ${done.status}, expected ${EXPECTED[day]}`);
  } finally {
    await host.stop(10_000).catch(() => undefined);
  }
}

function setting(name: string): string {
  const value = process.env[name];
  if (value === undefined || value === '') {
    console.error(`${name} is not set (see the README)`);
    process.exit(2);
  }
  return value;
}

try {
  await run();
} catch (error) {
  if (error instanceof SagatError) {
    // The code, the trace and what to do about it; never the payload or the token.
    console.error(`${error.code} (trace ${error.traceId}): ${error.remediation}`);
  } else {
    console.error(error instanceof Error ? error.message : String(error));
  }
  process.exitCode = 1;
}
