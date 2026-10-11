# Saga quickstart: rehearse a payment that fails

A saga is a sequence of steps that each have an undo. When a step fails for good, the platform runs the undo of the
steps that already happened, newest first, and the run ends `COMPENSATED`. When a step's answer is lost, the platform
does not guess: it asks you whether the effect happened (a **reconciliation**), and goes on from the answer.

The workflow is [`templates/compensation`](../../templates/compensation/workflow.json): inside a saga, `limit.reserve`
(undone by `limit.release`), then `payment.charge` (undone by `payment.refund`, looked up by `payment.find-charge`),
then `customer.notify`.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
sagat new compensation
sagat workflow publish compensation/workflow.json
```

## 1. With canned answers

```sh
sagat worker dev --mocks compensation/mocks.yaml --workflow compensation/workflow.json --bind
```

and, in another terminal:

```sh
sagat workflow run payment-saga --input '{"paymentId":"p-1","amount":10}' --wait
```

As shipped, `mocks.yaml` makes the card be declined for good (`fail: { code: CARD_DECLINED, retryable: false }`). The
journal shows `limit.release` running for the reservation and the run ends `COMPENSATED`. Change the mock and run again:

| To see | Change `payment.charge` in `mocks.yaml` to |
|---|---|
| It works | `output: { chargeId: ch-1 }` |
| A retry that works | `sequence: [ { fail: { code: PROVIDER_TIMEOUT, retryable: true } }, { output: { chargeId: ch-1 } } ]` |
| A worker that dies with the task | `sequence: [ { lose: true }, { output: { chargeId: ch-1 } } ]`: after the lease time (30 seconds by default, `SAGAT_LEASE_TTL_SECS` on the engine) the lease expires and the platform asks `payment.find-charge`, which the file answers with `effect: not_applied`, so it charges again with the same key |
| The provider charged and the answer was lost | the same, with the reconciler `effect: applied` and `output: { chargeId: ch-1 }`: the platform does not charge again |
| A compensation that fails | add `limit.release` under `compensations:` with `fail: { code: X, retryable: true }`: after its retries the run waits for an operator (`manual_review`) |

The engine is the real one. The retry, the compensation and the reconciliation are the platform's own, not an
emulation of them.

## 2. With a worker of your own

[`examples/payment-saga/typescript`](../../examples/payment-saga/typescript) serves the same six activities with the
TypeScript SDK, against a payment provider that is a few lines of memory, and runs it on three days:

```sh
sagat worker bind --workflow compensation/workflow.json
cd examples/payment-saga/typescript
./prepare.sh
./run.sh ok             # reserves, charges, tells the customer: SUCCEEDED
./run.sh decline        # the card is declined for good: the reservation is released: COMPENSATED
./run.sh lost-answer    # the provider takes the money and the answer is lost: the platform asks, and charges once: SUCCEEDED
```

Its [README](../../examples/payment-saga/typescript/README.md) shows the output of each day and what to notice. The three
things every saga activity has to be, in any SDK:

1. **Keyed.** Every effect at the provider is made under `ctx.idempotencyKey`, and a repeat of the key does nothing more.
   The platform delivers at least once; the key is what makes that safe. It never promises exactly-once for an external
   effect.
2. **Undoable, and tolerant.** A compensation is told which effect it undoes (`targetIdempotencyKey`) and must not fail
   because that effect never happened.
3. **Honest about not knowing.** When the answer is lost, fail with *outcome unknown* (not a plain error): the platform
   then reconciles instead of retrying blindly, and a person decides what an `unknown` verdict means.

## Further

[`docs/runbooks/sagas.md`](../runbooks/sagas.md) is the operator's side: what `COMPENSATION_FAILED`, `PARTIAL` and
`MANUAL_REVIEW` mean and what to do about them. [`examples/payment-saga.workflow.json`](../../examples/payment-saga.workflow.json)
is a larger document with a confirmation event and a timeout.
