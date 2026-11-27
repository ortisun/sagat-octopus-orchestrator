# Targeted re-read of design 046, fifth pass: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 1f12ee025677bceba768925427e0ad2848523c8c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A security read of the changes
made after `security.md` (at `4b6a3e6d`, nothing open) in answer to the architecture and correctness re-reads; the reviewer
read that diff, the design and its contracts, and ran nothing. It found no tenant-boundary, request-forgery or
gateway-dependency problem; the gateway allow-list, the `BACKFILL` keys and the deadlines read sound.

## Findings

1. Major. An attacker with `script:publish` in any tenant can stop the advisory switch: author-lock publications that
   commit between the application's read of the rows above the mark and the switch make five `StaleInputs` in a row and
   put the import on hold, so no block, exact-pair malware blocks included, is inserted, and runners holding a volume
   keep leasing. Fix: never hold on staleness; under the lock make the snapshot current, insert the exact-pair blocks,
   and insert a `SNAPSHOT` re-evaluation row for the tenant of each row not evaluated.
2. Major. The `script_jobs` quota stopped counting `RUNNING` jobs once their version is refused, while the heartbeat
   answers `409` only on a lost lease, so a refused version's running installs keep a runner for up to about 16 minutes
   uncounted; a tenant can publish versions that refuse fast on one target and install large files on another, freeing
   quota while runner work piles up. Fix: count `RUNNING` jobs until they end; the heartbeat answers `409` once the
   version is no longer `BUILDING`.
3. Major. The block pass writes `steps_failed_at`, but `script_artifact_blocks` grants only SELECT, INSERT, DELETE; a
   table-wide UPDATE added to fix it would let the runtime role rewrite `source` and pass the `ADVISORY_IMPORT` deletion
   trigger. Fix: UPDATE of `steps_failed_at` only, and a trigger refusing changes to any other column.
4. Nit. PostgreSQL grants `EXECUTE` on new functions to `PUBLIC`, so any role could call
   `sagat_lock_advisory_snapshot_share` and stall the barrier. Fix: `REVOKE EXECUTE ... FROM PUBLIC`.
5. Nit. A step failed by the materialization route is audited as `system:engine` only. Fix: also the runner principal.

Suspicion, not verified: row locks may not be fair, so new `FOR SHARE` lockers can pass the barrier's waiting
`FOR UPDATE`, and a 5 s `lock_timeout` drops its place. Fix if confirmed: a fairly queued table lock as the barrier
(`LOCK TABLE script_locked_packages IN SHARE MODE` through a definer function) and a held-lock test with overlapping
shared lockers.

Counts: Blocker 0, Major 3, Nit 2

## The author's answers (all fixed, the suspicion included; Decision 046-077)

1. The switch never refuses or holds on staleness; it blocks precomputed and range-evaluated matches and inserts a
`SNAPSHOT` re-evaluation row for each tenant with an unevaluated row above the mark;
`publications_during_the_switch_never_hold_it`. 2. `RUNNING` jobs count until they end; the heartbeat answers `409`
for a job of a refused version and the runner stops it. 3. UPDATE of `steps_failed_at` only, with the guarding trigger.
4. Both definer functions revoke `EXECUTE` from `PUBLIC`. 5. The audit names the runner principal. Suspicion: the
barrier is now a `SHARE` table lock through `sagat_advisory_barrier()` (import role only), with a 30 s `lock_timeout`,
retries and the alert; `the_barrier_is_not_starved_by_overlapping_writers`.
