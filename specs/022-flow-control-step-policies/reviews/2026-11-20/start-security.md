# Security review of T022-022 (2026-11-20, third set: the gate of a run's start)

Reviewer: `security-reviewer` subagent, read-only, given the working-tree diff of T022-022 with the spec (FR-022-017..022,
FR-022-053..060), the plan, the data model, the error contract and ADR-0056/0089/0093/0094/0096 (not the author's
conclusions). Nothing reachable in a released build: `PgRunStore::create` still refuses every gated definition (D022-037).

## Findings and resolutions

1. Major: a run with a start permit whose collection or loop finish path ends the run (`primitives.rs`,
   `loop_progress.rs`, raw SQL, no reducer) never gives the permit back: the key is dead until a hand delete. **Fixed:** a
   start gate with a `forEach`, `while` or `repeatUntil` node is refused at admission until G1197 routes those finishes
   through the reducer (D022-050 item 2). Proved by
   `run_admission::a_start_gate_with_a_collection_or_a_loop_is_refused_until_their_finish_gives_the_permit_back`, red with
   the refusal removed.
2. Major: `queued_runs` is counted without a lock, and the doc comment cited owner decisions that cover only the live
   keys. **Resolved:** the citation is corrected; the unlocked count until T022-105's usage rows is a delegated decision
   (OD-2026-11-20-6, D022-050 item 3) with its bound recorded (G022-011).
3. minor: a child start refused by its gate is retried by every pass. **Recorded:** G022-009, built by T022-102.
4. nit: a start's `rateLimit` key was evaluated before the unsupported gate was refused. **Fixed:** `Engine::start` refuses
   every unsupported start before it evaluates a key.
5. nit: a delayed start or a reset of a gated definition is refused with a retryable code. **Kept, delegated:** D022-050
   item 7 (`BINARY_CAPABILITY_MISSING`, as every gated path this build does not run; no process lists the capability).

Classes checked with no finding: tenant boundary of every new statement, key values (only lengths, digests and 12-hex
labels leave the transaction), `sagat_app` privileges, injection, fail-closed defaults, the test-only door.
