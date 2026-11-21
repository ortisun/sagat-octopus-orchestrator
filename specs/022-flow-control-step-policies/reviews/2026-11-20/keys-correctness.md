# Correctness review: run permits, the keys listing and the consistency and fault suites (T022-026..029)

Reviewer: `correctness-reviewer` (read-only), 2026-11-20. Commit `b3351f2a` against `466c151e`, specs excluded. Sources:
`spec.md` (FR-022-018..022, SC-022-003, SC-022-004), `contracts/openapi-flow-control.yaml`, `data-model.md` (invariants 1
and 2, lock order), `tasks.md` T022-026..029, D022-055, D022-056, and the code they rely on (`gate_queue.rs`,
`maintenance.rs`, `passes.rs`, `readiness_gate.rs`, `execution.rs`, the `concurrency_*` migration, `lock_order.rs`,
`queue.rs`). The reviewer ran nothing. (Report as returned; the reviewer's closing file list is omitted.)

## Summary

No Blocker; no production defect in the new reads: counters and `QUEUED_RUNS_SQL` read in one `REPEATABLE READ`
snapshot; the five sort columns are the semaphore key minus tenant and namespace (a total order); the cursor round-trips
to the microsecond and `'infinity'` is handled the same on both sides; `Run.concurrency` is read with the run row through
the two primary keys; foreign keys make orphan rows impossible. The weak points are the evidence of T022-028 and
T022-029: the 20-round race never puts both routines on the same waiter; SC-022-003's 20 rounds of crash-during-release
are one; the admission's lock order is ticked without a test that orders it.

## Major

### M1. The grant-versus-timeout race (SC-022-003) never makes a grant decide about the waiter

`tests/fault/flow-control-step-policies/concurrency.rs:298-369`; the unexercised guard is `gate_queue.rs:207`. Rounds 0
and 1 age the deadline before either routine starts, so `gate_key_waiters` (`maintenance.rs:208`, `w.deadline_at >
now()`) never lists the waiter; the grant goes straight to `rest_gate_key` (`maintenance.rs:243`, an UPDATE of the key)
and that UPDATE is what blocks on the held key. The grant never takes the run; "grant first" and "timeout first" are the
same experiment. Round 2 runs them one after the other. The real race (listed before D, waiting for the key past D, the
timeout waiting for the run) is never produced. The recorded hand mutation needed both guards removed; removing only
line 207 leaves every round green. The step twin `queue::a_timeout_and_a_grant_of_one_waiter_are_decided_by_the_first_commit`
(`queue.rs:508`) has the same shape and its round-0 comment is not what happens.

Fix: a round where the waiter's row and projection deadline is `clock_timestamp() + 300 ms`; hold the key, spawn the grant,
barrier(1) (it holds the run and waits in `locked_queue`); poll `clock_timestamp() > deadline` (a bounded condition, not a
sleep standing in for ordering); spawn the timeout, barrier(2); release; assert `gates_granted == 0`,
`gate_waits_timed_out == 1`. Same in `queue.rs`. With `gate_queue.rs:207-209` deleted, both suites stay green today and the
new round goes red.

### M2. SC-022-003 requires 20 rounds of crash-during-release; there is one

`concurrency.rs:264`; `spec.md:838-840`. One round on a fresh world would not expose order-dependent leftovers (a hint not
re-armed after a rolled-back release, a counter drift after a crash on a key with history). Fix: loop the T14 body 20 times
on one world with fresh accounts, asserting `assert_consistent`, counters `(0, 0)` once drained and exactly one grant per
round; otherwise record the reduction as a decision.

### M3. T022-029 ticks the admission's lock order, but no test orders the admission's counter row (slot 6) before its key (slot 7)

`concurrency_locks.rs` has no admission test; the evidence cites
`run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other`, whose held lock is the namespace's
advisory quota lock, before any row lock. The end of a run takes S then K (`execution.rs:5366-5373`); that is deadlock-free
only if every admission takes S (through the `run_active_counts` trigger on its `runs` insert) before K. Fix: a reference
`[(NewRuns, the 16 counter-slot rows of the namespace), (Semaphores, key_rows)]` against `w.start_with(...)`, or at T10's
barrier assert with `pgrowlocks('run_active_counts')` that the blocked backend holds the run's slot row. Mutation: move
`file_keys_with` before the runs insert; the suite stays green today, the new test reports a deadlock victim.

## minor

- **m1.** The cancellation and timeout lock tests leave out the counter row, though both are terminal and take it (through
  the `persist` pre-lock). Use `[run, counter_row, key]` for both, over `start in 0..2`.
- **m2.** The paging test cannot catch a keyset that ignores the tie-breakers: the three keys have three distinct sort
  heads. Add a second key nobody waits for (two keys in the `'infinity'` bucket) and assert `limit=1` paging returns all,
  each once.
- **m3.** A crafted cursor with an instant before 4713 BC gets a 500 instead of a 400 (`flow_control_keys.rs:131`). Bound
  `us` in `decode` (or reject negatives) and test it.
- **m4.** `run_permit` treats a decode failure as "no permit" (`flow_control.rs:406`); use `selected()` semantics.
- **m5.** The takeover test races a raw UPDATE, not a takeover, and checks "nothing twice" only afterwards: give alpha a
  short lease, block the grant on the key, wait for the lease to run out by the database clock, run `beta.round()`
  concurrently as the second lock waiter, then assert.
- **m6.** The lost-hint test cannot fail for the reason it states: the world has no broker and `one_task` polls
  PostgreSQL. Rename it to what it proves (a grant announces its step once) or run it with a hint consumer.
- **m7.** `key_consistency` leaves out two clauses: a waiter row whose run is terminal (invariant 2: "every waiter run is
  `WAITING`"), and an empty semaphore row (FR-022-020: "empty rows MUST be removed"). Add both checks and seeded tests.
- **m8.** SC-022-004 is cited, but at run level only its takeover part is here: five run starts on one key with
  `limit: 1` (at most one holder, grants in start order) and a mixed-limit pair sharing a group are missing for runs.

## nit

- **n1.** `rest.rs:206` compares `oldestWaitSince` as strings; RFC 3339 fractions with trimmed zeros do not sort as
  instants. Parse and compare.
- **n2.** D022-055 item 2 says a key's movement between pages is stated in the contract; `ConcurrencyKey` and
  `listConcurrencyKeys` say nothing of it. Write it in the contract.
- **n3.** `key_consistency` returns `Ok(vec![])` on 42P01 or 42703 at any check: a rename silently turns the check off in
  every suite. Probe `to_regclass` once and fail on 42703.
- **n4.** The race test's doc comment suggests both routines contend for the waiter (see M1).
- **n5.** `RunPermit.since` is `clock_timestamp()` read after the key's lock, not the commit time; reword.

## Checked and sound

Listing snapshot (`begin_safely` runs nothing before `SET TRANSACTION`); the row comparison and ORDER BY use the same
expressions and collation, `NOT $4 OR …` short-circuits; `limit + 1 ≤ MAX_PAGE_SIZE + 1`; the lateral prefers the holder
and uses both primary keys; one `queuedRuns` statement; `against_at` cannot pass vacuously and the reversed control does
deadlock; the fault tests' crash points are real (one backend waits on the held key and is the one terminated; retries
decide once); `assert_consistent` cannot flake under concurrent commits (one snapshot; each `verify_run` under its run's
lock).
