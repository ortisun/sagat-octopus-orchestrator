# Design critique: 041 step logs, revision of 2026-11-20 (leases and stream-row cut)

**Reviewer:** egm-design (Design Critic Goldfish), mode REVIEW, read-only, on `4d74b27a`. Saved verbatim.
**Target:** `/Users/marcelo/Documents/Projects/sagat-wt-041/docs/designs/041-step-logs.md` (Draft), the 2026-11-20 revision.
**Context read:**
- `docs/adr/ADR-0097-*.md`
- the ADR-0027 Amendment and its Note
- `specs/041-step-logs/{validation-report.md (2026-11-20 sections), research.md (D041-105 to D041-111, G041-023/024), data-model.md §2, §3, §3b, §4, §8a}`
- the documents diff
- `tests/performance/scalability-ha/step_log_spike.rs` (`install_leases`, `install_third_run_storage`)
- `step_log_leases_tests.rs`

**Standard:** the caller supplied no separate Target Standard. The review is judged against the task's questions and against the design's own success criteria, Constitution Check and Complexity Tracking.

## Assessment of the framing

- **The hot-row hypothesis is correctly retired for the 1 × 16 failure.** The paired blocks show:
  - (f)/`f-none` 0.982, p99 +1.1 ms
  - (f)/`f-slot` 0.993
  - (a)/`a-slot` 1.008, p99 +0.1 ms

  (validation-report.md:502-504.) D041-106 states the retirement honestly.
- **The usage row is a real ceiling at scale.** The `-cold` rows show 1,075 ingest `COMMIT`s timing out on it at 8 × 128 (validation-report.md:559). That is the strongest case for the leases. ADR-0097 does not cite it (see M3).
- **The core exactness argument is sound for charges and grants.** U ≤ S + G ≤ L holds. An unlocked, stale S + G read is conservative, because S + G only falls outside a limit change, and the limit change is fenced.
- **The defects are at the edges:** a lowered limit, the release path, and the at-limit regime. Those are also the places where a simpler exact design is available.
- **The storage cut is not shown to recover the failed gates** (B2).

---

## Blocker

### B1. A lowered limit below S + G stops a tenant that is under its limit, possibly for days. During that time every carrying report runs twice and batches loop.

**Evidence**
- data-model.md:233: "`S + G > L` raises `tenant_step_log_quota_exceeded`". The raise happens before any lease or refill is tried; step_log_spike.rs:346-348 does the same.
- ADR-0097:79-80: "leases are not revoked, as ADR-0027 keeps committed reservations, and the next refill or release reclaims their room".
- data-model.md:245-246: the release fold is "S += `used`, G −= `used`, `granted` −= `used`, `used` = 0". That leaves S + G unchanged.
- SC-041-001 (design:317-320): "a charge is refused only when the room left is in leases that open transactions hold (G041-023) or not there at all".
- design:831-834: on a refused batch, "its T6 reads the tenant over the limit and counts the records as `tenant_quota`".
- step_log_leases_tests.rs:229-252 only asserts that a charge is refused after the limit is lowered.

**Why**

Take S = 260 MiB, Σused = 40 MiB (so U = 300 MiB) and G = 100 MiB. The administrator lowers L from 1 GiB to 330 MiB, which is above U.

- S + G = 360 MiB > 330 MiB, so every charge raises.
- No refill runs, so no lease is reclaimed. Refill is the only place where reclaim happens.
- Releases leave S + G unchanged apart from the bytes they delete. The tenant therefore stays refused until the purge has deleted 30 MiB, which can take days of retention, even though 30 MiB of real room exists.

Meanwhile the unlocked allowance (T6 of a batch, CB of a carry) reads U = 300 MiB < L, so it lets every write through:
- Each carrying report fails at CD and pays a downgrade re-run, which doubles its transactions.
- Each batch gets `RETRY_LATER`; its resend passes T6 again and is refused again until the SDK gives up after 10 s.

This contradicts SC-041-001 and G041-023, and nothing tests it. The analogy in ADR-0097 with "committed reservations" is wrong: lease room is not usage.

A smaller version of the same mismatch (T6 reads U, while the charge refuses on room held by open transactions) also applies to G041-023 refusals.

