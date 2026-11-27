# Architecture review of design 051 (2026-11-24)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: aa07ae4189cc61ab3770aae749af36d5a6a124af
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only (the design, Architecture decisions 0133 and 0134, the claim code, feature 019, design 038) and ran nothing; it
found every arithmetic figure of the design correct.

## Blockers

1. The queue claim does nothing defined with a step that is `READY` but this session cannot take (protocol minor,
   subject, ChildRoot capacity window, `max_inflight`, activity quota, step permits, later 019 routing): rolling back
   returns the same head row to every poll, deleting hides the step until the reconciliation; two guards of design 038
   would break. Fix: a per-poll exclusion set, the filter columns on the queue row, permit-gated steps kept out, a
   barrier test per refusal kind.
2. `want_next` takes protocol minor 1.8, reserved for delivery acknowledgement (Owner decision 2026-11-21-6,
   Architecture decision 0099), and ignores the lease-withdrawal rules of Architecture decision 0098 for a lease handed
   to a unary answer, and what a replay returns.

## Majors

1. Architecture decision 0133's lock order is incomplete (chain lock, binding and secrets locks, capacity group G,
   semaphores, deployment fences) and does not refine Architecture decisions 0050 and 0056, which state the claim order.
2. The queue row's lifecycle is undefined: no sweeper task, purge behaviour, unique key, or off switch for the trigger.
3. Runs in flight at the upgrade get no queue rows; a backfill is needed.
4. Fan-out loses the multi-step lease of one run per claim, and its contention is not measured.
5. The `want_next` checks are a second, incomplete copy of the claim; `CompleteStepRequest` carries no session; the
   completion already holds the binding and membership locks; a new lease producer must enter the status-writer
   inventory.
6. Overlap with features 019 (priority on `steps`, `claim_run` rewritten, weighted fairness) and 022 is not
   coordinated, and Architecture decision 0051 is not refined.
7. Part B goes beyond the scope the owner approved: Owner decision 2026-11-24-1 lists a published benchmark and design
   038, not a new table, a protocol minor and six SDK changes.
8. The throughput gate cannot pass its own success criterion (one statement in 88 cannot cross a 10% threshold); the
   fixed order breaks the 1/64 assumption; a new pull-request gate widens the constitution's CI gates.
9. Long polls do not exist today; adding them changes an existing call and holds request permits.
10. Completion is observed at the last activity, not when the product records the workflow; duplicates need every
    activity to record.
11. A doubling ladder cannot rank products, and one start rate does not fit every shape (fan-out at 50 per second is
    5,100 activities per second).
12. A fixed product order and empty databases bias the comparison.
13. Crash recovery is not comparable across products (review instead of retry after a lapsed lease since feature 037,
    unequal timeouts, "one server process" undefined).
14. Fault (c) runs on a topology that cannot recover (one standby with commits refused without a standby).
15. The repetition rule and the budget do not match the plan (crash runs on one machine, variants not counted, Task
    051-12 without a budget).
16. Fixed choices contradict "best configuration for every product" (SQL visibility, one PostgreSQL configuration, the
    script shape's isolation).
17. Several rules have no failing check (the reconciliation's suite failure, the trigger's leasable condition, the
    `SKIP LOCKED` rule, the harness's isolation, Enforced-by lines naming tests that do not exist).

## Nits

1. "Surpass" is the owner's goal, but the cited decision says "a published benchmark"; its rows are on other branches.
2. The price has no source line.
3. No memory partition is specified; the CPU model is not reported.
4. CPU sets ignore SMT siblings.
5. "Sustained" has no latency bound.
6. The Temporal sidecar adds a hop for Temporal only; which Temporal features are allowed.
7. Enumerate `want_next` refusal reasons; reuse `TaskLease`.
8. Publish the method before the first paid run and invite the vendors.
9. The off switches need CI coverage; the cross-process wake cannot be judged on one server.

Counts: Blocker 2, Major 17, Nit 9
