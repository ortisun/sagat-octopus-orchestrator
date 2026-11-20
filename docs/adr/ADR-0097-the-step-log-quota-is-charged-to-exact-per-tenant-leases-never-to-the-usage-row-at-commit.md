# ADR-0097: The step-log quota is charged to exact per-tenant leases, never to the usage row at commit

**Status:** Proposed
**Decided by:** agent under the owner's delegation OD-2026-11-20-6 ("decide for advantage"), design 041's D041-108,
D041-112, D041-117 (the lease owned by its writer) and D041-119 (room made before the report, a fallback refill
bounded by a budget); adopted by the coordinating session on 2026-11-20. Acceptance (D041-115): README rule 4 requires a person to accept, and
the delegation does not stand in for that; the coordinating session puts this ADR to the owner with a recommendation once
design 041's re-review has no Blocker or Major and task T041-35's fourth run, which measures decisions 1 to 4 and 5 to
8 on the leases (decision 3b, the ensure, is measured by task T041-9), is recorded (question O041-042),
whatever that run's result (this decision rests on D041-106's ceiling and on D041-116's diagnoses, not on the 1 × 16
gate), together with the restated O041-040
**Refines / supersedes:** would supersede, when accepted:
- for the class `step_log`, the charge half of item 3 of the Amendment of 2026-11-15 of
  [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (a charge goes to a lease, not to
  `quota_transaction_deltas.step_log_delta` and the finalizer at commit) and the second bullet ("One flush per
  transaction") of its proposed amendment of 2026-11-20 (O041-040), whose statement-level measurement and per-row release
  it keeps; and, for that class, ADR-0027's rule that the usage counters never go below zero (S may, U may not);
- in [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) (Revision), the clause of
  decision 3 that a batch "MUST NOT take the tenant's `tenant_retained_usage` row before its commit" and refuses "by the
  commit-time finalizer's raise" (a batch now charges a lease at its insert, takes the usage row only for a refill, and
  is refused at its insert), and the ingest's reading of decision 11 (the class fences at the insert, a lease, the usage
  row on a refill);
- in [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) decision 7 ("MUST NOT
  fail, block, retry or delay a step beyond one bounded final flush"): a report may also wait for the ensure of decision
  3b (a transaction of its own, under no run lock, with its own budget) and for the fallback refill of decision 4 (each
  lock acquisition at most 200 ms); both are bounded delays, and neither fails, blocks or retries a step;
- in [ADR-0095](ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md), decision 2 (a carrying report
  adds no transaction: it may add the ensure, before the report and under no run lock), decision 4 (a carry may delay a
  report by the ensure and the fallback's budget) and decision 5 (the carry's block also answers a fallback past its
  budget, in the same call), revised with this ADR.

On acceptance ADR-0027 and ADR-0084 are marked "partly superseded by ADR-0097". The content and log classes, their deltas
and their finalizer are unchanged.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the invariant tests of decision 2, the held-lock tests of decisions 3 to 6
(held transactions and `pg_stat_activity`, never sleeps) and a property test over random interleavings of charges,
refills, reclaims, releases and limit changes, the runtime-role bypass tests of ADR-0027 extended to the lease table, the
new column and the functions, and a source-text check that only the ensure, the refill, the release branch, the idle reclaim
and the limit change lock `tenant_retained_usage` for the class `step_log` (T041-5); the lock-order test with slots for L3 and the lease (T041-6,
T041-10, T041-36); the budget lines of design 041 sections 2b and 2a (T041-6, T041-36). Until then, the prototype
`step_log_spike::install_leases` and its tests in `tests/performance/scalability-ha/step_log_leases_tests.rs` (run by
hand with `cargo test -p sagat-tests --test perf_scalability_ha --features perf-spike leases_tests`; they prove
exactness and lock behaviour, not the privileges of decision 1).

## Context

ADR-0027 charges a retained byte in the same transaction as the row that holds it: immediate triggers add the byte count
to the transaction's private row of `quota_transaction_deltas`, and a deferred finalizer, at commit, takes the class's
policy fences, then `SELECT … FOR UPDATE` of the tenant's one `tenant_retained_usage` row, checks the limit and updates
the row. PostgreSQL releases a row lock only when the transaction ends, after its commit record is flushed. So every
commit of one tenant that charges any class holds that row from its finalizer to the end of its WAL flush, and the
tenant's charging commits pass it one at a time, whatever the number of replicas and connections.

Design 041 makes far more commits charge: every report that carries a step-log record and every batch of a long step,
outputs of `{}` included. Measured on the hosted runners (D041-106, `specs/041-step-logs/validation-report.md`):

- At 1 replica and 16 workers the row is held from the lock to the `COMMIT`'s return p50 1.1 ms and p99 4.0 to 4.5 ms; it
  is busy 23% to 24% of the time when every completion charges once and 68% when every step charges twice (a batch and a
  report): one tenant's ceiling is roughly 800 charging commits a second on that machine, half that for long steps,
  whatever the number of replicas.
- At 8 replicas and 128 workers (run 44741773762, pools not filled first) the server logged 1,075 ingest `COMMIT`s
  cancelled by their 200 ms lock timeout while waiting for the row in the finalizer: the row turned batches away.
- The same diagnosis showed that this queue was not what failed T041-35 at 1 × 16: there it was not yet saturated.

The forces: the owner requires the quota to be exact (OD-2026-11-20-6: an overshoot of a ceiling is a calculation error
unless bounded, proved and documented); ADR-0027 requires the charge in the same transaction as the rows, measured by the
database, with no runtime path to skip it; a step log is lossy by ADR-0084, so a refused charge drops records and counts
them; a report must not wait under its run lock for another tenant's, replica's or attempt's commit (relaxed by
decision 4 to one counted wait of the fallback refill, each lock acquisition at most *b* = 200 ms, rare far from the
limit and with session affinity; the content class's finalizer already
waits at commit, G041-024); and a step-log refusal must never fail, block or retry a step (ADR-0084 decision 7,
ADR-0095 decision 4).

