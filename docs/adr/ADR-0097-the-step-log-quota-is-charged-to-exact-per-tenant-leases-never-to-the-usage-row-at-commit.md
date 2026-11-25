# ADR-0097: The step-log quota is charged to exact per-tenant leases, never to the usage row at commit

**Status:** Accepted (2026-11-20, OD-2026-11-20-15)
**Decided by:** owner, on the proposal of the agent under the owner's delegation OD-2026-11-20-6 ("decide for advantage"), design 041's D041-108,
D041-112, D041-117 (the lease owned by its writer), D041-119 (room made before the report, a fallback refill
bounded by a budget), D041-120 (the bounded ensure, the ended-backend reclaim) and D041-121 (the second re-reads'
answers); adopted by the coordinating session on 2026-11-20, and accepted by the owner on 2026-11-20 ("Accept both", with
ADR-0095; OD-2026-11-20-15, question O041-042), with its residual gaps G041-026 (the busy loss past the budget, accepted as
built: "Accept the visible loss", OD-2026-11-20-14) and G041-027 (behind a transaction pooler). Amended once since its
acceptance, by the owner on 2026-11-20 ("Top up after the commit", OD-2026-11-20-18, question O041-043): decision 3b's
ensure before the report is replaced by a top-up after the report commits (design 041's decision D041-124), and the
wait slots of decision 4 bound a tenant's waits on its usage row (D041-125, under OD-2026-11-20-6). Amended again
during the approval run of design 041's revision, under OD-2026-11-20-6: decisions 1, 3b and 4 by D041-134 (the
replica's wait slots beside the tenant's; a top-up that never waits, takes no slot and reclaims nothing) and decisions
3b, 4 and 8 by D041-137 (the replica's *Q* passed per transaction, the usage row tried `SKIP LOCKED`, the pooler's
sizing rule, the top-up's answers without `held`). Under rules 2 and 4 of `README.md` these delegate amendments take
effect with the owner's approval of that revision on the owner's path (OD-2026-11-20-17), which the design's approval
block records. The owner confirmed both on 2026-11-21 ("Confirm the amendments", OD-2026-11-21-9). Amended on
2026-11-24 by the building agent under OD-2026-11-24-4 (delegate), decision 1 by design 041's D041-143: the lease table
has no foreign key to the usage row, since PostgreSQL checks a foreign key on an update that keeps the key when the row
version was written by the updating transaction, so a second charge in one transaction took `FOR KEY SHARE` on the usage
row and waited, with no timeout, for a refill holding it; the cascade on a tenant's deletion is a delete trigger of the
usage row instead. Every passage a change touches cites its row.
**Refines / supersedes:** supersedes:
- for the class `step_log`, the charge half of item 3 of the Amendment of 2026-11-15 of
  [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (a charge goes to a lease, not to
  `quota_transaction_deltas.step_log_delta` and the finalizer at commit) and the deferred flush on insert of its proposed amendment of 2026-11-20 (O041-040), which its Amendment of the
  Amendment as accepted on 2026-11-20 (OD-2026-11-20-16) replaces by this ADR, keeping the statement-level measurement and
  the per-row release; and, for that class, ADR-0027's rule that the usage counters never go below zero (S may, U may not);
- in [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) (Revision), the clause of
  decision 3 that a batch "MUST NOT take the tenant's `tenant_retained_usage` row before its commit" and refuses "by the
  commit-time finalizer's raise" (a batch now charges a lease at its insert, takes the usage row only for a refill, and
  is refused at its insert), and the ingest's reading of decision 11 (the class fences at the insert, a lease, the usage
  row on a refill);