**Fix**
- Make a change of `tenant_step_log_bytes` (and of its ceiling) revoke every lease in the same transaction. That transaction holds the step-log fence exclusively, so no charge can hold a lease. Under the usage row it sets S += Σused, G = 0 and zeroes each lease. This is exact and adds no wait.
- Alternatively, or as well, send a charge that finds S + G > L to the refill path (which reclaims idle leases) instead of raising.
- Make T6 and CB read the same quantity the charge enforces, or state that they differ.
- Add a test: lower L to U plus a small margin, then assert that the next charge is admitted, and that a carry is not re-run.

### B2. The revision does not show that the stream-row cut can recover the failed gates. By its own breakdown it likely cannot recover (2b) or (4), and it does not say what happens if the third run fails.

**Evidence**
- The gates at 1 × 16 (validation-report.md:446-449):
  - (1) median 83.8%; it needs +6.2 points.
  - (2b) median +19.3 ms over an allowance of 5.5 to 9.0 ms.
  - (3) 86.9%; it needs +3.1 points.
  - (4) 16 to 44 tickets against 10, with S p99 of 42.5 to 125.7 ms (:453).
- The breakdown (validation-report.md:545-550):
  - the whole stream row costs 5.0% of the rate (`c-stream`/`c-locks` 0.950), and p99 goes from 62.3 to 65.9 ms (+3.6 ms);
  - the ten line rows take p99 from 65.9 to 78.1 ms (+12.2 ms);
  - `c-noidx` p99 71.3, `c-chunk` 70.0, `c-unlogged` 66.0.
- design:1169-1180 and D041-111 (research.md:122) reject the line options because they "moved the rate by less than the noise", and reject chunk rows as a change "for WAL alone".
- design:1986-1987: "If a gate fails: stop; the design changes and is reviewed again".
- D041-109 (research.md:120) only takes the environment question to the owner after another failure.

**Why**
- The cut removes only the update of the stream row; its insert stays, now with four index entries instead of five. The most it can recover is part of 5%, against the 6.2 points gate (1) needs.
- Gate (2b) is a p99 gate. The only p99 figures available put most of the carry's p99 in the lines, which the revision keeps. The rejected line options are exactly the ones that move p99: by 7 to 12 ms, through WAL volume.
- Nothing in the storage change reduces (g)'s S p99 from about 100 ms to the roughly 25 ms that gate (4) needs.
- The p99 differences are about as large as the noise (`c-locks` alone shows +6.5 ms of p99 at a rate of 0.996). That is a reason to argue p99 explicitly, not to leave it out.
- The gate's median of three unpaired rounds disagrees with the paired diagnosis: (a)/off-1K is 0.906 over 12 paired blocks (validation-report.md:496), against an 83.8% gate median. Run-to-run variance is a large part of the failure, and the revision does not mention it.
- If the third run fails, D041-107 and D041-111 have already used up the storage levers. A fourth revision would have no lever left, and the design does not say so.

**Fix**
- Before the third run, write down in T041-35 the expected effect of each change on each gate, with its basis in the breakdown, including p99.
- Either adopt a p99 lever (lower line WAL volume: fewer line indexes, or chunk rows weighed against their read-path cost), or put the variance and environment question and O041-039 (d) to the owner now, not after another failed job.
- State in T041-35 what follows a third failure.

## Major

### M1. A release waits for, and then holds, every lease of the tenant. That re-creates the hot row on the purge's cadence. It is not prototyped or measured, and it is not needed.

**Evidence**
- ADR-0097:72-76: "folds ... every lease of the tenant, waiting for each in lease order".
- data-model.md:243-246 and 251: "it holds no usage row while it does".
- step_log_spike.rs:265: "The release path (the purge) is not part of the spike".
- design:1979-1981: "a purge running beside the workloads" is not measured.
- design:1230-1232: 500 lines per purge transaction.

