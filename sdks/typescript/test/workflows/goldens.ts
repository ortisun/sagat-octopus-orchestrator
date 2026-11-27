/*
 * The reference functions of the golden corpus (`specs/044-workflow-as-code/contracts/goldens/scenarios/<scenario>/
 * function.md`), written in TypeScript against the SDK's workflow API. The corpus is the server lane's: these functions
 * are this SDK's reading of its pseudo-code, and the tests prove that they make exactly the corpus's commands and chain.
 * A workflow function enters its bundle as its source text, so each one names only `ctx`, its input and the runtime's
 * own classes (`WorkflowFailure`, `SagaCompensated`, `CommandBatchLimitExceeded`, `ActivityError`).
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defaultCorpusDir, type GoldenCommands } from '../fake-workflow-server.js';
import type { TestWorkflow } from './support.js';

/** A scenario run: its directory under `scenarios/`, and its reference function per code version (`v1`, `v2`). */
export interface ReferenceRun {
  scenario: string;
  run: string;
  versions: Record<string, TestWorkflow | string>;
  /**
   * Why the run cannot be served as the corpus stands, for a defect of the corpus that was relayed to the server lane;
   * the test then proves the defect is still there, so that it is noticed when the corpus is fixed.
   */
  corpusDefect?: string;
  /** The state's `maxBytes` of the run's version where the corpus declares a state without giving it (see corpusDefect). */
  stateMaxBytes?: number;
}

const order: TestWorkflow = async (ctx, input) => {
  const [reservation, authorization] = (await Promise.all([
    ctx.activity('inventory.reserve', { sku: input.sku, quantity: input.quantity },
      { retry: { maxAttempts: 3, initialDelayMs: 1000, strategy: 'exponential' }, timeoutMs: 30000 }),
    ctx.activity('payments.authorize', { orderId: input.orderId, amountCents: input.amountCents }, { timeoutMs: 30000 }),
  ])) as [{ reservationId: string }, { authorizationId: string }];
  await ctx.sleep(60000);
  return { orderId: input.orderId, reservationId: reservation.reservationId, authorizationId: authorization.authorizationId };
};

const cart: TestWorkflow = async (ctx, input) => {
  const items: string[] = [];
  ctx.onSignal('item.added', (payload: any) => { items.push(payload.sku); });
  await ctx.activity('cart.open', { cartId: input.cartId });
  const payment = await ctx.signal('checkout');
  return { cartId: input.cartId, items, checkout: payment };
};

const report: TestWorkflow = async (ctx, input) => {
  const notes: string[] = [];
  ctx.onSignal('note', (payload: any) => { notes.push(payload.text); });
  const built = (await ctx.activity('report.build', { reportId: input.reportId })) as { url: string };
  return { reportId: input.reportId, url: built.url, notes: [...notes] };
};

const creditLine: TestWorkflow = async (ctx, input) => {
  let limit = input.limit;
  ctx.onUpdate('raiseLimit', (args: any) => { if (!(args.amount > 0)) throw new Error('ValidationFailed'); }, async (args: any) => {
    const applied = (await ctx.activity('limits.apply', { accountId: input.accountId, amount: args.amount })) as { limit: number };
    limit = applied.limit;
    return { limit: applied.limit };
  });
  await ctx.sleep(3600000);
  return { accountId: input.accountId, limit };
};

const basket: TestWorkflow = async (ctx, input) => {
  const items: string[] = [];
  ctx.onUpdate('addItem', (args: any) => { if (items.includes(args.sku)) throw new Error('ValidationFailed'); }, async (args: any) => {
    items.push(args.sku);
    return { items: items.length };
  });
  await ctx.sleep(600000);
  return { basketId: input.basketId, items };
};

const fulfilment: TestWorkflow = async (ctx, input) => {
  ctx.setState({ status: 'reserving', items: 0 });
  const reserved = (await ctx.activity('inventory.reserve', { orderId: input.orderId })) as { items: number };
  ctx.setState({ status: 'charging', items: reserved.items });
  const charged = (await ctx.activity('payments.charge', { orderId: input.orderId })) as { receiptId: string };
  ctx.setState({ status: 'done', items: reserved.items, receiptId: charged.receiptId });
  return { orderId: input.orderId, receiptId: charged.receiptId };
};

const checkout = `async (ctx, input) => {
  try {
    await ctx.activity('payments.charge', { orderId: input.orderId, amountCents: input.amountCents });
  } catch (error) {
    if (error instanceof ActivityError) throw new WorkflowFailure('PAYMENT_DECLINED', 'The card was declined');
    throw error;
  }
  return { orderId: input.orderId };
}`;

