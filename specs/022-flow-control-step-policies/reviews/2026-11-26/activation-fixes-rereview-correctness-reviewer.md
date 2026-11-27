# Correctness re-read of the fixes `343bbe406..0f04b4429` (decisions D022-146 to D022-149), 2026-11-26

The repository's read-only `correctness-reviewer` read the fixes of the activation review against its findings
(`activation-review-correctness-reviewer.md`). The coordinator relayed the verdicts; the responses are rows of
`activation-review-responses.md`.

## Verdicts on the earlier findings

| Earlier finding | Verdict |
|---|---|
| Major 1, the purge deletes a leased step's grant | Met (D022-146), but the window locks the fix relies on open Major 1 below |
| Minor 1, the give-back filter | Met (D022-147) |
| Minor 2, `key_consistency` | Met |
| Minor 3, the pause test's comment | Met |
| Minor 4, the evidence | Met (task T022-117) |
| Nit 1, the race barrier | Partly met: see finding 6 |
| Nit 2, the down migration's comment | Met |

## Major 1 (plausible). Several leases of one run lock their windows outside the sorted order

- **Where:** `crates/application/src/worker/withdrawal.rs:466-515` runs `engine.run` once per lease, and each run's
  `persist` locks its keys in `persist_gate_returns` (`readiness_gate.rs:1210-1231`), sorted within that one call only.
- **Scenario:** run R1 has two rate-gated root steps on keys X1 < X2, both leased and not sent, handed to the withdrawal in
  the order (X2, X1): R1's transaction locks X2, then X1. A pause of run R2, which waits on both keys, locks X1 and waits
  for X2. Deadlock.
- **Asked:** before the loop, lock every rate-gated lease's window in one sorted call (`lock_existing`); a test with X1
  held and the leases passed as (X2, X1), red first.

## Minor 1 (the same defect as security Medium 1). A key with grants of different periods

- **Where:** `readiness_gate.rs:1700` and `:1781` judge a slot spent by the grant's own `period_ms`; a waiter counts each
  grant over the waiter's own period (`readiness_gate.rs:909-914`, `:990-991`).
- **Scenario:** see security Medium 1.

## Minor 2. A hand-back refused under the lock is a `409`, not the documented ordinary failure

- **Where:** `crates/application/src/worker.rs:2585-2601`, mapped by `crates/api-http/src/error.rs:849-852`;
  `withdrawal.rs:513`.
- **Scenario:** the commit's check refuses (`StateConflict`): the host gets `409 CONFLICT`, uncounted. The Rust SDK treats
  `CONFLICT` as not retryable, so the lease lapses at its time to live. In `withdraw_root` the error rolls back the run's
  other leases too.
- **Asked:** in `rate_slot_spent` lock the window, then the grant (`FOR UPDATE`), before judging, so that the check before
  the command is the decision; the commit's check stays a defence; the outcome counted as documented; test the locked race
  through `withdraw_root` too.

## Minor 3. A missing grant is counted `capped`

- **Where:** `readiness_gate.rs:1697-1700` (`NOT EXISTS`); the `continue` at `:1742-1745` is dead.
- **Asked:** return `corrupt` when a rate-gated root step `LEASED` at the attempt has no grant; remove the dead branch.

## Minor 4. Gap G022-042 understates its cost

- **Scenario:** for a last attempt or a step that does not retry, the hand-back fails the step, maybe the run, though
  nothing ran. With `per: second` nearly every hand-back made more than about a second after the lease is recorded is
  refused. A lease not sent that is refused also spends its attempt, at its time to live. And `capped` cannot be told apart
  from the cap of four withdrawals.
- **Asked:** rewrite G022-042 honestly; give the refusal its own counted outcome; weigh whether a better rule exists, for
  example honour the hand-back and count the slot again from the withdrawal when no other grant was made since (the
  attempt is kept and the limit is never exceeded); decide, choosing the stronger.

## Finding 6. Tests

- The two refusal tests should run `assert_consistent` and compare the key rows with the journal; assert that attempt 2
  gets exactly one `rate_limit.granted`.
- The give-back refusal test should assert which error.
- The race barrier should wait for the pass's own backend (`pg_blocking_pids` of the pass's backend contains the holder).
