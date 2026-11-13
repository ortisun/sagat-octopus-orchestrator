# Correctness review of design 042 (first draft)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 540154c80cb36cc585d18c5952f7c0abce77d430
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the subjects attributed to this reviewer, in the relay's words, with the answer of the second draft. The relay itemises 12 of the 13 Blocker and Major findings and 4 groups of Nits; the rest were not itemised. Counts are the reviewer's own.

## Blocker

1. The evaluation ceiling does not bound the chain's work; `flatMap` then `take` builds the whole list first; the inline input count is unchecked.
   *Answer:* running ceilings per stage, an explicit input count of 1,000 with operators, evaluations at most stages times input; `take` after an over-ceiling `flatMap` fails, stated as G042-013 (FR-042-007, section 3, T042-8).

## Major

2. Blob sources: the manifest checks and the `maxItems` bound of `materialize` contradict a chained membership.
   *Answer:* the chained manifest, both checks reading it, tests at entry and restore with `verify_run` (FR-042-010, T042-45).
3. `onError` at entry.
   *Answer:* the run fails with the typed code (FR-042-008, G042-008); CUJ 1 guards with `has()`.
4. The entry is not one transaction and reruns on several paths.
   *Answer:* recorded once; evaluations before a failed commit discarded; quota re-entry backed off and counted (FR-042-009, T042-52, T042-9).
5. Evaluation order and errors are unstated.
   *Answer:* stage by stage; the first failing stage, then position, with the operator, never a value; one code per ceiling; a property test against a reference implementation (FR-042-006, section 3 table, D042-024, T042-7).
6. `distinct` by serialised bytes disagrees with `==` (`1 == 1.0`, `-0 == 0`).
   *Answer:* the canonical form of FR-042-011 with tests on those cases; beyond 2^53 a documented gap (G042-010).
7. SC-042-003's restart test cannot fail.
   *Answer:* after entry the evaluator is replaced by one that panics on the chain call, for an inline and a blob source (the blob deleted after entry), asserting zero calls; races for cancellation and a moved fence with barriers (SC-042-003, SC-042-005, T042-48).
8. Rollout: capabilities are compile-time and an old replica's behaviour is unstated.
   *Answer:* the 0.3.0 fleet rule from the upgrade procedure, what an old replica does (refuses, never misreads), and an old-reader test (section 10, D042-028, T042-50).
9. ADR and approval wording.
   *Answer:* aligned with OD-2026-11-13-17 (D042-016); T042-4 waits for approval to be ticked (rule R9).
10. (M9, M10) Sealing: `received_at` is taken at insert and visible at commit, so a commit after the deadline can miss its window; claiming does not increase `attempts`, so the backlog rule can hold for ever.
    *Answer:* `admissionGraceMs` of at least the admission statement timeout, and `maxHoldMs` as a cap (FR-042-017, D042-033, T042-21). Later release.
11. (M11) One open window per key extended past an elapsed deadline.
    *Answer:* several open windows per key by opening instant; an elapsed deadline is never extended (FR-042-016, D042-034, T042-20). Later release.
12. (M12) `emit: last` depends on commit order, so SC-042-001 and the CUJ were not testable.
    *Answer:* `first`, `last` and `all` follow `(received_at, source, event_id)`; SC-042-001 and CUJ 3 say so (FR-042-016, D042-034). Later release.

## Nit

13. The SDK shape (`map` is already the alias).
    *Answer:* section 3 SDK table, T042-10.
14. (N1) Lock the chain row before inserting a key.
    *Answer:* chain, key, window, port, quota (FR-042-014, section 5, D042-037).
15. (N2) A chain without keyed stages need not lock a key.
    *Answer:* stated (FR-042-014).
16. S1 details: parameter ranges and `window {count: 0}`; empty output; body `index` after `filter`/`skip`; a named test for a `forEach` without operators; no new journal event type; S1 from `develop` after 014 merged.
    *Answer:* FR-042-004, SC-042-004, FR-042-005, FR-042-012 with `collection_entry_without_operators_is_unchanged` (T042-51), FR-042-009, and the plan's opening paragraph.

Counts: Blocker 1, Major 12, Nit 10
