# Implementation Plan: Per-field payload classification

**Branch**: `feature/016-payload-classification` | **Date**: 2026-10-16 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/016-payload-classification/spec.md`; decision [ADR-0033](../../docs/adr/ADR-0033-payload-values-are-classified-per-field.md).

## Summary

Add an author-declared classification (`public`, `internal`, `pii`, `secret`) to the JSON Schemas the
workflow IR carries, check it at publication, pin its interpretation to the published version, and
disclose values through one new audited operation whose audience follows the level. The structural
inspection of ADR-0028, the declared operation results of ADR-0031 and every payload-free read stay as
they are. The work is dependency-ordered: domain types and permissions, schema walker and publication
check, storage, the use case and endpoint, the grant resource, then the CLI, the Studio, the SDK
setters, the canary and fault suites and the benchmark. Nothing is implemented by this specification.

## Technical Context

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`); the locked TypeScript, Go, Python, .NET and
Java toolchains for the SDK setters; React/TypeScript for the Studio. No toolchain change.

**Primary Dependencies**: existing `jsonschema` 0.58 (already in `crates/infra-schema` and
`crates/infra-ir`, no network or file resolution), `serde_json` in adapter crates only, SQLx
PostgreSQL, Axum. No new crate, no new service, no infrastructure dependency.

**Storage**: PostgreSQL. One additive column (`workflow_versions.classification_rules`) and one additive
table (`payload_reveal_grants`). No change to `payloads`, `run_events`, `runs` or the blob catalog.

**Testing**: domain unit and property tests; adapter tests of the walker over a fixture corpus; real
PostgreSQL integration and fault suites; REST contract tests against OpenAPI; five-SDK conformance for
the builder golden; Studio component tests and a real Chromium journey; the canary suite across every
boundary; a performance run. Each new suite is a registered target in `tests/Cargo.toml`.

**Target Platform**: the existing single-region Linux containers and local Docker Compose; the Studio in
current browsers.

**Project Type**: modular monolith with shared contracts, five SDKs, a CLI and a web Studio.

**Performance Goals**: bound and then measure. Per response: payload at most 256 KiB, carrier at most
64 KiB, at most 1,024 returned nodes, response at most 256 KiB, one cached validator per version and
carrier. Report p50/p95/p99, cache hit ratio, allocations and response size on a stated environment
(T016-053). No latency SLO is promised before the numbers exist.

**Constraints**: the domain imports no `serde_json`, Axum, SQLx or `jsonschema` (the walker is an
adapter behind a port); the error set is closed (`details.reason` only); no value, key or schema
fragment reaches a log, span, metric, audit record or error; `begin_safely` for every new transaction
(ADR-0014); the API admission of ADR-0027 is charged once; existing published versions disclose nothing;
secrets and credentials are never classified into the journal by this feature.

**Scale/Scope**: 8 user stories, 29 functional and 6 non-functional requirements, 10 success criteria,
one new endpoint family (1 disclosure, 3 grant operations) and three event reads whose response changes,
2 permissions, 5 audit actions, 1 column, 1 table, 5 error reasons, 7 problem codes, 2 diff codes.

## Constitution Check

These are design checks, not implementation approvals or test results.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I - durable atomic transitions | Pass | Pass | The feature participates in no state transition, scheduling, timer, subscription or outbox write. Its own writes (grant plus audit, reveal read plus audit) are single transactions. `data-model.md` section 4; T016-029, T016-035, T016-036. |
| II - pure domain, inward dependencies | Pass | Pass | `Level`, `Audience` and the visibility function are pure domain; the walker and validator are adapter code in `crates/infra-schema` behind an application port; `scripts/check-architecture.py` stays green (`jsonschema` is not a domain dependency). No empty mapper: the HTTP body type is the one necessary wire boundary, as for inspection. T016-011, T016-054. |
| III - at-least-once effects | Pass (not applicable) | Pass (not applicable) | No external effect. A retried reveal audits again by design (`data-model.md` section 5). |
| IV - pinned, compatible | Pass | Pass | `classification_rules` pins the interpretation to the version; legacy is `0`; IR and OpenAPI changes are additive; five-SDK golden. T016-006, T016-008, T016-020, T016-042. |
| V - isolation of code, tenant and secrets | Pass | Pass | Tenant-scoped joins and composite foreign keys; opaque `404`; `secret` never returned; no oracle for callers without structure permission; classification is not the secret store (D287). T016-025, T016-049, T016-051. |
| VI - observability and audit | Pass | Pass | Span, closed-label metrics, five audit actions, typed errors with remediation, canary across logs, spans and metrics. T016-030, T016-049. |
| VII - inbox/outbox | Pass | Pass | The feature neither writes the outbox nor consumes events, and does not change how events are admitted, deduplicated or dispatched. It restricts what the inbox and dead-letter reads return (US08, D291): a read-side change with no effect on the transaction that stores an event. T016-057 to T016-064. |
| VIII - failure tested, contracts verified, honest | Pass | Pass | Crash before and after the audit commit, audit failure, duplicate reveal, grant revoked mid-flight, restart with an empty cache, concurrent publish; OpenAPI and IR schema compiled in CI; nothing ticked without evidence. T016-033, T016-050. |