const invoice = `async (ctx, input) => {
  class InvoiceMissing extends Error {}
  const loaded = await ctx.activity('invoice.load', { invoiceId: input.invoiceId });
  if (loaded.invoice == null) throw new InvoiceMissing();
  return loaded.invoice;
}`;

const reportRender: TestWorkflow = async (ctx, input) => {
  if (input.idle) { await ctx.sleep(604800000); return {}; }
  const render = ctx.activity('report.render', { reportId: input.reportId });
  await Promise.race([render, ctx.cancellation.promise]);
  if (ctx.cancellation.requested) {
    ctx.requestCancellation(render);
    await ctx.activity('report.cleanup', { reportId: input.reportId });
    ctx.acknowledgeCancellation();
  }
  return await render;
};

const trip: TestWorkflow = async (ctx, input) => {
  await ctx.saga({ onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' }, async (scope) => {
    await scope.activity('trip.hold', { tripId: input.tripId });
    await Promise.race([ctx.sleep(86400000), ctx.cancellation.promise]);
    if (ctx.cancellation.requested) ctx.acknowledgeCancellation();
  });
  return null;
};

const refundReview: TestWorkflow = async (ctx, input) => {
  const confirmed = ctx.waitForEvent('refund.confirmed', input.refundId, { timeoutMs: 86400000 });
  const review = ctx.approval('refund-review', { refundId: input.refundId, amountCents: 4200 });
  await Promise.race([confirmed, review, ctx.cancellation.promise]);
  if (ctx.cancellation.requested) {
    ctx.requestCancellation(confirmed);
    ctx.requestCancellation(review);
    await Promise.allSettled([confirmed, review]);
    ctx.acknowledgeCancellation();
  }
  return null;
};

const subscription: TestWorkflow = async (ctx, input) => {
  if (input.cycle >= 2) return { cycles: input.cycle, charged: (ctx.carriedState() as { charged: number[] }).charged };
  if (input.wait) {
    await ctx.sleep(1000);
    ctx.continueAsNew({ cycle: 2, amountCents: input.amountCents });
  }
  void ctx.activity('audit.write', { cycle: input.cycle });
  const charge = ctx.activity('billing.charge', { cycle: input.cycle, amountCents: input.amountCents });
  const deadline = ctx.sleep(30000);
  const winner = (await Promise.race([charge, deadline])) as { charged: number };
  ctx.requestCancellation(deadline);
  ctx.continueAsNew({ cycle: input.cycle + 1, amountCents: input.amountCents }, { version: 'active', state: { charged: [winner.charged] } });
};

const booking = `async (ctx, input) => {
  const options = { onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' };
  await ctx.saga(options, async (scope) => {
    await scope.activity('inventory.reserve', { orderId: input.orderId });
    await scope.activity('payments.charge', { orderId: input.orderId, amountCents: 12000 });
  });
  try {
    await ctx.saga(options, async (scope) => {
      const hotel = await scope.activity('hotel.book', { tripId: input.tripId });
      await scope.activity('car.rent', { tripId: input.tripId, hotelBookingId: hotel.hotelBookingId });
      await scope.activity('flight.book', { tripId: input.tripId });
    });
  } catch (error) {
    if (error instanceof SagaCompensated) return { orderId: input.orderId, order: 'charged', trip: 'compensated' };
    throw error;
  }
  return { orderId: input.orderId, order: 'charged', trip: 'booked' };
}`;

const settlement: TestWorkflow = async (ctx, input) => {
  await ctx.saga({ onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' }, async (scope) => {
    const charge = (await scope.activity('payments.charge', { invoiceId: input.invoiceId, amountCents: input.amountCents })) as { chargeId: string };
    await scope.activity('ledger.post', { invoiceId: input.invoiceId, chargeId: charge.chargeId });
  });
  return { invoiceId: input.invoiceId };
};

const payouts: TestWorkflow = async (ctx, input) => {
  const first = ctx.activity('payouts.send', { payoutId: 'p-1', amountCents: 250000 });
  const second = ctx.activity('payouts.send', { payoutId: 'p-2', amountCents: 990000 });
  await Promise.allSettled([first, second]);
  const refund = await ctx.approval('refund-review', { refundId: 'r-9', amountCents: 1200 });
  return { batchId: input.batchId, sent: ['p-1'], rejected: ['p-2'], refund };
};

