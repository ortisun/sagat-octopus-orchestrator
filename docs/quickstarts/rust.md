# Rust quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the Rust SDK ([`sdks/rust`](../../sdks/rust/README.md), the 0.3.0 candidate
of feature 034). You need Rust 1.98 or newer, the local stack and `sagat`; the [main quickstart](README.md) gets you
there.

Unlike the five other languages, Rust has no program under [`examples/hello-world`](../../examples/hello-world/README.md)
yet: the program is on this page, and the end-to-end test of the examples does not run it.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then make a binary crate beside this checkout whose `Cargo.toml` points the SDK at it, because the crate is not on
crates.io yet (once it is, that line becomes its version):

```toml
[package]
name = "hello-world"
version = "0.1.0"
edition = "2024"
rust-version = "1.98"

[dependencies]
sagat-octopus-orchestrator-sdk = { path = "../sagat-octopus-orchestrator/sdks/rust" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

put the worker and the client below in `src/main.rs`, and run it with `cargo run -- Ada`. It prints

```text
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 Succeeded
```

and `sagat workflow inspect <run>` shows the journal.

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`ctx.task.steps["compose"]` is the output of the step with that id (the SDK unwraps `output` for you). It returns its
output, or fails on purpose with `ActivityFailure::retryable` or `ActivityFailure::permanent`.

```rust
use sagat_octopus_orchestrator_sdk::{
    ActivityContext, ActivityFailure, CallOptions, Client, ClientOptions, TokenSource, Value,
    WaitOptions, WorkerHost, WorkerOptions, json,
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = |name: &str| std::env::var(name);
    let (api_url, grpc_url, namespace) = (env("SAGAT_SERVER")?, env("SAGAT_GRPC")?, env("SAGAT_NAMESPACE")?);
    let token = TokenSource::bearer(env("SAGAT_TOKEN")?);
    let name = std::env::args().nth(1).unwrap_or_else(|| "Ada".into());

    let mut host = WorkerHost::new(WorkerOptions::new(
        grpc_url.as_str(), namespace.as_str(), "hello-world-rust", token.clone(),
    ))?;
    host.activity("greeting.compose", |_ctx: ActivityContext, input: Value| async move {
        let who = input["name"].as_str().unwrap_or("world").to_owned();
        Ok::<_, ActivityFailure>(json!({ "greeting": format!("Hello, {who}!") }))
    })?;
    host.activity("greeting.deliver", |ctx: ActivityContext, _input: Value| async move {
        // A real delivery is an external effect: send ctx.task.idempotency_key to the provider,
        // which is the same for every attempt of this step.
        let greeting = ctx.task.steps["compose"]["greeting"]
            .as_str()
            .ok_or_else(|| ActivityFailure::permanent("NO_GREETING", "the compose step gave no greeting"))?
            .to_owned();
        println!("delivering {greeting}");
        Ok::<_, ActivityFailure>(json!({ "delivered": true, "greeting": greeting }))
    })?;
    host.start().await?;
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`ctx.task.idempotency_key` to its provider: it is the same for every attempt of the step, so a repeat does not do it
twice. The platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```rust
    let api = Client::new(ClientOptions::new(api_url.as_str(), namespace.as_str(), token))?;
    let started = api.start("hello-world", &json!({ "name": name }), CallOptions::default()).await?;
    println!("run {} started", started.id);
    let done = api.wait_for_completion(&started.id.to_string(), WaitOptions::default()).await?;
    println!("run {} {:?}", done.id, done.status);
    host.stop(Duration::from_secs(30)).await?;
    Ok(())
}
```

Starting a run takes the name of a workflow of the namespace (or its id). Without an idempotency key the SDK makes one
per call and reuses it for every retry of that call, so a lost answer never starts a second run. The tenant is never
configured: it is the token's. `WaitOptions::default()` waits at most 60 seconds.

## Testing

- **A workflow, before your worker exists:** `sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind` serves its
  activities from a file of canned answers and failures, on the real engine. Make `greeting.compose` fail once
  (`sequence: [{fail: {code: X, retryable: true}}, {output: ...}]`) and see the retry; make it `lose` the task and see the lease
  expire.
- **Your activities, against the platform:** the program above, against your own stack.
- **Your activities, alone:** a handler is an `async` closure of `(ctx, input)`: keep the effect behind a trait and unit
  test the logic it calls. The SDK ships no fake context yet.

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the Rust
SDK offers it once its runtime, Task 044-19 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12 (the crate's feature `workflows`, the same names in snake case as methods
of the workflow context `ctx`, durations as `Duration`); where the design leaves a name or a shape to the runtime's task
(the module path, the context and error types, registering the function on the host, the replayer's entry point), the
code below is a sketch that Task 044-19 settles. An operator also records the requirements `code-workflows-v1` and,
because this workflow declares a signal, `signal-inbox-v1` ([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is an `async` function. It reserves the stock, charges the card, sleeps one day and
sends a reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```rust
use sagat_octopus_orchestrator_sdk::workflows::{WorkflowContext, WorkflowFailure};
use sagat_octopus_orchestrator_sdk::{Value, json};
use std::{cell::Cell, rc::Rc, time::Duration};

async fn order(ctx: WorkflowContext, input: Value) -> Result<Value, WorkflowFailure> {
    let paid = Rc::new(Cell::new(false));
    let seen = Rc::clone(&paid);
    ctx.on_signal("paid", move |_payload: Value| seen.set(true));
    ctx.activity::<Value>("inventory.reserve", json!({ "orderId": input["orderId"] })).await?;
    ctx.activity::<Value>("payments.charge", json!({ "orderId": input["orderId"], "amount": input["amount"] })).await?;
    ctx.sleep(Duration::from_secs(24 * 60 * 60)).await?;
    if !paid.get() {
        ctx.activity::<Value>("customers.remind", json!({ "orderId": input["orderId"] })).await?;
    }
    Ok(json!({ "orderId": input["orderId"], "paid": paid.get() }))
}
```

Each `ctx` call is a command the platform records and executes; the activities run in an ordinary worker as above. The
function is run again from the start against the run's journal whenever a worker does not hold it in memory, so it must
be deterministic. The SDK polls it on a single-threaded executor of its own, on workflow threads where no Tokio runtime
is entered: `tokio::time::sleep` or `tokio::spawn` there panics, and the SDK fails the workflow task
(`NONDETERMINISM_RISK`). Branches go through `ctx.spawn` (not `Send`); `ctx.now()`, `ctx.random()` and `ctx.uuid()`
replace `SystemTime::now`, `rand` and `Uuid::new_v4`; and the `clippy.toml` fragment the SDK ships for workflow crates
refuses those calls and the default-hasher `HashMap` and `HashSet`, whose iteration order differs per instance. The
precondition is Rust 1.98, the SDK's baseline.

**One binary for workflow code, another for activities.** The code digest of a Rust workflow is the SHA-256 of the whole
executable, so a change of an activity linked into the same binary makes a new digest, and runs open on the old one move
to it only by a verified move. Keep the workflow functions in their own worker binary (`order-workflows`) and the
activities in another (`order-activities`).

**Write the manifest and publish the version.** The workflow binary prints its manifest:

```sh
cargo build --release --bin order-workflows
./target/release/order-workflows --sagat-manifest > manifest.json
```

```rust
let manifest: Value = serde_json::from_str(&std::fs::read_to_string("manifest.json")?)?;
let envelope = json!({ "activities": [
    { "name": "inventory.reserve" }, { "name": "payments.charge" }, { "name": "customers.remind" }
] });
api.definitions().publish_code(&manifest, &envelope, CallOptions::default()).await?;
```

`publish_code` saves the code version as the workflow's draft and publishes it, making it active. The envelope lists
what the function's commands may use; a command outside it puts the run under review. The `paid` signal is a declaration
of the code document beside the envelope (`signals: [{name: paid, buffered: true}]`, Technical Plan 3.1 of the design).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). In `order-workflows`:

```rust
let mut workflows = WorkerHost::new(WorkerOptions::new(
    grpc_url.as_str(), namespace.as_str(), "order-workflows", token,
))?;
workflows.workflow("order", order)?; // the registration's name is fixed by Task 044-19
workflows.start().await?;
```

Deploy a new workflow binary beside the old one: runs stay on the digest they started with, so keep the old binary
running until the version list shows no run left on it, or move them.

**Test it with the replayer.** Export the history of a run (`api.runs().export_history(run_id)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the current code with `cargo test`:

```rust
#[test]
fn the_new_code_replays_the_recorded_order_runs() {
    let history = std::fs::File::open("tests/histories/order-1.jsonl").expect("history");
    let verdict = Replayer::replay(std::io::BufReader::new(history), "order", order) // named by Task 044-19
        .expect("a readable history");
    assert!(verdict.matched(), "the first difference is at command {:?}", verdict.command_number());
}
```

A history of an export format other than 1 is refused, and a replay that reaches a value the export redacted stops
with `INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/rust/README.md): the client, the worker host, the builders of the workflow document, the
  errors, TLS and secrets.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the six SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
