# ADR-0097: The step-log quota is charged to exact per-tenant leases, never to the usage row at commit

**Status:** Proposed
**Decided by:** agent under the owner's delegation OD-2026-11-20-6 ("decide for advantage"), design 041's D041-108,
D041-112, D041-117 (the lease owned by its writer) and D041-119 (room made before the report, a fallback refill
bounded by a budget); adopted by the coordinating session on 2026-11-20. Acceptance (D041-115): README rule 4 requires a person to accept, and
the delegation does not stand in for that; the coordinating session puts this ADR to the owner with a recommendation once
design 041's re-review has no Blocker or Major and T041-35's fourth run, which measures this ADR's text, is recorded,
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
- in [ADR-0095](ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md), nothing it states for the carry
  except where its charge goes (ADR-0095, revised with this ADR, catches its own charge's refusal in the same call).

On acceptance ADR-0027 and ADR-0084 are marked "partly superseded by ADR-0097". The content and log classes, their deltas
and their finalizer are unchanged.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the invariant tests of decision 2, the held-lock tests of decisions 3 to 6
(held transactions and `pg_stat_activity`, never sleeps) and a property test over random interleavings of charges,
refills, reclaims, releases and limit changes, the runtime-role bypass tests of ADR-0027 extended to the lease table, the
new column and the functions, and a source-text check that only the refill, the release branch and the limit change lock
`tenant_retained_usage` for the class `step_log` (T041-5); the lock-order test with slots for L3 and the lease (T041-6,
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
decision 4 to one bounded, counted wait of the rare fallback refill, *b* = 200 ms; the content class's finalizer already
waits at commit, G041-024); and a step-log refusal must never fail, block or retry a step (ADR-0084 decision 7,
ADR-0095 decision 4).

## Decision

1. **Leases, one a writer.** A table `step_log_quota_leases (tenant_id, writer integer, granted bigint, used bigint,
   charged_at timestamptz)` holds one lease per tenant and server backend that charges for it, keyed by
   `pg_backend_pid()` (a foreign key to the tenant's usage row, `ON DELETE CASCADE`; fill factor 50). A backend runs one
   transaction at a time, so a writer's lease is never locked by another writer, behind a transaction pooler too (the
   key is the server backend, not the client connection); a pid reused after a backend ends inherits that lease's room,
   which is counted in G either way (D041-117). The purge pass folds and deletes, one tenant a transaction, every lease
   not charged for 10 minutes (the pools' idle timeout), holding the step-log fence shared (tried; a tenant whose
   fence is held is skipped) and then the usage row, leases `SKIP LOCKED`; a tenant's lease rows are then at most the
   backends that charged for it within 10 minutes;
   `tenant_retained_usage` gains `step_log_leased_bytes` (G). With S its `step_log_bytes` (the folded usage, which MAY be
   negative), the tenant's step-log usage is U = S + Σ `used` and its leased room is R = Σ (`granted` − `used`). The
   runtime role MUST have no privilege on the lease table, the new column's writes or the functions, revoked explicitly;
   only `SECURITY DEFINER` functions of ADR-0027's pattern (fixed `search_path = pg_catalog, pg_temp`, every name
   schema-qualified, trusted owner, `REVOKE ALL … FROM PUBLIC` and `FROM sagat_app`) read or write them.
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
3b. **Room made before the report** (D041-119). Before it opens a report transaction that will carry, the store MUST
   make sure the writer's lease of the report's tenant has room for the planned charge: when the room the previous carry
   on that connection returned (or nothing, for the connection's first carry of the tenant) is less, it calls
   `sagat_quota_step_log_ensure(tenant, planned)` in a transaction of its own on the same connection, so on the same
   backend, holding no run lock; the ensure takes the step-log fence shared and refills the writer's lease (decision 4,
   waiting for the usage row as the store's statement timeout allows) when its room is short. The report transaction's
   carry then reserves that room. The first charge of every (tenant, backend), and the steady refills, are thereby made
   outside the run lock; a refill inside a report remains only as the fallback when a reclaim took the room in between
   (near the limit, the idle reclaim, a limit change) or a carry outgrew its plan.
4. **A refill** (the writer's lease short of the charge) locks the usage row before it would ever insert it, then the
   writer's own lease `NOWAIT` (only a holder of the usage row, or a limit change holding the step-log fence exclusively,
   can hold it, so this never fails short of a prepared transaction): after the usage row nothing waits. Inside a report
   or a batch it waits for the usage row at most its budget *b* = 200 ms (`lock_timeout` set for that statement from a
   function no caller can change, a caller's lower `lock_timeout` kept, the caller's value restored after it); *b* is
   the budget an ingest batch already has on the same row, a latency budget, not a worst case: measured holds of the row
   are p50 1.1 ms and p99 4.5 ms (D041-106), and since `lock_timeout` applies to each lock acquisition a wait spanning
   several holders can exceed it. Past *b*, inside a carry, the carry's block catches the `55P03` and records a mark-only
   final mark whose reason is `tenant_quota_busy` (the records counted dropped for it), shown to the reader by the
   streams route and Studio, counted per caller and alerted (decision 8); the report lands. Inside a batch, it is the
   batch's `RETRY_LATER`, as before. The fallback refill is thereby the one wait a carry has under its run lock, rare
   (decision 3b), bounded and counted; no step-log refusal fails, blocks or retries a step. The ensure (decision 3b)
   waits without that budget, under no run lock. The wait cannot deadlock: no transaction waits for a lease it does not
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
   grant past the charge takes at most F / (2(*A* + 1)) of the free room; when *k* writers take fresh grants one after
   another, the *i*-th finds at least *i* − 1 other leases holding a grant, so F_i ≥ F_{i−1} (1 − 1/(2*i*)), and the
   product of (2*i* − 1)/(2*i*) for *i* = 1..*k* is C(2*k*, *k*) / 4^*k*: at least F₀ · C(2*k*, *k*) / 4^*k*
   (> F₀ / √(π(*k* + ½))) of the room F₀ free before them stays chargeable by another writer while they hold their
   leases (the 1 MiB cap only lowers grants). Grants made earlier, when more was free, can hold more: in general the
   room open writers hold is at most the sum, over their leases, of min(1 MiB, F / (2(*A* + 1))) at each grant. A carry
   refused as held records a mark-only final mark with `tenant_quota` (the tenant is at its limit but for room in flight,
   G041-023); idle leases give back all their room at the next refill near the limit.
5. **A release** (the purge's `DELETE`, which keeps ADR-0027's per-row delta and deferred flush) MUST NOT wait for a
   lease: in the finalizer's step-log branch, under the usage row, it reads Σ `used`, refuses with
   `step_log_usage_underflow` (counted) when S − released + Σ `used` < 0 (I3), and otherwise sets S −= released. No
   charge, refill or release waits for a lease.
5b. **Idle leases.** The purge pass folds and deletes, one tenant a transaction under the usage row, every lease not
   charged for longer than the idle interval (design 041 section 2b), `SKIP LOCKED`; the rows of a tenant are then at most
   the backends that charged for it within that interval.
6. **A change of the limit.** An `ENABLE ALWAYS` `SECURITY DEFINER` trigger on `quota_limits` and `quota_ceilings`, for
   `tenant_step_log_bytes`, takes the step-log fence exclusively itself (at once when the editor already holds it; so no
   transaction holds a lease), then the tenant's leases `NOWAIT`, then the usage row, and reclaims every lease (S += Σ
   `used`, G = 0), whoever wrote the row. A lowered limit therefore stops growth at once (I5) and leaves no stranded room; a charge that still meets
   S + G > L goes to the refill, which reclaims, and is refused only when what it needs is not there.
