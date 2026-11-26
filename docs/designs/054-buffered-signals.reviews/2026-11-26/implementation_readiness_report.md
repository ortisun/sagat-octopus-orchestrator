# Stage 9 (Implementation readiness) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 83005a378164dd974b4a1f6eb034efac7b87449b
Reviewer: egm-review, stage 9 (the egm-implementation-readiness lens, dispatched directly; standard implementation-readiness-standard.md)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding; each code claim was checked before it was answered.

Counts: Blocker 3, Major 4, Nit 3

## Blocker

1. **The `NO_CONSUMER_AHEAD` predicate refuses the early signal it exists for:** a root step exists only once routed, so "no wait
   still `PENDING`" holds before the run reaches its wait; branches not taken are unsaid. *Answer: fixed* (checked,
   `activate` in `crates/domain/src/execution/state.rs`): the predicate is over nodes; a node with no step or a `PENDING` step is a
   consumer ahead; a branch not taken keeps the signal until the end, which discards and counts it (Requirement 054-005).
2. **Where the consumed payload's content comes from** is unsaid; a later evaluation in the same transaction reads `RunData`.
   *Answer: fixed:* the one consuming statement returns the content by a join with `payloads`, and the engine stages it as the
   node's output (`stage_outputs`), with a test reading it in the entry transaction (Technical Plan 4).
3. **Shared texts that are not on this branch** (the hold's body, the floor's scaffold) and an undefined normalization. *Answer:
   fixed:* the body is copied into Technical Plan 15; the scaffold is specified (a table `platform_requirement_floors` of
   capability and `regprocedure`, a trigger function with its `READ COMMITTED` check that calls the registered function, one row
   and one function per design, so no design replaces another's code); the normalization is
   `regexp_replace(btrim(prosrc), '\s+', ' ', 'g')`.

## Major

4. **Slice 2 depends on slice 2a, and two tests of Task 054-3 need later slices.** *Answer: fixed:* slice 2 depends on 2a (or
   design 044's or 045's equivalent); the store-side release test and the per-path end tests moved to Task 054-9, with names.
5. **The suites are created after the behaviour; race tests have no task or file.** *Answer: fixed:* Task 054-6 creates the two
   suites' skeletons in slice 2; Technical Plan 12 assigns each race row to the task that builds it, with its test name in
   `tests/fault/buffered-signals/races.rs`; Task 054-12 completes the rest.
6. **Interfaces left to the builder.** *Answer: fixed:* Technical Plan 17 gives the signatures (`SignalOutcome`, `TakenSignal`,
   the events' fields, `PayloadDigest` and who computes it, `Effects.buffered_payload`, the `RunTx` methods,
   `WorkflowDefinition::signals()`, `hold_requirement`).
7. **More byte-check sites** (`primitives.rs` twice, the loop's carry check in `passes.rs`) and the accessors after the change.
   *Answer: fixed:* `EffectiveOutputBytes` gains `pending()`, `ledger()` stays the output bytes for the concurrency compare,
   `total()` includes pending, `legacy_bytes()` subtracts it; the sites are listed with a test each (Technical Plan 6).

## Nit

8. **Between slices 3 and 4 a buffered signal could not continue or end correctly.** *Answer: fixed:* a build lists
   `signal-inbox-v1` as offered only from slice 4 (Task 054-9), so no version of level 1 publishes earlier.
9. **Two owners for the event codec.** *Answer: fixed:* Task 054-3 owns it; Task 054-6 says so.
10. **The budget baseline "before" cannot be taken in slice 5.** *Answer: fixed:* recorded before slice 1 and run in every server
    slice (Task 054-16).