const shipment: TestWorkflow = async (ctx, input) => {
  const result = (await ctx.child('bulk-labels', { orders: ['ord-1', 'ord-2', 'ord-3'] },
    { cancelPolicy: 'propagate', businessId: 'labels-batch-42' })) as { printed: number };
  return { batchId: input.batchId, printed: result.printed };
};

const labelPrint: TestWorkflow = async (ctx, input) => {
  const label = (await ctx.activity('labels.render', { orderId: input.orderId, format: 'zpl' })) as { labelId: string };
  return { orderId: input.orderId, labelId: label.labelId };
};

const meter: TestWorkflow = async (ctx, input) => {
  let readings = 0;
  let total = 0;
  ctx.onSignal('reading', (payload: any) => { readings += 1; total += payload.value; });
  await ctx.signal('close');
  return { meterId: input.meterId, readings, total };
};

const notifyOrder = (channel: 'email.send' | 'sms.send'): string => `async (ctx, input) => {
  const order = await ctx.activity('order.load', { orderId: input.orderId });
  await ctx.activity('${channel}', { orderId: input.orderId, to: order.email });
  return { orderId: input.orderId };
}`;

const purchaseV1: TestWorkflow = async (ctx, input) => {
  const quote = (await ctx.activity('quote.fetch', { orderId: input.orderId })) as { quoteId: string };
  await ctx.activity('order.place', { orderId: input.orderId, quoteId: quote.quoteId });
  await ctx.sleep(86400000);
  return { orderId: input.orderId, status: 'placed' };
};

const purchaseV2: TestWorkflow = async (ctx, input) => {
  const quote = (await ctx.activity('quote.fetch', { orderId: input.orderId })) as { quoteId: string };
  if (ctx.patched('use-fraud-check')) await ctx.activity('fraud.check', { orderId: input.orderId, quoteId: quote.quoteId });
  await ctx.activity('order.place', { orderId: input.orderId, quoteId: quote.quoteId });
  await ctx.activity('notify.send', { orderId: input.orderId });
  await ctx.sleep(86400000);
  return { orderId: input.orderId, status: 'placed' };
};

const primitives: TestWorkflow = async (ctx) => {
  const firstNow = ctx.now();
  const numbers: number[] = [];
  for (let i = 0; i < 64; i++) numbers.push(ctx.random());
  const bounded = [1, 2, 3, 6, 10, 1000, 2 ** 31, 2 ** 32].map((n) => {
    const values: number[] = [];
    for (let i = 0; i < 8; i++) values.push(ctx.randomInt(n));
    return { n, values };
  });
  const ids: string[] = [];
  for (let i = 0; i < 8; i++) ids.push(ctx.uuid());
  await ctx.sleep(1000);
  const secondNow = ctx.now();
  return { nowMs: [firstNow.getTime(), secondNow.getTime()], random: numbers, randomInt: bounded, uuid: ids };
};

const notifyAll = `async (ctx, input) => {
  const started = [];
  let overflowFrom = null;
  try {
    for (let i = 1; i <= input.recipients; i++) started.push(ctx.activity('notify.send', { recipient: i }));
  } catch (error) {
    if (!(error instanceof CommandBatchLimitExceeded)) throw error;
    overflowFrom = started.length + 1;
  }
  await Promise.all(started);
  await ctx.child('notify-fanout', { from: overflowFrom, to: input.recipients }, { cancelPolicy: 'propagate' });
  return { sent: started.length, delegated: input.recipients - overflowFrom + 1 };
}`;

const feed = `async (ctx, input) => {
  const polled = await ctx.activity('feed.poll', { feedId: input.feedId });
  for (;;) { /* never yields */ }
  return polled;
}`;

const warehouseImport: TestWorkflow = async (ctx, input) => {
  const notes: string[] = [];
  ctx.onSignal('audit.note', (payload: any) => { notes.push(payload.text); });
  const rows = { rows: Array.from({ length: input.rowCount }, (_, i) => ({ i, label: `row-${i}` })) };
  await ctx.activity('warehouse.import', rows, { timeoutMs: 600000 });
  return { rows: Array.from({ length: input.rowCount }, (_, i) => ({ i, label: `out-${i}` })) };
};

