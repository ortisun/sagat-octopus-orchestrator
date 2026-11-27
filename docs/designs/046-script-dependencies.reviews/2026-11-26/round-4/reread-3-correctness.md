# Targeted re-read of design 046, third pass: correctness (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 99bd66440358d8c375b68c7b24c7b323199a217b
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes
of `reread-2-correctness.md`; the reviewer read the diff of the fixes, the design, the data model and contracts, and ran
nothing.

## The previous findings

All four answered: the backfill index over live rows with the version picked at claim (finding 1 below follows from
it); the sweep's refusal steps (finding 2 follows from it); the finish writing its evaluation; the protocol row,
`CANCELLED` and `503 SCRIPT_POLICY_BUSY`.

## New findings

1. Major. The general key `UNIQUE NULLS NOT DISTINCT (tenant_id, version_id, phase, target)` still covers `BACKFILL`:
   with `version_id` null until claim, two artifacts lacking the same target collide; with it set at insert, the old
   `FAILED` row blocks the new insert. Fix: limit the general key to `phase <> 'BACKFILL'`, or re-queue in place; test
   with two artifacts.
2. Nit. The sweep cancels every expired `RUNNING` job of a non-`BUILDING` version, `BACKFILL` included (its version is
   `READY`), with no code or alert. Fix: only `RESOLVE` and `INSTALL`.
3. Nit. The publication row of `rest-api.md` lists only `201`, `422` and `202`, and `SCRIPT_POLICY_BUSY` is in no code
   table.

Counts: Blocker 0, Major 1, Nit 2

## The author's answers (all fixed; Decision 046-076)

1. The general key is a partial index `WHERE phase <> 'BACKFILL'`; a `BACKFILL` row's `version_id` is null until claim;
the test uses two artifacts. 2. The sweep cancels only `RESOLVE` and `INSTALL`; an expired backfill lease is retried.
3. Both `503` codes on the publication row and in a code table of `errors.md`.
