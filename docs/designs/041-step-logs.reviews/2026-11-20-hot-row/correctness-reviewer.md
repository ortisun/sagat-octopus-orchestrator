# Correctness review of design 041's revision of 2026-11-20 (leases and the stream written once)

Reviewer: `correctness-reviewer` subagent, read-only, on `4d74b27a` (the documents and prototype diffs from `69dfa81b`).
Saved verbatim.

Scope: the documents diff, the prototype diff, and the current files in `/Users/marcelo/Documents/Projects/sagat-wt-041`. I ran nothing. Where a claim needs to be run, I name the command. Line numbers refer to the worktree files.

---

### B1 [Blocker] A lowered limit that stays above usage refuses every step-log charge, permanently

**Evidence**
- `docs/designs/041-step-logs.md:1136`: the charge "refuses when S + G > L (a lowered limit)".
- `specs/041-step-logs/data-model.md:233`: "`S + G > L` raises `tenant_step_log_quota_exceeded`". A refill runs only when the fast path finds no row (`:235`).
- Prototype: `tests/performance/scalability-ha/step_log_spike.rs:346-347` raises before the fast path. The refill (and with it the reclaim at `:296-303`) runs only from `:355`.
- The release fold moves only `used` (`data-model.md:245-246`: "S += `used`, G −= `used`, `granted` −= `used`, `used` = 0"), so S + G does not change.
- Yet the ADR claims the room comes back: ADR-0097 `:79-80` says "the next refill or release reclaims their room", and `:109-110` says the same.

**Failing scenario**
1. Tenant at the default L = 1 GiB. Forty concurrent writers have built 40 leases. Each grant was g = min(1 MiB, ⌊free/64⌋) = 1 MiB, so G ≈ 40 MiB, with U = 5 MiB.
2. The owner lowers the limit to 20 MiB. That is above U.
3. Every charge now reads S + G ≈ 40 MiB > 20 MiB and raises. Nothing ever lowers S + G:
   - the refill, which is the only path that reclaims, is never reached;
   - the release keeps all room leased.

**Consequences**
- The tenant stores no step-log line again, at 5 of 20 MiB.
- The allowance reads U = S + Σ`used` and says the records fit (`data-model.md:248-249`). So:
  - every ingest loops on `RETRY_LATER` for the SDK's 10 s and is then counted as a worker gap, not as `tenant_quota` (design `:831-835`);
  - every carrying report pays a failed transaction plus a downgrade re-run under the run lock, which can be refused as late (G041-021).
- This breaks G041-023's bound, "only until those transactions end" (`research.md:150`), and ADR-0097 decision 6.

**Why the tests miss it.** `step_log_leases_tests.rs:229-245` lowers the limit to exactly the usage (`"lower the limit to the usage"`), where refusing is correct. No test lowers it to a value between U and S + G.

**Fix**
- When S + G > L, do not raise in the charge. Keep the fast path from charging into leased room, and call the refill instead. The refill then:
  - reclaims every idle lease (L − S − G < 0 < max(n, 1 MiB));
  - refuses only by its I4 check, g > L − S − G.
- Since the release locks every lease anyway, have it reclaim fully: S += `used`, G −= `granted`, both set to 0.
- Add tests:
  - grant leases at a high limit, lower to U + 2n, and check that a charge of n is admitted and a charge past L is refused;
  - after a release, S + G = U.

**Confidence:** confirmed in the design text and in the prototype.

---

### M1 [Major] ADR-0097 decision 5 places the release's lease wait "under the usage row", and that order deadlocks

**Evidence**
- ADR-0097 `:73-75` says the release "folds, in the finalizer's step-log branch **under the usage row**, every lease of the tenant, waiting for each in lease order". The same item then says "the usage row comes after".
- data-model `:244-245` says "after the fences and before the usage row", and design `:866-867` says "holding no usage row while it does".
- data-model `:223-224` (I2) says G and the leases are "both written only while the usage row is locked". But in the data-model's order the release writes `granted −= used` on the lease rows before it takes L4.

