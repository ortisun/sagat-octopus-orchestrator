# Specification Quality Checklist: Run visibility, analytics and archival

**Purpose**: Validate specification completeness and quality before planning and implementation.
**Created**: 2026-10-26. **Feature**: [spec.md](../spec.md).
**Review ownership**: filled by the specification author on 2026-10-26 as a self-review; a reviewer must
confirm before `/speckit-implement` reads it. `[x]` means the requirement-quality criterion is met, not
that anything is built.

## Content quality

- [ ] No implementation details (languages, frameworks, APIs). *Not met, deliberately:* following the
  house style of 016 and 017, the FRs state the binding architectural guarantees (one transaction with the
  transition, the shard fence, the transaction horizon, the privileged purge) because those are what the
  tests check; sink kinds (ClickHouse, PostgreSQL, object storage) are user-visible product choices.
  Storage layout, crates and SQL live in plan, data-model and contracts.
- [x] Focused on user value: operators find runs, analysts receive data, administrators bound retention;
  each story says why it has its priority.
- [ ] Written for non-technical stakeholders. *Partly:* stories and success criteria are readable without
  the code; FRs assume the platform vocabulary (journal, run, sink, classification level).
- [x] All mandatory sections completed (stories, edge cases, FR, NFR, entities, SC, assumptions, scope).

## Requirement completeness

- [x] No `[NEEDS CLARIFICATION]` markers; three owner questions (O1080–O1082) carry recommended defaults
  that the plan assumes.
- [x] Requirements are testable: 43 FRs, each traced from at least one task and one SC (checked by script
  on 2026-10-26).
- [x] Success criteria are measurable (counts, 100% batteries, equality with a fresh derivation).
- [ ] Success criteria are technology-agnostic. *Partly:* SC-020-004 relies on a statement trace and
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
  each with a shared conformance scenario.
- [x] Encrypted payloads (031) are covered (FR-020-043).
- [ ] No implementation details leak into the specification. *Not met, deliberately* (see first item).

## Open gates before implementation

- [ ] Owner answers O1080 (default retention), O1081 (`pii` exports), O1082 (run stubs) — T020-002.
- [ ] Owner accepts or amends ADR-0053 and ADR-0054 — T020-002.
- [ ] The MINOR amendment of constitution principle I proposed in ADR-0054 is accepted by PR (terminal
  runs only; the verified archive as sole copy; `archive = NONE` means deletion) — T020-061; US08/US09
  are gated on it (G1101).
- [ ] The `RunSelector` contract (contracts/list-query.md) is agreed with 018, its co-owner.
- [ ] The shard-renewal contention gate (G1100) passes before analytics is enabled by default.
- [ ] 016's carrier list (FR-016-002) is refined in the same delivery that adds the two carriers (D1091) —
  T020-011.
- [ ] The worker-protocol minor for `AnalyticsExportService` is assigned at merge with 019/026 (G1097).
- [ ] G1083 (egress-guarded `object_store` client) is proven before customer object-storage endpoints
  are enabled.