**Why**
1. From its finalizer to the end of its WAL flush, a release holds all of the tenant's leases and then the usage row. During that time every charge of the tenant finds no free lease, goes to a refill, and queues on the usage row behind the purge's commit. For a busy tenant the purge runs continuously (deletions keep pace with writes: at 3,000 lines/s that is about 6 purge commits per second), so this is a periodic stall of every carry, under its run lock.
2. While the release is still taking leases one by one, the charges it pushes out make refills create new leases ("the lowest unused lease number", step_log_spike.rs:308-320). Each purge therefore ratchets the lease count toward 256. A lease created after the release's lock statement is not folded, so "every committed charge is then in S" (ADR-0097:75-76) needs to be stated precisely.
3. If a purge transaction spans tenants, the release holds tenant A's usage row while it waits for tenant B's leases. That contradicts data-model.md:251 unless the purge is single-tenant per transaction or processes tenants in order, and neither is specified.
4. The fold exists only to keep S ≥ 0 (I3). There is a simpler exact alternative: let the release apply S += delta under the usage row only. S may go negative; the invariant becomes U = S + Σused ≥ 0, checked under the row with a fresh read of Σused. U ≤ S + G ≤ L holds whatever the sign of S, and free room only grows. This removes the only wait on a lease, the special case for L3b, and the stall.

**Fix**
- Adopt the no-fold release, and relax I3 to U ≥ 0 for the step-log class.
- If the fold is kept: prototype it, add a purge beside (a), (f) and (g) in a measured row, state that purge transactions are single-tenant, and bound the lease-count growth.

### M2. In the at-limit regime the leases fall back to one usage-row lock per charge. That regime is a documented steady state, it is unquantified, and the third run never reaches it.

**Evidence**
- ADR-0097:65-70 and data-model.md:236-242:
  - a grant is min(1 MiB, free/64), which is below 1 MiB whenever free room is under 64 MiB;
  - a reclaim runs when free < max(n, 1 MiB) and takes every idle lease `FOR UPDATE SKIP LOCKED` (step_log_spike.rs:296-303), holding them until its commit.
- ADR-0097:85-86: "locked once per lease of up to 1 MiB (about 300 ten-line carries)".
- Risk row design:1658: the step-log quota "fills in hours at a high log rate and drops the newest lines until old streams expire".
- design:2371-2372: "the carry sets no lock timeout".

**Why**

A tenant whose log rate times its retention exceeds its limit lives at the limit; the design itself expects this (CUJ 3 and CUJ 8). In that state:
- Each refill reclaims, and so locks, every idle lease until its commit. Concurrent charges then find nothing free, go to refills, and queue on the usage row.
- Grants shrink to free/64, or to n − room, so there is about one refill per charge.
- Each refill holds the row to the end of its WAL flush (the measured 1.1 ms p50 and 4.0 to 4.5 ms p99), now in the middle of the transaction, under the run lock, with no timeout.

This is the pre-revision hot row again. The "once per 1 MiB" claim holds only while at least 64 MiB of room is free. The gate workloads use about 30 MiB per row against a 1 GiB default, so they never leave that regime.

**Fix**
- State the regime and its expected lock rate.
- Add an at-limit workload with a purge to T041-35 or T041-9.
- Consider:
  - a refill that only tries the usage row for a carry and otherwise answers `contended` or a counted drop, which matches ADR-0095's "only tried" rule;
  - a reclaim with hysteresis (do not reclaim on every refill);
  - reclaiming only leases with more than a threshold of room.

### M3. ADR-0097 misquotes its cost evidence, and the evidence measured a different mechanism.

**Evidence**
- ADR-0097:120-122: a SKIP LOCKED per-tenant charge "costs the same as no charge at all (the completion rate within 1% to 2% paired, its p99 within about 1 ms)".
- validation-report.md:503-504: `f-slot`/`f-none` 0.988, **p99 +3.4 ms**.
- validation-report.md:493-495: unpaired, `f-slot` has the worst p99 of the three (+13.3 ms, against +9.3 ms for (f) and +8.0 ms for `f-none`).
- The slot rows used "one of 64 per-tenant rows" with no limit read, no S/G read and no fences (validation-report.md:485-487).
- ADR-0097:21 "Enforced by" cites prototype tests that contain no release path.

**Why**
- Gate (2b) allows 2 ms of p99 at 1 × 16. A charge mechanism that adds +3.4 ms of p99 matters there.
- The actual lease charge, with 256 rows, three fences, a limit read and an S/G read, has not been timed under load before the run that gates it.
- The strongest evidence for the leases is missing from the ADR: the 1,075 ingest lock timeouts at 8 × 128 (validation-report.md:559). That evidence also comes from unwarmed pools, so it is confounded and should carry that caveat.