**Governance gate**: no new ADR (D290). ADR-0033 is the decision. The owner answered the four product
questions on 2026-10-17 (P1 grant list, P2 restrict event values now, P3 restrictive readings, P4 audited
self-grant); the answers are an addendum to ADR-0033's Consequences and rows of `docs/owner-decisions.md`.
A change of the audience table in ADR-0033 would need a new ADR before it is built.

## Project Structure

### Documentation (this feature)

```text
specs/016-payload-classification/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── analysis.md
├── contracts/
│   └── README.md
├── checklists/requirements.md
└── tasks.md
```

### Source code (where the work lands)

```text
crates/domain/src/classification.rs                  # new: Level, Audience, Visibility, ClassificationRules
crates/domain/src/identity.rs                        # PayloadReadInternal, PayloadRevealPii; Admin = ALL minus reveal
crates/domain/src/audit.rs                           # five actions, collapse rules
crates/domain/tests/{classification.rs,identity.rs}
crates/application/src/payload_disclosure.rs         # new: prepare/disclose use case, ports (store, projector)
crates/application/src/reveal_grants.rs              # new: grant use case and port
crates/application/src/{api_admission.rs,definitions.rs,diff.rs,events.rs,connectors.rs}   # events.rs, connectors.rs: US08
crates/infra-schema/src/classification.rs            # new: publication check, walker, projector, validator cache
crates/infra-ir/src/{classification_check.rs,schema.rs,convert.rs,lib.rs,diff.rs}
crates/infra-postgres/migrations/<timestamp>_payload_classification.{up,down}.sql
crates/infra-postgres/src/{payload_disclosure.rs,reveal_grants.rs,definitions.rs,events.rs,connectors.rs}   # events.rs, connectors.rs: US08
crates/infra-postgres/storage-classification.json    # the two new text columns
crates/api-http/src/routes/{payload_disclosure.rs,reveal_grants.rs,events.rs,connectors.rs}   # events.rs, connectors.rs: US08
crates/api-http/src/{error.rs,lib.rs,exec.rs,dto_gov.rs,dto_events.rs}
apps/cli/src/{workflow.rs,args.rs,reveal.rs}
frontend/apps/studio/src/features/runs/{PayloadDisclosure.tsx,NodePanel.tsx,RunDetailPage.tsx}
frontend/apps/studio/src/features/workflows/WorkflowSettings.tsx   # classification report
frontend/apps/studio/src/api/schema.d.ts             # generated from OpenAPI
sdks/{typescript,go,python,dotnet,java}/             # outputSchema setters, node outputSchema option
specs/001-platform-foundation/contracts/{workflow.schema.json,openapi.yaml}
tests/contract/payload-classification/               # IR corpus, OpenAPI, audience matrix, events
tests/{contract,integration,e2e}/events-webhooks-messaging/   # existing readers of event data updated for US08
tests/integration/payload-classification/            # real PostgreSQL
tests/fault/payload-classification/                  # crashes, audit failure, races
tests/e2e/payload-classification/                    # API + engine + CLI
tests/contract/sdk-conformance/scenarios/classification.json   # + expected/
tests/performance/payload-classification/            # + tests/performance/reports/payload-classification.md
frontend/e2e/tests/payload-disclosure.spec.ts
docs/runbooks/payload-classification.md              # new
docs/runbooks/events.md                              # US08: what GET events and dead letters return now
docs/reference/{workflow-document.md,api.md,cli.md}  # regenerated
```

The disclosure use case copies the *shape* of `crates/application/src/payloads.rs` (a `prepare` that
authorizes and resolves, a `disclose` that reads and audits) without sharing its types: the structural
projection has no value to carry and the new result has no use for its tree type except in the
`structure-only` arm, where it reuses `PayloadStructure` unchanged. The walker and the validator live in
one adapter crate so that publication (`infra-ir`) and runtime disclosure interpret an annotation with
the same code.

## Phase 0 - Research and decisions

`research.md` records the repository evidence, decisions D261-D295, gaps G272-G293 and the owner's
answers of 2026-10-17 to the four product questions. Nothing blocks code on a product question any more.
What stays undecided (event field classification, dead-letter reveal on the list, retention, taint
tracking, a membership surface) is listed at the end of `research.md`.

## Phase 1 - Design and contracts

- `data-model.md`: types, tables, lifecycle, failure table, tenant boundary and the map from
  requirement to code.
- `contracts/README.md`: the exact IR keyword and carrier list, publication findings, OpenAPI
  operations and schemas, the audience matrix, audit actions and telemetry.
- `quickstart.md`: how to validate each story, separating what can run today (document checks) from
  suites that exist only once built.
- `tasks.md`: dependency-ordered tasks, tests before implementation.

## Phase 2 - Dependency-ordered implementation