**Failing scenario (if the ADR wording is built)**
1. Purge P takes T's L4 and waits for lease 7.
2. Lease 7 is held by a carrying report X with output (it charged it in CD).
3. X's content finalizer at its commit waits for L4.
4. PostgreSQL breaks the deadlock. If it chooses X, X fails at its commit outside the carry call, so the downgrade re-run does not apply and the report answers `Unavailable`.

**Why the tests miss it.** The prototype has no release (`step_log_spike.rs:265`, "The release path (the purge) is not part of the spike"). The lock-order test of T041-10 and T041-36 does not exist yet.

**Fix**
- Make ADR-0097 decision 5 say "before the usage row".
- Restate I2 as true at commit.
- Specify that the release sums Σ`used` and applies it relative to the S and G it reads under L4, never as absolute values read earlier.
- Add the T041-10 test: a release waits for a held lease and does not hold L4 meanwhile.

**Confidence:** confirmed in the text; the deadlock is plausible if the ADR wording is built.

---

### M2 [Major] Lease contention now aborts report transactions, and a refill waits under the run lock; the design still says neither happens

**Evidence**
- Design `:1000-1002`: "So contention never aborts a report, and a carry adds no wait under the run lock before the commit."
- Contradicted by:
  - design `:1035-1037`: a charge refused because the room is "held only by open transactions' leases, G041-023 … fails the report's transaction";
  - design `:873`: the carry "waits for L4 only on a refill". ADR-0095 `:47` notes the report transaction has no lock timeout, so this is a wait under the run lock.
- `research.md:148` (G041-021) still says "no contention causes the first failure (D041-097)".

**Failing scenario**
1. Tenant within about 1 MiB of its limit; 16 carrying reports in flight, each holding a lease with room.
2. A 17th report's carry finds the remaining room only in those held leases. CD raises and the report transaction rolls back.
3. The downgrade re-run is fenced. If the lease expired meanwhile, the report is refused as late.
4. So capture now changes a report's answer because of contention, which ADR-0095 decision 4 and G041-021 rule out.

**Stale rejection.** The option of a savepoint was rejected because "a finalizer raise happens at commit, which a savepoint cannot catch" (ADR-0095 `:111-112`; design `:1721-1723`). That reason no longer holds: the raise is now in CD.

**Why the tests miss it.** The T041-36 list (design `:2371-2375`) covers two runs crossing the limit, not refusals caused by held leases. No test measures a refill's L4 wait inside CD.

**Fix**
- Wrap CC and CD in a `BEGIN … EXCEPTION WHEN` block inside `sagat_step_log_carry` that catches `tenant_step_log_quota_exceeded`. That is one subtransaction and no extra round trip.
- On the refusal, write the stream as mark-only (`tenant_quota`) in the same transaction. This removes the quota re-run, and G041-021 for quota.
- T041-35 must measure the subtransaction's cost.
- If the fix is not taken, at least:
  - correct design `:1000-1002`, G041-021 and ADR-0095 option 6;
  - add a T041-36 test of a held-lease refusal and of a carry waiting on a refill behind a held usage row.

**Confidence:** confirmed (internal contradiction); the behaviour change is confirmed in the design text.

---

### M3 [Major] A release waits for every held lease of the tenant, so each purge transaction pushes the tenant's charges back onto the usage row and inflates G

**Evidence**
- data-model `:244-246`: "every lease of the tenant `FOR UPDATE` in lease order (waiting)".
- refill `:239`: when every free lease lacks room it takes "the lowest unused lease number" (a new lease). Prototype: `step_log_spike.rs:305-321`.
- Design section 4 `:1231-1232` still says "500 rows per transaction bound how long the purge holds the tenant usage row".

**Failing scenario**
1. At 8 × 128 (completion p50 about 200 ms on the runner), dozens of T's leases are held at any moment by carrying reports between CD and their commits.
2. Each 500-row purge transaction, at its commit, locks leases 0..255 in order and waits on each held one in turn.
3. The leases it has already locked are skipped by every charge. As the sweep advances, more charges find no free lease with room and fall back to a refill. Each refill:
   - takes L4 and holds it to the end of its own commit and WAL flush;
   - creates a new lease of up to 1 MiB.
