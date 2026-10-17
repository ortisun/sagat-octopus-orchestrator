# Order processing in five languages

Three workflows, the same activities and five SDK implementations. These examples use the
**implemented v1 workflow IR**. The [additional primitives](../../specs/014-workflow-primitives/spec.md)
are a separate proposal; none is needed to run this directory.

| Use case | Workflow | What to observe |
|---|---|---|
| Validate and route an order | `order-routing` | `order.validate` returns `accepted`; the condition selects notification or the rejected end |
| Prepare fulfillment | `order-fulfillment` | Reservation and shipping quote run as static parallel branches; `join: all` waits for both before notification |
| Schedule a reminder | `order-reminder` | A persisted 100 ms timer schedules notification; no worker sleeps to implement the delay |

Each language implements all three workflows: [TypeScript](typescript/README.md),
[Python](python/README.md), [Go](go/README.md), [C#](dotnet/README.md), [Java](java/README.md).
The canonical workflow documents are shared in [`workflows/`](workflows/); the SDK language
changes the activity implementation, not the engine's orchestration semantics.

## Start and configure

Follow the [local stack quickstart](../../docs/quickstarts/README.md) to install `sagat`, start
Docker Compose and seed the `payments` namespace. From the repository root:

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
sagat doctor
for file in examples/order-processing/workflows/*.workflow.json; do
  sagat workflow validate "$file"
  sagat workflow publish "$file"
done
sagat worker bind order.validate order.reserve order.quote order.notify
```

`worker bind` requires administrator permission and **replaces** this identity's activity
bindings in the namespace. Use a dedicated development identity if you run other workers.
Keep credentials in the environment; do not paste them into source or enable shell tracing.
The development token issuer is only for the local stack. Use your identity provider and TLS
for a remote environment.

## Run the cases

Choose one language directory (replace `typescript` below with `python`, `go`, `dotnet` or
`java`). Each `prepare.sh` installs/builds the example against the SDK in this checkout;
`run.sh` starts a worker and client, runs one workflow, waits at most 60 seconds and stops.

```sh
examples/order-processing/typescript/prepare.sh
SAGAT_SCENARIO=routing SAGAT_REJECT_ORDER=0 SAGAT_FAIL_NOTIFY=0 examples/order-processing/typescript/run.sh
SAGAT_SCENARIO=routing SAGAT_REJECT_ORDER=1 SAGAT_FAIL_NOTIFY=0 examples/order-processing/typescript/run.sh
SAGAT_SCENARIO=fulfillment SAGAT_REJECT_ORDER=0 SAGAT_FAIL_NOTIFY=0 examples/order-processing/typescript/run.sh
SAGAT_SCENARIO=reminder SAGAT_REJECT_ORDER=0 SAGAT_FAIL_NOTIFY=0 examples/order-processing/typescript/run.sh
SAGAT_SCENARIO=reminder SAGAT_REJECT_ORDER=0 SAGAT_FAIL_NOTIFY=1 examples/order-processing/typescript/run.sh
```

The first four commands exit `0` and print `scenario <name> run <id> SUCCEEDED`. Rejection is
a successful **business decision**: validation returns `accepted: false` and notification never
runs. It is not an engine failure. The last command deliberately makes the activity fail with
`NOTIFICATION_REJECTED`, prints `FAILED` and exits `1`. Missing configuration or an unknown
scenario exits `2`.

Inputs are deterministic fixtures: `orderId: demo-order-001`, `amount: 100` (or `-1` for rejection),
and `failNotify: false` (or `true`). Each example's `order.validate` accepts `amount > 0`, so an
amount of `0` is rejected too; the shared suite exercises `100` and `-1` only. `SAGAT_SCENARIO` defaults to `routing`; the two flags are
active only when equal to `1`. Reservation, quote and notification are simulations, not real
inventory or messaging providers. In a real adapter, use the task's stable idempotency key with
provider-side deduplication, and reconcile an unknown outcome before repeating a critical effect.
A parallel join does not make external effects atomic and does not compensate them automatically.
See the [payment saga](../payment-saga/typescript/README.md) for compensation and reconciliation.

## Inspect and troubleshoot

Open [Studio](http://127.0.0.1:18080/studio/) or run `sagat workflow inspect <id>` using the ID
printed by the example. Inspect the run's trace and persisted journal: the rejected route has no
notification; fulfillment has both branch completions before notification; reminder has a timer
firing before notification. Branch execution order is intentionally unspecified.

| Symptom | Check |
|---|---|
| `SAGAT_* is not set` | Export all four connection variables in the terminal running the example |
| Unauthorized / expired token | Refresh the development token, or obtain a valid token from your configured provider |
| Workflow not found | Publish all three documents in the same namespace as the worker |
| Permission denied / worker cannot serve | Bind all four activity names in one command; a later binding replaces the earlier set |
| Run does not finish within 60 seconds | Use `sagat doctor`, check the engine at port 19000, bindings and Studio; client wait expiry does not cancel the durable run |
| Expected notification but none | Check `SAGAT_REJECT_ORDER` and `SAGAT_FAIL_NOTIFY`; use explicit values from the commands above |
| Build failure | Check the language README and toolchain version; first preparation needs dependency registry access |

## Validate all five SDKs

With Docker available and all five toolchains installed, from the checkout root:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The harness provisions a disposable real platform, publishes these documents and executes the
same five cases in every SDK. It checks persisted status, journal ordering and traces, expected
activities, failure handling, invalid configuration and token redaction. Set
`SAGAT_EXAMPLE_LANGS=typescript,python` to run a subset; a subset does not establish five-language
parity. This suite does not replace the engine's crash/restart, tenant isolation or fault suites.
