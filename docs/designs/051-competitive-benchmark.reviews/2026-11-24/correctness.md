# Correctness review of design 051 (2026-11-24)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: aa07ae4189cc61ab3770aae749af36d5a6a124af
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing.

## Blocker

1. A step that is `READY` but refused (paused run, stop reason, protocol minor, pool-restricted secrets, ChildRoot
   capacity, an open compensation breaker, a full activity quota or `max_inflight`) has no defined queue outcome: a
   rollback starves the queue behind it, a delete loses it until the reconciliation, and re-arming paths change no step
   status, so the trigger never fires again.

## Majors

1. One claim leases several steps of a run but deletes one queue row; fan-out collides on the run row and the same row
   returns to the poller that gave it up.
2. Uniqueness and the conflict behaviour of the queue are unspecified.
3. The next step leased inside a completion leaves its queue row.
4. The sweeper is required but neither specified nor tasked.
5. The completeness check cannot fail as described (the scan is per session; the pass runs every 30 s).
6. Success criterion 051-004 already passes without the queue (`window_order.rs` asserts one probe per lease in one
   process).
7. The statement budget counts only top-level statements and cannot see the trigger's insert.
8. The queue's query sorts every matching row on PostgreSQL 16 for several keys; deletes leave dead index entries at the
   head while any snapshot (the standby's feedback) holds them; a fill factor does nothing here.
9. A wake can be lost between the empty claim and parking.
10. The wake is not complete by construction (scheduler paths, signals, and re-arming without a status change).
11. A wake storm ends streams through discovery admission (`admit_discovery` refuses and breaks the stream).
12. Long polls do not exist; adding them changes an existing call.
13. A lost `CompleteStep` answer strands the leased next step and burns an attempt.
14. The completion's lock order with the next step is not the claim's order (binding before drain against the scoped
    claim's drain before binding; a four-transaction cycle), and the session and count locks would be held through the
    whole completion.
15. "Nothing else changes" contradicts "the same checks as a claim" (some refusals write; the answer's byte budget).
16. The regression gate's alternating order and shared database break the 1/64 assumption.
17. Success criterion 051-006 cannot be met: one statement in 88 is about 1.1%, under a 10% threshold.
18. A doubling ladder cannot rank products (factor-2 resolution).
19. The sample rule makes fan-out unmeasurable (50 workflows per second is 5,100 activities per second).
20. Completion is observed at the last activity, and repeats are invisible.
21. Workflows still open at the end of a rung are dropped from the latency.
22. "Back to 95% of the rate" is reached during catch-up, and a 1 s count is noisy.
23. The minute and job-limit arithmetic leaves work out (the crash drain, the variants, product switches, Task 051-12).

## Nits

1. The claim's lock chain omits the chain row and the capacity group lock.
2. Architecture decision 0133 says "nothing else inserts" and has the reconciliation insert.
3. `idle_retry` is the upper bound, not "the floor of correctness".
4. Patroni's detection time dominates fault (c); report the promotion separately and the strict setting.
5. "Arrivals" and rejected starts are undefined.
6. Timeouts must be equal across products for faults (a) and (b).
7. Hyperthreads cross CPU sets.
8. The price needs a source.
9. The in-process gate cannot see regressions in the wake or the next step.
10. One PostgreSQL configuration against "best configuration": say which wins.
11. "The first ready step of the run" has no order and bypasses the queue's order.

Counts: Blocker 1, Major 23, Nit 11
