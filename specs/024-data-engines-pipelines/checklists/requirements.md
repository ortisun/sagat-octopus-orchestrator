# Specification Quality Checklist: Data engines and pipelines

**Purpose**: Validate specification completeness and quality before planning and implementation
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

**Marker semantics**: `[x]` means the author checked the criterion against the text on 2026-10-26;
it is not a reviewer's approval and says nothing about implementation. Items left `[ ]` are open,
with the reason inline.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — *not met by design*: following the
  house style of feature 017, the spec names the executors (`apps/database-runner`,
  `apps/sandbox-runner`), the blob purpose, protocol identities and engine mechanisms (read-only
  transactions, job IDs) because they carry the security and durability guarantees. Crate-level
  placement is kept in the plan.
- [x] Focused on user value and business needs (eight user stories with failure scenarios).
- [ ] Written for non-technical stakeholders — *partly*: user stories and success criteria are
  readable by a product owner; the functional requirements target engineers and reviewers, as in
  every feature of this repository.
- [x] All mandatory sections completed (scenarios, requirements, success criteria, assumptions).

## Requirement Completeness

- [x] No `[NEEDS CLARIFICATION]` markers remain; the three open product choices are owner questions
  O1240–O1242 with recommended defaults.
- [x] Requirements are testable and unambiguous (each FR is traced to an SC and a task).
- [x] Success criteria are measurable (pass/fail proofs, 100% rounds, closed label sets).
- [ ] Success criteria are technology-agnostic — *partly*: SC-024-001 names the engine versions
  under test and SC-024-004 an architecture check; both are what the criterion measures.
- [x] All acceptance scenarios are defined (Given/When/Then plus a failure scenario per story).
- [x] Edge cases are identified (pins vs pointer moves, overlapping producers, quota during upload,
  tombstone in flight, encrypted parameters, cancellation of uploaded parts).
- [x] Scope is clearly bounded (out-of-scope list; gaps G1240–G1253).
- [x] Dependencies and assumptions identified (existing features, parallel features 018–032,
  ADR-0023/0035/0036 refinements).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (SC table maps FR-024-001..041).
- [x] User scenarios cover primary flows (operational engines, warehouses, produce/audit/publish,
  query, lineage/freshness, manual publish/rollback, Polars, external assets).
- [ ] Feature meets measurable outcomes defined in Success Criteria — *not applicable yet*: nothing is
  implemented.
- [x] Implementation detail that leaks into the spec is limited to what the guarantees depend on
  (see the first item).

## Gates before implementation (reviewer-owned)

- [ ] Owner review of the specification set (T024-001).
- [ ] ADR-0058 accepted; O1240–O1242 answered and recorded (T024-002).
- [ ] Dependencies acknowledged by 017 (driver boundary), 005 (runtime profile), 021 (catalog
  generalization) and the ADR-0023 owner (retention of one purpose).
- [ ] Worker protocol minor and migration versions allocated by the coordinator.

## Notes

- The ingestion path (step outputs into transforms, D1274) was added during self-review after the
  first draft lacked one; spec, data model, IR and runtime contracts were updated together.
- Facts about BigQuery and Snowflake that the consulted pages do not confirm are G1241, not
  assumptions.
- An independent read-only architecture review (2026-10-26) reported 2 blocking and 9 should-fix
  findings: publication event identity, lock order and key-share upgrade, double data charging,
  `pii` disclosure through the result view, quarantine laundering through `from`, audit/byte
  binding, unenforced read-only claims for Snowflake/BigQuery, metadata pruning versus foreign
  keys, spoofable external observations, gate ordering, and unacknowledged ADR refinements. Each
  was fixed in the spec set (D1265, D1267, D1273, D1275–D1278, G1254, ADR-0058 decisions 4, 5, 9,
  10); the reviewer did not re-review the fixes.
- Round 2 (2026-10-27): the coordinator's architecture review (B1–B3, S1–S7, minors) and coordinator
  decisions C1, C4, C6, C7, C8, C18 and C21 were applied: DuckDB became the `dataQuery` data node
  (D1279), the data container's three mounts are declared and tested, ADR-0058 now supersedes
  ADR-0013, ADR-0023 and ADR-0027 in part (O1243 for destructive deletion), warehouses run in their
  own runner deployment, the publication event is a protected-system record, Polars follows 023's
  `python-data` (FR-023-027), lock order cites ADR-0050 slot 4, journal events declare 018 reset
  rules, data-node parameters use 031's codes. Not re-reviewed after these edits.
