# Targeted re-read 2 of design 052: architecture (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 58b6fd20e8723d36fdac8be920b5cf5ca13dd3cb
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
fixes of `reread-1-architecture.md`, not a full review. The reviewer read files only, ran nothing, and could not list
branches.

Re-read 1: Major 1 and Nits 1, 2, 4, 5 and 6 resolved; Major 2 and Nit 3 partly (below). The later changes keep the
layers. ADR-0134 can be accepted with Nit 1 fixed; ADR-0133 should first carry the correctness confirmation's fix of
its decision 2.

No Blocker.

## Major

1. Two new suite selections the constitution does not admit ("and only those"): code changes selecting
   `integration_hot_path_wake`, and chart changes selecting `contract_scalability_ha`; the Risk table contradicts Task
   052-6. Fix: wait for 5.1.0 to admit them, or keep the lost-wake test in `integration_claim_transition_efficiency`
   and let the new target run when it changes and at release.

## Nit

1. ADR-0133's Enforced by omits `probed_counts_only_a_begun_claim_transaction`, ADR-0134's
   `a_short_wake_payload_is_dropped_as_malformed`.
2. The rank's read of the live engines has no row in the change table.
3. ADR-0134's decision 2 opens with "an internal event transport" and then excludes Redis.
4. `stage="poll"` mixes statements into a counter of candidates; a counter of its own.

Counts: Blocker 0, Major 1, Nit 4

All five were fixed at the next commit: the lost-wake test in the claim suite, the new target run when it changes and
at full runs, and the chart's checks as a checker (`scripts/check-chart-wake.py`) run on every pull request.