## Decision

1. **Leases, one a writer.** A table `step_log_quota_leases (tenant_id, writer integer, granted bigint, used bigint,
   charged_at timestamptz)` holds one lease per tenant and server backend that charges for it, keyed by
   `pg_backend_pid()` (a foreign key to the tenant's usage row, `ON DELETE CASCADE`; fill factor 50). A backend runs one
   transaction at a time, so a writer's lease is never locked by another writer, behind a transaction pooler too (the
   key is the server backend, not the client connection, though the ensure of decision 3b needs session affinity); a
   pid reused after a backend ends inherits that lease's room, which is counted in G either way (decision D041-117). The
   purge pass folds and deletes, one tenant a transaction, every lease not charged for 10 minutes (the pools' idle
   timeout) and every lease whose backend has ended (its pid absent from `pg_stat_activity`), holding the step-log fence
   shared (tried; a tenant whose fence is held is skipped) and then the usage row, leases `SKIP LOCKED`; it finds those
   tenants through a definer function that returns only their ids; a tenant's lease rows are then at most the backends
   that charged for it within 10 minutes;
   `tenant_retained_usage` gains `step_log_leased_bytes` (G). With S its `step_log_bytes` (the folded usage, which MAY be
   negative), the tenant's step-log usage is U = S + Σ `used` and its leased room is R = Σ (`granted` − `used`). The
   runtime role MUST have no privilege on the lease table, the new column's writes or the counter sequences, revoked
   explicitly; only `SECURITY DEFINER` functions of ADR-0027's pattern (fixed `search_path = pg_catalog, pg_temp`, every
   name schema-qualified, trusted owner, `REVOKE ALL … FROM PUBLIC`) read or write them. Of those, the runtime role may
   execute exactly `sagat_quota_step_log_reserve`, `sagat_quota_step_log_ensure`, `sagat_quota_step_log_usage`,
   `sagat_quota_step_log_counters`, `sagat_quota_step_log_idle_tenants` and `sagat_quota_step_log_reclaim_idle`; the
   charge, the refill, its budget function and the limit-change trigger function are revoked from it; the privilege
   audit checks exactly this list (design 041, NFR-041-SEC).
