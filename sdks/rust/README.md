# Sagat Rust SDK (0.3 candidate)

The native Tokio SDK uses the same public REST and worker gRPC contracts as the other official
SDKs. It does not connect to PostgreSQL, NATS, Redis or OpenBao directly. The server owns durable
workflow execution and scoped secret authorization. Rust 1.98 is the initially tested toolchain;
the package is MIT licensed.

This is an implementation candidate tracked by feature 034. The current increment provides
start/get/list/cancel/operate/signal, journal pages and completion waits, verified TLS and mTLS,
fresh credentials, declared Queries/Updates, event publication, approval pages/decisions, chain inspection/commands/waits, single-pass input uploads and an activity host,
protocol negotiation, heartbeat, cooperative cancellation, attempt-scoped secrets/blobs and typed
compensation/reconciliation handlers.
Builders,
the shared six-language conformance suite and packaged-consumer release acceptance are pending.
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