7. **Reads.** The usage of `tenant_step_log_bytes` that the quota API, `sagat_quota_step_log_allowance` and every
   reconciliation read MUST be S + Σ `used`, in one snapshot, through a `SECURITY DEFINER` accessor. With
   `step_log_enforcement_enabled` off, a refill grants without the limit check and I4/I5 do not apply; the usage stays
   exact.
8. **Telemetry**, closed labels, no tenant: `step_log_quota_refills_total{kind="refill"|"reclaim"|"idle"|"ensure"}`,
   `step_log_quota_refusals_total{cause="limit"|"held"|"busy"}` (`held` when the room is in leases open transactions hold,
   raised under its own constraint name `tenant_step_log_quota_held`; `busy` for a fallback refill past *b*),
   `step_log_quota_refill_budget_exceeded_total{caller="carry"|"ingest"}` (counted by the store from the carry's
   returned cause and from a batch's mapped `55P03`; the alert `SagatStepLogRefillBudgetExceeded` fires on any increase
   for `caller="carry"` over 5 minutes, an ingest's being its ordinary backpressure), `step_log_purge_failures_total`,
   and a gauge of lease rows. Events counted inside SQL MUST NOT update a counter row (a hot row again): they advance
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
- A report that carries pays one more short transaction on its connection, the ensure, when its writer's lease is short
  of the planned charge: once per grant, and for the first carry of each (tenant, backend), as after a deploy or a pool
  recycle. That transaction waits for the usage row under no run lock.
- The fallback refill under the run lock waits at most *b* = 200 ms; sibling reports, heartbeats, cancels and signals of
  the same run queue behind it for that long at most. Past *b* the carry's records are dropped with the reason
  `tenant_quota_busy`, shown to the reader and alerted: a bounded, visible loss in a rare path, traded for a report that
  always lands (ADR-0084 decision 7). Near the limit, where reclaims take room between the ensure and the report,
  fallbacks come more often; design 041's T041-9 measures that regime, a cold start and a small quota with many backends.
- A carry that needs the fallback opens one subtransaction (its block, ADR-0095 decision 5): a second transaction id for
  that report; the refill opens none. Design 041's T041-9 reports the XID rate and `pg_stat_slru` for `subtransaction`
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
- D041-117, D041-118 and D041-119 (research.md), and the reviews of the writer leases
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
