# ADR-0104: Replay follows the activation boundary of the journal, and the server holds the command chain

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4)
**Refines / supersedes:** refines ADR-0103; none superseded
**Enforced by:** the property test `activation_boundary_replays_every_interleaving_to_the_same_commands`; `a_completion_with_a_foreign_command_chain_is_refused_and_holds_the_run`; `the_chain_of_the_golden_commands_is_the_golden_hash`; `a_terminal_command_with_pending_events_is_refused_unhandled_events`; the conformance scenario `code-nondeterminism` (design 044, Tasks 044-4, 044-9, 044-11, 044-26)

## Context

Replay is correct only if every worker, live or replaying, delivers the same events to the function in the same
activations. Events (a signal, a timer, an activity's completion) can commit while a workflow task is in flight. Temporal
buffers them and writes them after the task's completion; this platform journals every fact at once, in its own
transaction. The SDKs compare the commands the function makes during replay with the recorded ones; a bug in that
comparison, in a sticky cache or in a worker that skipped replay would commit a divergent command, and Temporal's server
does not check.

## Decision

1. An activation MUST be defined by the journal: activation k delivers, in journal order, the events after the
   `workflow_task.completed` of activation k − 1 up to the `workflow_task.started` of activation k; events committed
   between that start and its completion MUST be delivered in activation k + 1. A live worker and a replaying worker
   MUST apply the same rule.
2. The SDK MUST apply one event at a time and let the function run until it blocks before applying the next.
3. The server MUST keep per code run a 32-byte chain `H_k = SHA-256(H_(k−1) ‖ uint32 big-endian length ‖ d_k)` with
   `H_0` all zero and `d_k` the RFC 8785 JSON of `{seq, kind, target}` of command k, MUST compare the chain a completion
   reports for its replayed commands with the stored one, and MUST check that the first new command's sequence follows
   the last accepted one. A mismatch MUST hold the run with cause `CHAIN_MISMATCH` and commit no command.
4. A completion with a terminal command MUST be refused (`UNHANDLED_EVENTS`, not counted as a failure) while an event is
   pending, so no run ends without its function having seen a signal or update that arrived during its last task.
5. A nondeterminism, a chain mismatch and a command outside the envelope MUST hold the run at once; other workflow task
   failures and timeouts MUST hold it at the fifth consecutive one, after waits of 1, 2, 4 and 8 s. A hold MUST keep the
   run's status, steps, timers and waits, and MUST end only by a move, a retry, a reset or a termination.

## Considered options

1. **Journal at once, boundary by two journal events, server chain** — one store, the boundary is replayable from the
   journal alone, defense in depth for 32 bytes per run.
2. **Buffer events during a workflow task (Temporal)** — lost: a second store of facts to keep consistent; a signal
   would not be a fact until later.
3. **Trust the SDK's comparison alone** — lost: an SDK bug would corrupt a run silently.
4. **Retry a failing workflow task forever (Temporal)** — lost: hides the problem and burns workers.

## Consequences

- Good: a divergence becomes a visible hold; replay needs nothing but the journal.
- Bad: one SHA-256 per command on the completion path; a hold needs an operator.
- Compatibility: additive; new journal kinds only.

## Evidence

Design 044, Technical Plan 4.3, 6; the Python and TypeScript spikes of Decision 044-041 (journal-order resolution of
races and timers).
