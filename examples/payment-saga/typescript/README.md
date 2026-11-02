# payment-saga in TypeScript

A saga served with the [TypeScript SDK](../../../sdks/typescript/README.md): reserve a limit, charge a card,
tell the customer. It is `templates/compensation` (`limit.reserve`, `payment.charge`, `customer.notify`) with
its undo steps (`limit.release`, `payment.refund`) and its reconciler (`payment.find-charge`) written as code,
against a payment provider that is a few lines of memory, so that you can see what the platform does on the
bad days.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the `sagat` CLI,
and Node.js 20 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new compensation                     # the workflow (skip it if you have it)
sagat workflow publish compensation/workflow.json
sagat worker bind --workflow compensation/workflow.json    # let your identity serve its six activities

./prepare.sh                               # once: npm ci for the SDK and for the example, then tsc
./run.sh ok                                # or: decline, lost-answer
```

## The three days at the provider

| Day | What the provider does | What the platform does | Ends |
|---|---|---|---|
| `ok` | reserves, charges | goes on to tell the customer | `SUCCEEDED` |
| `decline` | refuses the card, for good | runs the undo of what already happened: `limit.release` | `COMPENSATED` |
| `lost-answer` | takes the charge and the answer never arrives (`ActivityFailure.outcomeUnknown`) | does **not** charge again: it asks `payment.find-charge` whether it happened, is told it did, and goes on | `SUCCEEDED` |

The program prints what each side did, in order, and the provider's ledger:

```text
serving the payment saga at protocol 1.2
run 01a12b68-e024-72bc-b10e-9794164ca438 started
reserve p-1: r-p-1 for 10
charge p-1: declined (CARD_DECLINED)
release p-1: the reservation is released
ledger: reservations=1 released=1 charges=0 refunded=0
run 01a12b68-e024-72bc-b10e-9794164ca438 COMPENSATED
```

`sagat workflow inspect <run>` shows the same story from the platform's side: the journal with the
compensation, or with the reconciliation, and the attempts.

## What to notice

- Every effect at the provider is keyed by `ctx.idempotencyKey` and a repeat of a key does nothing more. That
  is what makes the platform's at-least-once delivery safe: the platform never promises exactly-once for an
  external effect, the provider's key does.
- A compensation is told which effect it undoes (`ctx.targetIdempotencyKey`) and tolerates one that never
  happened (releasing a reservation that was not made is not an error).
- A reconciler answers one question about one effect: `reconcile.applied(output)`, `reconcile.notApplied()` or
  `reconcile.unknown()` (a person decides).
- `package.json` points at the SDK in this checkout until the SDK is published (see the hello-world
  example).

## Interface

- `prepare.sh` builds the checkout SDK and example; `run.sh` launches compiled `dist/main.js`. [examples/payment-saga/typescript/prepare.sh:npm_install]
- Package scripts expose `build` and `start`. [examples/payment-saga/typescript/package.json:scripts]
- [src](src/README.md) — TypeScript payment saga demonstrating keyed effects, compensation, and unknown-outcome reconciliation.

## Invariants

- Preparation is fail-fast and disables dependency lifecycle scripts, audit, and funding output. [examples/payment-saga/typescript/prepare.sh:set -euo pipefail]
- The package requires Node 20 or newer and references the checkout SDK. [examples/payment-saga/typescript/package.json:engines]
- Compilation uses strict TypeScript with NodeNext modules and ES2022 output. [examples/payment-saga/typescript/tsconfig.json:compilerOptions]

## Side Effects

- Installs locked SDK/example dependencies and writes compiled output to `dist`. [examples/payment-saga/typescript/prepare.sh:npm_install]
- Replaces the shell with Node and forwards arguments. [examples/payment-saga/typescript/run.sh:exec]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored saga::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
