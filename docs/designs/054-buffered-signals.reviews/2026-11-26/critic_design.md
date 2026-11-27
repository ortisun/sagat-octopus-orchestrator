# Stage 7 (Design critic) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 378ac8cd8261b76a018cada5a12ba822a97c1de2
Reviewer: egm-review, stage 7 (the egm-design lens, dispatched directly; standard feature-spec-standard.md)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding; each code claim of the reviewer was checked in the code before it was answered.

Counts: Blocker 2, Major 5, Nit 3

The reviewer found the doc complete against the standard and checked its mechanisms against the code; three of the doc's
claims were false in the code (only activity entries at a handoff, one place where a run ends, three byte-check sites).

## Blocker

1. **The scheduler's handoff opens a successor's entry `waitEvent` store-side** (`continue_evaluated`,
   `crates/infra-postgres/src/chain_progress.rs`, `SuccessorEntry::Wait`), without the planner, so moved signals would be skipped
   and a later live signal would overtake them. *Answer: fixed* (checked in the code): publication refuses, in a graph version
   that can continue as new, an entry wait of a declared name (`SIGNAL_ENTRY_WAIT_CONTINUES`, Requirement 054-001); Technical Plan
   7 covers both handoffs and places the moved events after the successor's fixed events; Technical Plan 4 states the exception.
   Decision 054-013.
2. **Not every end of a run passes through the planner:** `finalize_chain` appends `run.finished` itself (segment limit, tree
   budget, invalid input, expression failure, deadline), and `primitives.rs` matches exact event shapes. *Answer: fixed* (checked):
   Technical Plan 8 lists the store-side terminal paths, each discards through one helper, the shape matchers admit the optional
   `SignalsDiscarded`, and a check on `runs` refuses a terminal status with pending signals so a forgotten path fails its
   transaction; Requirement 054-011 and Task 054-9.

## Major

3. **A buffering between the scheduler's evaluation and its handoff changes the run version and fails the handoff**, which a steady
   flow of signals could starve. *Answer: fixed:* both handoffs and `finalize_continuation_expression` accept a run version advanced
   only by `signal.buffered` events (Technical Plan 7), with a barrier test (Technical Plan 12, Task 054-9).
4. **More sites than three read the raw ledger** (the charging statements' concurrency predicates, `collection_outputs::inspect`,
   the finalization scan of `maintenance.rs`); returning a sum would break the compare on the current value. *Answer: fixed*
   (checked): the column stays the output bytes, the read returns both values, the predicates and the scan add the pending bytes,
   and each site has a test (Technical Plan 6, Task 054-7).
5. **A graph queue pays O(pending) per consumed signal at each handoff, and a chain's segments are capped.** *Answer: fixed in
   part:* stated in Technical Plan 7, Non-functional requirement 054-002 (rows and events grow with the count) and Risks, and the
   guide points a high-volume queue to a code version. The per-row events are kept: design 044's activation and the successor's
   consumption need one sequence per signal; keying the inbox by chain would make a run's replay depend on another run's journal.
6. **A signal after the last wait is answered `202` and then discarded:** "too late" looks like success. *Answer: fixed:* a graph
   run whose root waits of the name are all entered and whose version has no `continueAsNew` node refuses `409 NO_CONSUMER_AHEAD`;
   a step not yet entered counts as a consumer, so the refusal is certain (Requirement 054-005, Technical Plan 12, Decision 054-012
   revised).
7. **Design 044 classifies a pending signal by the version that buffered it, which the inbox row does not carry.** *Answer: fixed:*
   `run_signal_inbox.version_id`, kept by a move and a reset (Technical Plans 2 and 10, Gap 054-006).

## Nit

8. **Moved signals and a reset point before them.** *Answer: contested with the reason now in Technical Plan 9:* a continuation's
   successor is never a reset source (feature 018's registry excludes the chain's event types), so a reset never meets them.
9. **The successor's ledger update is not counted.** *Answer: fixed:* Non-functional requirement 054-002 and Technical Plan 7 count it.
10. **The idempotency receipt cannot hold a buffered answer.** *Answer: fixed:* a `#<sequence>` receipt, outside the node-id
    alphabet, read back by both readers, with the with-start replay test (Technical Plan 3, Task 054-8).