**Fix**
- Quote the paired `f-slot`/`f-none` figures exactly.
- Say that the slot rows are a proxy for the leases.
- Add the 8 × 128 lock-timeout evidence, with its caveat.
- Time `step_log_charge` itself (p50 and p99) in the third run.

### M4. The third run changes the charge and the storage together, with no row that separates their effects.

**Evidence**
- D041-108 (research.md:119): leases are adopted "in the same revision as the storage change so that one re-review covers both".
- design:2018-2028: the third run's workloads are otherwise the second run's.
- D041-106 (research.md:117): the leases "lift a tenant's ceiling ..., not this fail".
- validation-report.md:498-499: the usage row is busy 68.1% in (g) and 20.9% in `g-slot`, but the (g) batch S is not reported for either.

**Why**
- If the third run passes or fails, nobody can tell which change caused it, and M3 suggests the leases may cost p99.
- The leases are the only plausible lever on gate (4): batches stop queuing on a row that is 68% busy. The data to estimate that (batch S for (g) against `g-slot` from run 44730444900) exists, but it is not shown.

**Fix**
- Report the S p99 of (g) against `g-slot` from the existing run now.
- In the third run, add paired 1 × 16 rows with the second run's charge on the new storage (or run the diagnosis set beside the gated rounds), so that each change is attributed.

### M5. Hidden coupling: the quota usage view and other readers of `step_log_bytes` under-report usage, and no task changes them.

**Evidence**
- ADR-0097:77-78: the usage "that the quota API ... read is S + Σ `used`".
- data-model.md:265: the runtime role has no grant on `step_log_quota_leases`.
- design:1194-1196: "The usage column comes with the resource in T041-4, so that its usage view has a column to read". T041-4 is already built and held (O041-037).
- T041-5 (design:2127-2147) adds the charge, refill and release functions but no usage accessor.

**Why**
- Under ADR-0097, `tenant_retained_usage.step_log_bytes` is S, not the usage. The quota REST, CLI and OpenAPI views (ADR-0027 slice 1), and any manifest or backfill test that compares the column with the stored rows, will show S.
- That is low by Σused, up to the leased bytes (as much as 256 MiB per tenant).
- The runtime role cannot add the leases itself.

**Fix**
- Add a `SECURITY DEFINER` accessor that returns S + Σused (or extend `sagat_quota_step_log_allowance` to cover the view).
- List the changes to the T041-4 view and its tests in T041-5.
- Test that the view equals the measure of the stored rows while bytes sit in leases.

### M6. The new failure modes cannot be observed, and lease refusals look like a real quota hit.

**Evidence**
- Section 9 (design:1583-1603) adds no metric for leases.
- G041-023 (research.md:150): such refusals are "counted `tenant_quota`".
- design:993: a mark-only carry after a quota refusal records `tenant_quota` as `ceiling_reason`.

**Why**

None of the following can be seen:
- how often refills and reclaims run;
- the "all 256 held" direct charge;
- how long a release waits for leases;
- the lease count;
- refusals caused by room held in open transactions' leases.

A stream refused for G041-023, or because of B1, shows `ceiling_reason = tenant_quota` while the quota view shows the tenant under its limit. An operator cannot tell a false refusal from a real one, and so cannot detect B1 or M2 in production.

**Fix**
- Add closed-label counters: `step_log_quota_refills_total{kind=refill|reclaim|direct}` and refusals by cause (`limit` or `lease_held`).
- Add a histogram of release waits on leases and a gauge of the lease-row count (aggregate, no tenant label).
- Give held-room refusals a drop reason of their own, or document why they share `tenant_quota`.
- Add an alert entry to the runbook.

### M7. The lease table is a new hot, heavily updated table on the hot path, with no plan for bloat or vacuum.

**Evidence**
- data-model.md:211-219: the table has no fill factor.
- data-model.md:233-235: every charge scans all of the tenant's leases (`ORDER BY ... LIMIT 1 FOR UPDATE SKIP LOCKED`) and updates one of them.
- Risk row design:1650 still names `quota_transaction_deltas` as the table that bloats.
- design:1977: under (d), the third run records "the size of `quota_transaction_deltas`".

