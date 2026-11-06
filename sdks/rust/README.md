# Sagat Rust SDK (0.3 candidate)

> Documentation status: the contract sections below describe the inspected source. Existing runtime, test-count and advisory records are historical developer notes; this documentation update did not rerun or revalidate those records.

The native Tokio SDK uses the same public REST and worker gRPC contracts as the other official
SDKs. It does not connect to PostgreSQL, NATS, Redis or OpenBao directly. The server owns durable
workflow execution and scoped secret authorization. Rust 1.98 is the initially tested toolchain;
the package is MIT licensed.

This is an implementation candidate tracked by feature 034. The current increment provides
start/get/list/cancel/operate/signal, journal pages and completion waits, verified TLS and mTLS,
fresh credentials, declared Queries/Updates, event publication, approval pages/decisions, chain inspection/commands/waits, single-pass input uploads and an activity host,
protocol negotiation, heartbeat, cooperative cancellation, attempt-scoped secrets/blobs and typed
compensation/reconciliation handlers and typed declarative builders.
All 60 shared scenarios passed for Rust against the real server/PostgreSQL, together with the
SDK user-story and fault suites. The integrated six-language release run and packaged-consumer
acceptance remain separate gates.
Do not infer complete SDK parity from the local client/worker tests.

```rust,no_run
use sagat_octopus_orchestrator_sdk::{
    ActivityContext, CallOptions, Client, ClientOptions, TokenSource, Value,
    WorkerHost, WorkerOptions,
};
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = TokenSource::bearer(std::env::var("SAGAT_TOKEN")?);
    let client = Client::new(ClientOptions::new(
        "https://api.example/v1", "payments", token.clone(),
    ))?;
    let mut worker = WorkerHost::new(WorkerOptions::new(
        "https://engine.example:7443", "payments", "rust-payments", token,
    ))?;
    worker.activity("echo", |_: ActivityContext, input: Value| async move { Ok(input) })?;
    worker.start().await?;
    let run = client.get("019b5d6e-0000-7000-8000-000000000001", CallOptions::default()).await?;
    println!("{} {:?}", run.id, run.status);
    worker.stop(Duration::from_secs(30)).await?;
    Ok(())
}
```

Feature 018 adds schedules, business identifiers, batch jobs, the with-start calls and resets to the
client: `client.schedules()`, `client.batch()` and `client.business_ids()` return light handles, and
`signal_with_start`, `update_with_start`, `reset_points`, `preview_reset`, `reset`, `list_resets` and
`get_reset` are methods of `Client`. `start` takes a business identifier (sent as given), its conflict
and reuse policies and a start delay as a `Duration` (truncated once to integer milliseconds before the
first attempt) and answers a `StartedRun`, which reads as the `Run` and says in `started` whether this
call created it. Every call that changes something carries one `Idempotency-Key`, given in
`CallOptions` or generated once, that every retry keeps. Instants you send are a date-time or RFC 3339
text (`Moment`) and are written in UTC; values of an enumeration the SDK does not know stay as the raw
string (`Other`), never an error. A start with an input blob takes neither a business identifier nor a
delay.

```rust,no_run
use sagat_octopus_orchestrator_sdk::{
    BusinessIdConflictPolicy, CallOptions, Client, OverlapPolicy, ScheduleAction, ScheduleSpec,
    ScheduleWrite, StartOptions, json,
};
use std::time::Duration;

async fn operations(client: &Client, namespace_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let started = client
        .start(
            "order-flow",
            &json!({"orderId": "o-1"}),
            StartOptions {
                business_id: Some("order-o-1".into()),
                business_id_conflict_policy: Some(BusinessIdConflictPolicy::UseExisting),
                start_delay: Some(Duration::from_secs(600)),
                ..StartOptions::default()
            },
        )
        .await?;
    println!("{} created by this call: {}", started.id, started.started);
    let spec = ScheduleSpec {
        cron: vec!["0 9 * * *".into()],
        timezone: Some("UTC".into()),
        ..ScheduleSpec::default()
    };
    let action = ScheduleAction::new("019b5d6e-0000-7000-8000-000000000002", json!({}));
    let schedule = client
        .schedules()
        .create(namespace_id, &ScheduleWrite::new("nightly", spec, action), CallOptions::default())
        .await?;
    client
        .schedules()
        .trigger(
            namespace_id,
            &schedule.schedule_id.to_string(),
            Some(OverlapPolicy::Skip),
            CallOptions::default(),
        )
        .await?;
    Ok(())
}
```

Use a dynamic `TokenSource` for rotation. An explicit `CallOptions.traceparent` is validated before
I/O. Retries preserve the exact body and idempotency key; they never retry an activity's external
effect for the application. Use the task's idempotency key with external providers.

`TlsOptions` adds PEM trust anchors and an optional redacted `TlsIdentity` for mutual TLS; native
roots and certificate/hostname verification remain enabled. The REST `server_name` overrides
certificate verification; SNI follows the URL hostname. Use that hostname in the URL for servers
that route certificates by SNI. Never print secret values or persist them as workflow outputs.

`ActivityContext::secrets()` resolves declared secrets lazily, once after a successful resolution.
`SecretSet` views refuse access when the handler returns; `get` returns an owned zeroizing copy.
A caller that copied bytes owns that copy's lifetime. Cancellation is cooperative; a stale lease
is fenced by the server. `wait_for_failure()` observes fatal discovery errors, including unsupported
explicit streaming; the application's existing handlers keep their original lease identity.

For repository checks use the private external `CARGO_TARGET_DIR` required by `sdks/AGENTS.md`:
`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`.
The package embeds its protobuf and error catalog; native drift tests compare them to the canonical
repository files. The standalone consumer is a separate Cargo workspace under `consumer/`.

`Telemetry` provides an optional ambient trace-context hook and bounded request/worker events for
OpenTelemetry adapters. Explicit traceparent wins. Observer failures are isolated; events carry
fixed operation/code labels and durations, without run IDs, URLs, tokens or payloads. The SDK does
not install a global telemetry provider or replace the application's Rust panic hook.

`workflow::Builder::new(name)` creates a workflow; `Builder::flow()` creates a joined flow.
Methods retain the first local validation error; `build()` returns an owned document and
`json()` returns canonical text with the shared safe integer policy. Joined flows are read at
build time. Root-only options on nested flows fail locally. Typed options cover activities,
collections/loops, approvals, continuation/state, scripts, triggers and managed database nodes.
The SDK generates IR; the server resolves connection references and executes managed SQL.

<!-- EGM directory contract -->

## Interface

- Package sagat-octopus-orchestrator-sdk 0.3.0 declares Rust 1.98 and published source/proto/build/license/README contents. build.rs generates tonic client and test server bindings. [Cargo.toml:name] [Cargo.toml:version] [Cargo.toml:rust-version] [Cargo.toml:include] [build.rs:main]

- [consumer](./consumer/README.md) — component contract.
- [proto](./proto/README.md) — component contract.
- [src](./src/README.md) — component contract.

## Invariants

- The SDK declares an independent workspace and Clippy warnings for unwrap/expect/panic/todo/unimplemented. [Cargo.toml:workspace] [Cargo.toml:lints.clippy]

## Side Effects

- Invoked Cargo builds watch the proto and write generated protobuf bindings. [build.rs:main]

## Verification

Run from the repository root:

```sh
cd sdks/rust && cargo test --locked
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
