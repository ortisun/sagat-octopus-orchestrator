# SDKs of the Sagat Octopus Orchestrator

> Documentation status: the contracts below describe inspected source. Existing runtime, test-count and advisory records are historical developer notes; this update did not rerun or revalidate those records.

Six SDKs — TypeScript, Python, Go, C#, Java and Rust — each with a **client** (start, get, list, cancel, signal, publish
an event, read the journal, wait for a run), a **worker host** (activities, compensations and reconcilers over the
gRPC worker protocol) and **builders** of the workflow document. They share one
specification and one conformance suite; none of them keeps a journal of its own — the platform owns every state,
every timer and every retry decision.

| | Package | Path | Needs | Unit tests | Read this |
|---|---|---|---|---|---|
| TypeScript | `@sagat-octopus-orchestrator/sdk` | [`typescript/`](typescript/) | Node ≥ 20 (run: 26.8.1) | 96 | [README](typescript/README.md) |
| Python | `sagat-octopus-orchestrator` | [`python/`](python/) | Python ≥ 3.10 (unit tests: 3.10–3.14; conformance: 3.14, once on 3.10) | 226 | [README](python/README.md) |
| Go | `github.com/ortisun/sagat-octopus-orchestrator/sdks/go` | [`go/`](go/) | Go ≥ 1.25 (run: 1.25.14) | 165 | [README](go/README.md) |
| C# | `SagatOctopusOrchestrator.Sdk` | [`dotnet/`](dotnet/) | .NET 8 or 10 (run: 10; net8.0 conformance once) | 410 | [README](dotnet/README.md) |
| Java | `dev.sagatoctopusorchestrator:sagat-octopus-orchestrator-sdk` | [`java/`](java/) | Java ≥ 17 (run: 23; unit tests on 26) | 202 | [README](java/README.md) |
| Rust | `sagat-octopus-orchestrator-sdk` | [`rust/`](rust/) | Rust 1.98 / Tokio | 109 native | [README](rust/README.md) |

"Run" is what was actually executed, on macOS arm64; the other versions are declared, not tested, and each README
says which. Five SDK packages declare `0.2.0`; the new Rust crate is a `0.3.0` candidate.
The coordinated 0.3 release updates the existing package versions during release preparation.
Before `1.0`, a minor version may change an API and says so; the wire contracts
(REST under `/v1`, the worker protocol `1.x`) are additive and do not move with it. **Publishing to the registries is
not automated**. All six packages use the MIT license: see `docs/runbooks/sdk-packaging.md` and
`specs/006-sdk-multilanguage/research.md` (G122, G123).

## What they have in common

- [`SPEC.md`](SPEC.md) — the normative behaviour: configuration and credentials, the canonical errors, the client, trace
  and idempotency, retries (only when safe), the worker host (negotiation, the poll loop, heartbeat, cancellation,
  reporting, compensation and reconciliation, secrets, lifecycle), the builders, serialization, versioning, safety.
- [`error-catalog.json`](error-catalog.json) — the 19 canonical errors with their HTTP and gRPC mapping, whether they may
  be repeated and what to do about each. Each SDK embeds a copy and a test fails when it is behind.
- `specs/001-platform-foundation/contracts/worker.proto` — the worker protocol. Each SDK copies it with a script and a
  test fails when its copy is behind.
- **The conformance suite**, [`tests/contract/sdk-conformance/`](../tests/contract/sdk-conformance/README.md): 60
  scenarios that each SDK plays through a small runner against a real platform (PostgreSQL, the execution engine, the
  REST API) of its own, with faults injected at the SDK's transport hooks. The six results — and the state the
  platform ends in, read without the SDK — must equal each other and the golden files. Decision:
  [ADR-0015](../docs/adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md).

```text
cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture      # the six languages, every scenario (Docker)
cargo test -p sagat-tests --test e2e_sdk_multilanguage --test fault_sdk_multilanguage -- --ignored            # the user stories and the faults
cargo test -p sagat-tests --test perf_sdk_multilanguage --release -- --ignored    # the benchmark
```

