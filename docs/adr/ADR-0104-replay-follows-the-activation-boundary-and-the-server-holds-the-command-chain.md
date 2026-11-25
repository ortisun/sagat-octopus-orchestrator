# ADR-0104: Replay follows the activation window of the journal, and the server holds a keyed command chain

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4)
**Refines / supersedes:** refines ADR-0103; none superseded
**Enforced by:** the property test `activation_boundary_replays_every_interleaving_to_the_same_commands`; `a_completion_with_a_foreign_command_chain_is_refused_and_puts_the_run_under_review`; `a_changed_activity_input_on_replay_is_a_chain_mismatch`; `the_chain_of_the_golden_commands_is_the_golden_hash`; `a_terminal_command_with_pending_events_is_refused_unhandled_events`; `three_unhandled_refusals_close_the_run_to_new_signals`; the conformance scenario `code-nondeterminism` (design 044, Tasks 044-4, 044-9, 044-11, 044-26)

## Context

Replay is correct only if every worker, live or replaying, delivers the same events to the function in the same
activations. Events (a signal, a timer, an activity's completion) can commit while a workflow task is in flight. Temporal
buffers them and writes them after the task's completion; this platform journals every fact at once, in its own
transaction. The SDKs compare the kind and target of the commands the function makes during replay with the recorded
ones; that comparison misses a changed input, and a bug in it, a stale sticky cache or a worker that skipped replay would
commit a divergent command. A chain over data the worker receives in the history could be produced without running the
function.

## Decision

1. Let `S(k)` be the journal sequence of the `workflow_task.started` of the workflow task that produced completion k.
   Activation k MUST deliver the visible events with a journal sequence in `(S(k − 1), S(k)]`, in journal order, one at
   a time, letting the function run until it yields before the next; `ctx.now()` MUST be the time of `S(k)`. A live and a
   replaying worker MUST apply the same rule.
2. The lease MUST clear the run's pending flag, and every visible event MUST set it in its own transaction.
3. The server MUST keep per code run a 32-byte chain `H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length ‖ d(k))`, with
   `H(0)` all zero and `d(k)` the RFC 8785 JSON of `{seq, kind, target, fragment}` of command k, where `fragment` is
   `HMAC-SHA-256(seed, canonical fragment)`. The history sent to workers MUST NOT carry the fragments of past commands.
   The server MUST compare the chain a completion reports for its replayed commands with the stored one and check that
   the first new command number follows the last accepted one; a mismatch MUST put the run under review with cause
   `CHAIN_MISMATCH` and commit no command.
4. A completion with a terminal command MUST be refused (`UNHANDLED_EVENTS`, retried at once, not counted) while an event
   is pending; after 3 consecutive refusals the run MUST refuse new signals and updates until its next completion commits.
5. The causes MUST behave as the table of design 044, Technical Plan 6.3, says: the causes that the same code would raise
   again (`NONDETERMINISM`, `CHAIN_MISMATCH`, `COMMAND_OUTSIDE_ENVELOPE`, `COMMAND_INVALID`, `NONDETERMINISM_RISK`,
   `VALIDATOR_COMMAND`) put the run under review at once; other workflow task failures and timeouts at the fifth
   consecutive one, after waits of 1, 2, 4 and 8 s. A run under review MUST keep its status, steps, timers and waits, and
   MUST leave review only by a move, a retry where the table allows it, a reset, a termination, its deadline or a forced
   cancellation.
6. A failure's detail MUST be a closed cause, a command number, an actual kind and target checked against the grammar of
   its kind, and an exception type name matching `^[A-Za-z_][A-Za-z0-9_.$]{0,127}$`; the expected command MUST come from
   the server's record.

## Considered options

1. **Journal at once, a window of two journal events, a keyed chain over fragments the worker is not sent** — one store;
   the window is replayable from the journal alone; a worker can produce the chain only by producing the same commands,
   inputs included.
2. **Buffer events during a workflow task (Temporal)** — lost: a second store of facts to keep consistent.
3. **Trust the SDK's comparison alone** — lost: an SDK bug or a changed input would corrupt a run silently.
4. **A chain over kind and target only** — lost: a worker could compute it from the history without running the code.
5. **Retry a failing workflow task forever (Temporal)** — lost: hides the problem and burns workers.

## Consequences

- Good: a divergence, inputs included, becomes a visible review; replay needs nothing but the journal.
- Bad: one HMAC and one SHA-256 per command on the completion path; the server cannot name which command differs from a
  completion alone (a dry-run move or the replayer does); a review needs an operator.
- Compatibility: additive; new journal kinds only.

## Evidence

Design 044, Technical Plan 4.3, 6; the Python and TypeScript spikes of Decision 044-041 (journal-order resolution of races
and timers); the correctness, design and security findings of round 1.
