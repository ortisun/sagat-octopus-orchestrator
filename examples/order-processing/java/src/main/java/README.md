# java

## Overview

Java order-processing command serving four activities for three workflow scenarios.

## Interface

- `OrderProcessing.main` is the local executable entry point; its input/output records belong to this command. [examples/order-processing/java/src/main/java/OrderProcessing.java:OrderProcessing.main]

## Invariants

- `SAGAT_SERVER`, `SAGAT_GRPC`, `SAGAT_NAMESPACE`, and `SAGAT_TOKEN` are mandatory; a missing setting exits 2. [examples/order-processing/java/src/main/java/OrderProcessing.java:requiredSetting]
- `SAGAT_SCENARIO` defaults to `routing` and accepts only `routing`, `fulfillment`, or `reminder`. [examples/order-processing/java/src/main/java/OrderProcessing.java:selectedScenario]
- Registers `order.validate`, `order.reserve`, `order.quote`, and `order.notify`; simulated notification rejection is permanent `NOTIFICATION_REJECTED`. [examples/order-processing/java/src/main/java/OrderProcessing.java:run]

## Side Effects

- Starts and stops the worker, starts a workflow through the HTTP client, waits at most 60 seconds, and prints diagnostics. A shutdown hook gives the worker ten seconds to stop. [examples/order-processing/java/src/main/java/OrderProcessing.java:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
