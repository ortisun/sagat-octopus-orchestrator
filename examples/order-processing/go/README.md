# order-processing in Go

This executable uses the [Go SDK](../../../sdks/go/README.md) as both a worker and a client for the
`order-routing`, `order-fulfillment`, and `order-reminder` workflows. The activities only simulate
validation, reservation, quoting, and notification; they do not call an external provider.

## Setup and run

Start the local stack, publish the three workflows from the repository's order-processing workflow
directory, and bind the worker identity to them. Then set the four required settings:

```sh
cd examples/order-processing/go
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
for file in ../workflows/*.workflow.json; do
  sagat workflow validate "$file"
  sagat workflow publish "$file"
done
sagat worker bind order.validate order.reserve order.quote order.notify

./prepare.sh
./run.sh                         # routing (the default)
SAGAT_SCENARIO=fulfillment ./run.sh
SAGAT_SCENARIO=reminder ./run.sh
```

`SAGAT_SCENARIO` accepts `routing`, `fulfillment`, or `reminder`. Set `SAGAT_REJECT_ORDER=1` to
submit amount `-1`; otherwise it is `100`. Set `SAGAT_FAIL_NOTIFY=1` to make `order.notify` return
the permanent typed error `NOTIFICATION_REJECTED`. A failed run is printed with status `FAILED` and
the executable exits 1. Missing required settings and unknown scenarios exit 2.

The final line has the same form for every language:

```text
scenario routing run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

## Effects and troubleshooting

The example's effects are in memory. For a real notification or reservation, send
`worker.TaskFrom(ctx).IdempotencyKey` to a provider that deduplicates it. If the outcome is unknown,
look it up and reconcile before retrying; an idempotency key cannot create exactly-once behavior in
a provider that does not support deduplication.

- `... is not set`: export all four required settings above.
- `WORKFLOW_NOT_FOUND`: publish the workflow selected by `SAGAT_SCENARIO` in `SAGAT_NAMESPACE`.
- a wait timeout: confirm the identity is bound to the four activities (`sagat worker bind` above)
  and that this process registered all four.
- `UNAUTHENTICATED` or `FORBIDDEN`: obtain a fresh development token with the needed namespace and
  worker bindings.

## Interface

- The `main` package runs the local `run` driver and its activity handlers. [examples/order-processing/go/main.go:main]

## Invariants

- The four connection settings are mandatory; missing settings exit 2. [examples/order-processing/go/main.go:requiredSetting]
- Scenario defaults to routing and accepts only routing, fulfillment, or reminder. [examples/order-processing/go/main.go:selectedScenario]
- The command waits at most 60 seconds and treats a terminal status other than `SUCCEEDED` as an error. [examples/order-processing/go/main.go:run]
- The Go 1.25 module replaces its SDK dependency with the checkout SDK. [examples/order-processing/go/go.mod:replace]

## Side Effects

- Builds `bin/order-processing`. [examples/order-processing/go/prepare.sh:go build]
- Launch replaces the shell with the compiled executable; missing output exits 2. [examples/order-processing/go/run.sh:exec]
- Starts/stops the worker, starts/waits for a workflow, handles OS interrupt, and prints diagnostics. [examples/order-processing/go/main.go:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
