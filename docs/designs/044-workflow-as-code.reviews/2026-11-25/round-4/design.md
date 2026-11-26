# egm-review Stage 7 design lens, re-audit, round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45
Reviewer: egm-design, feature-spec standard, FeatureRange 1-1743
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 1's design
findings, Blocker 1 and Majors 2, 3, 4, 6, 9, 11, 13, 14, 15, 17, 18 are closed; 5, 10, 12, 16 are closed as raised with
new defects below; 7 and 8 are partly closed (Nits 7 and 8 below).

## Major

1. Code sagas would run through the graph saga planner (`sagas::advance`, `unhandled_failure`, `body_done`), which
   aborts or completes scopes by itself; state how the code definition answers the saga accessors and stop the planner's
   automatic transitions for code scopes; say what row the open command creates and where its policies live.
2. Signal consumption has no command record and is not chained; make it a numbered typed command with a record.
3. The Python importer has no defined pass-through set (`asyncio`, `dataclasses`, `logging` import refused modules), and
   the audit hook is process-wide; define shared, per-digest and refused modules; scope the hook to the activation.
4. An `OUTSTANDING_OBLIGATIONS` refusal has no consistent state change and could spin without backoff; define it.
5. The lost-wake reconciler re-reads every idle run on every pass and names a partial instance index that misses
   run-level events; check once per wait, read by the primary key, bound the batch.

## Nit

6. A lease costs two transactions in the budget suite, so the commit counts are off.
7. The statement budget omits approval rows, look-back reads, gate rows, eager leases, the pause and binding reads.
8. A completion over the retained bytes has no cause row; declared state charged per write or per value.
9. `waiting_kind` already has `APPROVAL` with `approval_request_id`.
10. Test files insert into `run_events`; scope the architecture check to `crates/`.
11. Runtime patch releases change every digest; add to the risks.
12. A code compensation identifier is the same across runs for different activities; providers deduplicate on the key.
13. The overlay read of completed steps of open scopes is unbounded.
14. `batch_items.reason` is a closed code; the first difference needs a column.
15. `version: active` to a graph version or another workflow type is unspecified.
16. The first history page is read under the claim's run lock.

Counts: Blocker 0, Major 5, Nit 11
