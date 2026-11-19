# Security review of T022-097, T022-098, G1199 and G022-001 (2026-11-19)

Reviewer: `security-reviewer` subagent, read-only, given the commits `4cc39427`, `bc492b6b`, `d89086b9` with the
constitution, the spec and the ADRs (not the author's conclusions).

## Findings

1. **Major: a rate key whose window fills is never made due again by time.** The hint is set only by a return; queueing
   does not set it, and a pass that grants nothing clears it; plan.md (grant pass) wants the instant the oldest counted
   grant ages out.
2. **Major: paused runs at the head of a key starve every later waiter.** `gate_key_waiters` reads the oldest 32 with no
   pause, status or deadline filter; paused heads are `Busy` and the pass never reads further.
3. **Major: a full key costs up to 32 run transactions every pass and grants nothing** (a full listing kept the hint).
4. minor: the count of waiters ahead has no bound (read under the key's lock).
5. nit: `rest_gate_key` has no fence.
6. minor (suspicion): a process without the digest key ring stops timeouts as well as grants, and logs per waiter.
- Checked with no finding: tenant boundary of every statement, the key-shard fence, the runtime role's privileges, no key
  value or digest in logs, spans or metrics, no dynamic SQL.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| 1 | Fixed in part: a queued waiter makes its key due (`key_is_due`); a window rests until its oldest counted leased grant ages out (`rest_gate_key`). A grant's lease instant is still never recorded (T022-074), so a leased grant of a rate key counts for ever until then: T022-097 is not ticked for rate keys (G022-003) | `queue::a_waiter_queued_behind_an_ineligible_one_is_granted_without_a_release`, red before the fix |
| 2 | Fixed: the listing reads eligible waiters only (live, not paused, before their deadline); a pause and a resume make the run's keys due (`rearm_queued_keys`) | `queue::paused_heads_do_not_hold_back_the_waiters_behind_them_and_a_resume_rearms_the_key` (33 paused heads), red before the fix |
| 3 | Fixed: a full listing in which nothing was granted rests the key; the per-key bound is the plan's 256 | same tests; `a_released_permit_is_granted_to_the_oldest_waiter_by_the_next_pass` (the key rests) |
| 4 | Fixed: the count stops at the waiter's own limit (`LIMIT $10`) | queue suite |
| 5 | Fixed: the rest is conditional on the key's home-shard lease (owner, generation, expiry) | queue suite (`a_stale_replica_is_refused_by_the_key_fence`) |
| 6 | Fixed: the timeout routine reads the stored digests without the ring; the grant pass stops at a key's first `BINARY_CAPABILITY_MISSING` and keeps its hint (`unsupported`); the scheduler's need of the ring is G1198 | review only (no process has the gate without a ring in this build, D022-037) |
