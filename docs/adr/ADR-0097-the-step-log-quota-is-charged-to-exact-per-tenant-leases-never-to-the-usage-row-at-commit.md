# ADR-0097: The step-log quota is charged to exact per-tenant leases, never to the usage row at commit

**Status:** Proposed
**Decided by:** agent under the owner's delegation OD-2026-11-20-6 ("decide for advantage"), design 041's D041-108, adopted
by the coordinator on 2026-11-20; accepted under the same delegation once design 041's re-review closes
**Refines / supersedes:** would supersede, when accepted, the charge half of item 3 of the Amendment of 2026-11-15 of
[ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) for the class `step_log` (a charge goes to a lease, not
to `quota_transaction_deltas.step_log_delta` and the finalizer's step-log branch) and the first two bullets of its
"Proposed amendment of the Amendment (2026-11-20)" (O041-040), whose statement-level measurement it keeps and whose
deferred flush it removes; refines ADR-0027's "Exact byte accounting and transactions" for the class `step_log` only. The
content and log classes, their deltas and their finalizer are unchanged. ADR-0095's carry is unchanged except that its
rows are charged to a lease.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): the invariant tests of decision 2 and the held-lock tests of decisions 3 to 5
(held transactions and `pg_stat_activity`, never sleeps), the property test of decision 2 over random interleavings of
charges, refills, reclaims, releases and limit changes, the runtime-role bypass tests of ADR-0027 extended to the lease
table and the functions, and a source-text check that only `sagat_quota_step_log_refill` and the release branch of the
finalizer lock `tenant_retained_usage` for the step-log class (T041-5); the lock-order test with a lease slot (T041-6,
T041-10, T041-36); the budget equalities of design 041 sections 2 and 2a with the lease charge's nested statements
(T041-6, T041-36). The prototype `step_log_spike::install_leases` is proved by
`tests/performance/scalability-ha/step_log_leases_tests.rs` (five tests, six hand mutations each failing one).

## Context

ADR-0027 charges a retained byte in the same transaction as the row that holds it: immediate triggers add the byte
count to the transaction's private row of `quota_transaction_deltas`, and a deferred finalizer, at commit, takes the
class's policy fences, then `SELECT … FOR UPDATE` of the tenant's one `tenant_retained_usage` row, checks the limit and
updates the row. PostgreSQL releases a row lock only when the transaction ends, after its commit record is flushed. So
every commit of one tenant that charges any class holds that row from its finalizer to the end of its WAL flush, and the
tenant's charging commits pass it one at a time, whatever the number of replicas and connections.

Design 041 makes far more commits charge: every report that carries a step-log record and every batch of a long step,
outputs of `{}` included. Measured on the hosted runners (run 44730444900, `specs/041-step-logs/validation-report.md`,
D041-106), at 1 replica and 16 workers: the row is held from the lock to the `COMMIT`'s return p50 1.1 ms and p99 4.0 to
4.5 ms; it is busy 23% to 24% of the time when every completion charges once and 68% when every step charges twice (a
batch and a report). That puts one tenant's ceiling at roughly 800 charging commits a second on that machine, half that
for long steps, independent of how many replicas serve it. The same run showed that this queue is not what failed
T041-35 at 1 × 16 (moving or removing the step-log charge changed the rate by at most 2%): the ceiling is real, it was
simply not reached there.

The forces: the owner requires the quota to be exact (OD-2026-11-20-6: an overshoot of a ceiling is a calculation error
unless bounded, proved and documented); ADR-0027 requires the charge in the same transaction as the rows, measured by the
database, with no runtime path to skip it; a step log is lossy by ADR-0084, so a refused charge drops records and counts
them; a report must never wait for another tenant's, another replica's or another attempt's commit beyond what it waits
for today.

## Decision

1. **Leases.** A table `step_log_quota_leases (tenant_id, lease smallint, granted bigint, used bigint)` holds at most 256
   leases per tenant; `tenant_retained_usage` gains `step_log_leased_bytes` (G). With S its `step_log_bytes` (the folded
   usage), the tenant's step-log usage is U = S + Σ `used` and its leased room is R = Σ (`granted` − `used`). The runtime
   role MUST have no privilege on the lease table nor on the two columns' writes; only `SECURITY DEFINER` functions of
   ADR-0027's pattern (fixed `search_path`, trusted owner, `REVOKE ALL … FROM PUBLIC`) write them.
2. **Invariants.** (I1) 0 ≤ `used` ≤ `granted` on every lease (a `CHECK`); (I2) G = Σ `granted` of the tenant's leases,
   changed only while the usage row is locked; (I3) S ≥ 0 and G ≥ 0 (`CHECK`s); (I4) a grant commits only if
   S + G ≤ L after it, L the tenant's limit read after the step-log policy fence; (I5) a charge commits only if S + G ≤ L
   for the L it read after that fence. A charge moves bytes from R to U and changes neither S nor G, so U + R = S + G at
   every commit and **U ≤ S + G ≤ L: the usage never exceeds the limit in force when the charge was admitted**, the
   exactness ADR-0027 gives today, with no overshoot to bound.
3. **A charge** is made by the statement trigger of the step-log rows, once per insert statement and tenant, with the
   bytes `sagat_quota_measure` measures from the rows stored, never a caller's number. It MUST take the content, log and
   step-log policy fences shared, in ADR-0027's order; read L and, without a lock, S and G (I5); then add its bytes to the
   `used` of one lease with room, chosen `FOR UPDATE SKIP LOCKED LIMIT 1` in an order that starts at the backend's own
   slot (`pg_backend_pid()` modulo 256). It MUST NOT lock `tenant_retained_usage` and MUST NOT wait for a lease.
