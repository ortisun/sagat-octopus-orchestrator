# SDKs of the Sagat Octopus Orchestrator

Five SDKs — TypeScript, Python, Go, C# and Java — each with a **client** (start, get, list, cancel, signal, publish
an event, read the journal, wait for a run), a **worker host** (activities, compensations and reconcilers over the
gRPC worker protocol) and **builders** of the workflow document. They behave the same because they are held to one
specification and one conformance suite; none of them keeps a journal of its own — the platform owns every state,
every timer and every retry decision.

| | Package | Path | Needs | Unit tests | Read this |
|---|---|---|---|---|---|
| TypeScript | `@sagat-octopus-orchestrator/sdk` | [`typescript/`](typescript/) | Node ≥ 20 (run: 26.8.1) | 96 | [README](typescript/README.md) |
| Python | `sagat-octopus-orchestrator` | [`python/`](python/) | Python ≥ 3.10 (unit tests: 3.10–3.14; conformance: 3.14, once on 3.10) | 226 | [README](python/README.md) |
| Go | `github.com/ortisun/sagat-octopus-orchestrator/sdks/go` | [`go/`](go/) | Go ≥ 1.25 (run: 1.25.14) | 165 | [README](go/README.md) |
| C# | `SagatOctopusOrchestrator.Sdk` | [`dotnet/`](dotnet/) | .NET 8 or 10 (run: 10; net8.0 conformance once) | 410 | [README](dotnet/README.md) |
| Java | `dev.sagatoctopusorchestrator:sagat-octopus-orchestrator-sdk` | [`java/`](java/) | Java ≥ 17 (run: 23; unit tests on 26) | 202 | [README](java/README.md) |

"Run" is what was actually executed, on macOS arm64; the other versions are declared, not tested, and each README
says which. Every package is `0.2.0`: before `1.0` a minor version may change an API and says so; the wire contracts
(REST under `/v1`, the worker protocol `1.x`) are additive and do not move with it. **Publishing to the registries is
not automated**. All five packages use the MIT license: see `docs/runbooks/sdk-packaging.md` and
`specs/006-sdk-multilanguage/research.md` (G122, G123).

## What they have in common

- [`SPEC.md`](SPEC.md) — the normative behaviour: configuration and credentials, the canonical errors, the client, trace
  and idempotency, retries (only when safe), the worker host (negotiation, the poll loop, heartbeat, cancellation,
  reporting, compensation and reconciliation, secrets, lifecycle), the builders, serialization, versioning, safety.
- [`error-catalog.json`](error-catalog.json) — the 19 canonical errors with their HTTP and gRPC mapping, whether they may
  be repeated and what to do about each. Each SDK embeds a copy and a test fails when it is behind.
- `specs/001-platform-foundation/contracts/worker.proto` — the worker protocol. Each SDK copies it with a script and a
  test fails when its copy is behind.
- **The conformance suite**, [`tests/contract/sdk-conformance/`](../tests/contract/sdk-conformance/README.md): 15
  scenarios that each SDK plays through a small runner against a real platform (PostgreSQL, the execution engine, the
  REST API) of its own, with faults injected at the SDK's transport hooks. The five results — and the state the
  platform ends in, read without the SDK — must equal each other and the golden files. Decision:
  [ADR-0015](../docs/adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md).

```text
cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture      # the five languages, every scenario (Docker)
cargo test -p sagat-tests --test e2e_sdk_multilanguage --test fault_sdk_multilanguage -- --ignored            # the user stories and the faults
cargo test -p sagat-tests --test perf_sdk_multilanguage --release -- --ignored    # the benchmark
```

These three are `#[ignore]` in `cargo test --workspace` because they need the five toolchains (Node, Python, Go, .NET and a JDK) and Docker; the `sdks` job of CI runs them with `--ignored`. A scenario changes no golden by itself: `SAGAT_SDK_BLESS=1` rewrites them and the diff must be read.

## What an SDK promises, in one screen

- **Retries only when safe**: a `GET`, or a `POST` that carries an idempotency key or an event id; the SDK generates a
  key per call and keeps it across the retries. Never a 4xx other than a timeout or a rate limit.
- **Trace, correlation, deadline and idempotency key** are propagated; the trace of the caller is the trace of the run.
- **One error type** with a code, a status, whether it may be repeated, the trace id and a remediation sentence — never
  a payload, a token or a secret.
- **The worker host never busy-polls**, heartbeats every lease until the report is made, cancels an activity's token
  (and does nothing else to it) when the run is cancelled, the lease is lost or the host stops, and reports each
  result with a key that is the same on every retry. Against an older server it runs at that server's protocol minor.
- **Platform guarantees stay at-least-once for external effects**: the SDK gives the activity an idempotency key and a
  reconciliation hook; it cannot make a provider exactly-once.
- **A hostile or broken server is bounded**: bodies, waits, messages and polls have limits; TLS verification is never
  off; a redirect is never followed; a credential is in no log, error or printed form.
