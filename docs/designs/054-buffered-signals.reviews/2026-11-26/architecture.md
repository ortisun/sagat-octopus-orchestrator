# Architecture review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 0473c4dca31ced3f9bf61c718831becf6328275b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A full review of the design
and Architecture decision 0111 against the constitution, the ADRs and the layering. The reviewer read files only and ran
nothing. The author's answer follows each finding.

Counts: Blocker 0, Major 7, Nit 4

Checked and sound, per the reviewer: the domain stays free of `serde_json` and the types it relies on exist; root steps are
planned in node-id order; Architecture decision 0050's lock order is kept; contracts and migrations are additive.

## Major

1. **Principle I: a signal that loses to a timeout is buffered, not a no-op;** the Check argued from principle IV's exception for
   code runs, a new reading of the constitution that is the owner's. *Answer: fixed by restating the evidence, no new reading:* the
   timed-out wait is untouched by the signal (a no-op for it) and the signal is audited; its admission for a consumer still ahead is
   a separate transition, refused `NO_CONSUMER_AHEAD` when none is ahead (Constitution Check row I; Decision 054-016).
2. **Three records describe the floor differently, and no ADR records the shared, hard-to-reverse floor.** *Answer: fixed:*
   Architecture decision 0112 (proposed with this design) records one hold and one registry of floors, refining 0082 and 0098;
   Task 054-18 carries an `Owner-gate:`; Gaps 054-007 and 054-010 ask designs 044 and 045 to cite it.
3. **Tasks 054-7, 054-9 (beyond the handoff) and 054-18 change other features' code without a gate.** *Answer: fixed:* Owner
   decision 2026-11-26-540 covers them, and Tasks 054-7 and 054-18 carry the gate.
4. **Architecture decision 0111 lacks its divergence evidence, the permanent floor and the refinement of 0032 decision 8.**
   *Answer: fixed:* the header refines 0032 decision 8; Consequences state the permanent floor and the handoff cost; Enforced by
   adds "until they exist the rules are review-only"; Evidence names the benchmark and failure tests as pending implementation
   evidence.
5. **The version tolerance is a hand-written list with no check, misses the public `expectedRunVersion`, and is not in the
   ADR.** *Answer: fixed:* a source rule requires every `run_version` comparison in `crates/infra-postgres` and
   `crates/scheduler` to use the helper or be allowlisted; the public `expectedRunVersion` keeps its exact meaning, bounded by the
   inbox's fill (at most 1,000 advances); decision 5 of 0111 states it (Technical Plan 7).
6. **The fitness rule over writers of `run.finished` misses string literals** (`loop_progress.rs`). *Answer: fixed:* it matches
   the string literals too (Technical Plan 8).
7. **The invariant's enforcement contradicts itself.** *Answer: fixed:* one helper, `assert_inbox_invariant(tx)`, after every
   commit of the two suites; Terms, Requirement 054-020 and Task 054-3 agree.

## Nit

8. `format('SELECT %s()', holds)` doubles the argument list. *Answer: fixed:* `format('SELECT %s', holds)`, tested with two
   capabilities.
9. The ordered refusals are split across layers. *Answer: fixed:* `buffer_signal` takes the name's pending count and owns the
   order it can decide; the store-only checks follow (Technical Plan 17).
10. Two records own the code-consumer refinement. *Answer: fixed:* 0111 owns it; Gap 054-006 asks design 044 to drop the claim
    from 0108.
11. No principle rows for the new rules; Complexity Tracking omits three items. *Answer: fixed:* Task 054-1 writes four rows of
    `architecture-principles.md`; three Complexity Tracking rows added.