1. **Foundations (blocks everything)**: contract schema and fixtures; domain `Level`, `Audience`,
   `Visibility`, permissions, audit actions; unit tests first.
2. **US01 (publication)**: walker and publication check in `infra-schema`/`infra-ir`; migration and
   publish path pinning `classification_rules`; `validate`, diff findings; OpenAPI problem codes.
3. **US02 + US04 core (disclosure of input and result)**: application use case, store, projector,
   endpoint, admission; the canary suite starts here and grows with each surface.
4. **US03 (reveal)**: grant table and resource, reveal path, audit; CLI grants (P1 decided: per-namespace list).
5. **US08 (event values)**: tests first, then the OpenAPI change, application, stores, routes; the
   repository's own readers of `data`, `body` and `headers` are updated with it. Depends on the grant
   table and the reveal path of step 4.
6. **US05 (step outputs)**: node-level `outputSchema` in the IR, codec and journal selector; SDK
   setters and golden.
7. **US06 (Studio)** and CLI `--values`.
8. **US07 (report)**: Studio settings panel, CLI validate report.
9. **Release gates**: fault suite, canary across all boundaries, benchmark, runbook, documentation,
   traceability, ADR-0033 follow-up note. `source: "result"` ships when the G21 change is merged.

## Rollout, compatibility and rollback

Expand first: the migration adds a defaulted `NOT NULL` column and a table, both ignored by the
previous release (an older engine never reads `classification_rules` and never touches the grant table).
Publish and disclosure ship together behind the same release; there is no feature flag because the
operation is additive and discloses nothing for a version without annotations. Rolling back the
application leaves the column and table in place (the migration's `down` removes them only on an
explicit down-migration, after which versions revert to disclosing nothing). No stored run, journal
event or payload is rewritten. Old Studios and clients keep working: they never call the new paths. The exception is the three event
reads of US08, whose response changes for every client that read the values (G292); that change is
announced in the release notes and has no switch.

## Risks and mitigations

| Risk | Mitigation / evidence required |
|---|---|
| A walker bug discloses a `pii` or `secret` value | The domain visibility function is pure and exhaustively tested; the walker only ever raises levels (`max`); the canary suite covers every level in values and keys, under `allOf`/`oneOf`/`$ref`, undeclared, truncated; property tests assert monotonicity (no child below its parent) |
| An evaluator and validator disagree about the shape | The validity gate runs first on the whole payload; any doubt (cycle, budget, unsupported) ends as structure only, never as a value |
| `jsonschema` 0.58 reacts to the unknown keyword | T016-013 proves the validation result is identical with and without annotations; fallback: compile a stripped copy |
| An annotated root makes every unannotated child public | `CLASSIFICATION_INHERITED_OPEN` warning, the report and the diff findings (G286) |
| Restricting event values breaks clients that read `data`, `body` or `headers` (G292) | Owner decision D291; OpenAPI descriptions, API reference, runbook and release notes; the repository's own readers (contract and e2e suites) updated in T016-064; Studio, CLI and SDKs read none of them (checked) |
| Cost of compiling and validating per response | Bounds, cache, one admission charge; measured (T016-053); a request cannot exceed 256 KiB of payload and 64 KiB of schema |
| Reveal audit records grow without bound | They are bounded by the namespace API bucket (ADR-0027) and the number of holders; the audit retention of feature 009 applies |
| A grant outlives a demotion or a removed member | The grant row cascades with the membership; the effective check also requires `payload:read-internal` at read time |
| A published document is read differently by a later release | `classification_rules` pins the interpreter; version 1 is never reinterpreted |
| Studio or CLI echo a revealed value into a store, log or clipboard history | No persistence, abort and clear on change, the CLI prints to stdout only, control characters neutralised; Studio storage assertions in the Chromium journey |
| Dependency on unmerged branches (run result; 014) | Recorded in the spec; `source: "result"` is its own task gated on G21; nothing links to files absent from this branch |

## Complexity Tracking

| Item | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A grant table outside the role-to-permission matrix | ADR-0033 requires `payload:reveal-pii` to be granted explicitly while the roles stay six; G39's pattern for narrower control is a list at the resource | Granting it by role (not explicit), a seventh role (breaks one role per membership), a tenant-wide flag (wider than needed); the owner chose the list on 2026-10-17 |
| `workflow_versions.classification_rules` | Makes "immutable with the version" true of the interpretation, not only of the document | Storing nothing (a later release reinterprets old versions); a derived index per version (a second artifact to keep consistent) |
| A second payload operation beside `payload-inspections` | Different audience, different response, different policy; the old one must not change | Changing the old response (breaks clients); a policy switch on one operation (two shapes under one id) |
| A per-process validator cache | Compiling a 64 KiB schema per request dominates the cost | Compile per request (simple, expensive); a shared cache (needs invalidation the immutable version removes) |
| Projection computed inside the read transaction | The audit record has to describe what the answer contains and must precede the bytes | Audit before knowing the content (records a reveal that returned no `pii`); project after commit (the audit would not precede the decision) |