- in [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) decision 7 ("MUST NOT
  fail, block, retry or delay a step beyond one bounded final flush"): a report's commit may also wait for the fallback
  refill of decision 4 (each lock acquisition at most 200 ms, and only while holding a wait slot), a bounded delay that
  neither fails, blocks or retries a step nor makes its report late (OD-2026-11-20-18);
- in [ADR-0095](ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md), decision 4 (a carry may delay
  a report's commit by the fallback's budget, after the report's fence checks, so never making it late) and decision 5
  (the carry's block also answers a fallback past its budget or with no wait slot free, in the same call), revised with
  this ADR. ADR-0095 decision 2 (a carrying report adds no transaction) holds: the top-up of decision 3b runs after the
  report commits and is the store's, not the report's (OD-2026-11-20-18).

ADR-0027 and ADR-0084 are marked "partly superseded by ADR-0097". The content and log classes, their deltas
and their finalizer are unchanged.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the invariant tests of decision 2, the held-lock tests of decisions 3 to 6
(held transactions and `pg_stat_activity`, never sleeps) and a property test over random interleavings of charges,
refills, reclaims, releases and limit changes, the runtime-role bypass tests of ADR-0027 extended to the lease table, the
new column and the functions, and a source-text check that only the top-up, the refill, the release branch, the idle
reclaim and the limit change lock `tenant_retained_usage` for the class `step_log` (T041-5); the wait-slot test with a
second tenant as the victim (T041-36); the lock-order test with slots for L3 and the lease (T041-6,
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
decision 4 to one counted wait of the fallback refill, each lock acquisition at most *b* = 200 ms and only with a wait
slot, measured at 0.31% of ten-line carries far from the limit; the content class's finalizer already
waits at commit, G041-024); and a step-log refusal must never fail, block or retry a step (ADR-0084 decision 7,
ADR-0095 decision 4).

## Decision

1. **Leases, one a writer.** A table `step_log_quota_leases (tenant_id, writer integer, granted bigint, used bigint,
   charged_at timestamptz)` holds one lease per tenant and server backend that charges for it, keyed by
   `pg_backend_pid()` (no foreign key: the usage row's delete trigger removes a tenant's leases, decision D041-143; fill
   factor 50). A backend runs one
   transaction at a time, so a writer's lease is never locked by another writer, behind a transaction pooler too (the
   key is the server backend, not the client connection; the top-up of decision 3b reaches the carry's own backend only
   with session affinity, and is correct without it); a
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
   execute exactly six definer functions, `sagat_quota_step_log_reserve`, `sagat_quota_step_log_top_up`
   (OD-2026-11-20-18), `sagat_quota_step_log_usage`, `sagat_quota_step_log_counters`,
   `sagat_quota_step_log_idle_tenants` and `sagat_quota_step_log_reclaim_idle`, and two invoker helpers that read or
   write nothing, `sagat_quota_step_log_top_up_mark()` (`IMMUTABLE`) and `sagat_quota_step_log_wait_slots()`
   (`STABLE`, design 041's decision D041-134, confirmed by OD-2026-11-21-9); the
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
3b. **Room topped up after the report** (amended by the owner, "Top up after the commit", OD-2026-11-20-18; design
   041's decision D041-124; it replaces the ensure before the report of decisions D041-119 to D041-121, which needed the
   server backend of a pooled connection, which sqlx 0.9 does not expose without a statement). No transaction runs
   before a carrying report. The carry's function returns the room left in its writer's lease after its charge,
   measured by the database, and `top_up`, true when that room is under the top-up mark *M* = 65,536 bytes (null room and
   false when it charged nothing). After a carrying report commits with `top_up` true, the store MUST run, on the same
   connection and off the run lock, one statement in its own implicit transaction:
   `sagat_quota_step_log_top_up(tenant)`, with the principal's tenant (never one from the request or the run); it MUST
   NOT delay the report's answer for it, and it counts the answer (`step_log_quota_top_ups_total{outcome}`) and never
   fails or re-runs anything because of it. The function takes no caller amount: it tries the step-log fence shared
   (held: `contended`); answers `room` when the writer's lease has at least *M*; tries the usage row `SKIP LOCKED`
   only, opening no subtransaction, taking no wait slot and never waiting (held: `busy` at once); reclaims no other
   lease; answers `limit` when the share, min(1 MiB, ⌊F / (2(*A* + 1))⌋) (decision 4), is 0; and grants only when the
   share is above the writer's current room, folding the writer's lease and granting exactly that share with no charge,
   answering `made`, and otherwise `room` (design 041's decisions D041-134 and D041-137, confirmed by OD-2026-11-21-9, so that a top-up never takes
   another writer's room or a wait slot; it has no `held` answer, since it reclaims nothing). It never raises for these; any other error is
   rolled back and is the store's `error`. *M* is above the reservation of any carry whose stored text is no longer than
   its 16 KiB encoding (16,384 + 65 × 128 = 24,704 bytes), so the next such carry after a top-up takes the fast path; far
   from the limit a backend tops up once per 1,048,576 − 65,536 = 983,040 bytes charged. The store holds no rule, no
   constant and no state for this: `top_up` is the database's answer. A refill inside a report (the fallback of decision
   4) remains for the first carry of each (tenant, backend), for a carry after a batch that left its backend's lease
   short, for a carry whose room a reclaim took (near the limit, the idle reclaim, a limit change), and, behind a pooler
   in transaction mode, for a carry on a backend no top-up reached (the top-up can run on another backend there, which
   it tops up if short: correct, and the fallback rate is then about task T041-35's fourth run's, which had every refill
   in a report; design 041's gap G041-027, accepted with this ADR). The top-up runs only after a committed report, so
   after its authorization and fence checks.
4. **A refill** (the writer's lease short of the charge) locks the usage row before it would ever insert it, then the
   writer's own lease `NOWAIT` (only a holder of the usage row, or a limit change holding the step-log fence exclusively,
   can hold it, so this never fails short of a prepared transaction): after the usage row nothing waits. A refill first
   tries the usage row without waiting (`SKIP LOCKED`, then an unlocked check that it exists, so no exception block is
   needed); when it is held or absent, it waits only while holding two *wait slots*, one of the tenant's *P*
   on its replica and then one of the replica's *Q*, which all tenants share (single 64-bit advisory keys the function
   builds with `hashtextextended` from the tenant, the replica's `application_name` and the slot, tried in order and
   never waited for; with none free it raises `55P03` at once; design 041's decisions D041-125 and D041-134, under
   OD-2026-11-20-6, confirmed by OD-2026-11-21-9). *Q* = max(1, ⌊N / 4⌋), N the replica's pool, reaches SQL per transaction through the
   transaction-local setting `sagat.step_log_wait_slots`, which the store sets in a statement it already sends
   (D041-137, confirmed by OD-2026-11-21-9), so it travels with its transaction through a pooler in transaction mode; *P* = ⌈*Q* / 2⌉: 5 and 3 at a
   pool of 20. So one tenant's waits on its usage row hold at most *P* of a replica's pool connections and all tenants'
   at most *Q*, and on a replica's own pool step-log waits and Logs calls hold at most the Logs cap ⌊N / 2⌋ plus *Q* of
   them; behind a pooler in transaction mode the server backends MUST number at least the sum over the replicas of
   ⌊N / 2⌋ + *Q*, plus one a replica, for that bound to hold there. Each lock
   acquisition on the usage row waits at most its budget *b* = 200 ms (`lock_timeout` set for that
   statement from a function no caller can change, a caller's lower `lock_timeout` kept, the caller's value restored
   after it), and the statement timeout (5 s, 3 s for some transactions) is the hard bound, past which the carry call
   fails and its report is re-run without the carry (ADR-0095 decision 5, the records counted `failed`); *b* is
   the budget an ingest batch already has on the same row, a latency budget, not a worst case: measured holds of the row
   are p50 1.1 ms and p99 4.5 ms (D041-106), and since `lock_timeout` applies to each lock acquisition a wait spanning
   several holders can exceed it. Past *b*, or with no wait slot free, inside a carry, the carry's block catches the
   `55P03` and records a mark-only final mark whose records are dropped as `service_busy` and whose stream's
   `not_stored_reason` says so (a reason of its own, not a ceiling; design 041's decision D041-129), shown to the reader
   by the streams route and Studio, counted and alerted (decision 8); the report lands. Inside a batch, it is the batch's
   `RETRY_LATER`, as before. The fallback refill is thereby the one wait a carry has under its run lock, measured far
   from the limit at 0.31% of ten-line carries with every refill in a report, each lock acquisition bounded by *b*, held
   only with a wait slot, and counted; it runs after the report's fence checks, so it delays the commit and never makes
   the report late; no step-log refusal fails, blocks or retries a step. The top-up (decision 3b) never waits: it tries
   the usage row without waiting, after the report. The wait cannot deadlock: no transaction waits for a lease it does not
   own (every other lease is taken `SKIP LOCKED`, or `NOWAIT` by a limit change holding the fence exclusively, and the
   writer's own lease `NOWAIT` after the usage row), and no holder of the usage row waits after taking it for anything a
   waiter holds (a content or log finalizer is the commit's last step; a refill then takes only its own lease `NOWAIT`
   and writes rows only its transaction touches; a release and a reclaim take nothing more). It reads L itself. When the
   free room L − S − G is under max(*n*, 1 MiB), *n* the charge, it first reclaims every other lease no open
   transaction holds (`SKIP LOCKED`: S += `used`, G −= `granted`, both set to 0). It then folds the writer's own lease
   whole (S += `used`, G −= `granted`; inserted when absent) and grants it g = max(*n*, min(1 MiB, ⌊F / (2(*A* + 1))⌋)),
   F = L − S − G and *A* the tenant's other leases holding a grant, raising when g > F (I4):
   `tenant_step_log_quota_held` when the room missing is in leases it could not lock, `tenant_step_log_quota_exceeded`
   otherwise; else it charges *n* there (the top-up, with no *n*, grants its share alone). **Room held by open writers,
   bounded.** A grant's unused room is at most its share, min(1 MiB, ⌊F / (2(*A* + 1))⌋) at the grant: the rest of a
   grant of *n* past the share is the planned charge itself, charged at once by the refill that grants it (a top-up
   grants its share and no more). So the room open writers hold is at most the sum, over their leases, of each grant's share,
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
8. **Telemetry**, closed labels, no tenant, one producer for each metric (the list of design 041's data-model §7,
   decision D041-128): `step_log_quota_fallbacks_total` (a carry that returned `path = refill`; the store),
   `step_log_quota_top_ups_total{outcome="room"|"made"|"limit"|"busy"|"contended"|"error"}` (the store),
   `step_log_quota_refusals_total{cause="limit"|"held"|"busy"}` (the store, from the carry's returned cause and from a
   batch's insert raise by its constraint name: `held` when the room is in leases open transactions hold, raised under its
   own constraint name `tenant_step_log_quota_held`; `busy` for a fallback refill past *b* or with no wait slot free; a
   batch's `55P03` is a lock-timeout retry, never `busy`; the alert `SagatStepLogServiceBusyDrops` fires when
   `cause="busy"` exceeds 0.1% of `step_log_carries_total` for ten minutes), `step_log_purge_failures_total`, and, from
   SQL, `step_log_quota_refills_total{kind="refill"|"reclaim"|"idle"|"ended"}` and a gauge of lease rows. Events counted
   inside SQL MUST NOT update a counter row (a hot row again): they advance sequences (which count attempts, rolled-back
   ones included), read as deltas with the lease-row count by a definer accessor that the replica holding shard 0 calls;
   the carry returns its refusal cause.

## Considered options

1. **Exact leases, one a writer, topped up after the report** (chosen, D041-117, D041-119): each server backend
   charges its own lease, so no writer queues behind another's commit or lock for the step-log class; exact by I1 to
   I5. The usage row is locked by a top-up after a report (once per grant a writer: far from the limit and with
   F ≥ 2(*A* + 1) MiB, a grant is 1 MiB, about 300 ten-line carries; a tenant with less volume per backend per 10
   minutes, or near its limit, refills more often), a fallback refill, a batch's or a marker's refill, a release, an idle
   reclaim and a limit change (OD-2026-11-20-18).
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
   made by a fallback inside that backend's first carrying report (decision 3b), and lease rows bounded by the backends active within the idle
   interval. Kept as the alternative if backend keys prove hard to observe (the lease-row gauge and the per-replica
   `application_name`, T041-5, are the criterion).
   *Restated by decision D041-121*: the deadlock argument above does not hold against 1c, since decision 4 takes the
   writer's own lease `NOWAIT` and a charge takes it `SKIP LOCKED`, so a shared slot would make the second backend fail
   fast (`55P03`, a busy answer), not wait. What 1c would gain: behind a transaction pooler a slot follows the client
   connection, so the top-up and the next report would use the same lease and gap G041-027 would not arise; and a slot
   survives pool recycles. What it costs: the key is the client's word, not the server's (a store defect, or two
   backends on one slot during a deploy's overlap, sends charges to a lease another backend uses, and every such
   overlap answers busy); a slot token on every charging call; a replica's lapse waits for the idle interval instead of
   the ended-backend reclaim; and a deploy still starts new slots, so the cold-start herd stays. Backend keys are kept;
   1c is the named alternative if task T041-37's pooler scenario shows carry fallbacks
   (`step_log_quota_fallbacks_total`) above half of all refills (top-ups answered `made` plus carry fallbacks), or
   carries answered busy above the 0.1% that reopens question O041-041 (decisions D041-121 and D041-130).
1d. **A refill that never waits (`NOWAIT`) and answers mark-only when the row is busy** (D041-112): loses a tenant's
   records under load for no quota reason; replaced by decision 3b and the bounded fallback of decision 4.
1e. **A background refiller per replica** keeping every lease above a low-water mark: would take the refill off the
   report's path too, but needs a component that finds tenants and backends to serve and competes with the writers for
   the usage row on its own cadence; the top-up of decision 3b does it on the writer's own connection after a report
   that left its lease short, with no new component.
1f. **A refill inside the report that waits longer and fails the report past its bound** (D041-117 as first written,
   with a bound derived from `max_connections`): a report failed and resent can exhaust a worker's retries or outlive its
   lease, changing a step's outcome against ADR-0084 decision 7 and ADR-0095 decision 4; and the bound was neither a
   worst case nor independent of an unrelated server setting. Replaced by decisions 3b and 4.
1g. **Protecting a fresh grant from the near-limit reclaim** (a `reserved_until` on the lease, so that another refill
   cannot take a top-up's room before the next carry reserves it): fewer fallbacks near the limit, but room that cannot
   be reclaimed for a while turns into held refusals there instead; not taken, and task T041-37 measures the fallbacks
   near the limit.
1h. **An ensure on every carrying report**, which keeps no room in the store: correct whatever the cache, but one more
   transaction on every carrying report; rejected for its cost.
1i. **An ensure before the report when the store's kept room was short, unknown or stale** (decision 3b as accepted on
   2026-11-20, D041-119 to D041-121): it needed the store to know the server backend of each pooled connection, which
   sqlx 0.9 does not expose without a statement of its own, and its wait before the report could make a report late;
   replaced by the top-up after the commit (OD-2026-11-20-18), which needs no backend identity and no state in the store.
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
   block for the carries that need a fallback refill (0.31% of ten-line carries far from the limit with every refill in
   a report, fewer with the top-up of decision 3b; the refill itself opens no block).

## Consequences

- A tenant's step-log charges no longer pass one row one at a time; a report that also stores output still takes the
  usage row at its commit for the content class (ADR-0027 as built; design 041's G041-024, outside this ADR).
- A charge can be refused while room exists only in leases that open transactions hold: at most the room of those
  leases (decision 4's bound), and only until those transactions end. It is counted (`cause="held"`), never an
  overshoot (design 041's G041-023).
- After a carrying report that left its lease under 64 KiB, the store runs one more statement on that connection, the
  top-up, off the run lock and after the report's answer: about once per 300 ten-line carries of a backend far from the
  limit, after nearly every carry near it. It holds that pool connection a little longer, never the run row, and never
  delays, fails or makes late a report (OD-2026-11-20-18).
- A fallback refill waits for the usage row under the run lock: each lock acquisition at most *b* = 200 ms, only with a
  wait slot, the statement timeout the hard bound; sibling reports, heartbeats, cancels and signals of the same run
  queue behind it meanwhile. Past the budget, or with no slot free, the carry's records are dropped as `service_busy`,
  shown to the reader ("not stored: the log service was busy (not your quota)") and alerted on its rate: a bounded,
  visible loss, traded for a report that always lands (ADR-0084 decision 7), accepted by the owner (design 041's gap
  G041-026, "Accept the visible loss", OD-2026-11-20-14). Fallbacks are measured at 0.31% of ten-line carries far from
  the limit with every refill in a report; they come more often near the limit, where reclaims take room between a
  top-up and the next carry, and behind a transaction pooler, where a top-up may serve another backend (gap G041-027).
  A report waits on the usage row at most once before its commit. Design 041's task T041-37 measures these regimes.
- One tenant's waits on its usage row hold at most *P* of a replica's pool connections and all tenants' at most *Q*
  (3 and 5 at a pool of 20; the wait slots); past them carries answer busy at once rather than hold more connections
  that task leases, heartbeats and other tenants' reports need. The replica's cap couples tenants through loss: while
  other tenants hold all *Q* slots, a carry of a tenant far from its limit whose own usage row is held at that moment
  answers busy at once instead of waiting up to *b* (counted and shown as any busy answer; design 041's task T041-37
  bounds it at 0.1% of that tenant's carries). Behind a pooler in transaction mode the bound needs the server-pool
  sizing of decision 4.
- A carry that needs the fallback opens one subtransaction (its block, ADR-0095 decision 5): a second transaction id for
  that report; the refill and a carry with nothing to charge (no record and no marker planned) open none. Design 041's T041-37 reports the transaction-id rate and `pg_stat_slru` for `subtransaction`
  near the limit and for many low-volume tenants.
- Every step-log charge holds the content and log fences shared (ADR-0027's order against an editor that takes several
  fences), so a change of `tenant_content_bytes` or `tenant_log_bytes` waits for the charges in flight and, while queued,
  makes carries `contended`.
- S may be negative while bytes charged in leases are released; U is what is shown and checked.
- One lease row per tenant and backend that charged for it within the idle interval, five columns each, updated by
  every charge (fill factor 50, per-table autovacuum settings by T041-5). A tenant that stops logging keeps its leases'
  room until the idle reclaim, a refill near the limit, a limit change or the down migration returns it; the usage it
  shows is exact meanwhile.
- One more table, two more columns' worth of rules, the charge, refill and top-up functions, a usage accessor, a branch
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
  another writer's lock; a charge with room never takes the usage row and a refill alone waits for it; the ensure (the
  top-up's predecessor, the same function with a caller amount) gives a carry its fast path and, with room, takes no
  lock; a carry whose refill meets a held usage row within its
  budget waits and writes; past the budget it answers busy (the prototype's `tenant_quota_busy`, `service_busy` since decision D041-129) and the report commits, whatever the caller's
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
