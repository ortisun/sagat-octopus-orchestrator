# Implementation Plan: CLI, quickstarts, local testing and documentation

**Branch:** `012-developer-experience` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Technical approach:** Rust CLI with clap or a similar tool; templates tested in CI; versioned docs with source generators.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Onboarding with dependencies installed: target <15min in UX evaluation, without promising a universal time.
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

As built (the placeholders of the first draft — `tools/cli`, a `df` binary, `tests/e2e/quickstart` — were replaced by what the
repository already had; D206):

```text
apps/cli/                         sagat: new, workflow, script, worker dev|bind, dev token, doctor, and feature 011's manifest commands
apps/dev-idp/                     the development identity provider (G10): dev-idp serve|healthcheck
apps/control-plane/               seed-dev also leaves a workspace and a namespace (SAGAT_SEED_NAMESPACE)
crates/api-http/                  GET /v1/version
templates/                        hello-world, event-trigger, cron, approval, compensation, inline-script
examples/hello-world/<language>/  the hello-world in TypeScript, Go, Python, C# and Java
examples/payment-saga/typescript/ a saga with compensation and reconciliation against an in-memory provider
docs/quickstarts/                 the first day, one page for each SDK, the saga, the errors
docs/reference/                   generated from the OpenAPI document, the workflow schema and the tool's help
infra/docker/                     compose gains dev-idp; the control-plane image carries the Studio's files
tests/e2e/developer-experience/   stories, doctor, diagnostics, the provider and the seed, the script commands, the examples
tests/fault/developer-experience/ restarts, signals, caps, tenants, the token, the environment guard
tests/contract/developer-experience/  the generated reference and the examples in the documents
tests/performance/developer-experience/  the machine's part of the first day
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

- **Functional risk:** client without Docker. Mitigate via adversarial scenario with fixture.
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

A new team member executes their first runs in TS and C# following only the documentation; CLI tests cover invalid workflows.

## Example catalog extension (014)

`examples/order-processing/workflows/` supplies the canonical v1 IR for the five executable
SDK examples in sibling language directories. `tests/e2e/developer-experience/order_processing.rs`
uses the existing real-platform fixture to check the same success, business rejection and
permanent failure cases. No new engine semantics, SDK methods or migrations are required.
The detailed delivery tasks and fresh evidence belong to feature 014.
