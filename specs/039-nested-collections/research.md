# Research: Nested collections and the capacity program (design 039)

The registers of design 039 (`docs/designs/039-nested-collections.md`). Append-only. Feature 014's registers
(`specs/014-workflow-primitives/research.md`: Decisions 701 to 715, Gaps 706 to 726, Owner questions 701 to 706) stay
there and are cited, not copied.

## Decisions

| Id | Decision | Date |
|---|---|---|
| D039-001 | Accept ADR-0078 as written; decide its open items in ADR-0154. | 2026-11-26 |
| D039-002 | The engine creates groups and members in the admitting transaction; a child joins after the parent-first creation locks, taking the group `FOR UPDATE NOWAIT`; `Busy` retries on the next tick with no attempt consumed. | 2026-11-26 |
| D039-003 | The sweeper's five selections (final holder, final non-holder, quiescent holder to vacate, vacant group to acquire, vacant group with only final members); at most 256 candidates and 64 commands per tick per replica. | 2026-11-26 |
| D039-004 | Fairness: a quantum of 1 second (100 milliseconds to 60 seconds) and oldest-ready-first `Acquire`; `held_since` on the header. | 2026-11-26 |
| D039-005 | Scoped-path members get `VacatePath`, `AcquirePath` and `capacity_path_transfer_receipts`, quiescence over the iteration path. | 2026-11-26 |
| D039-006 | Receipts: the latest 16 per group and those younger than 24 hours; purged in batches of 1,000. | 2026-11-26 |
| D039-007 | The worker's lock sequence is `contracts/worker-lock-sequence.json`, checked by `scripts/check-capacity-activation.py`. | 2026-11-26 |
| D039-008 | Nesting depth at most 4 collection or loop levels in 0.3.0. | 2026-11-26 |
| D039-009 | Activation through the registry requirement `nested-collections-v1`; a dedicated full-stack nested run before activation, repeated by the release-candidate run. | 2026-11-26 |
| D039-010 | The codec opening is listed shape by shape (design 039, Technical Plan 8) against the gates of `published_collection_definition` read on 2026-11-26. | 2026-11-26 |

## Gaps

| Id | Gap | State |
|---|---|---|
| G039-001 | A zombie worker or a blocking activity holds possession (feature 014's Gap 723). | Open; HELD-age alert |
| G039-002 | The quantum's default is unmeasured. | Open; Task 039-15 |
| G039-003 | Depth beyond 4 levels. | Open; follow-up |
| G039-004 | `onError` at the entry of a nested collection (as Gap 042-008). | Open; follow-up |
| G039-005 | Work leased before a join has no possession; asserted for every join other than a new child. | Open; Task 039-5 |
| G039-006 | Triggers on a workflow with a `forEach` (taken over from Gap 042-003). | Open; follow-up |
| G039-007 | An approval inside a body waits for scoped compensation of approvals (Decision 295 of feature 014). | Open; Task 039-10 checks it |

## Owner questions

The open item of the first draft ("whether the first full-stack run may be the release event itself") and the open items
of ADR-0078 are decided under Owner decision 2026-11-24-4 (Owner decisions 2026-11-26-140 to 2026-11-26-150).