These three are `#[ignore]` in `cargo test --workspace` because they need the six toolchains (Rust, Node, Python, Go, .NET and a JDK) and Docker; the `sdks` job of CI runs them with `--ignored`. A scenario changes no golden by itself: `SAGAT_SDK_BLESS=1` rewrites them and the diff must be read.

## Shared SDK behavior and limits

- **Retries only when safe**: a `GET`, or a `POST` that carries an idempotency key or an event id; the SDK generates a
  key per call and keeps it across the retries. Never a 4xx other than a timeout or a rate limit.
- **Trace, correlation, deadline and idempotency key** are sent to the platform; the client supplies the caller's trace context.
- **One error type** with a code, a status, whether it may be repeated, the trace id and a remediation sentence — never
  a payload, a token or a secret.
- **The worker host never busy-polls**, heartbeats every lease until the report is made, cancels an activity's token
  (and does nothing else to it) when the run is cancelled, the lease is lost or the host stops, and reports each
  result with a key that is the same on every retry. Against an older server it runs at that server's protocol minor.
- **Platform guarantees stay at-least-once for external effects**: the SDK gives the activity an idempotency key and a
  reconciliation hook; it cannot make a provider exactly-once.
- **A hostile or broken server is bounded**: bodies, waits, messages and polls have limits; SDK-created TLS transports retain verification and refuse redirects. Custom transport behavior and
  handler-authored log text remain the caller's responsibility.

Rust acceptance is tracked in [feature 034](../specs/034-rust-sdk/tasks.md). Historical five-language
receipts remain historical; current six-language acceptance requires the common suite. For a Rust-only
local run, set `SAGAT_SDK_LANGS=rust` and an owned external `SAGAT_RUST_SDK_TARGET_DIR`, separate
from the backend `CARGO_TARGET_DIR`.

<!-- EGM directory contract -->

## Interface

- The shared catalog maps 19 canonical error codes to HTTP/gRPC/default retryability/remediation metadata. The copied WorkerService schema defines registration, task delivery, heartbeat, outcomes, secrets, blobs and additive managed-database messages. [error-catalog.json:errors] [typescript/proto/worker.proto:WorkerService]
- The language packages expose REST clients, activity hosts and workflow IR authoring; detailed language surfaces and internal boundaries are linked below. [typescript/src/index.ts:SagatClient] [python/sagat_octopus_orchestrator/__init__.py:__all__] [go/client/client.go:Client] [java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/SagatClient.java:SagatClient] [dotnet/src/SagatOctopusOrchestrator.Sdk/ISagatClient.cs:ISagatClient] [rust/src/lib.rs:Client]

- [typescript](./typescript/README.md) — language package contract.
- [python](./python/README.md) — language package contract.
- [go](./go/README.md) — language package contract.
- [dotnet](./dotnet/README.md) — language package contract.
- [java](./java/README.md) — language package contract.
- [rust](./rust/README.md) — language package contract.

## Invariants

- The ordinary SDK hosts advertise protocol 1.5; the copied schema separately includes additive 1.6 ResolveDatabase for database-runner identities. Wire declarations do not by themselves prove server authorization, durability or external-effect guarantees. [typescript/src/config.ts:PROTOCOL_VERSION] [python/sagat_octopus_orchestrator/config.py:PROTOCOL_VERSION] [go/version.go:ProtocolVersion] [java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/SagatSdk.java:PROTOCOL_VERSION] [dotnet/src/SagatOctopusOrchestrator.Sdk/SdkInfo.cs:ProtocolVersion] [rust/src/worker.rs:WorkerOptions] [typescript/proto/worker.proto:ResolveDatabase]

## Side Effects

- Clients and worker hosts perform HTTP/gRPC I/O and invoke application handlers; language contracts describe their lifecycle, retry, credential and telemetry effects. Builder validation and JSON construction are local operations. [typescript/src/client.ts:SagatClient] [typescript/src/worker/host.ts:WorkerHost] [typescript/src/workflow.ts:WorkflowBuilder]

## Verification

Use the per-language commands in the linked package contracts. From the repository root, the shared harness target is:

```sh
cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

The target is declared in `tests/Cargo.toml`; it requires the SDK toolchains and repository-owned test infrastructure. Use private external Cargo target directories as described in [AGENTS.md](./AGENTS.md). No build or test command was executed during this documentation update.
