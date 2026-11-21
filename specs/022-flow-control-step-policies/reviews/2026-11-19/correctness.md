# Correctness review of T022-097, T022-098, G1199 and G022-001 (2026-11-19)

Reviewer: `correctness-reviewer` subagent, read-only, given the commits `4cc39427`, `bc492b6b`, `d89086b9` with the
spec, plan, data model, contracts and ADRs.

## Findings

- **M1 Major: lost wake-up.** The readiness gate counts every waiter of a key and the grant routine only eligible ones;
  queueing never sets the hint (a waiter behind one refused by its own lower limit, a reserving head that pauses).
- **M2 Major: lock order.** `refusing_gate` locked the waiter's keys before the round; a gated failure route then filed
  its keys after them, out of the one sorted order (D022-038).
- **M3 Major: the rate hint** (as security 1).
- **M4 Major: starvation** by paused heads that pin their keys (as security 2).
- m1 a flaky ordering assertion in the dual-gated head test (the key passes run concurrently); m2 the 20-round race test
  shows two orders, and the "grant commits while the timeout waits" order is not raced; m3 `verify_run` does not check
  that a `wait_timed_out` is not before its journaled deadline; m4 a domain refusal after the grant decision was taken as
  a lost race; m5 the reservation over-reserves (ahead counted without its own position).
- Fine: the lock order of grant and timeout (run, then semaphores, then windows), the deadline checked under the locks,
  both or neither, idempotent passes.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| M1 | Fixed: `key_is_due` when a waiter is queued; `rearm_queued_keys` on pause and resume | `queue::a_waiter_queued_behind_an_ineligible_one_is_granted_without_a_release`, red before |
| M2 | Fixed: the timeout reads which gate refuses without the keys' locks; its rows go under the round's or `persist`'s one sorted lock. The gate named may be stale by a concurrent commit (precedence only) | `queue::a_timeout_whose_route_is_gated_decides_the_route_in_its_commit` |
| M3 | As security 1 (G022-003) | — |
| M4 | As security 2 | `queue::paused_heads_…`, red before |
| m1 | Fixed: that test's pass runs one key at a time | queue suite |
| m2 | Accepted and stated in the test's text: with `SKIP LOCKED` a grant never waits for the run, and a due waiter is never granted; the order "grant commits before the deadline" is round 3 | — |
| m3 | Gap G022-004 | `research.md` |
| m4 | Fixed: such a refusal is a failure (the key keeps its hint, the error is logged) | — |
| m5 | Accepted: conservative (never over-grants); recorded in G022-004 | — |