const paymentFlow: TestWorkflow = async (ctx, input) => {
  const requestId = ctx.uuid();
  await ctx.activity('payments.charge', { orderId: input.orderId, requestId });
  const fx = ctx.sideEffect(() => ({ pair: 'EURUSD', rate: 1.0842 }), { id: 'fx-rate' });
  const settled = (await ctx.waitForEvent('payment.settled', input.orderId, { timeoutMs: 3600000, lookbackMs: 60000 })) as { settlementId: string };
  await ctx.script('sha256:c4a45a6892a43d9557dab01899a89444a99702f4c9eb4b86e79bead5bc544d59', { settlementId: settled.settlementId, rate: fx.rate });
  return { orderId: input.orderId, requestId, settlementId: settled.settlementId };
};

export const REFERENCE_RUNS: ReferenceRun[] = [
  { scenario: 'code-activation-deadline', run: 'main', versions: { v1: feed } },
  { scenario: 'code-blob-values', run: 'main', versions: { v1: warehouseImport } },
  { scenario: 'code-cache-eviction', run: 'main', versions: { v1: paymentFlow } },
  { scenario: 'code-cancellation', run: 'main', versions: { v1: reportRender } },
  { scenario: 'code-cancellation', run: 'acknowledge-in-saga', versions: { v1: trip } },
  { scenario: 'code-cancellation', run: 'grace-expires', versions: { v1: reportRender } },
  { scenario: 'code-cancellation', run: 'own-steps', versions: { v1: refundReview } },
  { scenario: 'code-command-batch-limit', run: 'main', versions: { v1: notifyAll } },
  {
    scenario: 'code-continue-as-new', run: 'main', versions: { v1: subscription }, stateMaxBytes: 131072,
    corpusDefect: 'the run carries a state into its successor while its version declares no state, so its lease carries state_max_bytes 0 and the carried state is over it',
  },
  { scenario: 'code-continue-as-new', run: 'moved', versions: { v1: subscription, v2: subscription } },
  { scenario: 'code-continue-as-new', run: 'successor', versions: { v2: subscription } },
  {
    scenario: 'code-declared-state', run: 'main', versions: { v1: fulfilment }, stateMaxBytes: 131072,
    corpusDefect: 'the corpus declares a state without its maxBytes and its lease carries none, so state_max_bytes is 0 and every setState is over it',
  },
  { scenario: 'code-deterministic-primitives', run: 'main', versions: { v1: primitives } },
  { scenario: 'code-failures', run: 'main', versions: { v1: checkout } },
  { scenario: 'code-failures', run: 'uncaught-exception', versions: { v1: invoice } },
  { scenario: 'code-gated-activity', run: 'main', versions: { v1: payouts } },
  { scenario: 'code-graph-mixed', run: 'main', versions: { v1: shipment } },
  { scenario: 'code-graph-mixed', run: 'grandchild', versions: { v1: labelPrint } },
  { scenario: 'code-history-paging', run: 'main', versions: { v1: meter } },
  { scenario: 'code-nondeterminism', run: 'main', versions: { v1: notifyOrder('email.send') } },
  { scenario: 'code-patch-and-move', run: 'main', versions: { v1: purchaseV1, v2: purchaseV2 } },
  { scenario: 'code-patch-and-move', run: 'unmatched', versions: { v1: purchaseV1 } },
  { scenario: 'code-run-lifecycle', run: 'main', versions: { v1: order } },
  { scenario: 'code-saga', run: 'main', versions: { v1: booking } },
  { scenario: 'code-saga-uncertain', run: 'main', versions: { v1: settlement } },
  { scenario: 'code-signal-at-return', run: 'main', versions: { v1: report } },
  { scenario: 'code-signals', run: 'main', versions: { v1: cart } },
  { scenario: 'code-update-with-start', run: 'main', versions: { v1: basket } },
  { scenario: 'code-updates', run: 'main', versions: { v1: creditLine } },
];

/** The changed code the second worker of `code-nondeterminism` runs under the same digest. */
export const NONDETERMINISM_CHANGED = notifyOrder('sms.send');
/** The second version of `code-patch-and-move`, replayed against the unmatched run's history. */
export const PURCHASE_V2 = purchaseV2;

export function commandsOf(scenario: string, run: string): GoldenCommands {
  const directory = join(defaultCorpusDir(), 'scenarios', scenario, ...(run === 'main' ? [] : [run]));
  return JSON.parse(readFileSync(join(directory, 'commands.json'), 'utf8')) as GoldenCommands;
}

export function historyLinesOf(scenario: string, run: string): string[] {
  const directory = join(defaultCorpusDir(), 'scenarios', scenario, ...(run === 'main' ? [] : [run]));
  return readFileSync(join(directory, 'history.jsonl'), 'utf8').split('\n').filter((line) => line.length > 0);
}
