# Targeted re-read 2 of design 043: correctness (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: d890b9263cd8b84a7156edf66c3f08224b0d1617
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read of
the changes since a763d9d0 (OD-2026-11-16-3), not a full review. The reviewer read files only and ran nothing.

The five Majors of re-read 1 are resolved in code.

## Major

1. The inventory exits 2 on a database of 0.2.x, where it must run before the migration: `version_still_started` names
   `schedules` and `schedule_revisions`, which feature 018 creates and no 0.2.x release has. Every inventory test runs on a
   migrated database.

## Nit

2. Text and comments still describe the old counting (design Technical Plan 5, `runs.rs`, `pending_reset.rs`, the
   pre-check and `validate` docs).
3. Tests that no longer separate their cases: two equal `Counting` constants; `a_refusing_recheck_is_counted` expects what
   production no longer produces; no test of a transient counted ahead; the replay covered only through `unless_busy`.
4. The inventory stops on a panicking schema with "run it again", and a retry panics again.
5. The cost line leaves out the schedules scan; D043-184 claims Nit 9 of re-read 1, which has no test.

Counts: Blocker 0, Major 1, Nit 4
