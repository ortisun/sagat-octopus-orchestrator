# Correctness review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 87897d4a1059becbc95ff577fc3e856d0cce54ae
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A full review of the design
against the constitution, Architecture decisions 0030 and 0050, feature 018's registry and the code it cites. The reviewer read
files only and ran nothing. The author's answer follows each finding.

Counts: Blocker 2, Major 4, Nit 4

## Blocker

1. **The predecessor's `signal.moved` and `signal.discarded` were placed after its handoff event**, which ends the run;
   `RunState::apply` refuses any event after a terminal one, so every replay of the predecessor would fail, and the telemetry head
   would not cover them (`crates/domain/src/execution/state.rs`, `crates/infra-postgres/src/chain_progress.rs`). *Answer: fixed:*
   they come before the handoff event, the replay finds that event by type, and `mark_telemetry` covers both runs' last sequences
   (Technical Plan 7).
2. **The loop's two store-side ends** (`loop_progress.rs`: a loop whose successor is `end`, a drained or stopped loop) were missing;
   with the check on `runs`, a cancelled run with an active loop and pending signals could never finish its drain. *Answer: fixed:*
   both are in Technical Plan 8 with the helper and their shape matchers, tested per path, and a rule of
   `scripts/check-architecture.py` lists every writer of `run.finished` against an allowlist.

## Major

3. **How a consumed payload reaches the journal's digest and the charge was unsaid;** without the digest a reset loses the consumed
   output. *Answer: fixed:* `Effects.consumed` sets the `wait.satisfied` row's `payload_digest` and the step's `output_digest` and is
   left out of the charge; a test resets after a consumption and reads the output (Technical Plan 4, Tasks 054-6 and 054-10).
4. **A reset's re-emitted `signal.buffered` would re-create rows through the store's hook**, bringing back consumed signals.
   *Answer: fixed:* the hook skips restored events; one rebuild statement from the prefix runs before the settle reduction; a test
   covers a prefix that buffered and consumed (Technical Plan 9).
5. **Only the handoffs tolerated a run version advanced by buffering;** `finalize_chain`, the loop and collection transitions and
   the scheduler's collection passes also refuse on a changed version. *Answer: fixed:* one helper,
   `advanced_only_by_buffering`, serves all of them, with a barrier test on the deadline pass (Technical Plan 7, Task 054-9).
6. **Reading the ledger at the three in-memory sites adds statements for every run**, level 0 included. *Answer: fixed:* they add
   the pending bytes to their in-memory sum and read the ledger only when `runs.pending_signals > 0`; budget lines for an event's
   wake and an `end` output at level 0 (Technical Plan 6).

## Nit

7. The invariant "at every commit" cannot be checked from the journal. *Answer: fixed:* store tests check it after each commit; the
   replay exempts the consumption's adjacent pair (Requirement 054-020).
8. The replay cannot derive pending bytes from the settling events alone. *Answer: fixed:* it joins each settled sequence to its
   `signal.buffered` (Requirement 054-020).
9. Inline column checks cannot be `NOT VALID`. *Answer: fixed:* each check is added separately `NOT VALID` (Technical Plan 2).
10. A code run continued under another version may exceed that version's bounds. *Answer: fixed:* the exception is stated
    (Requirement 054-007): moved signals are kept, new ones refused until below `maxPending`.