2. **Invariants**, each checked by a constraint or a test: (I1) 0 ≤ `used` ≤ `granted` on every lease (a `CHECK`); (I2) at
   every commit G = Σ `granted` of the tenant's leases, both written only by a transaction holding the usage row; (I3)
   G ≥ 0 (a `CHECK`) and U ≥ 0 (checked by every release); (I4) a grant commits only with S + G ≤ L after it, L the
   tenant's limit read after the step-log fence; (I5) a charge into a lease commits only with S + G ≤ L for the L it
   read. A charge moves bytes from R to U and changes neither S nor G, so U + R = S + G at every commit and
   **U ≤ S + G ≤ L: the usage never exceeds the limit in force when the charge was admitted**, the exactness ADR-0027
   gives today, with no overshoot to bound.
3. **A charge** is made by the statement trigger of the step-log rows, once per insert statement and tenant, with the
   bytes `sagat_quota_measure` measures from the rows stored, never a caller's number. It MUST take the content, log and
   step-log policy fences shared, in ADR-0027's order; read L and, without a lock, S and G; when S + G ≤ L (or the
   quota is not enforced), add its bytes to the `used` of the writer's own lease when it has room, locked by primary key
   `FOR UPDATE SKIP LOCKED` (only a reclaim, which holds the usage row, can hold it; one this transaction holds is
   found). It MUST NOT lock `tenant_retained_usage` and MUST NOT wait for a lease. Otherwise it calls the refill. A carry
   first reserves its own lease with room for its planned charge (`sagat_quota_step_log_reserve`: the step-log fence
   tried shared, the lease locked `SKIP LOCKED`, nothing written; false for a non-positive amount), so that its charge
   cannot be refused and needs no
   subtransaction (ADR-0095 decision 5). The planned charge MUST bound what the trigger will measure: every sent record's
   `sagat_quota_measure` sizes plus 128 a row, plus 128 for a possible marker, computed before CC for either stream case.
