# ADR-0134: A ready step wakes parked streams after its commit, within and across processes

**Status:** Proposed (2026-11-24; revised 2026-11-26 after design 052's full review round)
**Decided by:** agent proposal (design 052, first drafted as part B of design 051), to be accepted under Owner decision 2026-11-24-4 once design 052's review is complete
**Refines / supersedes:** refines [ADR-0029](ADR-0029-worker-task-streams-spend-finite-receive-credit.md) (a stream also claims on a wake permit, not only after its idle interval; a window ended by its deadline asks to reopen within 50 ms; a refusal by discovery admission still ends the stream), ADR-0002 and ADR-0069 (the wake is an internal, unstored signal on the selected transport outside the outbox: it carries no record, is never the only signal of work, and the `task.ready` outbox row stays the durable one); supersedes nothing. The next step in a completion's answer, which this record's first draft also decided, is not decided here (design 052, Open Question 3)
**Enforced by (pending until built):** `a_commit_between_an_empty_claim_and_the_park_wakes_the_stream`, `one_step_made_ready_wakes_at_most_one_of_eight_streams`, `a_full_stream_is_not_woken_and_a_free_one_is`, `a_stream_whose_woken_claim_leased_nothing_waits_its_idle_retry`, `a_deadline_window_asks_to_reopen_within_fifty_milliseconds`, `the_registry_holds_no_entry_without_a_registered_stream`, `a_second_step_after_a_fan_out_is_woken_at_once`, `the_permits_of_a_key_never_exceed_its_registered_streams` (Task 052-5); `a_wake_from_another_instance_claims_at_once`, `a_failed_commit_publishes_nothing`, `the_wake_prefix_is_outside_the_stream_filter`, `a_stopped_broker_does_not_slow_a_completion`, `a_wake_flood_is_limited_per_key`, `a_process_drops_its_own_wake`, `a_lost_wake_is_bounded_by_idle_retry`, the telemetry vocabulary test, `an_open_broker_keeps_the_wake_local`, `a_wake_for_an_absent_key_touches_no_state`, `a_session_revoked_while_parked_leases_nothing_on_a_wake`, the chart's contract test, the AsyncAPI baseline check and `the_claim_suite_passes_with_the_wake_off` in `integration_claim_transition_efficiency` (Task 052-6); ADR-0092's check unchanged; design 038's guards unmodified

## Context

A stream that finds nothing sleeps `idle_retry` (500 ms) and claims again, so a step that becomes ready waits up to
500 ms, up to 1,500 ms of idle in a chain of three under light load. The broker's `task.ready` hints go through the
outbox, each an acknowledged JetStream publish at the scheduler's pace, and nothing subscribes to them.

## Decision

1. Each process MUST keep a wake registry of per-key permits, without a runtime in the application: a commit that
   makes `k` steps of a key ready adds `k` permits after it succeeded, never more than the streams registered for the
   key, and a claim's leases debit them; a stream registered from before its claim until it leaves its park takes one or
   parks; a stream whose permitted claim leased nothing takes none until `idle_retry`; a full or drained stream does not
   register.
2. With an internal event transport configured, the committing process MUST publish one message per key per commit, on
   `sagat.wake.v1.<tenant>.<namespace>.<activity token>` (core NATS or Redis `PUBLISH`, outside the platform stream's
   filter), from a bounded queue it never waits on; each engine subscribes and adds one permit per message that is not its own, at most
   20 per second per key; the channel is in the AsyncAPI contract, and only the platform's own broker user may publish or
   subscribe on it. A lost wake costs at most `idle_retry`; paths with no commit of a step stay bounded by it.
3. A stream window ended by its deadline MUST ask the SDK to reopen after 50 ms; a refusal by discovery admission ends
   the stream as today.

## Considered options

1. **A local registry with a best-effort wake on the selected transport, core NATS or Redis `PUBLISH` (chosen).** Fast, lossy by design, bounded by `idle_retry`.
2. **The outbox's `task.ready` hint.** Durable and slow; a wake needs to be fast and may be lost.
3. **`LISTEN` and `NOTIFY`.** Its commit-time lock serializes the commits of every notifying transaction.
4. **A shorter `idle_retry`.** An empty claim per idle stream per interval.
5. **Wake counters compared by version** (the second draft). Every stream ending an empty claim would claim again,
   uncapped; permits bound the claims by the steps made ready.

## Consequences

- Good: a step's wait for a lease under light load follows the commit, not a polling interval.
- Bad: one publish per key and commit; a forged wake, if the broker's permissions were bypassed, costs at most 20 claims
  per second per key and process.
- Harder: the wake must never be relied on; every test also holds with it lost.
- Compatibility: an older process ignores the wake subjects; nothing is stored.
- Operations: the control plane gains a broker connection of its own, publish-only, with its own credentials and an
  egress rule opened only with the wake; the Redis transport lives beside the platform's Redis publisher.
- Rollback: the setting `SAGAT_WAKE` turns it off.

## Evidence

Design 052 and its review reports (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`). To be added when built: the
wake tests and the measurement of Task 052-9.
