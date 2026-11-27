# Correctness review of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and its severity is kept, with the author's answer.
Architecture decisions 0098 and 0099 were read from the local branch `docs/delivery-acknowledgement`; feature 022's gates were
checked against their specification, since they are not built. Nothing was run.

Counts: Blocker 1, Major 14, Nit 9

## Blocker

1. **The close-deadline index holds steps waiting for reconciliation, review or an operator hold**, which keep their close
   deadline; the reducer accepts `step.failed` from `WAITING` with no fence, so the pass would end an unknown outcome as a known
   failure (principle III); if it checked the waiting kind, those rows, with the oldest deadlines, would fill every window as
   no-ops. Paused and stopping runs' `READY` rows starve the scan the same way. *Answer: accepted.* The close deadline is cleared
   when an attempt goes to reconciliation, to review or to an operator hold (journaled on those events); the index covers `READY`
   rows only; a gated step is ended by its gate's own timeout, whose deadline becomes the earlier of the queue deadline and the
   close deadline (Major 5); the candidate query excludes paused and stopping runs, as `due_start_releases` does; tests for a
   reconciling step past its deadline and for many such rows ahead of one due row.

## Major

1. **The claim clears the start deadline that a withdrawal should keep.** *Answer: accepted*: it is kept through `LEASED` and
   `RUNNING`, in the column and in the domain state, replaced when the next attempt becomes claimable, cleared at the step's end;
   a withdrawal test asserts it still fires.
2. **The claim's condition is in SQL, but the lease is a journaled domain decision**; the transaction's time is `db_now`
   (`clock_timestamp()` at the run lock), and the candidate scans would keep returning unclaimable runs. *Answer: accepted*: the
   lease commands take the time and the deadlines and refuse; the design names `db_now`; the same predicates are hints in
   `CLAIMABLE`, `CLAIMABLE_PATHS` and `RUN_FOR_CLAIM`.
3. **An attempt can be leased a moment before the close deadline**, and then ends in review rather than as a known failure.
   *Answer: accepted*: a floor of time left, the earlier of the node's `timeoutMs` and 6,000 ms (one receive bound): the claim
   refuses and row 6 fails when less than the floor is left before the close deadline; the step ends `SCHEDULE_TO_CLOSE_TIMEOUT`.
4. **The retry table names only `end_attempt`**, while body steps are decided by `fail_scoped_attempt`. *Answer: accepted*: one
   shared decision function for both, with row tests for a scoped instance and a bounded path.
5. **Feature 022's gates need more than removing the waiter row**: a granted `READY` step must return its slot and permit; the
   grant must carry the start deadline; the race with the queue timeout is missing; a past-deadline waiter at the head of the queue
   can block others. *Answer: accepted*: a gated step's queue deadline is written as the earlier of 022's queue deadline and the
   close deadline, so 022's own timeout path ends it (code `SCHEDULE_TO_CLOSE_TIMEOUT` when the close deadline was the earlier);
   a granted `READY` step ended by a deadline returns its slot and permit as 022 specifies, in 022's lock order; the grant events
   carry the start deadline; the queue-timeout race joins the table.
6. **What an operator's retry does to the close deadline is unsaid.** *Answer: accepted* (Decision 045-018): an operator's retry
   is a person's decision, clears the close deadline and writes a new start deadline; `step.retry_requested` is a writer.
7. **Pause is not addressed.** *Answer: accepted* (Decision 045-019): the pass skips a paused run; the resume moves each claimable
   step's start deadline later by the time the run was paused (a pause is not a missing worker); the close deadline keeps
   running, because it is the author's business window, and is enforced at resume.
8. **Old binaries cannot replay the new journals** (`step.failed` from `READY` or `RETRY_WAIT` is a new transition), and removing
   the requirement would allow a rollback anyway. *Answer: accepted*: the requirement is a replay floor, as Architecture decision
   0098 decision 8's is: removal is refused while any unpurged run journal holds such an event; Technical Plan 12 says the reducer
   gains that transition; a test refuses a build without the capability.
9. **Two derivations of the deadline columns** (SQL and domain). *Answer: accepted*: the domain computes each instant, truncated to
   microseconds, and puts it in the event; the store writes the event's value; the replay test compares the columns.
10. **A 3 s heartbeat timeout is below the 6 s receive bound.** *Answer: accepted*: the minimum is 10,000 ms, above the receive bound
    plus one heartbeat bound (6,000 + 2,000 = 8,000 ms).
11. **A per-code override can produce an invalid policy.** *Answer: accepted*: each effective policy is built through
    `RetryPolicy::new` at publication; `RETRY_OVERRIDE_INVALID` with its pointer.
12. **The lists accept fewer codes than a worker can send.** *Answer: accepted, the documented option*: the lists use the grammar
    every SDK host sends (`[A-Z0-9_]{1,64}`, `sdks/SPEC.md` §6.5); a code outside it, which only a client outside the six SDKs can
    send, matches no entry and the base policy decides; the SDK exception map refuses such a code at registration; a test each.
13. **CUJ 2 ignores jitter.** *Answer: accepted*: the story sets `jitter: false`, and its test injects the sample.
14. **The barrier test of the claim cannot fail** (the claim skips a locked run). *Answer: accepted*: three tests: no lock, deadline
    at `db_now` plus or minus a margin; the claim's transaction held while the pass runs; the pass's transaction held while the
    claim runs.

## Nit (each accepted)

1. "Row 6 is the only place the close deadline enters the domain" is false: restated (the retries and deadline passes decide on it
   too, through domain commands).
2. A fresh grant writes an expiry not capped by the attempt deadline, and not every writer of a step takes the run lock: both
   statements corrected.
3. A provisional lease at its earlier deadline is withdrawn, not timed out, and a platform withdrawal can be followed by
   `SCHEDULE_TO_START_TIMEOUT`: said.
4. A worker can report the two new codes: the server maps them to `WORKER_ERROR` (also security, Major 6).
5. `next_retry_delay_ms` joins the fingerprint of `FailStep` (also security, Nit 1).
6. The claim span's skipped-row count cannot be had for free: dropped.
7. Detection bounds and attempt counts: corrected (the interval after the last `heartbeat()`, the step's count, a base of at least
   4 in the example, the exponent uses the step's attempt number).
8. The Risks row on publishing a handler source before 1.9 had no requirement: `activity-timeouts-handler-v1` (Requirement 045-017).
9. The compensation policy also follows row 3; body steps use the path transaction and the scoped protocol filter: both said.