**Why**
- Every charging statement of a tenant updates one of at most 256 rows that sit on one or two heap pages.
- The design itself expects held read horizons (risk row 1644, workload (d)). Under one, HOT pruning cannot remove dead versions, and each charge's scan walks longer chains, under the run lock.
- The measurement and the risk row still point at the table the revision stopped writing.

**Fix**
- Give the lease table a fill factor and per-table autovacuum settings.
- Under (d), record the lease table's size and the charge time.
- Rewrite risk row 1650 for the lease table.

## Minor

### m1. The constants are not derived, and the stated bounds are inaccurate.

**Evidence**
- ADR-0097:49 sets 256 leases; the measured proxy used 64 (validation-report.md:485).
- ADR-0097:69 gives the grant formula. ADR-0097:107 says "Near the limit a grant is at most 1/64 of the free room".
- G041-023 (research.md:150).

**Why**
- Nothing ties 256 to the tenant's maximum number of concurrent charging transactions, which is the number of replicas times the pool size (160 at 8 × 128). Above 256, the "all held, charge S under the row" path is the hot row again.
- 1 MiB, 1/64 and max(n, 1 MiB) have no rationale.
- A grant is n − room whenever that exceeds free/64, and it is more than 1 MiB when n is more than 1 MiB (the test charges 4 MiB).
- In practice the G041-023 bound is min(open charging transactions, 256) × 1 MiB, because grants made far from the limit keep their 1 MiB of room.

**Fix**
- Derive 256 from the connection budget and check or document the relation.
- Give a target refill rate for 1 MiB and 1/64.
- Correct the "at most 1/64" text.
- State the worst-case false-refusal room in G041-023.

### m2. The starting slot does not spread charges across leases.

**Evidence:** data-model.md:234 and step_log_spike.rs:352 use `ORDER BY (lease + pg_backend_pid()) % 256`. ADR-0097:63-64 says the order "starts at the backend's own slot".

**Why**
- Leases are numbered from 0 densely, so with k leases every backend whose pid mod 256 is at most 256 − k simply scans in lease order. With 16 leases that is about 94% of backends.
- SKIP LOCKED keeps the charge correct, but every charge probes the same held rows first, and the claim in the ADR is false.

**Fix:** rotate modulo the current lease count (for example `(lease - pid) mod k`), or use a hash. Otherwise remove the claim.

### m3. New coupling with ADR-0027's other classes: a content or log limit change now drops carries.

**Evidence**
- design:1011-1013: in CA the carry tries the content, log and step-log class fences, and "a change of a tenant class limit holding or queued for its fence makes the carry `contended`".
- step_log_spike.rs:410-416 does the same.

**Why**
- Before this revision, only step-log resource changes could make a carry `contended`.
- Now any administrative change of `tenant_content_bytes` or `tenant_log_bytes` also makes carries `contended`, so their final records are lost and the stream shows "ended without its final flush".
- Batches also wait for those fences at T8.
- None of this is listed in SC-041-003, the Risks or T041-36.

