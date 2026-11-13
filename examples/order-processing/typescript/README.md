# Order processing in TypeScript

This example serves four activities and starts one of the canonical order workflows with the
[TypeScript SDK](../../../sdks/typescript/README.md). The worker and client run in one process.

## Set up

Start the local stack as described in the [quickstart](../../../docs/quickstarts/README.md), install
the `sagat` CLI, and use Node.js 20 or newer. From this directory, set the connection
settings, publish the workflows and allow the development identity to serve their activities:

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat workflow publish ../workflows/routing.workflow.json
sagat workflow publish ../workflows/fulfillment.workflow.json
sagat workflow publish ../workflows/reminder.workflow.json
sagat worker bind order.validate order.reserve order.quote order.notify

./prepare.sh
./run.sh
```

`prepare.sh` uses the checked-in npm lockfile, builds the SDK from this checkout, and compiles the
example. It is safe to run again.

## Scenarios

`SAGAT_SCENARIO` defaults to `routing` and accepts these values:

```sh
SAGAT_SCENARIO=routing ./run.sh
SAGAT_SCENARIO=fulfillment ./run.sh
SAGAT_SCENARIO=reminder ./run.sh
SAGAT_SCENARIO=routing SAGAT_REJECT_ORDER=1 ./run.sh
SAGAT_SCENARIO=reminder SAGAT_FAIL_NOTIFY=1 ./run.sh
```

The rejection input has amount `-1`; other runs use `100`. `SAGAT_FAIL_NOTIFY=1` makes
`order.notify` fail permanently with the typed code `NOTIFICATION_REJECTED`. The program prints
only fixed activity names while handlers run, followed by:

```text
scenario routing run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

It exits zero only for `SUCCEEDED`, so the injected notification failure exits 1 after printing
the terminal `FAILED` status. A missing required setting or unknown scenario exits 2.

## External effects and troubleshooting

The reserve and notify handlers are simulations and call no provider. A real external effect is
at least once: send `ctx.idempotencyKey` to a provider that deduplicates by that key. If an outcome
is unknown, look it up at the provider before retrying and reconcile the workflow from that result.
Do not assume exactly-once delivery.

An activity can read earlier outputs from optional `ctx.steps` keys: routing can observe `validate`
and fulfillment can observe `reserve` and `quote`. Check each key before consuming it; the reminder
does not require the timer to produce an output.

SDK errors print their stable code, trace id, and remediation. Use the trace id with platform logs;
inspect the run with `sagat workflow inspect <run-id>`. The example never prints the bearer token,
activity input, or step output. If waiting reaches 60 seconds, verify the four activity bindings,
workflow publication, namespace, and gRPC address.

## Interface

- `prepare.sh` builds the checkout SDK and example; `run.sh` launches compiled `dist/main.js`. [examples/order-processing/typescript/prepare.sh:npm_install]
- Package scripts expose `build` and `start`. [examples/order-processing/typescript/package.json:scripts]
- [src](src/README.md) — TypeScript executable serving common order activities and starting a selected workflow.

## Invariants

- Preparation is fail-fast and disables dependency lifecycle scripts, audit, and funding output. [examples/order-processing/typescript/prepare.sh:set -euo pipefail]
- The package requires Node 20 or newer and references the checkout SDK. [examples/order-processing/typescript/package.json:engines]
- Compilation uses strict TypeScript with NodeNext modules and ES2022 output. [examples/order-processing/typescript/tsconfig.json:compilerOptions]

## Side Effects

- Installs locked SDK/example dependencies and writes compiled output to `dist`. [examples/order-processing/typescript/prepare.sh:npm_install]
- Replaces the shell with Node and forwards arguments. [examples/order-processing/typescript/run.sh:exec]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