3b. **Room made before the report, best effort** (decisions D041-119, D041-120 and D041-121). After the carry's checks
   that run before the transaction (C1 to C3) and only for a carry that will write records, before it opens the report
   transaction, the store MUST make sure the
   writer's lease of the report's tenant (the principal's tenant, never one from the request or the run) has room for
   the planned charge, when the room it last saw for that backend and tenant is less, unknown, or stale (9 minutes
   after the lease's last charge or grant, which each call returns):
   it calls `sagat_quota_step_log_ensure(tenant, planned)` in a transaction of its own on the connection it will use for
   the report, holding no run lock. The ensure refuses a planned charge larger than any valid carry's; tries the
   step-log fence shared (held: `contended`); answers `room` when the writer's lease already has the room; reads the
   allowance without a lock and answers `limit` when the charge cannot fit; and otherwise refills the writer's lease
   (decision 4) under the same budget as the fallback, answering `made`, `busy`, `held` or `limit`. It never raises for
   any of these; it returns its answer, the backend's pid and the room left. Any other error of its transaction is
   rolled back and is the answer `error`, after which the report proceeds as if no ensure had run. The ensure is a step
   of the store's method that begins a carrying report, which returns the answer as a closed `CarryRoom`; the
   application maps `limit` and `held` to a mark-only carry plan (ADR-0095 decision 3), and the adapter holds no rule.
   The store counts the answer (`step_log_quota_ensures_total{outcome}`) and never fails or re-runs a report because of
   it. After `limit` or `held` the carry answers mark-only at once without a refill; after `busy`, `contended` or
   `error`, or a `made` whose room a reclaim took before the reservation, the carry may fall back and wait again, under
   the run lock: a report waits on the usage row at most twice, once under no run lock and once under it. Every call
   that charges or refills (the carry, the ensure, a batch, the script marker) returns the backend's pid, the room left
   and the lease's last charge, and the store keeps the room per (backend pid, tenant) until 9 minutes after that last
   charge or grant, under the idle interval. The ensure helps only with session affinity (direct connections or a
   session pooler): behind a pooler in transaction mode the ensure and the report can run on different backends, the
   report then falls back (design 041's gap G041-027), and this decision has no effect; the store counts those affinity
   misses and every fallback (decision 8). The first charge of every (tenant, backend), and the steady refills, are thereby made outside
   the run lock; a refill inside a report remains the fallback when a reclaim took the room in between (near the limit,
   the idle reclaim, a limit change) or the room the store kept was wrong.
4. **A refill** (the writer's lease short of the charge) locks the usage row before it would ever insert it, then the
   writer's own lease `NOWAIT` (only a holder of the usage row, or a limit change holding the step-log fence exclusively,
   can hold it, so this never fails short of a prepared transaction): after the usage row nothing waits. Inside a report
   or a batch each lock acquisition on the usage row waits at most its budget *b* = 200 ms (`lock_timeout` set for that
   statement from a function no caller can change, a caller's lower `lock_timeout` kept, the caller's value restored
   after it), and the statement timeout (5 s, 3 s for some transactions) is the hard bound, past which the carry call
   fails and its report is re-run without the carry (ADR-0095 decision 5, the records counted `failed`); *b* is
   the budget an ingest batch already has on the same row, a latency budget, not a worst case: measured holds of the row
   are p50 1.1 ms and p99 4.5 ms (D041-106), and since `lock_timeout` applies to each lock acquisition a wait spanning
   several holders can exceed it. Past *b*, inside a carry, the carry's block catches the `55P03` and records a mark-only
   final mark whose reason is `tenant_quota_busy` (the records counted dropped for it), shown to the reader by the
   streams route and Studio, counted per caller and alerted (decision 8); the report lands. Inside a batch, it is the
   batch's `RETRY_LATER`, as before. The fallback refill is thereby the one wait a carry has under its run lock, rare
   far from the limit and with session affinity (decision 3b), each lock acquisition bounded by *b*, and counted; no
   step-log refusal fails, blocks or retries a step. The ensure (decision 3b) waits under the same budget, under no run
   lock. The wait cannot deadlock: no transaction waits for a lease it does not
   own (every other lease is taken `SKIP LOCKED`, or `NOWAIT` by a limit change holding the fence exclusively, and the
   writer's own lease `NOWAIT` after the usage row), and no holder of the usage row waits after taking it for anything a
   waiter holds (a content or log finalizer is the commit's last step; a refill then takes only its own lease `NOWAIT`
   and writes rows only its transaction touches; a release and a reclaim take nothing more). It reads L itself. When the
   free room L − S − G is under max(*n*, 1 MiB), *n* the charge, it first reclaims every other lease no open
   transaction holds (`SKIP LOCKED`: S += `used`, G −= `granted`, both set to 0). It then folds the writer's own lease
   whole (S += `used`, G −= `granted`; inserted when absent) and grants it g = max(*n*, min(1 MiB, ⌊F / (2(*A* + 1))⌋)),
   F = L − S − G and *A* the tenant's other leases holding a grant, raising when g > F (I4):
   `tenant_step_log_quota_held` when the room missing is in leases it could not lock, `tenant_step_log_quota_exceeded`
   otherwise; else it charges *n* there (or, for the ensure, only leases g). **Room held by open writers, bounded.** A
   grant's unused room is at most its share, min(1 MiB, ⌊F / (2(*A* + 1))⌋) at the grant: the rest of a grant of *n*
   past the share is the planned charge itself, charged at once by a refill and, after an ensure, by the reservation's
   carry in the same report. So the room open writers hold is at most the sum, over their leases, of each grant's share,
   plus the planned charges of carries that have reserved and not yet charged. When *k* writers take fresh grants one
   after another for charges within their share, the *i*-th finds at least *i* − 1 other leases holding a grant, so
   F_i ≥ F_{i−1} (1 − 1/(2*i*)), and the product of (2*i* − 1)/(2*i*) for *i* = 1..*k* is C(2*k*, *k*) / 4^*k*: at least
   F₀ · C(2*k*, *k*) / 4^*k* (> F₀ / √(π(*k* + ½))) of the room F₀ free before them stays chargeable by another writer
   (the 1 MiB cap only lowers grants). For an operator: a charge of *n* is refused as held only when the unleased room is
   under *n* and the tenant's open charging transactions, *k*_open of them, hold at most *k*_open × 1 MiB plus their
   planned charges. A carry refused as held records a mark-only final mark with `tenant_quota` (the reader is told the
   quota is full or its last room is in use by other steps, G041-023); idle leases give back all their room at the next
   refill near the limit.
5. **A release** (the purge's `DELETE`, which keeps ADR-0027's per-row delta and deferred flush) MUST NOT wait for a
   lease: in the finalizer's step-log branch, under the usage row, it reads Σ `used`, refuses with
   `step_log_usage_underflow` (counted) when S − released + Σ `used` < 0 (I3), and otherwise sets S −= released. No
   charge, refill or release waits for a lease.
6. **A change of the limit.** An `ENABLE ALWAYS` `SECURITY DEFINER` trigger on `quota_limits` and `quota_ceilings`, for
   `tenant_step_log_bytes`, takes the step-log fence exclusively itself (at once when the editor already holds it; so no
   transaction holds a lease), then the tenant's leases `NOWAIT`, then the usage row, and reclaims every lease (S += Σ
   `used`, G = 0), whoever wrote the row. A lowered limit therefore stops growth at once (I5) and leaves no stranded room; a charge that still meets
   S + G > L goes to the refill, which reclaims, and is refused only when what it needs is not there.
7. **Reads.** The usage of `tenant_step_log_bytes` that the quota API, `sagat_quota_step_log_allowance` and every
   reconciliation read MUST be S + Σ `used`, in one snapshot, through a `SECURITY DEFINER` accessor. With
   `step_log_enforcement_enabled` off, a refill grants without the limit check and I4/I5 do not apply; the usage stays
   exact.
8. **Telemetry**, closed labels, no tenant (the list of design 041's data-model §7):
   `step_log_quota_refills_total{kind="refill"|"reclaim"|"idle"|"ended"}`,
   `step_log_quota_ensures_total{outcome="room"|"made"|"limit"|"held"|"busy"|"contended"|"invalid"|"error"}`,
   `step_log_quota_fallbacks_total{caller="carry"|"ingest"|"marker"}`, `step_log_quota_ensure_affinity_misses_total`
   (a report whose carry returned a pid other than its ensure's), `step_log_quota_refusals_total{cause="limit"|"held"|"busy"}`
   (`held` when the room is in leases open transactions hold, raised under its own constraint name
   `tenant_step_log_quota_held`; `busy` for a fallback refill past *b*),
   `step_log_quota_refill_budget_exceeded_total{caller="carry"|"ingest"|"marker"}` (counted by the store from the
   carry's returned cause, a batch's mapped `55P03` and the marker's; the alert `SagatStepLogRefillBudgetExceeded` fires
   when `caller="carry"` exceeds 0.1% of `step_log_carries_total` for ten minutes, an ingest's being its ordinary
   backpressure), `step_log_purge_failures_total`, and a gauge of lease rows. Events counted inside SQL MUST NOT update a counter row (a hot row again): they advance
   sequences, read as deltas with the lease-row count by a definer accessor that the purge pass calls; the carry
   returns its refusal cause.

## Considered options

1. **Exact leases, one a writer, given room before the report** (chosen, D041-117, D041-119): each server backend
   charges its own lease, so no writer queues behind another's commit or lock for the step-log class; exact by I1 to
   I5. The usage row is locked by an ensure outside any report (once per grant a writer: far from the limit and with
   F ≥ 2(*A* + 1) MiB, a grant is 1 MiB, about 300 ten-line carries; a tenant with less volume per backend per 10
   minutes, or near its limit, refills more often), a fallback refill, a release, an idle reclaim and a limit change.
1b. **A shared pool of 256 leases picked `SKIP LOCKED`** (this ADR's text before D041-117): measured to collide (runs
   44783898911 and 44793160071: 6 to 13 leases for 16 or 128 writers, the refills set by collisions, 1.8% to 11.7% of
   carries against about 0.3% to 2% by the bytes), and with a `NOWAIT` refill to answer 0.08% to 21% of carries
   mark-only while the tenant had room. Topping the pool up below a low-water mark leaves the collisions (the pick still
   fails while writers outnumber leases with room); growing it to the peak of writers scans as many rows on every pick and
   races at each growth.
1c. **A lease per (replica, connection slot)**: as collision-free, stable across pool recycles (no first refill per new
   backend) and attributable to a replica; but the key is not unique by construction: two backends can share a slot
   during a rolling deploy's overlap or after a replica's identity is reused, and then a refill's lock of "its own"
   lease after the usage row could meet a peer, which the deadlock argument of decision 4 rules out by uniqueness
   (PostgreSQL guarantees one live backend per pid). The backend key's costs are a first refill per (tenant, backend),
   made by the ensure outside the report (decision 3b), and lease rows bounded by the backends active within the idle
   interval. Kept as the alternative if backend keys prove hard to observe (the lease-row gauge and the per-replica
   `application_name`, T041-5, are the criterion).
   *Restated by decision D041-121*: the deadlock argument above does not hold against 1c, since decision 4 takes the
   writer's own lease `NOWAIT` and a charge takes it `SKIP LOCKED`, so a shared slot would make the second backend fail
   fast (`55P03`, a busy answer), not wait. What 1c would gain: behind a transaction pooler a slot follows the client
   connection, so the ensure and the report would use the same lease and gap G041-027 would not arise; and a slot
   survives pool recycles. What it costs: the key is the client's word, not the server's (a store defect, or two
   backends on one slot during a deploy's overlap, sends charges to a lease another backend uses, and every such
   overlap answers busy); a slot token on every charging call; a replica's lapse waits for the idle interval instead of
   the ended-backend reclaim; and a deploy still starts new slots, so the cold-start herd stays. Backend keys are kept;
   1c is the named alternative if task T041-9's pooler scenario shows fallbacks above the reopening threshold of
   question O041-041 in the deployed topology.
1d. **A refill that never waits (`NOWAIT`) and answers mark-only when the row is busy** (D041-112): loses a tenant's
   records under load for no quota reason; replaced by the ensure of decision 3b and the bounded fallback of decision 4.
1e. **A background refiller per replica** keeping every lease above a low-water mark: would take the refill off the
   report's path too, but needs a component that finds tenants and backends to serve and competes with the writers for
   the usage row on its own cadence; the ensure of decision 3b does it on the writer's own connection, only when the
   writer's next carry needs it, with no new component.
1f. **A refill inside the report that waits longer and fails the report past its bound** (D041-117 as first written,
   with a bound derived from `max_connections`): a report failed and resent can exhaust a worker's retries or outlive its
   lease, changing a step's outcome against ADR-0084 decision 7 and ADR-0095 decision 4; and the bound was neither a
   worst case nor independent of an unrelated server setting. Replaced by decisions 3b and 4.
1g. **Protecting a fresh grant from the near-limit reclaim** (a `reserved_until` on the lease, so that another refill
   cannot take an ensure's room before the report reserves it): fewer fallbacks near the limit, but room that cannot be
   reclaimed for a while turns into held refusals there instead; not taken, and task T041-9 measures how often an
   ensure's room is reclaimed before the reservation.
1h. **An ensure on every carrying report**, which keeps no room in the store: correct whatever the cache, but one more
   transaction on every carrying report; rejected for its cost, the cache being keyed by backend and expiring before
   the idle interval.
2. **The charge folded at commit by a deferred trigger** (ADR-0027 as built; T041-35's second run): the lock is already
   taken only at commit, and a row lock lasts to the end of the commit, its WAL flush included; no trigger releases it
   earlier. It keeps the ceiling of the Context.
3. **A step-log usage row of its own** (O041-026 (b)): moves the queue to another row, does not remove it; every carrying
   report would still take it at commit.
4. **Striped counters.** With per-stripe ceilings L/K exact, but a tenant whose writers are uneven is refused while other
   stripes have room, by up to (K − 1)/K of its quota. With one ceiling read without a lock, an overshoot of at most the
   charges in flight (the pool size times a 24 KiB carry): bounded, and so admissible under the owner's rule if proved
   and documented, but every quota view and refusal would then carry a tolerance, where the leases need none for the same
   concurrency.
5. **Locking only near the limit** (a margin bounding the charges in flight, as ADR-0084 once did within 2 MiB of the log
   limit): exact only if the margin is proved against the concurrency, which changes with the deployment; the leases give
   the same exactness with no deployment-dependent margin.
6. **A ledger folded later** (each charge appends a row, a job folds them): an admission that reads the folded usage
   overshoots by the unfolded charges; exact only if the admission locks, which is option 2.
7. **Charging after the report commits** (a second transaction): breaks ADR-0027's rule that the charge and the rows
   commit together; a crash between them leaves rows uncharged.
8. **Deferring the leases** (D041-106 showed the queue did not fail T041-35 at 1 × 16): keeps a measured ceiling of about
   800 charging commits a second per tenant and the batch refusals at 8 × 128; rejected for advantage (OD-2026-11-20-6).
9. **A release that folds every lease, waiting for each** (this ADR's first text): it would hold a tenant's leases on the
   purge's cadence and send its charges to the usage row meanwhile; replaced by decision 5, which waits for no lease.
10. **A subtransaction on every carrying report** (one `BEGIN … EXCEPTION` block around its writes, this ADR's second
   text): every carrying report would take a second transaction id (XID use and freeze pressure doubled for them, and
   `pg_subtrans` lookups for readers under a long transaction); replaced by the reservation of decision 3, which keeps the
   block for the carries that need a fallback refill (rare once the ensure of decision 3b made their room; the refill
   itself opens no block).

## Consequences

- A tenant's step-log charges no longer pass one row one at a time; a report that also stores output still takes the
  usage row at its commit for the content class (ADR-0027 as built; design 041's G041-024, outside this ADR).
- A charge can be refused while room exists only in leases that open transactions hold: at most the room of those
  leases (decision 4's bound), and only until those transactions end. It is counted (`cause="held"`), never an
  overshoot (design 041's G041-023).
- A report that carries pays one more short transaction on its connection, the ensure, when the room the store kept
  for its backend is short, unknown or stale: once per grant, for the first carry of each (tenant, backend), as after
  a deploy or a pool recycle, and after an idle reclaim. That transaction holds no run lock, each of its lock
  acquisitions waits at most the same budget, and it never fails the report; it does delay it, so a report sent close
  to its lease's expiry can arrive late and be refused (design 041's gap G041-021). The ensure's `made` answer writes
  inside a block of its own (one subtransaction id), which task T041-9's subtransaction criterion counts.
- A fallback refill waits for the usage row under the run lock: each lock acquisition at most *b* = 200 ms, the
  statement timeout the hard bound; sibling reports, heartbeats, cancels and signals of the same run queue behind it
  meanwhile. Past the budget the carry's records are dropped with the reason `tenant_quota_busy`, shown to the reader
  ("not stored: the log service was busy, and these lines were not checked against your quota") and alerted on its
  rate: a bounded, visible loss in a
  rare path, traded for a report that always lands (ADR-0084 decision 7). Rare means far from the limit and with
  session affinity: near the limit, reclaims take room between the ensure and the report, and behind a transaction
  pooler the ensure may serve another backend, so fallbacks come more often (design 041's gaps G041-026 and G041-027;
  the owner chooses between this loss, failing and resending, and writing the records after the report commits,
  question O041-041). After an ensure answered `busy`, the fallback can wait a second time, so a report waits on the
  usage row at most twice, once under the run lock (decision 3b). Design 041's task T041-9 measures these regimes.
- A carry that needs the fallback opens one subtransaction (its block, ADR-0095 decision 5): a second transaction id for
  that report; the refill and a carry with nothing to charge (no record and no marker planned) open none. Design 041's T041-9 reports the XID rate and `pg_stat_slru` for `subtransaction`
  near the limit and for many low-volume tenants.
- Every step-log charge holds the content and log fences shared (ADR-0027's order against an editor that takes several
  fences), so a change of `tenant_content_bytes` or `tenant_log_bytes` waits for the charges in flight and, while queued,
  makes carries `contended`.
- S may be negative while bytes charged in leases are released; U is what is shown and checked.
- One lease row per tenant and backend that charged for it within the idle interval, five columns each, updated by
  every charge (fill factor 50, per-table autovacuum settings by T041-5). A tenant that stops logging keeps its leases'
  room until the idle reclaim, a refill near the limit, a limit change or the down migration returns it; the usage it
  shows is exact meanwhile.
- One more table, two more columns' worth of rules, the charge, refill and ensure functions, a usage accessor, a branch
  of the finalizer for the release and one for a limit change. The down migration refuses while a step-log line exists,
  and otherwise folds Σ `used` into S (U = 0, so S = 0), then drops the leases and the column: once lines exist,
  rolling back means purging every line first.
- Runs in flight are unaffected: nothing is charged under this rule before the migration that adds it, and step logs do
  not exist before design 041.

## Evidence

- Run 44730444900 (validation-report.md, 2026-11-20, "diagnosed"; D041-106): what a charging `COMMIT` waits on (the usage
  row, nothing else), how long the row is held, how busy it is. In paired blocks a charge into one of 64 per-tenant rows
  taken `SKIP LOCKED` (a proxy of the leases: pre-granted 1 TiB, no fences, no limit read, no refill) against no charge at
  all: rate 0.988 [0.842, 1.006], p99 +3.4 ms; against the usage row at commit 0.993 of its rate, p99 −1.3 ms; (a) on the
  usage row against (a) on the slots 1.008, p99 +0.1 ms. The cost of the lease charge itself is measured by design 041's
  T041-35 third run.
- Run 44741773762 (validation-report.md, "The carry's cost, broken down"): 1,075 ingest `COMMIT`s at 8 × 128 cancelled by
  their lock timeout on the usage row (with pools not filled first, so the load at the start of a row was higher).
- Decisions D041-117 to D041-120 (research.md), and the reviews and re-reads of the writer leases
  (`docs/designs/041-step-logs.reviews/2026-11-20-writer-leases/`).
- Runs 44783898911 and 44793160071 (validation-report.md, "split by cause"; D041-117): the shared pool's collisions and
  the `NOWAIT` refill's mark-only answers; the slow path's p99 near 95 ms at 8 × 128 against the fast path's 32.7 ms.
- `step_log_spike::install_leases` and `step_log_leases_tests.rs`: each writer charges its own lease and never meets
  another writer's lock; a charge with room never takes the usage row and a refill alone waits for it; the ensure
  gives a carry its fast path and, with room, takes no lock; a carry whose refill meets a held usage row within its
  budget waits and writes; past the budget it answers `tenant_quota_busy` and the report commits, whatever the caller's
  settings; a refill restores the caller's `lock_timeout`; the coordinating session's deadlock cases (a completion
  holding the row before its own refill against another refill; a writer holding its lease waiting for a reclaiming
  refill; an ingest refill against a carry refill in each order; a release and a limit change around a waiting refill;
  a limit change against an open idle reclaim), each ending with the invariants checked; a small quota with twenty idle
  writers admits everything their leases hold; the held-room bound for *k* = 1, 2, 8 and 32 from a quarter-used limit;
  concurrent charges (32 tasks on a pool of 8 connections) never pass a 200,000-byte limit and every byte charged is
  counted once; the last bytes of a 2 MiB limit are admitted from the room of idle leases and one byte more is refused;
  a lowered limit stops charges, and one lowered above the usage still admits what fits; a release never waits for a
  held lease and never takes the usage below zero; the budget line of a charged insert. Hand mutations, each failing
  its test, are recorded in the validation report.
- [design 041](../designs/041-step-logs.md), [research.md](../../specs/041-step-logs/research.md) D041-106, D041-108,
  D041-112.
