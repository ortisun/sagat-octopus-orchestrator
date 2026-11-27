# Specification Quality Checklist: Run visibility, analytics and archival

**Purpose**: Validate specification completeness and quality before planning and implementation.
**Created**: 2026-10-26. **Feature**: [spec.md](../spec.md).
**Review ownership**: filled by the specification author on 2026-10-26 as a self-review; a reviewer must
confirm before implementation reads it. `[x]` means the requirement-quality criterion is met, not
that anything is built.

## Content quality

- [x] No implementation details (languages, frameworks, APIs). Accepted 2026-11-24, delegate (Owner decision
  2026-11-24-4): the transaction, fence and purge guarantees are what the tests check, and the house style of
  014, 016, 017, 030 and 031 states them the same way; layout stays in plan, data model and contracts. Was
  *Not met, deliberately:* following the
  house style of 016 and 017, the FRs state the binding architectural guarantees (one transaction with the
  transition, the shard fence, the transaction horizon, the privileged purge) because those are what the
  tests check; sink kinds (ClickHouse, PostgreSQL, object storage) are user-visible product choices.
  Storage layout, crates and SQL live in plan, data-model and contracts.
- [x] Focused on user value: operators find runs, analysts receive data, administrators bound retention;
  each story says why it has its priority.
- [x] Written for non-technical stakeholders. Accepted 2026-11-24, delegate (Owner decision 2026-11-24-4): the
  stories and success criteria are plain, and the requirements serve the reviewers who must test them. Was
  *Partly:* stories and success criteria are readable without
  the code; FRs assume the platform vocabulary (journal, run, sink, classification level).
- [x] All mandatory sections completed (stories, edge cases, FR, NFR, entities, SC, assumptions, scope).

## Requirement completeness

- [x] No `[NEEDS CLARIFICATION]` markers; three owner questions (O1080–O1082) carry recommended defaults
  that the plan assumes.
- [x] Requirements are testable: 43 FRs, each traced from at least one task and one SC (checked by script
  on 2026-10-26). *2026-11-24:* 46 requirements now (FR-020-044 was added on 2026-10-27, FR-020-045 and
  FR-020-046 on 2026-11-24; 43 + 3 = 46); the two new ones are traced from Task 020-064 and Task 020-066.
- [x] Success criteria are measurable (counts, 100% batteries, equality with a fresh derivation).
- [x] Success criteria are technology-agnostic. Accepted 2026-11-24, delegate (Owner decision 2026-11-24-4):
  write-path cost and deduplicated sink content are the observable outcomes a buyer of this feature cares
  about; naming their measure is what makes them testable. Fixed with it: SC-020-004 measures WAL bytes,
  index-tuple inserts and commit latency since 2026-10-27, not a statement trace. Was *Partly:* SC-020-004
  relies on a statement trace and
  SC-020-005 on deduplicated sink content; both are observable outcomes but name platform mechanisms.
- [x] Acceptance scenarios defined for all nine stories, with failure scenarios.
- [x] Edge cases identified: long transactions, purged notes, sink edits mid-batch, concurrent
  registration, tightening/retiring, continuation, unfinished trees, concurrent digest reuse, missing
  buckets, timeout/signal race, clocks.
- [x] Scope bounded: out-of-scope list; ownership boundaries with 018, 019, 022, 026, 028, 029, 031, 032.
- [x] Dependencies and assumptions identified, with verified paths (research §1).

## Feature readiness

- [x] Every FR has acceptance criteria through a story scenario or an SC.
- [x] User scenarios cover the primary flows of the three capabilities and their failures.
- [x] Transaction boundaries, lock order, authorization matrix, finite ceilings, failure matrix,
  observability vocabulary and Constitution Check are in the plan.
- [x] Tenant boundary, typed errors (closed codes, reasons only), audit and OTel are specified.
- [x] Five-SDK scope decided: builders and clients (US01), history client (US07), exporter helper (US06),
  each with a shared conformance scenario. *Corrected 2026-11-24:* six SDKs, the Rust work in Task
  020-063 (research Decision 020-001).
- [x] Encrypted payloads (031) are covered (FR-020-043).
- [x] No implementation details leak into the specification. Accepted 2026-11-24 with the first item, for
  the same reason. Was *Not met, deliberately* (see first item).

## Open gates before implementation

- [x] Owner answers O1080 (default retention), O1081 (`pii` exports), O1082 (run stubs) — T020-002.
  Decided 2026-11-24, delegate (Owner decision 2026-11-24-4): no default, the uniform rule of D1617,
  stubs kept (research.md §5).
- [x] Owner accepts or amends ADR-0053 and ADR-0054 — T020-002. Accepted 2026-11-24, delegate (Owner
  decision 2026-11-24-4): ADR-0053 revised first (no shard fence, placement by connection), ADR-0054 with
  its archive destinations and step logs (research Decisions 020-013 and 020-014).
- [ ] The MINOR amendment of constitution principle I proposed in ADR-0054 is accepted by PR (terminal
  runs only; the verified archive as sole copy; `archive = NONE` means deletion) — T020-061; US08/US09
  are gated on it (G1101). *2026-11-24: left open:* amending the constitution is not a delegate's act
  (Owner decision 2026-11-24-4 does not cover it); the text is proposed for constitution 5.0.0 (Owner
  decision 2026-11-24-2) or a later MINOR amendment. Only User stories 020-08 and 020-09 wait for it. *(2026-11-26:
  made in constitution 5.1.0 without `archive = NONE` or erasure; see G1101 and Task 020-070.)*
- [x] The `RunSelector` contract (contracts/list-query.md) is agreed with 018, its co-owner. Fixed
  2026-11-24: 018 is built, and the contract now pages, bounds and starts a selection exactly as 018's code
  does (`select_cursor`, `selection_bound_at`, `selection_horizon`; research Decision 020-006), with
  `QUERY_UNSUPPORTED_OPERATOR` and `CONFLICT`/`RUN_PURGED` as 018's contracts name them. One sentence of
  018's FR-018-029 is for the coordinator to align (Gap 020-005).
- [x] The shard-renewal contention gate (G1100) passes before analytics is enabled by default. Fixed
  2026-11-24: analytics no longer locks `scheduler_shards` at all (research Decision 020-003, FR-020-014),
  so there is no contention to gate; a lock-monitor assertion in Task 020-024 checks it.
- [x] 016's carrier list (FR-016-002) is refined in the same delivery that adds the two carriers (D1091) —
  T020-011. Accepted 2026-11-24, delegate (Owner decision 2026-11-24-4), as a delivery condition rather
  than a gate before building: Task 020-005 cannot publish a declaration in production without the
  consolidated rules version, and the build slices order it before slice A2.
- [x] The worker-protocol minor for `AnalyticsExportService` is assigned at merge with 019/026 (G1097).
  Fixed 2026-11-24 by the register's rule (Owner decision 2026-11-15-8): the next free minor of
  `sdks/SPEC.md` §9 when the contract merges, 1.10 or later at the time of writing (research Decision
  020-009).
- [x] G1083 (egress-guarded `object_store` client) is proven before customer object-storage endpoints
  are enabled. Fixed 2026-11-24: the engine no longer writes to customer object storage; the `lake`
  profile of `apps/database-runner` does, under 017's destination policy (research Decision 020-002), so
  G1083 is closed.
