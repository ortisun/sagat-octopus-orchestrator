# ADR-0104: Replay follows the activation window of the journal, pending work is derived from the journal, and the server holds a command chain over data the worker is not sent

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4); no person has reviewed it
**Refines / supersedes:** refines ADR-0103; none superseded
**Enforced by:** `integration_workflow_as_code::activation_window::live_and_replayed_activations_are_equal` (generated interleavings against PostgreSQL with barriers); `is_visible_and_is_waking_cover_every_event_kind_and_origin`; `pending_is_true_exactly_when_a_waking_event_follows_the_last_completed_start`; `every_journal_writer_wakes_a_code_run_in_its_transaction`; `the_reconciler_repairs_a_pending_run_from_the_journal`; `the_reconcilers_waking_kinds_equal_is_waking`; `a_completion_below_the_window_reviews_as_nondeterminism_risk`; `a_deferred_command_re_emitted_with_another_body_is_a_chain_mismatch`; `a_completion_with_a_foreign_command_chain_puts_the_run_under_review`; `a_changed_activity_input_on_replay_is_a_chain_mismatch`; `the_chain_of_the_golden_commands_is_the_golden_hash`; the conformance scenarios `code-nondeterminism` and `code-signal-at-return` (design 044, Tasks 044-4, 044-5, 044-9, 044-11, 044-26)
**Required before acceptance:** the fault tests of design 044 Tasks 044-9 and 044-28

## Context

Replay is correct only if every worker, live or replaying, delivers the same events to the function in the same
activations. Events can commit while a workflow task is in flight; this platform journals every fact at once, in its own
transaction. About ten code paths can produce an event the function must see; a flag that each must set would drift. The
SDKs compare the kind and target of the commands made during replay with the recorded ones, which misses a changed
input; a chain over data the worker receives in the history could be produced without running the function.

## Decision

1. Let `S(k)` be the journal sequence of the `workflow_task.started` of the workflow task that produced completion k.
   Activation k MUST deliver the visible events with a journal sequence in `(S(k − 1), S(k)]`, in journal order, one at a
   time, letting the function run until it yields before the next; `ctx.now()` MUST be the time of `S(k)`, never earlier
   than the previous one. `S(0)` MUST be 0, and `run.started`, visible and waking, MUST carry the run's input (for a
   successor, the transferred input and the carried state). The flags `continueAsNewSuggested` and
   `historyLimitReached` MUST be journaled in each `workflow_task.started` and answered from the one that opens the
   activation. A live and a replaying worker MUST apply the same rule.
2. What is visible and what is waking MUST be decided by the domain from the event and its origin
   (`RunEvent::is_visible`, `RunEvent::is_waking(origin)`), exhaustively with no catch-all arm: a completion's command
   records and the workflow-task events never wake; a command's immediate effect does. Every journal write of every run
   MUST go through one typed append function that applies them, and no other code MAY insert into the journal; a
   source-text test hosted where pull-request CI selects it (`code_workflow_sites::only_append_events_inserts_into_run_events`)
   holds that, besides the release rule of `scripts/check-architecture.py`.
3. A workflow task MUST be pending exactly when the journal sequence of the run's last waking event is greater than `S`
   of its last completion. The typed append MUST move the first for every waking event, in the
   transaction that appends it, and make the workflow-task step `READY` with its hint only when it was waiting with kind
   `DECIDER` (never under review, never closed); a completion MUST decide the step's state after appending its own
   events; a failed task MUST move neither. The reconciler MUST compare the journal itself, not the run's columns, and
   repair a pending run whose step waits with kind `DECIDER`.
4. Every journal row MUST store its origin, every command record its node spec and body digest, `run.started` the
   seed, so that the journal alone rebuilds a code run. A completion MUST carry `applied_through_seq`; the SDK MUST hold a terminal command until every event of the window
   has been applied; the server MUST refuse a completion below `S(k)` as an SDK fault and put the run under review at
   once with `NONDETERMINISM_RISK`. A terminal or continue-as-new command MUST be deferred, not counted, while a waking
   event after the activation's start is unseen or an obligation (an aborting saga scope, a review, an unknown outcome,
   and for continue-as-new an open activity, child or approval) is open: the completion's other commands commit, the
   deferral is recorded with the deferred command's kind, target and body digest, the command takes no number, the SDK
   re-emits it last at each later activation, and a re-emitted command that differs from the record MUST put the run
   under review with `CHAIN_MISMATCH`; a replaying SDK MUST expect the deferred command last in that activation. A
   terminal or continue-as-new command while a scope is open and not aborting MUST put the run under review
   (`COMMAND_INVALID`), since only the function can close it. After 3 deferrals for unseen events
   the run MUST refuse new signals and updates until a completion commits without one. A deferral MUST be answered as a
   success (`ACCEPTED_DEFERRED` with its cause), never as an error status, because the rest of the completion commits. A command on one of the run's own
   steps that is already terminal MUST be a no-op with its own command record, counted and chained.
5. The server MUST keep per code run a 32-byte chain `H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length ‖ d(k))`, with
   `H(0)` all zero and `d(k)` the RFC 8785 JSON of `{seq, kind, target, body}` of command k, `body` the SHA-256 of the
   command's canonical body as received after a canonical check (a work command's fragment, a typed command's body), in
   which every value field that may travel as a blob is its descriptor (the SHA-256, as 64 lowercase hexadecimal
   characters, and the size of its RFC 8785 bytes) whatever its carrier, never a blob identifier, so a replay reproduces
   it without uploading and the server checks a stored blob against it; each command kind's `kind` string and `target`
   are those of design 044, Technical Plan 6.2. The
   history sent to workers MUST NOT carry the bodies of past commands. The domain MUST hold the chain rule over opaque
   digests behind a trait of the domain implemented in `infra-ir`. A
   mismatch MUST put the run under review with cause `CHAIN_MISMATCH` and commit no command. A side-effect marker's
   value is in the history, so its chain entry proves only that the function asked for that marker, not that it
   computed the same value.
6. The causes MUST behave as the table of design 044, Technical Plan 6.3, says, with the count of consecutive failures
   stored on the run; a failure's detail MUST be a closed cause, a command number, an actual kind and target checked
   against the grammar of its kind, and an exception type name matching `^[A-Za-z_][A-Za-z0-9_.$]{0,127}$`; the expected
   command MUST come from the server's record.

## Considered options

1. **A journal window, derived pending, a chain over fragments the worker is not sent** — one store; the window is
   replayable from the journal alone; no path can forget to wake the function; a worker can produce the chain only by
   producing the same commands, inputs included.
2. **Buffer events during a workflow task (Temporal)** — lost: a second store of facts to keep consistent.
3. **A pending flag set by each waking path** — lost: one forgotten path leaves a run waiting for ever.
4. **Trust the SDK's comparison alone** — lost: an SDK bug or a changed input would corrupt a run silently.
5. **A chain over kind and target only** — lost: a worker could compute it from the history without running the code.

## Consequences

- Good: a divergence, inputs included, becomes a visible review; replay needs nothing but the journal; waking is one rule.
- Bad: one SHA-256 per command on the completion path; the server cannot name which command differs from a completion
  alone (a dry-run move or the replayer does); a sticky worker whose cache is corrupted but whose chain is intact is caught
  only by the next replay; a review needs an operator.
- Compatibility: additive; new journal kinds and two run columns.

## Evidence

Design 044, Technical Plan 4.2, 4.3, 4.4, 6; the spikes of Decision 044-041 (journal-order resolution of races and
timers); Decisions 044-069, 044-072, 044-079 and 044-082; the correctness and architecture findings of round 2.