4. A purge backlog (back-to-back 500-row transactions) therefore keeps bringing back, periodically, the serialization on L4 that ADR-0097 removes. It also grows G (stranded room), which widens G041-023 near the limit, and it lengthens the purge's commit by the residual hold of every held lease.

**Why the tests miss it.** The spike has no release. T041-5 tests only that "a release waits for a held lease and then folds it" (design `:2141`).

**Fix**
- A release needs no lease when S covers it. The deleted rows are at least one retention period old, so their bytes are almost always already folded into S. So:
  - fold leases `SKIP LOCKED`;
  - apply the delta when S + Σfolded ≥ |delta|;
  - wait in lease order only when S would go below 0.
- Alternative: relax I3 to U ≥ 0 so that a release never touches a lease. This needs the existing `tenant_retained_usage_step_log_nonnegative` to change.
- Add a test: a release commits while N leases are held, with `pg_stat_activity` showing it never waits.

**Confidence:** plausible.

---

### M4 [Major] T041-6 and the verification table still specify the old barrier test (K ingests holding the usage row after their flush)

**Evidence**
- Design `:2196-2197`, the T041-6 hook: "hold after the quota flush (`SET CONSTRAINTS ALL IMMEDIATE`; the usage row held)".
- `:2199-2200`: "its barrier tests, as SC-041-001 states them, every one of the K ingests held after its flush".
- Verification row `:2672`: "every one of K ingests held after its quota flush, the completion blocked only by them".
- SC-041-001 `:313-316` now states the opposite: the ingests hold leases and do not take the usage row.

**Why it matters.** Under leases, `SET CONSTRAINTS ALL IMMEDIATE` runs no step-log flush, so the hook holds nothing. A test built from the task text either fails, or gets adapted (for instance by forcing refills) until it passes while asserting the old queue.

**Fix**
- Replace the hook with two:
  - "hold after T8 (a lease held)";
  - "hold inside a refill (the usage row held)".
- Rewrite `:2199-2200` and `:2672` to the bullet of SC-041-001.

**Confidence:** confirmed.

---

### M5 [Major] ADR-0095, which the owner is asked to accept (O041-038), still describes the commit-time finalizer charge

**Evidence** (only decision 2 was edited, `:59-61`):
- `:79`: "after any raise of the commit finalizer's step-log branch".
- `:89-91`: decision 7's lock order, "then at commit the fences … and the tenant usage row".
- `:111-112`: option 6's rejection, "a finalizer raise happens at commit, which a savepoint cannot catch".
- `:113-114`: option 7, "from its first deferred flush, the tenant usage row".
- `:123-126`: Consequences, "takes the tenant's usage row at its commit even when its output stores nothing … at commit, one deferred flush".

**Why it matters.** Accepting O041-038 "as written" would accept a lock order and a refusal point that contradict ADR-0097 and the design.

**Fix**
- Amend decisions 5 and 7, options 6 and 7, and the Consequences to the lease charge.
- Add L3b and the refill's L4 to decision 7.

**Confidence:** confirmed.

---

### m1 [Minor] The quota usage of `tenant_step_log_bytes` reads S only; nothing in the tasks changes it or tests it

**Evidence**
- `crates/infra-postgres/src/quotas.rs:1228-1235` reads `SELECT coalesce(step_log_bytes, 0)` (built by T041-4).
- ADR-0097 `:77-78` and data-model `:248-249` require S + Σ`used`.
- T041-5's Files (design `:2170-2174`) list `content_usage.rs` but not `quotas.rs`, and no test checks the view.
- Separately, `tests/integration/workflow-primitives/scoped_event_storage/guards.rs:98` drops only `'step_log_bytes'` from the usage snapshot it compares across a migration rollback. The new `step_log_leased_bytes` will show as a difference after T041-5's down.

**Why it matters.** The view under-reports usage by up to 256 leases of used bytes. An owner lowering a limit "to the usage" sets it too low. In the prototype test (`step_log_leases_tests.rs:245`) S = 0 while 1,000 bytes are charged.

**Fix**
- Add `quotas.rs:1228-1235` (one statement, S + Σ`used`) and `guards.rs:98` to T041-5's Files.
- Add a test that the view equals the measured rows while leases hold `used`.