**Fix:** document the effect, count it, and test it in T041-36. Also say why all three fences are needed (ADR-0027's lock order against a multi-fence limit change) rather than only the step-log fence.

### m4. Fill factor 70 on a table that is never purged, mostly to benefit rows that are never HOT-updated.

**Evidence**
- data-model.md:92.
- Risk row design:1651: "One small row per attempt".
- G041-004: stream rows are never deleted.
- data-model.md:95-97: `removed_at` and `purged_at` appear in partial-index predicates.

**Why**
- Short-step streams, the majority, are inserted once and only updated later by the purge mark. That update is not HOT, because `removed_at` is in the predicates.
- So 30% of every page is reserved permanently for HOT updates that only long-step streams make: roughly 43% more heap, forever, unmeasured.

**Fix:** measure the HOT ratio of (g) and the table size at fill factor 100 and at 70, then justify the choice or restrict the claim.

### m5. The purge's mark scan by day may walk most of today's still-live rows on every pass.

**Evidence:** design:1225-1227: the mark selects "`expires_day` at most today's and `expires_at` past", with the exact time rechecked.

**Why**
- The index key is a whole day, so every pass must recheck `expires_at` for each row of today's bucket.
- Ties are ordered by heap TID, which stops following time once vacuum frees space (fill factor 70 makes page reuse likely).
- A tenant creating hundreds of streams per second puts millions of rows in one day's bucket.

**Fix:** state and test, in T041-10, the cost of one pass at a day's volume, or mark past days through the index and scan today's bucket within a bound.

### m6. The stream run index is dropped with no measured gain, against the evidence standard D041-111 applied to the lines.

**Evidence**
- design:1175-1177 and data-model.md:93-94.
- FR-041-011 (design:212): the streams route pages with a cursor.
- design:1240: the delete route locks "`ORDER BY stream_id FOR UPDATE`".
- D041-111 keeps the lines' read indexes because removing them gave "no measurable gain".

**Why**
- Without `(tenant_id, run_id, stream_id)`, each page of `listRunLogStreams` sorts all of a run's streams. A `forEach` run (CUJ 6) can have thousands.
- `run_id` is never updated, so this index never blocked HOT. Its only cost is one entry per insert, which D041-107 did not isolate.

**Fix:** keep the index, or measure both its write cost and the read cost at a large collection's fan-out.

### m7. The acceptance route for ADR-0097 is inconsistent.

**Evidence**
- ADR-0097:3-5: "accepted under the same delegation once design 041's re-review closes".
- design:431-435 (O041-040): "its acceptance goes with this question". That is an owner question, and it amends the owner-accepted ADR-0027 Amendment (OD-2026-11-15-16).

**Why:** a delegate's acceptance cannot supersede text the owner accepted, nor wording that is still waiting for the owner.

**Fix:** make ADR-0097's acceptance part of O041-040 (or a new owner question), and change ADR-0097:5 to match.

### m8. The down migration refuses while any lease holds room, and nothing can drain the leases.

**Evidence:** ADR-0097, Consequences: "The down migration refuses while a lease holds room".

**Why:** no function or procedure folds or revokes leases, so a rollback is blocked indefinitely for any tenant that has logged.

**Fix:** add the lease-revoke function proposed in B1 and call it from the down migration under the fences.

### m9. Complexity Tracking has no row for the largest addition of the revision.

**Evidence**
- design:1680-1699 has no entry for the lease table, the two lease functions, the finalizer branch or the property tests.
- The revision's list of changed sections (design:35-36) omits Complexity Tracking.

**Fix:** add a row, with the simpler alternatives that were rejected and the no-fold release of M1.

### m10. Stale and contradictory text after the revision.

**Evidence**
- ADR-0097:71 says "No transaction ever waits for a lease, so no lock cycle can contain one", while decision 5 (:72-74) has the release wait for leases.
- Design text that still describes the old charge at commit, or omits the leases:
  - design:993 ("after the finalizer's refusal");
  - design:1200-1205 (the list of `SECURITY DEFINER` functions omits the two lease functions);
  - design:1231-1232 (the purge holding "the tenant usage row", with no mention of the leases);
  - Risks rows design:1629, 1631 and 1634, and 1650 (also in M7);
  - design:1677 ("a carry refused at commit");
  - design:2672 ("every one of K ingests held after its quota flush, the completion blocked only by them", which contradicts the new SC-041-001 barrier test at :313-316).

**Fix:** update each place, and reconcile ADR-0097:71 with decision 5 (or remove the wait, per M1).

## Nit

### n1. "No quota lock at all" is loose.
design:1033-1034 says the carry "takes no quota lock at all". It takes three shared advisory fences and a lease row lock. Say "does not take the usage row".

### n2. The writer of `expires_at` is out of date.
design:1216-1217 says `expires_at` is "written by T9 of every batch and CE of every carry". For a new stream, CC writes it.

### n3. Test and mutation claims in ADR-0097.
ADR-0097:21 says "five tests, six hand mutations". The file has nine tests, five of them on leases. The mutations are not recorded anywhere reproducible, and the release is not prototyped. Record the mutations and say that the release is unprototyped.

### n4. Ambiguous index wording.
design:1165-1167 ("with a run-scoped and a stream-scoped read index") is ambiguous now that streams have no run index. Say that these are the lines' indexes.

---

**Count:** Blocker 2, Major 7, Minor 10, Nit 4.
