# Architecture re-read of design 051 (2026-11-24)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c095c5c5e36495cbe5a43ae3933c4220c7ef4718
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The second read, of the
benchmark alone. The reviewer found every written figure of arithmetic and every cited figure of the repository right.

## Blockers

1. The six-hour job limit is broken: a fan-out half-job is 306 + 75 = 381 minutes; the window rule lets any rung last 17
   minutes (3 × 12 × 17 + 75 = 687); halving rungs and about 35 product switches per job are not counted.
2. The 75-minute history load has no basis: it needs 667 workflows per second averaged over the three products, while
   Windmill's own figure gives about 173 chains per second with 28 workers (about 96 minutes for its history alone); the
   crash jobs' history is unstated.
3. The end of a workflow is observed differently per product and the observer loads the system: this platform's run
   result is polled every 500 ms with jitter (`sdks/rust/src/client.rs`), its stream is capped at 20 per subject, Temporal
   long-polls. Fix: take each product's recorded close time after the rung.
4. Fault (c) promotes a 4-vCPU standby in place of the 16-vCPU primary, and nothing restores the topology for the next
   product.

## Majors

1. The timer burst measures start admission, not timers, and does not fit the rung model.
2. A split fan-out repetition runs on two machines.
3. Designs 048 and 052 reuse the driver where Requirement 051-014 forbids it.
4. Whether this platform loses acknowledged work under fault (c) depends on design 048's Tasks 048-4 to 048-9, and equal
   heartbeat timeouts on design 045; Task 051-7 has no dependency line.
5. Fault (a) is a different fault per product (Temporal's services in one process, Windmill's server off the execution
   path, no balancer in the CPU table).
6. Several crash metrics give no information (first completion is about 0 with a second process; "back to level" has no
   tolerance or sample floor; a stuck workflow counts as lost).
7. The maximum sustained rate has no latency bound, a topped-out ladder gives a lower bound, and the ranking ignores the
   resolution.
8. One rung contaminates the next (leftover backlog, rotation with unequal ladders, the shared PostgreSQL's vacuum and
   counters).
9. Configuration choices bias the comparison (Windmill's sandbox on every flow, its dedicated workers excluded without a
   deviation line, Temporal's eager start borrowing the driver's CPU).
10. The pilots cannot do what is asked (27 pairs in 720 minutes; the free runner cannot host the topology; configurations
    published before the paid run that chooses them).
11. The budget covers less than the design commits to (later campaigns, design 052's measurements).
12. Rules have no failing check (the workspace exclusion, the driver's and sidecar's audits, the workflow's trigger,
    permissions, timeout and digests, the containers' network).

## Nits

1. Five crash repetitions cannot balance three positions.
2. Script containers have no network, so they cannot record to the driver's socket.
3. The saga's failed step as a transition, the payload size and each product's idempotency mechanism belong in the design.
4. Report Patroni's promotion time and its timing settings.
5. "No work sent to a person" holds by declaring activities retry-safe; say so.
6. Owner decisions 2026-11-24-1 and 2026-11-24-4 are not rows in this tree.
7. Raw data as an artifact, the report by a pull request (no write permission in the job).
8. CPU sets do not isolate disk and caches.
9. On one machine the cost per million is the reciprocal of throughput; a three-repetition variation is weak.
10. Task 048-15 is the mixed-writer test, not a drill.

Counts: Blocker 4, Major 12, Nit 10
