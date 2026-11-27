# Correctness review of design 044, round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewer: correctness
Reviewed-commit: 9b64ac22698c34d847c9658b8a08b5a5c68814d2
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 4's Majors 2 to 6
are closed (refused terminal command, saga accessors, signal consumption, body digests, origin and the reconciler);
Major 1 is partly closed and returns as finding 1.

## Major

1. Cooperative cancellation is bypassed by three other entry points that call `stop_run(.., Cancelled, ..)` directly:
   the children pass for `propagate` (`crates/scheduler/src/passes.rs`), the chain-addressed cancel
   (`crates/application/src/chain_progress.rs`) and the source cancel of a pending reset
   (`crates/application/src/runs/pending_reset.rs`). Route every cancel through one domain function with a code arm, a
   source-text check and three tests.
2. The closing window admits neither signal consumption nor update accept or reject: a function with a signal handler
   crossing 51,200 can only be terminated. Admit both; extend the history-limit test.
3. A deferred terminal command is outside the chain and its body is not recorded: a later worker can re-emit a different
   terminal unseen, and the move check is blind to deferrals. Record kind, target and body digest in the deferral; match
   or `CHAIN_MISMATCH`; include it in the replay-check verdict; say how a replayer treats a deferral record.
4. Updates admitted or accepted stay non-final when the run ends or continues as new. Fail them with a closed reason in
   the stop drain and the handoff; refuse admission in the drain; tests.
5. A reset copies `run.cancel_requested` but does not re-arm `code_cancel_deadline_at`, and `verify_run` does not
   compare it: a reset run can stay open for ever. Carry the grace in the event and re-arm, or exclude; a reset test.

## Nit

6. Architecture decisions 0103, 0104, 0105, 0107 and 0108 contradict the design after this round.
7. Test names asserting the superseded refusals.
8. `consumed_signal_seqs` beside the chained command; no field for the deferred terminal.
9. The deadline pass keeps selecting a run already stopping; clear the deadline.
10. Whether a deferred terminal counts as the function's answer at the cancellation deadline.

Counts: Blocker 0, Major 5, Nit 5