**Confidence:** confirmed.

---

### m2 [Minor] `step_log_enforcement_enabled` has no defined meaning under leases

**Evidence**
- data-model `:184` keeps the flag, and the allowance returns whether the quota is enforced (`:384`).
- §3b's charge and refill (`:231-242`) never read the flag, and I4 and I5 refuse unconditionally.

**Why it matters.** With enforcement off, either the leases still refuse (the flag is dead) or they must grant past L (and I4 no longer holds). The design states neither.

**Fix.** Specify that with enforcement off the refill grants g = max(n − room, 1 MiB) without the L check, and that I4 and I5 apply only when enforced. Test both settings.

**Confidence:** confirmed.

---

### m3 [Minor] The purge's "today" is undefined, and the `expires_day` index makes each pass rescan today's streams that have not yet expired

**Evidence**
- Design `:1225-1226`: "`expires_day` at most today's and `expires_at` past (… the exact time rechecked)".
- The column is in UTC days (data-model `:66-69`). Readers stop serving lines only once `removed_at` is set (data-model `:22-25`).

**Failing scenarios**
- If "today" is `current_date` in a time zone behind UTC (UTC−5 at 21:00 local is 02:00 UTC of D+1), streams with `expires_day` = D+1 and `expires_at` already past are skipped until local midnight. Their lines are served up to 5 h past retention.
- Every pass scans in the index all of today's streams whose `expires_at` has not yet passed (the recheck filter), and does so again on every pass. With 1 M streams expiring a day, that is up to 1 M index and heap visits per pass once the backlog is empty.

**Fix**
- Write the predicate in data-model §8 with the same expression applied to `now()`.
- Have T041-9 measure the purge pass with a day's worth of not-yet-expired streams, or accept marking `expires_day < today` plus a reader-side `expires_at` check.

**Confidence:** plausible.

---

### m4 [Minor] Other text still describes the charge at commit

**Evidence**
- Design `:721`: "one statement-trigger call and one deferred flush calls".
- Design `:627` (Terms): the fences are taken by "the finalizer … before the usage row", but a charge takes them at its insert.
- Design `:993`: "`tenant_quota` after the finalizer's refusal".
- Design `:1629` (Risks): "The tenant row only at commit; … the barrier tests of T041-6".
- Design `:1631`: "one statement-trigger call and one deferred flush".
- Design `:1634`: "a finalizer raise".
- Design `:2690`: "the finalizer's new branch raises at the limit (T041-5)".
- Design `:1231-1232`: see M3.
- data-model `:499`: "`COMMIT`, with `tenant_step_log_quota_exceeded` counted as `skipped_quota`". The marker now raises at its INSERT.
- Design `:834` ("its T6 reads the tenant over the limit") and T041-6 `:2207-2208` ("then the resend's `FINAL_ONLY`") hold only for a true limit crossing. After a G041-023 refusal, the U-based allowance says the records fit, and the resend gets `RETRY_LATER` or `CONTINUE`.

**Fix.** Edit each line to the lease charge. For `:834` and `:2208`, say which refusal leads to which resend outcome.

**Confidence:** confirmed.

---

### m5 [Minor] The prototype tests prove less than ADR-0097's Evidence claims

**Evidence**
- ADR-0097 `:124-125`: "32 concurrent writers never pass a 200,000-byte limit".
- But `w.db.pool` has `max_connections(8)` (`tests/support/mod.rs:263-264`), so at most 8 transactions are concurrent.
- With L = 200,000 (`step_log_leases_tests.rs:175`) and n = 3,000:
  - every refill sees L − S − G < 1 MiB and reclaims;
  - each grant is about ⌊free/64⌋ ≈ 3,125, which leaves 125 bytes of room;
  - so nearly every charge is a refill serialized on L4. The concurrent fast path, and its races with reclaim and fold, are barely exercised.
- Not tested at all:
  - the all-256-held direct charge (`step_log_spike.rs:311-318`);
  - the release fold (absent from the spike);
  - a lowered limit between U and S + G (B1).
- The six hand mutations have no recorded command or output.

