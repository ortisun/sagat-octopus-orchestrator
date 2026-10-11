// A workflow written with the builders: a saga whose steps name their undo, a wait for an event and a
// sleep. The builder has no side effects; `build()` returns the document to publish.
import { flow, workflow } from '../src/index.js';

const document = workflow('order-fulfilment')
  .description('Reserve, charge, then ship once the payment is confirmed')
  .label('team', 'payments')
  .saga(
    'order',
    { onCompensationFailure: 'manual_review' },
    flow()
      .step('reserve', 'inventory.reserve', { compensateWith: 'inventory.release' })
      .step('charge', 'payments.charge', {
        compensateWith: 'payments.refund',
        reconcileWith: 'payments.find-charge',
        retry: { maxAttempts: 3, initialDelayMs: 500, maxDelayMs: 5000, strategy: 'exponential', jitter: true },
      }),
  )
  .waitForEvent('confirmed', { eventType: 'payment.confirmed', correlationExpr: 'input.orderId', timeoutMs: 600_000 })
  .sleep('settle', 1_000)
  .step('ship', 'shipping.create')
  .build();

console.log(JSON.stringify(document, null, 2));
