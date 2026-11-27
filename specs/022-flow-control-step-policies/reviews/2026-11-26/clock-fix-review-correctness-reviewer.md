# Correctness review of the clock fix `e0fffa984` (decision D022-160), 2026-11-26

The repository's read-only `correctness-reviewer` read commit `e0fffa984` (decisions D022-160 to D022-162: a new
waiter's `enqueued_at` is at least one microsecond after its key's youngest waiter's, against a database clock that
steps back). The answers are decisions D022-163 to D022-165 of `research.md`.

## Major 1. The raised `enqueued_at` can pass the deadline, and the insert is refused

- **Where:** the waiter tables check `deadline_at > enqueued_at` (`concurrency_waiters_deadline_after`, migration
  `20270130000000` line 118; `rate_limit_waiters_deadline_after`, migration `20270130000100` line 109). The deadline is
  still the admission's `now + min(queueTimeout)` (`crates/infra-postgres/src/execution/readiness_gate.rs:443`), while
  the insert raises `enqueued_at` to the key's youngest waiter plus one microsecond (`readiness_gate.rs:1133-1138` and
  `1174-1179`).
- **Scenario:** the database clock steps back by more than the queue timeout after a key's youngest waiter was
  queued. The next admission of that key has `enqueued_at` later than its deadline; the insert fails with SQLSTATE
  23514, which the caller sees as a 500, on a start or on a worker completion that queues the next step. A same-key
  fan-out of 1,001 steps with a 1 ms timeout fails on every retry: each waiter is a microsecond after the previous one,
  so the thousandth is a millisecond after the first.
- **Asked:** keep the order instant and the deadline apart. Under the same key lock, read each gate's
  `greatest(now, youngest + 1 µs)` before the deadline is computed, and set the deadline to
  `max(now + timeout, that instant + 1 µs)`, the delay NFR-022-008 allows, so that the journal's `queue_deadline_us`
  is the rows' deadline. Test, red first: move the older waiter further ahead than the younger one's `queueTimeout`;
  and a fan-out case. Correct D022-160 ("the deadline is unchanged") and the runbook.

## Minor 2. The D022-160 test moves the older waiter only 300 ms ahead

- **Where:** `tests/integration/flow-control-step-policies/queue.rs:466-472` and `498-507`.
- **Scenario:** on a slow runner more than 300 ms pass between the move and the next admission, whose own clock reading
  is then after the older waiter's: the test passes without the fix.
- **Asked:** move it to `clock_timestamp() + interval '60 seconds'`.

## Minor 3. The D022-161 test races the real clock

- **Where:** `queue.rs:434-440`: the test checks `now < hint`, then runs a whole `tick()`; the clock can pass the hint
  between the two, and the pass then grants.
- **Asked:** move the hint far ahead (60 s), run the pass and assert that it grants nothing; then move the hint back
  and assert the grant.

## Minor 4. The run view's waiting `since` is now `enqueued_at`

- **Where:** `flow_control.rs:425` and `448`: the waiting `since` of the run view reads `enqueued_at`, which after
  D022-160 can be later than the instant of the journal's `queued` event.
- **Asked:** say so in one sentence in the runbook and in the API description (the fragment and the canonical
  `openapi.yaml`; regenerate the Studio client if the description is in it).

## Nit 5. The flood test's order rests on transaction ids

- **Where:** the doc comment of the flood test, `tests/fault/flow-control-step-policies/rate_limits.rs:144-155`.
- **Asked:** state that `age(xmin)` orders by the assignment of transaction ids, which is the commit order here only
  because the starts are issued one after another and `grant_key` decides a key's waiters in turn.
