# Stage 9 (Codebase details) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 561517d6129ad497557536d7e731bbdc5bd87a70
Reviewer: egm-review, stage 9 (the egm-codebase-details lens, dispatched directly; standard implementation-readiness-standard.md)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding; the code claims checked by the author are marked.

Counts: Blocker 0, Major 9, Nit 4

Most cited paths, functions, suites and constraints exist as described.

## Major

1. **The consumed payload would be charged twice:** `charge_logical_output_bytes` charges outputs whose digest was null and checks
   `total() + charge` first, and `total()` now includes pending bytes. *Answer: fixed:* the consumption moves its bytes by a
   statement of its own and is left out of the output charge (Technical Plan 4; Non-functional requirement 054-001 now counts two
   statements for a consuming entry).
2. **The handoff's events and sequences are not as the doc said** (`continue_evaluated` journals `continuation.selected`, events are
   appended one call at a time, the successor starts at `run_version` 3, the replay looks at `expected_run_version + 1`). *Answer:
   fixed:* Technical Plan 7 states both handoffs' events, the sequences (successor rows and events from 4, a bulk append; the
   predecessor's two events by `append_event` after its own), and the replay's lookup of the first `continuation.selected` after
   the expected version.
3. **"Ends the reapply, as today" would abort the reset on any new refusal** (only `NOT_WAITING` maps to `REJECTED`, checked in
   `reset.rs`). *Answer: fixed:* four new `ReapplyReason` values, each a `REJECTED` candidate that ends the reapply of signals and
   lets the reset go on (Technical Plan 9, Task 054-10).
4. **The chain path's receipt reader (`parse_nodes`, scope `chain.signal:{chain}`) is not named.** *Answer: fixed* (Technical Plan 3,
   Task 054-8).
5. **A name is an `EventType` (ASCII grammar, `$defs/name`), not 128 bytes of UTF-8** (checked in `crates/domain/src/workflow.rs`).
   *Answer: fixed:* Requirement 054-001, Technical Plan 1 (`$ref`) and Task 054-2's tests.
6. **The journal already has `run_events.payload_digest`, and `Digest` exists** (checked in the execution migration and
   `reapply.rs`). *Answer: fixed:* the digest goes to that column, never to the detail; `Digest` replaces the invented type
   (Technical Plans 9, 11 and 17).
7. **`Capability` in `crates/application/src/instances.rs` and its equality test are left out.** *Answer: fixed:* Task 054-8.
8. **Validation in the same migration departs from the pattern and the lock impact is unstated.** *Answer: fixed:* `NOT VALID`
   checks validated by a companion migration under `SHARE UPDATE EXCLUSIVE`; the lock impact is stated (Technical Plan 2, Task
   054-4).
9. **A buffered Signal-with-Start signal would be reapplied, which today's code deliberately skips** (checked in `reapply.rs`).
   *Answer: fixed:* `signal.buffered` carries `via`; reapply skips `signal_with_start` (Terms, Technical Plans 9 and 11).

## Nit

10. Site names (`charge_collection_owner_output`; three `total()` comparisons in `primitives.rs`). *Answer: fixed.*
11. `Engine::evaluate_run_result` holds no transaction. *Answer: fixed:* a `retained` parameter, two callers named.
12. Signatures disagree; the store-side paths have no `RunTx`. *Answer: fixed:* one signature for `consume_pending_signals`, and a
    free helper `discard_pending_signals(conn, tenant, run)` that the `RunTx` method calls.
13. The floor trigger's update branch cannot run (`platform_requirements_guard` refuses every `UPDATE`; checked in the registry
    migration). *Answer: fixed:* `BEFORE DELETE`, reached only inside `platform_requirement_remove`.
