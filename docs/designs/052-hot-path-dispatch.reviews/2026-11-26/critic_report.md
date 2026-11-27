# egm-design (critic) on design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 2348509a1eddabf7d7f44658fab074b7861e6f9d
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and took PostgreSQL behaviour from its knowledge.

## Blocker

1. The queue hands probes the refusals and stale rows today's scan filters in SQL (paused runs, the stop reason, the
   minor and blob condition, `pool_refused!` with its sample of 4, `capacity_window!` of ADR-0075, open breakers,
   activities at quota); a refused row now costs a full run load, a quota lock or an audit row; claims cannot tell a
   stale row (`RUN_FOR_CLAIM` returns `None` for locked, gone and not ready alike), so stale rows gather at the head and
   a refused head of 64 starves the activity, and the wake repeats the walk. Fix: join `steps` and `runs` with the
   scan's own predicates under `FOR UPDATE OF q SKIP LOCKED`; delete a row whose step is not `READY` without the run
   lock, the generation invariant stated and tested.
2. The replay rule re-leases a lease already handed to the transport, which ADR-0098 and ADR-0099 forbid ("not started"
   proves nothing; a retry can deliver the first answer meanwhile), with no fitting cause and each retry counting
   towards the cap of four. Fix: on `already_applied` never lease or withdraw (`next_declined = REPLAYED`); accept
   `want_next` only when the next lease is marked under ADR-0099 decision 2.

## Major

1. The next-step lease inside a completion inverts the claim's lock order (the report path holds the binding lock, the
   secrets lock and the membership row before the drain fence and the session row), allowing a cycle with a claim of
   the same session and a revocation. Fix: lease in a second transaction after the completion commits, as an ordinary
   claim of that run.
2. Fan-out: one row per step, siblings adjacent, pollers collide on the run's `SKIP LOCKED`, one poll may retry the same
   locked run 64 times; "at most 2" is unreachable. Fix: exclude every row of a skipped run; one row per run in the peek.
3. Where the lock sits in the per-key lateral merge is unspecified, and both obvious shapes misbehave. Fix: an unlocked
   peek of the first m ids per key, then one `FOR UPDATE SKIP LOCKED ... LIMIT 1`.
4. The switch commands race: `fill` can miss a step committed after the backfill's snapshot; `fill off` empties the
   table while processes still claim from it. Fix: wait for the snapshot horizon; refuse `fill off` while claiming;
   batched deletes; the runbook's order.
5. The protocol minor is wrong (1.9 is design 045's, 1.10 to 1.12 draft 019's; minors are cumulative), and a
   `next_task` at 1.8 or later must be marked provisional and acknowledged, a round trip CUJ 2 omits.
6. The cross-process wake is unbounded (hints go through the outbox dispatcher and acknowledged JetStream publishes),
   the same-process case is rare with two replicas, time-based paths have no commit to publish from, and every woken
   stream writes its session row. Fix: a best-effort core NATS wake right after commit; measure with two replicas; wake
   at most as many streams as steps became ready; time-based paths bounded by `idle_retry`.
7. The gate's statistics do not hold: a minimum detectable effect at n = 30 is about 2.7 times too small for six pairs;
   "5 of 6 slower" fires 7/64 under no change, near 20% for two scenarios; power at 15% is 88.6% at best; the metric is
   undefined (at a fixed rate below saturation throughput equals the offered rate). Fix: define the metric; a pilot of
   30 A/A pairs; MDE = (t(0.975, 5) + t(0.8, 5)) · sd / √6; fail only on 6 of 6 and the median beyond the MDE; report
   the A/A false-positive rate and the power, corrected across scenarios.
8. `pg_stat_xact_user_tables` cannot count the queue's rows across a lease's two transactions on pooled connections.
9. The trigger cannot change under design 053's rules (no `CREATE OR REPLACE` of a trigger an earlier migration made).

## Nit

1. Queue rows are said to be locked "before the chain and run rows", but the next-step lease takes its row after; `ON
   CONFLICT DO NOTHING` waits only for an in-progress insert or delete of the key.
2. "On a refusal, roll back" contradicts the routine, which commits pool-refusal audits and other leases.
3. CUJ 2 gives the next step to "one worker per activity", but only steps the session serves are leased.
4. The unique key and the unlocked deletion depend on the generation growing between `READY` episodes; state and test it.
5. Stale references in the ADRs (Task 051-12, Success criterion 051-005, 1.1 against 2, "a setting", "each engine").
6. Do not wake keys leased in the same transaction; the hint subject over-wakes (no namespace).
7. The trigger as `AFTER INSERT OR UPDATE OF status ... WHEN ...`; `verify_run` conditional on `filling`.
8. `CompleteStepRequest` has no `worker_id`; the session is the lease's `leased_by`.
9. Dead-tuple thresholds and an oldest-xmin alert; the gate's selector misses migrations and dependency bumps.

Counts: Blocker 2, Major 9, Nit 9
