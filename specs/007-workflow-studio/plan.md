# Implementation Plan: Visual Studio, Monaco and runtime graph

**Branch:** `007-workflow-studio` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Technical approach:** React Flow for the DAG; Monaco in the panel; versioned server-side state (optimistic concurrency) and runtime graph projected by event cursor; virtualization of large DAGs.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** 1k-node DAG usable in the reference browser; virtualized lists for 10k+; FPS targets per UX benchmark.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Visual direction:** [`design/README.md`](./design/README.md) (FR-007-009, D154–D159): tokens in `frontend/apps/studio/src/styles/base.css`, English labels, the accessibility floor above the prototype, no element without a data source.
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
frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx
frontend/apps/studio/src/features/workflows/NodeInspector.tsx
frontend/apps/studio/src/features/runs/ExecutionGraph.tsx
frontend/apps/studio/src/features/runs/Timeline.tsx
crates/api-http/src/routes/runs.rs
tests/contract/
tests/integration/
tests/fault/
tests/e2e/workflow-studio/
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

- **Functional risk:** graph with 50k nodes. Mitigate via adversarial scenario with fixture.
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

E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces.

## Visual defect correction (2026-10-12)

T007-051 applies the owner's explicit Temporal/Windmill UX references to graph information
hierarchy: readable identity/type/status on cards, configuration and execution detail through the
existing selected inspector. Replace cramped card budgets with fixed variant dimensions and
non-shrinking text rows, an independent port rail and moderate corner radii. Bound preview content
deliberately; do not conceal ordinary identifiers or primary states with hover-only popups.
Scope the amount/unit layout to the duration field, since its row class is also reused by other
controls. Test glyph rectangles and overlap, not merely presence/page overflow; run the graph
benchmarks because the larger dimensions change visible density. No runtime, API, tenant, audit
or OTel semantics change; G159 remains the existing browser telemetry gap.

Asset replacement recovery (T007-050): serve the current bounded `index.html` on each page request,
including deep links and `/studio/index.html`, instead of retaining its old asset hashes for the
process lifetime. Preserve CSP, `no-store`, path confinement, asset transfer limits and missing-file
404s. An unavailable page returns a retryable 503, never the obsolete page. Eagerly include a route
error boundary with a user-triggered reload and a warning about unsaved edits and signing in again;
do not automatically reload or expose exception details. Test rebuilds without restarting the HTTP
server and a failed lazy CSS request followed by recovery in Chromium. No API, tenant, operation
audit or OTel contract changes; browser telemetry remains G159.

Preserve the approved tokens and English labels. Separate readable initial graph framing from the
explicit fit-all command, keeping fixed-size nodes, virtualisation and the user's camera after
updates. Reflow palette/canvas/inspector according to available width; keep selected system-colour
controls legible. Compact the run header, retain all technical facts in a native disclosure, and
size the run canvas to the remaining viewport. Verify with the existing real-platform browser
harness plus geometry assertions and review captures. No API, tenant, audit, typed-error or OTel
semantics change; existing protected operations remain the data source (browser telemetry remains
the documented G159 gap).

## Workflow schema editor completion (2026-10-12)

Reuse the existing script editor's lazy-loaded Monaco JSON service and bundled meta-schema for
workflow input/output schemas, with separate model addresses. The document contract remains an
object or absent; incomplete JSON remains local and the last valid schema stays in the draft.
Plain-text mode preserves that local text and exposes parse errors through accessible descriptions.
No remote schema requests or changes to workflow validation semantics are introduced.

## Authorized structural payload inspection (T007-048)

Implement [ADR-0028](../../docs/adr/ADR-0028-authorized-structural-payload-inspection.md) as an
additive audited read, using feature 002's immutable inline payload/journal references, feature
009's permission/audit boundaries, ADR-0023's external catalog provenance and ADR-0027's canonical
namespace API admission. This dependency approval does not authorize changes to execution
transactions, blob capabilities, retention or customer value-disclosure policy.

The application defines the selector/provenance/projection types and inspection use case in a new
`crates/application/src/payloads.rs`; a small projector port is implemented in `crates/infra-ir`.
A new PostgreSQL `payload_inspection.rs` reads the selected immutable data and commits the bounded
audit before exposing a projection. The HTTP adapter adds
`POST /v1/runs/{run_id}/payload-inspections`, with only input or journal-sequence selectors, and
the dedicated permission. The Studio adds a lazy inspection view to run input/history; its
server-provided tree contains no original keys or scalar values. External content returns verified
catalog metadata with an explicit notice; it is not downloaded for this operation.

Build the cross-tenant, canary-in-key/value, audit-failure, historical-selector and resource-bound
tests first; then the PostgreSQL/use-case/REST path and accessible Studio view. Keep graph/SSE
responses unchanged. Check generated OpenAPI types, late-response cancellation and a real browser
journey. At implementation delivery update `data-model.md`, `research.md` G144, the design table,
traceability and quickstart with the actual tests. T007-048 is not complete at design acceptance;
customer classification and raw-value disclosure remain explicitly pending afterward.
