# Implementation Plan: GitOps, versioning and definition promotion

**Branch:** `feature/011-gitops-versioning` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Technical approach:** Digest-immutable registry; active-version alias/pointer as transactional metadata; exportable JSON/YAML manifests.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Asynchronous publishing in builds; the active pointer switch must be transactional.
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

```text
crates/domain/src/versioning.rs                      the policy, the request and its rules (built)
crates/application/src/publishing.rs                 the use cases: policy, requests, activation, promotion (built)
crates/infra-postgres/src/versioning.rs              the adapter, and migrations/20261016000000_versioning (built)
crates/api-http/src/routes/versioning.rs             the REST operations (built; the plan said routes/versions.rs)
apps/cli/                                            the `sagat` command line (built; the plan said tools/cli/)
crates/script-registry/src/registry.rs               not used: artifacts are pinned by digest from the versions (G205)
frontend/apps/studio/src/features/workflows/VersionDiff.tsx   not built (G198)
specs/011-gitops-versioning/contracts/manifest.schema.json
tests/contract/gitops-versioning/{rest,manifest,fixtures}/
tests/integration/gitops-versioning/
tests/fault/gitops-versioning/
tests/e2e/gitops-versioning/
tests/performance/gitops-versioning/  and  tests/performance/reports/gitops-versioning.md
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

- **Functional risk:** draft edit conflict. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Benchmark (T011-033)

`tests/performance/gitops-versioning/main.rs` measures publication as the document grows, the switch of the pointer with and without triggers, runs started while it moves, contention on one workflow and across workflows, a long history, the approval path and `sagat` over HTTP for a manifest of fifty workflows; the hypotheses it holds the system to and the figures are in `tests/performance/reports/gitops-versioning.md`.

## Delivery Gate

V2→V3 with a suspended V2 run resumes correctly; rollback only affects new runs. Proved by `tests/e2e/gitops-versioning/us02.rs` (a run waiting on version 1 resumes with version 1's activity after version 2 is published, and a run started after it uses version 2) and `us04.rs` (a rollback moves the pointer and rewrites nothing; the run in flight keeps its version).


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Published definitions using new capabilities and operation disclosure policies are immutable and each run pins its immutable workflow version and capability requirements. Continuation keeps the same version; rollback disables new starts but retains compatible readers/workers for live runs using those capabilities. Existing root-workflow and publication approval semantics remain unchanged.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
