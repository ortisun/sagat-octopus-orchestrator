# Implementation Plan: Script registry, code editor and inline execution

**Branch:** `005-inline-code-sandbox` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Allow writing and publishing code directly on the platform without manual application build/deploy.

**Technical approach:** ScriptRegistry controls src/digest and the build pipeline; worker supervisor manages isolated pools. Inline Go/C#/Java are compiled automatically into artifacts and executed outside privileged processes.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Cold/warm start metrics per language; caches only if policy/digest are identical; builds separate from execution.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Scale/Scope:** scale horizontally without keeping live processes for waits.

## Constitution Check

- [x] State and retry semantics documented (`I`).
- [x] Modularity/ports and adapters planned (`II`).
- [x] Saga and reversibility applied where relevant (`III`).
- [x] Compatibility and version pinning (`IV`).
- [x] Code/tenant boundary and secrets (`V`).
- [x] OTel and auditable journal (`VI`).
- [x] Redeliverable events and inbox/outbox (`VII`).
- [x] Fault injection/contract plan (`VIII`).

These checks indicate conformance **of the design**, not approval of the implementation.

## Project Structure

The plan proposed `crates/script-registry`, `crates/sandbox`, `apps/worker-supervisor` and a Studio editor. As built (ADR-0013, G109, G110), by the same rule as feature 004: a crate boundary has to buy isolation, and these would have had to import the whole run state or the whole application layer for none.

```text
crates/domain/src/scripts.rs                       languages, source, lockfile, digest, limits and policy, versions, failures, redaction
crates/domain/src/{identity,audit,quota}.rs        script:* permissions, the runner role, script.* audit, scripts / script_jobs quotas
crates/application/src/scripts.rs                  ScriptService (authors), RunnerService (the runner), ports ScriptStore / RunnerStore / ScriptLookup
crates/application/src/sandbox.rs                  Sandbox and HostCalls ports, the build key
crates/application/src/script_signing.rs           Ed25519 signer and verifier of artifacts
crates/infra-sandbox/                              the container driver, the JSON-line protocol, the lockfile policy, the shims (Node, Python)
crates/infra-schema/                               JSON Schema: check a schema, validate without echoing, infer from a value
crates/infra-postgres/src/scripts.rs               the SQL; migration 20261014000000_scripts
crates/infra-ir/src/convert.rs                     a script node becomes the activity script.sha256-<hex>
crates/api-http/src/routes/{scripts,runner}.rs     25 operations
apps/control-plane                                 SAGAT_SCRIPT_SIGNING_KEY, SAGAT_SCRIPT_CEILING, `script-key`
apps/sandbox-runner/                               the runner: builds, test runs and script steps in containers
infra/docker/                                      the runner image and the `sandbox` compose profile
tests/integration/005-inline-code-sandbox/{registry,sandbox}/   PostgreSQL (versions, leases, quotas, retention, migration) and real containers (isolation)
tests/e2e/005-inline-code-sandbox/                 US01..US04 through the API, the worker protocol and the runner
tests/fault/005-inline-code-sandbox/               runner death, restart, engine down, cancellation, room
tests/contract/005-inline-code-sandbox/rest/       every operation against openapi.yaml
tests/performance/005-inline-code-sandbox/         the benchmark; report in tests/performance/reports/
```

## Implementation phases

1. Domain contracts and types, invariant validation + tests.
2. Persistence and port adapters + migration and integration tests.
3. APIs/command processing + versioning + authorization and observability.
4. Happy path flow with e2e tests.
5. Fault injection, redelivery, timeouts, concurrency and regression.
6. Security/performance review, documentation and release gate.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- Canonical contracts in `specs/001-platform-foundation/contracts/` with reviewed migrations.
- Applicable ADRs in `docs/adr/`.

## Risks & Mitigations

- **Functional risk:** dependency changed remotely. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivery Gate

TS/Python/C# script goes from the editor to execution; escape tests and CPU/memory/network limits verified.
