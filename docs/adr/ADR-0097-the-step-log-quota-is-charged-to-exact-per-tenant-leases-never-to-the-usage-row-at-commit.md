# ADR-0097: The step-log quota is charged to exact per-tenant leases, never to the usage row at commit

**Status:** Proposed
**Decided by:** agent under the owner's delegation OD-2026-11-20-6 ("decide for advantage"), design 041's D041-108 and
D041-112; adopted by the coordinating session on 2026-11-20. Acceptance (D041-113): the coordinating session accepts it under the
same delegation once design 041's re-review has no Blocker or Major and T041-35's third run is recorded, whatever that run's
result (this decision rests on D041-106's ceiling, not on the 1 × 16 gate), together with the restated O041-040; ADR-0095
(O041-038) is accepted the same way only with a pass. README rule 4 ("a person accepts") is met by the owner's delegation
for 0.3.0 (OD-2026-11-20-6), as the coordinating session decided
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
them; a report must never wait under its run lock for another tenant's, replica's or attempt's commit.

## Decision

1. **Leases.** A table `step_log_quota_leases (tenant_id, lease smallint, granted bigint, used bigint)` holds at most 256
   leases per tenant (a foreign key to the tenant's usage row, `ON DELETE CASCADE`; fill factor 50);
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
   quota is not enforced), add its bytes to the `used` of one lease with room, picked by an uncorrelated scalar subquery
   (run once) `… ORDER BY (lease - pg_backend_pid() % 256 + 256) % 256 LIMIT 1 FOR UPDATE SKIP LOCKED`, so that a lease
   another open transaction holds is skipped and one this transaction holds is found. It MUST NOT lock
   `tenant_retained_usage` and MUST NOT wait for a lease. Otherwise it calls the refill. A carry first reserves a lease
   with room for its planned charge (`sagat_quota_step_log_reserve`: the step-log fence tried shared, the lease locked `SKIP
   LOCKED`, nothing written; false for a non-positive amount), so that its charge cannot be refused and needs no
   subtransaction (ADR-0095 decision 5). The planned charge MUST bound what the trigger will measure: every sent record's
   `sagat_quota_measure` sizes plus 128 a row, plus 128 for a possible marker, computed before CC for either stream case.
4. **A refill** locks the usage row (the one wait a charge can have; `NOWAIT` when the transaction set
   `sagat.step_log_charge_nowait`, as a carry does, ADR-0095) before it would ever insert it: an `INSERT … ON CONFLICT`
   first would wait for a holder that updated the row, whatever `NOWAIT` says. The setting is set transaction-locally by
   the carry; it can only turn that wait into a refusal, never admit more (ADR-0027's rule on session flags). It reads L
   itself. When the free room L − S − G is under
   max(*n*, 1 MiB), *n* the charge, it first reclaims every lease no open transaction holds (`SKIP LOCKED`: S += `used`,
   G −= `granted`, both set to 0). It then takes the free lease with the least room (or the lowest unused number below
   256), folds it (S += `used`, G −= `used`, `granted` −= `used`, `used` = 0) and grants it
   g = max(*n* − room, min(1 MiB, ⌊(L − S − G) / 64⌋), 0), raising when g > L − S − G (I4): `tenant_step_log_quota_held`
   when the room missing is in leases it could not lock, `tenant_step_log_quota_exceeded` otherwise; else it charges *n*
   there. When all 256 leases are held by open transactions it charges S directly under the row,
   exactly. After the usage row a refill takes no lock it waits for.
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
8. **Telemetry**, closed labels, no tenant: `step_log_quota_refills_total{kind="refill"|"reclaim"|"direct"}`,
   `step_log_quota_refusals_total{cause="limit"|"held"}` (the second when the room is in leases open transactions hold,
   raised under its own constraint name `tenant_step_log_quota_held`), and a gauge of lease rows. Events counted inside
   SQL MUST NOT update a counter row (a hot row again): they advance sequences, read as deltas with the lease-row count
   by a definer accessor that the purge pass calls; the carry returns its refusal cause.

## Considered options

1. **Exact leases** (chosen): a tenant's charges spread over up to 256 rows that are only tried, so no replica or
   connection of one tenant queues behind another's commit for the step-log class; exact by I1 to I5. The usage row is
   locked by a refill (once per lease of up to 1 MiB, about 300 ten-line carries, while at least 64 MiB is free), a
   release and a limit change.
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
   block for the carries that need a refill (about one in 300 far from the limit).

## Consequences

- A tenant's step-log charges no longer pass one row one at a time; a report that also stores output still takes the
  usage row at its commit for the content class (ADR-0027 as built; design 041's G041-024, outside this ADR).
- A charge can be refused while room exists only in leases that open transactions hold: at most the room of those
  leases (each grant at most 1 MiB, or *n* − room for a larger charge), and only until those transactions end. It is
  counted (`cause="held"`), never an overshoot (design 041's G041-023).
- Near the limit (free room under 64 MiB) grants shrink to 1/64 of the free room and refills come more often; at the
  limit about one refill per charge, each holding the usage row to the end of its commit. A carry's refill never waits
  for it (`NOWAIT`, decision 4); an ingest's waits under its 200 ms lock timeout. Design 041's T041-9 measures this regime.
- A carry that needs a refill opens one subtransaction (its block, ADR-0095 decision 5): a second transaction id for that
  report. Far from the limit that is about one carry in 300; at the limit, most of them. Design 041's T041-9 reports the
  XID rate and `pg_stat_slru` for `subtransaction` there.
- Every step-log charge holds the content and log fences shared (ADR-0027's order against an editor that takes several
  fences), so a change of `tenant_content_bytes` or `tenant_log_bytes` waits for the charges in flight and, while queued,
  makes carries `contended`.
- S may be negative while bytes charged in leases are released; U is what is shown and checked.
- Up to 256 lease rows per tenant that logs, four columns each, updated by every charge (fill factor 50, per-table
  autovacuum settings by T041-5). A tenant that stops logging keeps its leases' room until a refill, a limit change or
  the down migration reclaims it; the usage it shows is exact meanwhile.
- One more table, two more columns' worth of rules, the charge and refill functions, a usage accessor, a branch of the
  finalizer for the release and one for a limit change. The down migration refuses while a step-log line exists, and
  otherwise folds Σ `used` into S (U = 0, so S = 0), then drops the leases and the column.
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
- `step_log_spike::install_leases` and `step_log_leases_tests.rs`: a held lease is skipped and never waited for; a charge
  into a free lease never takes the usage row and a refill alone waits for it; concurrent charges (32 tasks on a pool of
  8 connections) never pass a 200,000-byte limit and every byte charged is counted once; the last bytes of a 2 MiB limit
  are admitted from the room of idle leases and one byte more is refused; a lowered limit stops charges, and one lowered
  above the usage still admits what fits; a release never waits for a held lease and never takes the usage below zero;
  the budget line of a charged insert. Hand mutations, each failing its test, are recorded in the validation report.
- [design 041](../designs/041-step-logs.md), [research.md](../../specs/041-step-logs/research.md) D041-106, D041-108,
  D041-112.