4. **A refill**, only when no free lease has room, locks the usage row (the one wait a charge can have). When the free
   room L − S − G is under max(n, 1 MiB), n the charge, it first reclaims every lease no open transaction holds
   (`SKIP LOCKED`: S += `used`, G −= `granted`, both set to 0). It then takes the free lease with the least room (or a new
   one below 256), folds it (S += `used`, G −= `used`, `granted` −= `used`, `used` = 0) and grants it
   g = max(n − room, min(1 MiB, ⌊(L − S − G) / 64⌋), 0), refusing with `tenant_step_log_quota_exceeded` when g > L − S − G
   (I4); else it charges n there. When all 256 leases are held by open transactions it charges S directly under the row,
   exactly. No transaction ever waits for a lease, so no lock cycle can contain one.
5. **A release** (the purge's and the delete route's `DELETE`, which keep ADR-0027's per-row delta and deferred flush)
   folds, in the finalizer's step-log branch under the usage row, every lease of the tenant, waiting for each in lease
   order, then applies its negative delta to S. Only a release waits for a lease; a lease holder never waits for what a
   release holds before its leases (the step-log fence is shared on both sides, the usage row comes after). Every
   committed charge is then in S, so S stays ≥ 0 and the release equals what was deleted.
6. **Reads.** The usage of `tenant_step_log_bytes` that the quota API, `sagat_quota_step_log_allowance` and the carry's
   all-or-nothing allowance read is S + Σ `used`, in one snapshot. A lowered limit stops every charge at once (I5) and every
   grant (I4); leases are not revoked, as ADR-0027 keeps committed reservations, and the next refill or release reclaims
   their room.

## Considered options

1. **Exact leases** (chosen): a tenant's charges spread over up to 256 rows that are only tried, so no replica or
   connection of one tenant queues behind another's commit for the step-log class; exact by I1 to I5. The usage row is
   locked once per lease of up to 1 MiB (about 300 ten-line carries) instead of once per charging commit.
2. **The charge folded at commit by a deferred trigger** (ADR-0027 as built; T041-35's second run): the lock is already
   taken only at commit, and a row lock lasts to the end of the commit, its WAL flush included; no trigger releases it
   earlier. It keeps the ceiling of the Context.
3. **A step-log usage row of its own** (O041-026 (b)): moves the queue to another row, does not remove it; every carrying
   report would still take it at commit.
4. **Striped counters**: with per-stripe ceilings L/K exact, but a tenant whose writers are uneven is refused while other
   stripes have room, by up to (K − 1)/K of its quota; with one ceiling read without a lock, an overshoot of up to the
   charges in flight (the pool size times a 24 KiB carry), which the owner's rule forbids.
5. **A ledger folded later** (each charge appends a row, a job folds them): an admission that reads the folded usage
   overshoots by the unfolded charges; exact only if the admission locks, which is option 2.
6. **Charging after the report commits** (a second transaction): breaks ADR-0027's rule that the charge and the rows
   commit together; a crash between them leaves rows uncharged.

## Consequences

- A tenant's step-log charges no longer pass one row one at a time; the usage row is locked by a refill (once per lease),
  a release (the purge, off the hot path) and, as before, by the content and log classes' finalizer. A report that also
  stores output still queues on the row for the content class: ADR-0027 as built, which off-1K measures; extending leases
  to that class is a follow-up gap of design 041, not part of this decision.
- A charge can be refused while room exists only in leases that open transactions hold: at most the room of those leases
  at that instant, and only until those transactions end. Near the limit a grant is at most 1/64 of the free room, so that
  margin shrinks with it. It is a counted drop (`tenant_quota`), never an overshoot.
- Up to 256 lease rows per tenant that logs, four columns each. A tenant that stops logging keeps its leases' room until a
  refill or a release reclaims it; the usage it shows is exact meanwhile.
- One more table and two more columns in the quota schema, three functions, and a branch of the finalizer for the
  release. The down migration refuses while a lease holds room, as the line table's down refuses rows.
- Runs in flight are unaffected: nothing is charged under this rule before the migration that adds it, and step logs do
  not exist before design 041.

## Evidence

- Run 44730444900 (validation-report.md, 2026-11-20, "diagnosed"; D041-106): what a charging `COMMIT` waits on (the
  usage row, nothing else), how long the row is held (p50 1.1 ms, p99 4.0 to 4.5 ms to the `COMMIT`'s return; gaps
  between back-to-back locks p50 0.69 to 0.80 ms, p99 3.1 to 4.1 ms), how busy it is (23% to 24% with one charge a step,
  68% with two), and that a charge on per-tenant rows taken `SKIP LOCKED` costs the same as no charge at all (the
  completion rate within 1% to 2% paired, its p99 within about 1 ms).
- `step_log_spike::install_leases` and `step_log_leases_tests.rs`: a held lease is skipped and never waited for; a charge
  into a free lease never takes the usage row and a refill alone waits for it; 32 concurrent writers never pass a
  200,000-byte limit and every byte charged is counted once; the last bytes of a 2 MiB limit are admitted from the room of
  idle leases and one byte more is refused; a lowered limit stops charges into leases with room. Each of six hand
  mutations (no `SKIP LOCKED`, the charge locking the row, no lowered-limit check, a grant past the free room, no reclaim,
  no fold) fails one of them.
- [design 041](../designs/041-step-logs.md), [research.md](../../specs/041-step-logs/research.md) D041-106 and D041-108.
