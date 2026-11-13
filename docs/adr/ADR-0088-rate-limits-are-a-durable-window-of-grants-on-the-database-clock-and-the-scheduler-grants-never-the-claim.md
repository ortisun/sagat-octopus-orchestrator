# ADR-0088: Rate limits are a durable window of grants on the database clock, buffered work is granted by its shard owner's scheduler pass, and a claim never reads the window

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-11-13; the owner asked for the capability in OD-2026-11-13-11 and has not chosen the mechanism)
**Refines / supersedes:** refines ADR-0056 (durable permits, one admission port, grant by the waiter's
shard owner), ADR-0050 decision 7 (merged admission lock order, slot 8b), ADR-0020 (sharded, fenced
scheduler) and ADR-0027 (rate and usage rows come last); builds on ADR-0002 and ADR-0003. Supersedes none.
**Enforced by:** proposed checks of feature 022's release 0.3.0 slice — the fault suite
`tests/fault/flow-control-step-policies/rate_limits.rs` (T022-080), the journal window invariant added to
`verify_run` (T022-079), the lock-order barriers (T022-081), the hot-path budget of design 038 extended
with the claim of a rate-limited step (T022-076) and the run-creator test (T022-086). None exists yet;
until they do, this rule is review only.

## Context

The owner asked for throughput control in release 0.3.0: concurrency limits by key (feature 022 US02,
ADR-0056) and a rate limit per time window, per key, where the excess waits durably instead of failing
(`specs/022-flow-control-step-policies/spec.md`, US07, FR-022-039..050). The forces:

- **A rate slot is freed by time.** A concurrency permit is freed by a holder's transaction, which is the
  event ADR-0056's grant follows. Nothing commits when a window ages, so something has to look at the
  clock on its own.
- **The engine's own clock has a resolution.** Scheduler passes run every 500 ms by default
  (`SAGAT_SCHEDULER_INTERVAL_MS`). Any grant made by a pass arrives in batches of that size.
- **The claim path is measured and budgeted.** Design 038 found that pollers probing candidates they
  cannot lease were the main waste of the hot path, and put a test on the statements of a lease
  (FR-038-001). A grant decided at claim time would add a row lock and a count to the claim and offer
  every poller steps it must then refuse.
- **Several clocks exist.** Each replica has one; PostgreSQL has one; they disagree. The platform already
  decides time-dependent facts on the database clock inside the deciding transaction.
- **Replay never re-decides.** The interpreter journals decisions (ADR-0003); a replayed run must not
  consult a clock or a counter.
- **A granted step may wait for a worker.** If the limit counted from the grant, steps granted while no
  worker polls would be leased together when one returns, which is the burst the limit exists to prevent.
- **A run may declare both a concurrency key and a rate limit.**

Unknown: the cost of one hot key with thousands of waiters, and the grant lag under load (G1174, G1178).
The meaning of "N per window" is a product choice put to the owner (O1163); this ADR records the
mechanism for the recommended answer and says what changes with the other answers.

## Decision

1. **The window is rows.** For each tenant, namespace, pool, scope (run or step), group and keyed digest
   of the key value there is one window row, the grants of the key that can still count, and its waiters
   in enqueue order. They MUST change only in the transactions listed in the plan of feature 022
   (T19–T25). Process memory, NATS, Redis and a replica's own clock MUST NOT decide, remember or shorten
   a grant.
2. **Strict sliding window on the database clock.** An acquirer with `(limit, per)` MUST be granted only
   when fewer than `limit` grants of its key count at the instant the deciding transaction reads from the
   database clock, and no admitted waiter of the key is ahead of it. A run-scope grant counts for `per`
   after its grant. A step-scope grant counts while its attempt has not been leased and for `per` after
   the lease instant. Hence no interval of length `per` holds more than `limit` run grants, or more than
   `limit` step leases, of a key. There is no `burst`. A slot MUST be returned only when a granted step
   ends without ever being leased. Each acquirer applies its own pinned `limit` and `per`, clamped by the
   current policy, to the key's shared grants; lowering a limit MUST NOT revoke a grant.
