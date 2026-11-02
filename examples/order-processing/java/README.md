# order-processing in Java

This executable uses the [Java SDK](../../../sdks/java/README.md) as both a worker and a client for
the `order-routing`, `order-fulfillment`, and `order-reminder` workflows. Its four activities simulate
their results without contacting an external provider.

## Setup and run

Start the local stack, publish the three workflows from the repository's order-processing workflow
directory, and bind the worker identity to them. Install JDK 17 or newer and set:

```sh
cd examples/order-processing/java
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

`prepare.sh` installs the SDK from this checkout into the local Maven repository and builds the
example with the checked-in Maven wrapper. It is safe to rerun. `SAGAT_SCENARIO` accepts `routing`,
`fulfillment`, or `reminder`. `SAGAT_REJECT_ORDER=1` submits amount `-1`, while
`SAGAT_FAIL_NOTIFY=1` returns the permanent typed error `NOTIFICATION_REJECTED`. A failed run prints
status `FAILED` and exits 1. Missing settings and unknown scenarios exit 2.

The final line has the same shape in all languages:

```text
scenario routing run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

## Effects and troubleshooting

The example's effects are in memory. A real provider request must carry `ctx.idempotencyKey()` and
the provider must deduplicate it. When a response is lost, reconcile the provider result before
retrying; this avoids an unsupported exactly-once claim.

- `... is not set`: export all four required settings.
- `WORKFLOW_NOT_FOUND`: publish the workflow selected by `SAGAT_SCENARIO` in `SAGAT_NAMESPACE`.
- a wait timeout: check the identity's workflow bindings and access to all four activities.
- `UNAUTHENTICATED` or `FORBIDDEN`: obtain a fresh token with the namespace and worker bindings.

## Interface

- `prepare.sh` installs the checkout SDK and builds this standalone example. [examples/order-processing/java/prepare.sh:mvnw]
- `run.sh` launches `OrderProcessing` using the generated runtime classpath. [examples/order-processing/java/run.sh:exec]
- [src](src/README.md) — Java order-processing command serving four activities for three workflow scenarios.

## Invariants

- The project compiles for Java 17 and generates `target/classpath.txt` during package. [examples/order-processing/java/pom.xml:maven.compiler.release]
- Preparation skips SDK tests, packages with strict checksums, and requires a nonempty runtime classpath. [examples/order-processing/java/prepare.sh:mvnw]
- Launch requires both the runtime classpath file and compiled classes directory; absence exits 2. [examples/order-processing/java/run.sh:target/classpath.txt]

## Side Effects

- Writes SDK artifacts to the local Maven repository and build output under `target`. [examples/order-processing/java/prepare.sh:mvnw]
- Reads the runtime classpath and replaces the shell with Java. [examples/order-processing/java/run.sh:exec]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