**Fix**
- Run a second variant with L = 64 MiB, n = 300 and its own pool of at least 32 connections.
- Add a test with 257 lease holders (the direct charge).
- Add B1's test.
- Record each mutation's command and the failing test name (for example `cargo test -p sagat-tests --test perf_scalability_ha --features perf-spike leases_tests` after each edit).
- Change "32 concurrent" to what actually runs.

**Confidence:** confirmed.

---

### m6 [Minor] ADR-0097's Evidence misquotes the measurement of the slot charge

**Evidence**
- ADR-0097 `:121-122`: "a charge on per-tenant rows taken `SKIP LOCKED` costs the same as no charge at all (… its p99 within about 1 ms)".
- `validation-report.md:503-504`: "`f-slot` / `f-none` 0.988 [0.842, 1.006], p99 +3.4 ms".
- Also, the slot prototype has 64 slots of 1 TiB, no fences, no limit read and no refill (`step_log_diag.rs`, `set_charge`). It is not the lease charge being adopted.

**Why it matters.** The owner's rule against calculation errors (OD-2026-11-20-6).

**Fix.** Quote the slot-against-none pair as measured (rate −1.2%, p99 +3.4 ms), and state that the lease charge's own cost is T041-35's third run.

**Confidence:** confirmed.

---

### m7 [Minor] The two down-migration rules disagree, and the ADR's rule would never let the down run

**Evidence**
- ADR-0097 `:112`: "The down migration refuses while a lease holds room".
- T041-5 (design `:2147`): "a down that refuses while a line exists".
- Idle room is reclaimed only by a refill near the limit (B1), so after any logging some lease keeps room indefinitely.

**Fix.** Use T041-5's rule. Once no line exists, U = 0, so the down can drop the leases and G safely. Say so in the ADR.

**Confidence:** confirmed.

---

### m8 [Minor] Dropping `step_log_streams_run_idx` makes `listRunLogStreams` paging cost grow with the run's stream count

**Evidence**
- data-model `:93-94`: streams are read through the attempt key's prefix, "sorted by `stream_id`".
- FR-041-011 (design `:212`) pages with `cursor` and `limit`.

**Failing scenario.** A `forEach` run with 10,000 iterations has 10,000 or more streams. Each page reads and sorts all of them: O(N) per page, O(N²/limit) to list the run.

**Fix.** Either have T041-9 measure the route on a run with 10,000 streams, or page in the attempt key's order (`step_id`, `attempt`, `generation`) instead of `stream_id`.

**Confidence:** plausible.

---

### n1 [Nit] The HOT test can fail at UTC midnight

`step_log_leases_tests.rs:579` inserts `expires_at = clock_timestamp() + 14 days`. The carry recomputes it at `:588`, and `:590` asserts HOT. If the two straddle UTC midnight, `expires_day` changes and the update is not HOT.

**Fix.** Insert with `date_trunc('day', clock_timestamp() AT TIME ZONE 'UTC') … + 14 days 12 h`, or with a `last_at` pinned to midday.

---

### n2 [Nit] The SC-041-001 table omits the lease row

SC-041-001 `:308` gives "1 stream, 10 lines". Section 2a `:1070` adds "one lease row besides".

**Fix.** Say whether the second measure counts the lease table, and use the same text in both places.

---

### n3 [Nit] Two wordings that are not exact

- Design `:1034`: "takes no quota lock at all". The carry takes the three L3 fences, and L4 on a refill.
- Design `:1029`: "on most days". It should be "on every update that does not cross a UTC midnight".

---

### n4 [Nit] A purge of several tenants holds an earlier tenant's L4 while it waits for a later tenant's leases

Design `:866-867` says a release holds "no usage row while it does". The finalizer loops tenants `ORDER BY tenant_id` (`20261024000000_durable_quotas.up.sql:308`), so a purge covering several tenants holds the earlier tenants' L4 while waiting on later tenants' leases. The tenant order keeps this acyclic, but the sentence is false as written.

**Fix.** Qualify it ("no usage row of that tenant"), or say the purge works one tenant per transaction.

---

Count: Blocker 1; Major 5; Minor 8; Nit 4.