3. **The scheduler grants buffered work; admission and readiness grant only when nobody waits.** The
   admission transaction of a run (through ADR-0056's one admission port, used by every run creator) and
   the transaction that would make a rate-limited step ready MUST either grant, when decision 2 allows it,
   or buffer the run or step (`WAITING`, wait kind `RATE_LIMIT`, a timer at its queue deadline, no task
   and no attempt). A buffered run or step MUST be granted only by the scheduler replica that holds its
   run's shard lease, inside a transaction fenced by that lease, which locks the run, re-checks decision 2
   under the window row, journals the grant, removes the waiter and creates the run's first tasks or makes
   the step `READY`. A grant whose fence, count or ordering check fails changes nothing.
4. **One lock order.** Every transaction that touches a window MUST already hold (or be creating) the run
   concerned and MUST have taken every entity lock of its path, then the concurrency semaphore when one is
   involved (ADR-0050 slot 7), then window rows sorted by (pool, scope, group, key) and their grant and
   waiter rows, then quota and usage rows last. Window rows are rate rows of slot 8b. After a window row a
   transaction MUST NOT lock a run, an aggregate count, an activity-quota row or a semaphore. The one
   transaction that holds no run lock is lease recording (decision 6), which writes only window and grant
   rows.
5. **A claim never touches a window.** A step with a node-level rate limit is not `READY` until granted,
   so no claim considers it; once granted it is claimed by the unchanged claim code. The claim of a
   rate-limited step MUST execute the same statements as the claim of a plain step, and a definition
   without `rateLimit` MUST add no statement to any claim, completion or admission.
6. **The pass records leases; late is safe.** The pass MUST copy the lease instant of a granted attempt
   from the `step.leased` fact the claim already commits, and MUST delete the grant of a step that ended
   unleased. Until the lease is recorded the grant counts as unleased. The pass can therefore be late,
   crash or be taken over without ever admitting a grant early.
7. **Permit and slot together.** When a run declares both a concurrency key and a rate limit, admission
   and the grant pass MUST give it both in one transaction or neither. It waits once, until the earlier of
   the two queue deadlines; a grant and the timeout are decided by first commit (ADR-0056 decision 6). At
   step level the slot comes first, from the pass, and the permit at the claim (ADR-0056 decision 3).
8. **Every decision is a journal fact; queues are bounded.** `rate_limit.queued`, `rate_limit.granted`
   (with `limit`, `period_ms` and the granted instant), `rate_limit.wait_timed_out` and
   `rate_limit.dequeued` MUST commit with the transition they cause; replay MUST NOT read the clock or
   the window. Waiters count against the per-key waiter ceiling and the namespace `queued_runs` ceiling of
   ADR-0056 decision 7; a full buffer refuses a synchronous start and leaves event and cron starts
   pending. A reset (feature 018) takes a slot or is refused and never waits. A deployment MUST refuse to
   activate a new digest key version while any waiter or grant row exists under the old one.

## Considered options

1. **Strict sliding window of durable grant rows, granted by the shard owner's pass, lease-anchored for
   steps** — chosen. It is the only option here that is exact under a 500 ms pass: the pass grants every
   free slot at once and the bound does not depend on when it runs. The first `limit` start immediately,
   which is what "N per minute" is read as. The claim path is untouched.
2. **Token bucket with `burst`, one row per key** (the pattern of `quota_rate_buckets`) — rejected as the
   default. One row instead of up to `limit` rows, and a smoother load, but with a pass every 500 ms a
   strict bucket (capacity 1) cannot deliver more than two grants a second per key whatever the declared
   rate, and a bucket deep enough to keep up admits up to `limit + burst` in one window. If the owner
   prefers evenly spaced grants (O1163 option b), decisions 1 and 3 to 8 stand and decision 2 is replaced
   by the bucket, with the bound restated honestly.
3. **Fixed window aligned to the clock, one counter per key and window** — rejected: up to twice the limit
   across a boundary, and every waiter of a key granted at the boundary instant.
4. **Grant at claim time** (as ADR-0056 does for step permits) — rejected: exact at the lease without
   lease recording, but it adds a lock and a count to the claim, makes pollers probe steps they cannot
   lease (the waste design 038 removed), and still needs something to wake an idle long poll when a slot
   frees.
5. **Count from the grant for steps, with no lease recording** — rejected: after a worker outage every
   granted step is leased at once.
6. **Pre-assign each waiter a start instant at enqueue (a reservation)** — rejected: a cancelled waiter
   leaves a hole nobody uses, a changed limit cannot apply to those already scheduled, and the claim scan
   does not filter on a future `ready_at`, so it would have to change.
7. **A limiter in the worker or SDK, or in Redis** — rejected: lost on restart, one per process or a
   second source of truth, invisible to the operator, and not checkable by the transaction that acts.
8. **Refuse instead of buffering** — rejected: the owner asked for buffering, and event and cron starts
   must not be lost. A full buffer still refuses synchronously.

## Consequences

- Good: a limit that is never exceeded across crashes, restarts, duplicate passes, shard moves and clock
  disagreement between replicas; buffered work is ordinary runs and steps that can be listed, cancelled
  and timed out; uncontended keys pay no pass latency; nothing is added to the claim; replay is
  unaffected.
- Bad: one row per counted grant (at most `limit` plus the unleased grants per key, kept at most an hour
  after the lease); a buffered waiter is granted up to one pass interval after its slot frees, plus a
  shard takeover; grants of a busy key arrive as `limit` at once and again as they age out, with no
  smoothing; a hot key serializes on its window row; a per-key bound on grants per pass caps one key's
  throughput. A database clock that moves backwards delays grants until it catches up.
- Harder: every future writer that makes a rate-limited step ready, and every future run creator, must go
  through decisions 3 and 4; rate limits inside collection bodies (G1175) need 014's exact-path
  transactions to do the same; a second meaning of the window (decision 2) would need its own rows.
- Compatibility: additive tables, journal events, IR fields and run fields; no worker-protocol change;
  definitions without `rateLimit` are unaffected. Rollback disables the `rateLimits` capability for new
  publications and starts while the pass, the timers and lease recording keep draining waiters and
  grants; the tables are dropped only when empty (the down migration refuses otherwise).

## Evidence

None yet. Required before the capability is enabled by default: real PostgreSQL tests for the
grant-versus-timeout and grant-versus-cancel races (20 rounds each), a crash before and after each commit
of T19–T24, duplicate grant attempts from two replicas across a shard takeover, a restart after several
windows, a flood of 1,000 starts on one key with a second key and tenant alongside, a limit lowered while
runs wait, replicas with offset process clocks, a database clock set back, a worker outage with granted
steps, both gates on one run, the lock-order barriers of decision 4, the window invariant from the
journals, and the hot-path budget of decision 5; a benchmark of grant lag at 1, 16 and 256 waiters, one
key with 10,000 waiters and 1,000 keys on a stated environment. Spec:
`specs/022-flow-control-step-policies/spec.md` (FR-022-039..050, NFR-022-007..008, SC-022-017..023);
plan: `specs/022-flow-control-step-policies/plan.md` (Release 0.3.0 slice); research: D022-001..017,
G1174..G1183, O1163..O1168; journal events and reset rules:
`specs/022-flow-control-step-policies/contracts/journal-events.md`.
