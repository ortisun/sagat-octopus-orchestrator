# TypeScript quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the TypeScript SDK, in [one file](../../examples/hello-world/typescript/src/main.ts). You need Node.js 20 or newer, the
local stack and `sagat`; the [main quickstart](README.md) gets you there.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then, in [`examples/hello-world/typescript`](../../examples/hello-world/typescript), follow its [README](../../examples/hello-world/typescript/README.md). It prints

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal. The same program in each of the five languages makes the same run,
the same journal and the same outputs, byte for byte, which the end-to-end test of the examples checks.

`prepare.sh` builds the SDK from this checkout (`npm ci`, then `tsc`) and installs the example against it, because the
package is not published yet. `package.json` has the dependency as `"@sagat-octopus-orchestrator/sdk": "file:../../../sdks/typescript"`:
once the package is published, that line becomes its version.

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`ctx.steps['compose']` is the output of the step with that id (the SDK unwraps `output` for you). It returns its output. It fails on purpose with a retryable or a permanent failure; anything else it
raises is reported as `ACTIVITY_ERROR`, retryable, with only the type of the error, never its text.

```ts excerpt=examples/hello-world/typescript/src/main.ts
host.registerActivity<Input, Greeting>('greeting.compose', (input) => ({ greeting: `Hello, ${input.name}!` }));
host.registerActivity<Input, Delivery>('greeting.deliver', (_input, ctx) => {
  // A real delivery is an external effect: send ctx.idempotencyKey to the provider, which is
  // the same for every attempt of this step.
  const composed = ctx.steps['compose'] as Greeting;
  console.log(`delivering ${composed.greeting}`);
  return { delivered: true, greeting: composed.greeting };
});
await host.start();
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`ctx.idempotencyKey` to its provider: it is the same for every attempt of the step, so a repeat does not do it twice. The
platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```ts excerpt=examples/hello-world/typescript/src/main.ts
const api = new SagatClient({ apiUrl, namespace, token });
const started = await api.start('hello-world', { name });
console.log(`run ${started.runId} started`);
const done = await api.waitForCompletion(started.runId, { timeoutMs: 60_000, signal: interrupted.signal });
console.log(`run ${done.runId} ${done.status}`);
```

Starting a run takes the name of a workflow of the namespace (or its id). Without an idempotency key the SDK makes one per
call and reuses it for every retry of that call, so a lost answer never starts a second run. The tenant is never
configured: it is the token's.

## Testing

- **A workflow, before your worker exists:** `sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind` serves its
  activities from a file of canned answers and failures, on the real engine. Make `greeting.compose` fail once
  (`sequence: [{fail: {code: X, retryable: true}}, {output: ...}]`) and see the retry; make it `lose` the task and see the lease
  expire.
- **Your activities, against the platform:** the example is the harness. It is what the end-to-end test of this
  repository runs, with the platform it starts, and what you can run against your own stack.
- **Your activities, alone:** A handler is a function of `(input, ctx)`: keep the effect behind a small interface and unit test the logic by calling
the function with a hand-made context (only the fields it reads). The SDK ships no fake context yet.

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the
TypeScript SDK offers it once its runtime, Task 044-10 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12. Besides the SDK, the platform must be ready for it: an operator records
the requirements `code-workflows-v1` and, because this workflow declares a signal, `signal-inbox-v1`
([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is a function. It reserves the stock, charges the card, sleeps one day and sends a
reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```typescript
export async function order(ctx, input: { orderId: string; amount: string }) {
  let paid = false;
  ctx.onSignal('paid', () => { paid = true; });
  await ctx.activity('inventory.reserve', { orderId: input.orderId });
  await ctx.activity('payments.charge', { orderId: input.orderId, amount: input.amount });
  await ctx.sleep(24 * 60 * 60 * 1000);
  if (!paid) {
    await ctx.activity('customers.remind', { orderId: input.orderId });
  }
  return { orderId: input.orderId, paid };
}
```

The function never touches the outside world: each `ctx` call is a command the platform records and executes (two
activities, a durable timer of one day and, when no payment came, a third activity), and the activities run in an
ordinary worker as above. A `paid` signal sent at any moment waits in the run's inbox and is handed to the handler in
the function's next activation; `await ctx.signal('paid')` waits for one instead. The function is run
again from the start against the run's journal whenever a worker does not hold it in memory, so it must be
deterministic: no `fs`, `fetch` or `crypto`, and `ctx.now()`, `ctx.random()` and `ctx.uuid()` instead of the clock and
`Math.random`. The SDK runs it in a `vm` context of its own where `Date`, `Math.random` and the timers are already
replaced. Its precondition: Node 22.8 or later and a process time zone of UTC for the workflow worker (the activity worker
keeps Node 20).

An activity failure the function does not catch fails the workflow task, not the run: the task is retried after 1, 2, 4
and 8 s, and the fifth consecutive failure puts the run under review. To fail the run on purpose, throw the SDK's
`WorkflowFailure` with a code (Requirement 044-009).

**Bundle it and publish the version.** The bundler of the separate package `@sagat-octopus-orchestrator/workflows-bundler`
writes the bundle the worker loads and its manifest, which names the code digest (a SHA-256 computed from the bundle,
the Node version and the SDK); it refuses an import of a Node built-in:

```sh
npx sagat-workflows bundle
```

```typescript
const manifest = JSON.parse(await readFile('dist/manifest.json', 'utf8'));
const envelope = {
  activities: [{ name: 'inventory.reserve' }, { name: 'payments.charge' }, { name: 'customers.remind' }],
};
const declarations = {
  signals: [{ name: 'paid', buffered: true, maxPending: 10,
    schema: { type: 'object', properties: {}, additionalProperties: false } }],
};
await api.definitions.publishCode(manifest, envelope, declarations);
```

`publishCode` assembles the code document from the manifest, the envelope and the declarations, saves it as the
workflow's draft and publishes it, making it active, as publishing a graph does (Decision 044-238). The envelope lists
what the function's commands may use; a command outside it puts the run under review. The declarations hold the rest of
the code document of Technical Plan 3.1, here the `paid` signal (`sagat workflow publish-code` takes them with
`--declarations <file>`).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). The worker host loads the bundle
and registers the manifest's workflow type and digest:

```typescript
const workflows = new WorkerHost({
  grpcUrl, namespace, token, workerName: 'order-workflows',
  workflowBundlePath: 'dist/workflows.js',
  workflowManifestPath: 'dist/manifest.json',
});
await workflows.start();
```

The paths are this example's. Runs started with `api.start('order', { orderId: 'o-1', amount: '19.99' })` are pinned to
the version's digest; a new bundle is a new version, and open runs stay on their current code version's digest (the
start version's until a move) until a person moves them.

**Test it with the replayer.** Export the history of a run (`api.runs.exportHistory(runId)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the new bundle in `node:test`:

```typescript
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createReadStream } from 'node:fs';
import { createInterface } from 'node:readline';
import { Replayer } from '@sagat-octopus-orchestrator/sdk/workflows';

test('the new bundle replays the recorded order runs', async () => {
  const history = createInterface({ input: createReadStream('histories/order-1.jsonl') });
  const verdict = await Replayer.replay(history, { bundlePath: 'dist/workflows.js', workflowType: 'order' });
  assert.equal(verdict.matched, true); // the field of the verdict is named by Task 044-10
});
```

A mismatch names the first command that differs. A history of an export format other than 1 is refused, and a replay that
reaches a value the export redacted stops with `INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/typescript/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the six SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
