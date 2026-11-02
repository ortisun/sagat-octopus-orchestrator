# order-processing in C#

This executable uses the [.NET SDK](../../../sdks/dotnet/README.md) as both a worker and a client for
the `order-routing`, `order-fulfillment`, and `order-reminder` workflows. Its validation, reservation,
quote, and notification activities are simulations and call no external provider.

## Setup and run

Start the local stack, publish the three workflows from the repository's order-processing workflow
directory, and bind the worker identity to them. Install .NET SDK 10 or newer and set:

```sh
cd examples/order-processing/dotnet
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

`SAGAT_SCENARIO` accepts `routing`, `fulfillment`, or `reminder`. `SAGAT_REJECT_ORDER=1` submits
amount `-1`; the normal amount is `100`. `SAGAT_FAIL_NOTIFY=1` makes `order.notify` return the
permanent typed error `NOTIFICATION_REJECTED`. A failed run is printed with status `FAILED` and exits
1. Missing required settings and unknown scenarios exit 2.

The final line is identical in shape across the language examples:

```text
scenario routing run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

## Effects and troubleshooting

The example's effects are in memory. A real provider request must carry
`ActivityExecutionContext.IdempotencyKey`, and the provider must deduplicate that key. Reconcile an
unknown result before retrying; this avoids claiming exactly-once behavior the provider cannot prove.

- `... is not set`: export all four required settings.
- `WORKFLOW_NOT_FOUND`: publish the workflow selected by `SAGAT_SCENARIO` in `SAGAT_NAMESPACE`.
- a wait timeout: check the identity's workflow bindings and that it can serve all four activities.
- `UNAUTHENTICATED` or `FORBIDDEN`: create a fresh development token with the namespace and worker
  bindings.

## Interface

- Top-level command execution uses local `RunAsync` and internal input/output records. [examples/order-processing/dotnet/Program.cs:RunAsync]

## Invariants

- The four connection settings are mandatory; missing settings exit 2. [examples/order-processing/dotnet/Program.cs:RequiredSetting]
- Scenario defaults to routing and accepts only routing, fulfillment, or reminder. [examples/order-processing/dotnet/Program.cs:SelectedScenario]
- The command waits at most 60 seconds and succeeds only with `RunStatuses.Succeeded`. [examples/order-processing/dotnet/Program.cs:RunAsync]
- The executable targets `net10.0` and references the checkout SDK with nullable types and invariant globalization enabled. [examples/order-processing/dotnet/OrderProcessing.csproj:PropertyGroup]

## Side Effects

- Writes Release output with telemetry/first-run behavior disabled. [examples/order-processing/dotnet/prepare.sh:dotnet build]
- Launch requires the built DLL, then replaces the shell with `dotnet`; a missing DLL exits 2. [examples/order-processing/dotnet/run.sh:exec]
- Starts/stops a worker, starts/waits for a workflow, handles Ctrl-C, and prints diagnostics. [examples/order-processing/dotnet/Program.cs:RunAsync]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
