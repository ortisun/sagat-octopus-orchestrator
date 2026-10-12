# @sagat-octopus-orchestrator/sdk — TypeScript

The TypeScript SDK of the Sagat Octopus Orchestrator: a **client** for runs and events, a **worker host**
for activities, compensations and reconciliations, and **builders** of the workflow document. One package.

The SDK keeps no journal of its own. The platform owns every state, every retry decision and every
timer; the SDK speaks its REST API and its worker protocol (gRPC) and reports faithfully. It behaves like
the Go, Python, C# and Java SDKs because all five are held to the same conformance suite
(`tests/contract/sdk-conformance/`); the behaviour is written down in [`../SPEC.md`](../SPEC.md).

## Install

```text
npm install @sagat-octopus-orchestrator/sdk
```

## Start a run

[`examples/start-run.ts`](examples/start-run.ts)

```ts
import { SagatClient, SagatError } from '@sagat-octopus-orchestrator/sdk';

const client = new SagatClient({
  apiUrl: 'https://octopus.example.com/v1',
  namespace: 'payments',
  token: () => process.env['OCTOPUS_TOKEN'] ?? '', // called before every request
});

const run = await client.start('charge-card', { orderId: 'o-1', amount: '19.90' }, {
  idempotencyKey: 'order-o-1-start', // a repeat of the call is the same run
  correlationId: 'order-o-1',
});
const done = await client.waitForCompletion(run.runId, { timeoutMs: 60_000 });
```

`start` takes a workflow name or id. The SDK propagates the W3C `traceparent` (the ambient context, or one
you pass), the correlation id and the deadline, and retries **only when that is safe** (a `GET`, or a `POST`
that carries an idempotency key or an event id; never a `4xx` other than `429`).

Every failure is a `SagatError` with `code`, `status`, `retryable`, `traceId`, `reason`, `retryAfterMs`
and a `remediation` sentence. It never repeats a payload, a token or a secret.

## Serve activities

[`examples/worker.ts`](examples/worker.ts)

```ts
const host = new WorkerHost({
  grpcUrl: 'https://octopus.example.com:443', namespace: 'payments',
  token: () => process.env['OCTOPUS_TOKEN'] ?? '', workerName: 'payments-worker-1', maxInflight: 16,
});

host.registerActivity('payments.charge', async (input, ctx) => {
  // ctx.idempotencyKey is stable across attempts: send it to the provider of the external effect.
  return { chargeId: await provider.charge(input.amount, { idempotencyKey: ctx.idempotencyKey, signal: ctx.signal }) };
});
host.registerCompensation('payments.refund', async (_input, ctx) => provider.refund(ctx.targetIdempotencyKey));
host.registerReconciler('payments.find-charge', async (_i, ctx) => reconcile.applied({ chargeId: '…' }));

await host.start();
process.on('SIGTERM', () => void host.stop());
```

The host registers the activities it can run, negotiates the protocol version, polls without ever
busy-polling (an empty answer is always followed by a wait), heartbeats every lease, cancels an activity
(`ctx.signal`) when the run is cancelled, the lease is lost or the host stops, and reports each result with
a key that is the same on every retry. A handler fails on purpose with `ActivityFailure.retryable(code, message)`,
`ActivityFailure.permanent(...)` or `ActivityFailure.outcomeUnknown(...)`; any other error is reported as
`ACTIVITY_ERROR` with only its type name, never its text.

Platform guarantees stay at-least-once for external effects: the SDK gives you the idempotency key and the
reconciliation hook, it cannot make a provider exactly-once.

## TLS

`https://` and `grpcs`-style (`https://`) URLs use the system trust roots and verify the server certificate;
nothing in the library turns that off (a test fails if the source contains the code that would). Pass `tls`
to the client or to the worker host for your own trust roots and for mTLS:

```ts
const tls = { ca: readFileSync('ca.pem'), cert: readFileSync('client.pem'), key: readFileSync('client.key') };
new SagatClient({ apiUrl: 'https://octopus.internal/v1', namespace: 'payments', token, tls });
new WorkerHost({ grpcUrl: 'https://octopus.internal:443', namespace: 'payments', token, workerName: 'w1', tls });
```

`tls.serverName` is the name the server certificate was issued for when it is not the host of the URL. **Over
TLS to a bare IP address the worker host requires it**: `@grpc/grpc-js` sends the host as the TLS server name and
Node does not accept an IP address as one (the SDK says so up front rather than failing as `UNAVAILABLE`). The
name is verified, not skipped. Plain `http://` is accepted anywhere, with one warning (naming only the host)
outside loopback. A redirect is never followed, so a credential cannot travel to a place you did not name.

Tested with throw-away `openssl` certificates: an untrusted server is a `TRANSPORT` error; the configured trust
root works; a server that requires a client certificate refuses a client without one and sees the one given; a
wrong server name fails — for both the REST client and the worker host.

## Write a workflow

[`examples/workflow.ts`](examples/workflow.ts)

```ts
const document = workflow('order-fulfilment')
  .saga('order', { onCompensationFailure: 'manual_review' },
    flow()
      .step('reserve', 'inventory.reserve', { compensateWith: 'inventory.release' })
      .step('charge', 'payments.charge', { compensateWith: 'payments.refund', reconcileWith: 'payments.find-charge' }))
  .waitForEvent('confirmed', { eventType: 'payment.confirmed', correlationExpr: 'input.orderId', timeoutMs: 600_000 })
  .sleep('settle', 1_000)
  .step('ship', 'shipping.create')
  .build();
```

The builders (`step`, `sleep`, `waitForEvent`, `invokeChild`, `condition`, `parallel`, `saga`, `end`) have no
side effects and refuse what the server would refuse and they can see (a duplicate id, a dangling reference,
a saga without a body). `build()` returns the document; `toJson()` its canonical JSON (keys sorted, no whitespace).

## Versions

**SemVer.** `0.x` is pre-release: a minor may change the public API and says so in the changelog. From `1.0`
the public API follows SemVer. The SDK's version is independent of the platform's. The **worker protocol** has
its own version, negotiated at registration: this SDK offers `1.2`; a server that speaks an older minor
serves it at that minor, a server of another major refuses with `UNSUPPORTED_PROTOCOL`
([ADR-0015](../../docs/adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md)).

**Runtime matrix.**

| Runtime | Status |
|---|---|
| Node.js 26.8.1 | **tested**: unit tests (96) and the conformance suite |
| Node.js 20, 22, 24 | declared (`engines.node >= 20`), **not run here** |
| Browsers, Deno, Bun | not supported (the worker host uses `@grpc/grpc-js`; the client uses the global `fetch`) |

Dependencies: `@grpc/grpc-js`, `@grpc/proto-loader`. The package is ESM only.

## Develop

```text
npm test                 # builds, then 96 unit tests against a fake HTTP server and a fake worker service
npm run sync             # copies worker.proto and the error catalog from the repository; npm test fails when behind
cargo test -p sagat-tests --test sdk_conformance -- --ignored   # the conformance suite (needs Docker), from the repository root
```

License: MIT. The package includes the license text in `LICENSE`.
