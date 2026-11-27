# Approval confirmation of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c621f25b4f50038411fd38bc746a4d92867dc9d5
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`, at 66634fb9, 2
Blockers and 9 Majors), then the fixes re-read in `reread-1-architecture.md` to `reread-6-architecture.md`, each Major
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 6's Nits 1 and 3 are resolved; Nit 2 is resolved in the design (Nit 1 below for the decision record). Checked
with no layering or decision problem: Requirement 049-001's ended-month branch, the mapping of step 2's refusals, steps 2
and 3 and their call of `ensure_journal_buckets()`, the merge order with design 053's tasks, Tasks 049-3 to 049-10 and
049-14; `journal_bucket` stays in the adapter and the creation flag.

No Blocker, no Major.

## Nit

1. ADR-0132's "Enforced by" names only "function grants" for Task 049-7, not `prosecdef` and `proconfig`.
2. `mark_projected`'s carried bucket could only travel in `Pending`, an application type, against row II. Fix: read it
   from `runs` in the store's `mark_projected`, off the hot path, or carry it as an opaque value.
3. The gauge must be read from the catalog on every housekeeping attempt, failed ones included, or a failing ensure
   leaves it stale.
4. The retryable refusals `runs_journal_bucket_retiring` and the purge's refusal during a retirement name no contract
   reason.
5. The Risks row and row VI call held runs a metric; the Retirement section prints them.
6. Lines past the wrap width.

Counts: Blocker 0, Major 0, Nit 6

Settled: Nit 1 is fixed in ADR-0132; Nits 2 to 5 are recorded as design 049's follow-up Task 049-19, added after the
approval as the approval rule allows; Nit 6 (layout only) waits for the next edit of the design.
