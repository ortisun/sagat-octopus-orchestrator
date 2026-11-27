# Stage 5 (Initial Zoom Out) review of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 66543332
Reviewer: egm-review, stage 5 (the egm-zoom-out lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding.

Counts: Blocker 0, Major 8, Nit 7

The reviewer confirmed against the code: seven outcome-unknown codes (`crates/domain/src/execution/event.rs`), the 30 s
`lease_ttl` (`crates/application/src/worker.rs`), the `409` text of `runs.rs`, the 4 MiB retained bytes, the budget lines 38/2
and 50/1 (`work_budget.rs`), `TaskLease` fields 1 to 18, and the shard-index pattern of the passes.

## Major

1. **Buffered signals are bundled into a timeouts design, and the reasons for bundling argue against it.** The shared race does
   not exist (a wait that consumes at its entry has no timer to race), the fleet switch is its own requirement, the conformance
   scenario is separate; what is left is the numbering reason. The bundle brings an ADR that supersedes an owner-accepted decision
   onto the timeouts' critical path. Fix: move them into their own design and ADR.
   *Answer: accepted.* Buffered signals left the design; ADR-0111 stays proposed for their own design (The Problem, last section;
   Decision 045-009 revised).
2. **Buffered signals ignore feature 004's mechanism for events that arrive before a wait** (`lookbackMs`, `inbox_events`,
   `event_deliveries`, `steps.event_since`). Fix: weigh the reuse in ADR-0111 and define which source wins when both apply.
   *Answer: accepted for ADR-0111*, which now carries the option and the obligation to decide it before acceptance.
3. **`byErrorCode` goes beyond what was asked**; `nextRetryDelay` plus `maxAttempts` meets the rate-limit case. Fix: ship the list
   and the delay, record `byErrorCode` as a gap.
   *Answer: contested* (Decision 045-015): the brief asks for a retry policy per error type, and a delay cannot give a rate limit
   more attempts than a bug.
4. **Protocol 1.9 carries fields nothing needs**: `heartbeat_timeout_ms` (an old host derives its interval from the expiry) and
   `attempt_time_left_ms` (repeats 1.8, no requirement). Fix: cut 1.9 to the handler flag and the delay.
   *Answer: accepted in part* (Decision 045-014): `attempt_time_left_ms` is dropped; `heartbeat_timeout_ms` stays because the
   expiry is an instant of the database's clock and a duration makes the host independent of its own clock (Complexity Tracking).
5. **`HEARTBEAT_TIMEOUT` is a new code where the design elsewhere uses a cause field.** Fix: keep `LEASE_EXPIRED` and add
   `deadline: heartbeat`.
   *Answer: accepted* (Decision 045-013; Requirement 045-005; ADR-0110 decision 4 and option 4).
6. **The whole inbox is loaded into run state on every transition.** Fix: read only the oldest pending row at a wait's entry.
   *Answer: accepted for ADR-0111* (its consequences now say a wait's entry reads one indexed row and the budget suite gets a line).
7. **The design rests on decisions and features that are not in the tree** (Architecture decisions 0098 and 0099, the 1.8 row of
   the register, feature 022's gates). Fix: state status and location, add a Risks row for 1.8 slipping, say what ships if it does.
   *Answer: accepted*: Terms, Technical Plan 8 and 14 give each status and branch; a Risks row says slices 1 to 6 ship without 1.8.
8. **The IR scope leaves out the body variants** (`bodyActivity`, `bodyScript`, `bodyRetry`). Fix: say whether they get the fields.
   *Answer: accepted*: they do (Requirement 045-001, 045-002, Technical Plan 1, Task 045-1; Decision 045-006 revised).

## Nit

9. `signals[].buffered` means nothing when false. *Answer: moved with buffered signals.*
10. Some text drops to code level (a statement's parameter number, function signatures). *Answer: accepted*; the contract is
    described and the signatures are left to the tasks.
11. The new pass's name would collide with the run-level `deadlines` pass; add the two indexes to the invalid-index check of
    `20261017000000_scheduler_shards.up.sql`. *Answer: accepted*: the pass is `step_deadlines`; Technical Plan 2 and Task 045-6.
12. `RETRY_WAIT` in the close-deadline index is almost never reached. *Answer: accepted* (Decision 045-017): the retries pass ends it.
13. The exception-to-code map is sugar. *Answer: contested* (Decision 045-016).
14. `listSignals` in six SDK clients. *Answer: moved with buffered signals.*
15. `min_protocol_minor` justified by another draft; three overlapping inbox bounds. *Answer: accepted for the column*, now
    justified by this design's own need (Technical Plan 2); the bounds moved with buffered signals.
