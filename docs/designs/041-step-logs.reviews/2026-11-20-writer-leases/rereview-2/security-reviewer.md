# Security re-read of decision D041-120 (the best-effort ensure, ended-backend reclaim, idle-tenants listing)

Reviewer: `security-reviewer` subagent, read-only, on snapshot `9361a112`. Saved verbatim.

On 9361a112 I found 0 Blocker, 0 Major, 6 Minor and 4 Nit. Both of my earlier Majors are closed in substance; what is left of them is listed below as Minors. I ran nothing. Every finding comes from reading the documents and the prototype SQL. Line numbers are those of the snapshot.

## Status of my earlier findings (fc4adff7)

| Earlier finding | Status | Evidence |
|---|---|---|
| Major 1: an ensure failure fails the report; at the quota it always fails | **Closed for the answered cases.** The ensure returns `limit`, `held`, `busy`, `contended` or `invalid` and does not raise for them. The store never fails or re-runs a report because of it. The remaining gap, errors the ensure does not answer, is Minor 1 below. | ADR-0097:113-125; design :1201-1207; data-model :263-272; spike :387-414; test `the_ensure_answers_without_raising` (leases_tests :1731-1767) |
| Major 2: the ensure waits with no budget, no allowance check and a waited fence | **Closed.** The fence is tried (spike :389-391). The allowance is read without a lock, so a tenant at its limit gets `limit` before any row lock (:396-397). The refill runs with `bounded` true (:400). The "a reclaim is rolled back when the refill raises" part is moot in practice, because the refill now runs only when L − U ≥ n. What remains is that a `busy` ensure is still followed by a fallback wait (Minor 2). | as cited |
| minor 1: where the ensure's tenant comes from; it ran before C0; `p_bytes` had no bound | **Partly closed.** "The principal's tenant, never one from the request or the run" and "after C0 to C2" now appear in ADR-0097:113-115, design :1196-1197 and data-model :264. Three things are still open: the bound is the wrong number (Minor 3), T041-36's task bullet still has the old wording, and no cross-tenant test was added (Minor 4). | |
| minor 2: the idle reclaim could not find its tenants and locked before checking | **Closed.** `idle_tenants` returns ids only (data-model :273-275; spike :418-424), and a lockless pre-check comes before any lock (spike :433-438). The residual is Nit 3. | |
| minor 3: the privilege lists contradict each other | **Mostly closed.** ADR-0097:88-94, NFR-041-SEC (design :1977-1985), the Grants paragraph (design :1369-1377), data-model §3b (:235-240) and T041-5 (design :2453-2456) now carry the same list of six granted and four revoked functions. Two problems remain: data-model §4 still has five, and no listed function can return the "room left" that the carry and a batch must return (Minors 5 and 6). | |
| nit 1: "the quota had room" for `tenant_quota_busy` | **Partly closed.** The contract now says "could not be checked within its 200 ms budget", which is correct. It then adds "not the tenant quota", and Studio says "(this is not your quota)", which is the same unproven claim (Nit 1). | |
| Suspicion S2: three-way deadlock through the fence | **Closed.** I found no holder of the usage row that takes the step-log fence afterwards. `take_log_prefix` (content_usage.rs:254-260) takes its fences before the row, and the script marker runs in its own transaction after the report (design :520, data-model §9). Also, the ensure's and the fallback's 200 ms `lock_timeout` fire before the default 1 s `deadlock_timeout`. | |
| Suspicion: transaction pooler | **Closed as a suspicion.** It is now recorded as gap G041-027 (research.md), with a T041-9 scenario (design :2950-2951) and the statement in ADR-0097:127-129. | |

## Findings verified in the text and prototype

**Minor 1. The ensure can still raise for errors it does not answer, and the design neither names nor tests that path.**
- **Where:**
  - ADR-0097:121-123 and design :1204-1205: "It never raises for **these**".
  - Spike :402-409: the handler catches only `lock_not_available` and `program_limit_exceeded`.
  - Design :1312 and data-model :413-414: the closed `outcome` set has no error value.
  - T041-36's ensure tests (design :2692-2697) cover only `limit`, `busy` and a prepared transaction.
- **Scenario:**
  - Each acquisition of the usage row is capped at 200 ms. A chain of short holders can still run the ensure into the statement timeout (`57014`), which the design itself admits for the fallback (design :1228-1230).
  - The ensure can also hit a `40P01` chosen by the detector while a limit change is queued, or a `23503` when the tenant row disappears.
  - Each of these propagates out of `sagat_quota_step_log_ensure`. If it is built the obvious way (`?`), the report fails before its transaction starts and the worker resends it. That is the outcome ADR-0084 decision 7 and ADR-0095 decision 4 forbid.
- **Fix:**
  - State in 3b and T041-36 that any error of the ensure's transaction is rolled back, counted as `outcome="error"` (added to the closed set), and that the report then proceeds as if no ensure ran.
  - Add a test: hold the usage row and run the ensure under `SET LOCAL statement_timeout = '100ms'`. It must raise `57014` on the prototype today, and the report must still land.

**Minor 2. "A report waits on the usage row at most once" is false for a `busy` ensure.**
- **Where:** ADR-0097:123-125; design :1205-1207. Only `limit` and `held` are passed to the carry.
- **Scenario:**
  - The tenant's usage row is busy, for example from its own content commits queued at commit or its delete-route purges.
  - The ensure waits up to 200 ms and answers `busy`. The store does not pass that answer on.
  - The carry's reservation then fails (the lease is still short), and the fallback refill waits up to 200 ms more, this time under the run lock. Sibling reports, heartbeats and cancels of the run queue behind it.
  - The store's kept room stays short, so every later report on that backend repeats both waits.
  - This stays within one tenant. Across tenants the effect is the longer hold of a shared pool connection. Reports with output already wait on the same row at commit (G041-024), so this adds to an existing exposure rather than opening one.
- **Fix:** either pass `busy` to the carry too, so it answers `tenant_quota_busy` at once with no second wait, or correct the claim to "at most twice, once under the run lock" and have T041-9's fan-out scenario measure the double wait.

**Minor 3. The ensure's `invalid` bound is the attempt's ceiling, not a carry's, so it is about 1,000 times too loose.**
- **Where:**
  - Data-model :266-267: "the attempt byte ceiling's maximum plus 128 for each of the attempt record ceiling's maximum rows and a marker".
  - Spike :385-387: `16777216 + 128 * 100001` = 29,577,344 bytes.
  - Compare contracts.md:151-153: a carry is at most 64 records and adds at most 16 KiB to a report.
- **Scenario:** the ensure grants g = max(n, share). Suppose a store defect passes a planned charge near the cap, or the plan is computed from something other than the validated carry:
  - One backend's lease then holds about 28 MiB of room beyond its 1 MiB share.
  - The next reservation on that backend locks the whole row for its report transaction.
  - Meanwhile the tenant's other writers near the limit are refused `held`, and their records are dropped.
  - The operator's bound "*k*_open × 1 MiB plus their planned charges" (design :1263-1265) also stops being true, because the room left over by an ensure whose report never charged (refused, late or replayed) is not a planned charge of an open carry.
  - The effect stays within one tenant. Exactness is kept, because the refill still checks g ≤ F.
- **Fix:** derive the bound from the carry's own caps: what `sagat_quota_measure` can return for 64 records and 16 KiB of wire bytes, canonical-JSON escaping included, plus 65 × 128. Pin it with the planned-charge property test of T041-36.

**Minor 4. T041-36's task text still describes the old ensure, and no cross-tenant test exists.**
- **Where:** design :2638-2645 still says "the room the previous carry on that connection returned for the tenant (kept per connection, none for a fresh one)". It does not mention C0 to C2 or the principal's tenant, and it contradicts the cache keyed by (pid, tenant) with a 9-minute expiry (ADR-0097:125-127).
- **Scenario:** an implementer working from the task can key the cache per connection object and take the tenant from the report's run. No test in T041-36 or in the cross-tenant battery (design :3033) would catch it.
- **Fix:** restate the bullet from 3b. Add a test in which a worker of tenant A sends a carrying report on tenant B's run. It must get the same 404 or LEASE answer as for a missing run. B's usage row, leases and counters must be unchanged, and only A's lease may have been ensured.

**Minor 5. Data-model §4 still lists five granted lease functions, so NFR-041-SEC contradicts itself.**
- **Where:**
  - Data-model :351-353 grants `EXECUTE` to `sagat_app` on reserve, ensure, reclaim_idle, usage and counters. It has no `sagat_quota_step_log_idle_tenants`.
  - NFR-041-SEC (design :1977) says "No grant beyond those of data-model.md §4" and then lists six (:1981-1983).
- **Scenario:** a migration built from §4 leaves `idle_tenants` with no grant. It is revoked from `PUBLIC`, so the purge pass gets `42501`, ended leases are never reclaimed, and their room sits in G until a near-limit refill reclaims it. A migration built from the NFR passes an audit that §4 says should fail.
- **Fix:** add `sagat_quota_step_log_idle_tenants` to §4. T041-5's list of the functions its migration creates (design :2416-2420) also omits reserve, counters and idle_tenants; add them there too.

**Minor 6. Nothing on the privilege list can give the `SECURITY INVOKER` carry, or a batch, the "room left" the design says they return.**
- **Where:**
  - Data-model :518-522: `sagat_step_log_carry` returns `writer` and `room` and is `SECURITY INVOKER`.
  - Design :1207-1208 and ADR-0097:125-126: "Every call that charges or refills (the carry, the ensure, a batch's T8, the script marker) returns the backend's pid and the room left".
  - `sagat_app` has no privilege on `step_log_quota_leases` (data-model :340). Of the six granted functions, reserve returns a boolean, usage returns U and counters are global. Only the ensure returns room. The prototype carry returns no room (spike :674-684).
- **Scenario:** the builder of T041-36 must either grant `SELECT` on the lease table, which breaks decision 1 and opens every tenant's lease rows to the runtime role, or add an unlisted definer, which the privilege audit "checking exactly this list" fails.
- **Fix:** choose one and record it in the one list. For example, `sagat_quota_step_log_reserve` returns `(found boolean, room bigint)`, or a seventh definer, `sagat_quota_step_log_room(tenant)`, returns only the calling backend's own room. Apply it to the batch and marker paths as well.

**Nit 1. "(this is not your quota)" can be false in the regime where `busy` is most common.**
- **Where:** design :1234 and :1757; contracts.md:233 ("not the tenant quota"). The fallback refill goes straight to `FOR UPDATE` without reading the allowance (spike :329-331).
- **Scenario:** near or at the limit, other writers' refills reclaim this writer's lease, so the store's kept room is stale and no ensure runs. The carry falls back, finds the row hot, and answers `tenant_quota_busy`. Studio then tells a user whose quota is full that it is not their quota. Nothing crosses a tenant.
- **Fix:** in the refill, read L − U without a lock before the row lock and raise `tenant_step_log_quota_exceeded` when n does not fit, as the ensure already does. That makes the text true, as of that read, and removes waits under the run lock at a full quota. Otherwise drop the parenthesis.

**Nit 2. The 9-minute kept room can outlive the lease.**
- **Where:** ADR-0097:125-127. A `room` answer from the ensure, or a carry that charged nothing, does not update `charged_at` (spike :392-395), but the store would timestamp the room it was told.
- **Scenario:** a backend charges at t = 0, gets a `room` answer at t = 8.9 min and keeps it until 17.9 min. The idle reclaim folds the lease at 10 min. A report at 10.5 min skips the ensure and falls back under the run lock, where it may answer `busy`.
- **Fix:** have the store date the kept room from the last *charge*, or have the functions return `charged_at` with the room. This is mostly the correctness reviewer's area.

**Nit 3. `reclaim_idle` waits for the usage row with no budget, and its `pg_stat_activity` view is fixed at its first read.**
- **Where:** spike :433-447; data-model :276-281.
  - The function holds the step-log fence shared while it waits on `FOR UPDATE` of the usage row, limited only by the purge connection's statement timeout.
  - The backend-status snapshot is taken at the lockless pre-check and kept for the whole transaction.
- **Scenario:**
  - A limit change queued exclusively behind that shared fence makes every carry of the tenant `contended` (records lost) for the length of the wait.
  - A pid reused by a new backend during the wait can have its fresh lease treated as ended and folded. This is exact, but it leads to a fallback.
- **Fix:** take the usage row under the same 200 ms budget and skip the tenant on `55P03`. Treat a lease as ended only when `charged_at` is earlier than the start of the transaction.

**Nit 4. "Nothing to charge" is defined as "no record kept", which ignores a planned ceiling marker.**
- **Where:** data-model :581-582 and :554-556 (a `ceiling_marker` is planned when the ceiling is first met); spike :677 (`v_kept = 0`).
- **Scenario:** if the built function writes the marker when the first record already overflows the ceiling, that 128-byte charge runs with no reservation and no block. At the tenant's limit the refusal is not caught, and the report is re-run.
- **Fix:** skip the block only when the planned charge is 0, meaning no kept row and no marker.

## Suspicions I did not verify

- **Cross-tenant pool pressure.** Reports have no per-tenant admission cap; `SAGAT_STEP_LOGS_TENANT_CALLS` applies only to ingest calls. A tenant whose own usage row is hot can therefore hold shared pool connections for up to two budget waits per report (Minor 2). I believe this mostly predates D041-120 (commit waits for content, G041-024). T041-9's cold-start criterion "no heartbeat or lease call failing to get a connection" (design :2933-2938) is the right check, but it covers a deploy, not an adversarial tenant. To confirm, add a row in which tenant A keeps its usage row held for 150 ms at a time while tenant B's heartbeats and reports run on the same replica, and record B's pool-acquire p99.
- **`idle_tenants` with `LIMIT` and no `ORDER BY`, and a `NULL` max.** With a fixed physical order the same tenants come first on every pass. If they cannot be reclaimed (fence held), others wait. A `NULL` max returns every tenant. I found no way for a tenant to keep itself unreclaimable, so I see no attack, only fairness. Order by the oldest `charged_at` and clamp `p_max` in SQL.

## Classes checked with no finding

- **Tenant boundary of the ensure.** It depends only on the principal's tenant (ADR-0097:115) and runs before the report's run lookup. A report on a missing run and one on another tenant's run therefore pay the same ensure: the same timing class and no row of the other tenant touched. Every lease statement in the ensure, refill, reserve and idle reclaim filters on `p_tenant` and `writer = pg_backend_pid()` (spike :339-373, :392-394, :413-414, :433-452, :473-475). Room never moves between tenants.
- **Ended-backend reclaim and pids.**
  - `pg_stat_activity.pid` is visible to any role, so the check works whatever the definer owner's statistics privileges are.
  - `pg_stat_activity` resolves through the pinned `search_path = pg_catalog, pg_temp`, with `pg_temp` last, so no temporary object can shadow it.
  - Neither the pids nor the leases are returned to any API. `idle_tenants` returns only tenant ids, and only to the runtime role. The ensure returns only the caller's own pid.
  - A backend of another tenant, or of another database, only ever makes a lease look alive, which is the conservative direction.
  - `SKIP LOCKED` keeps the reclaim off any lease an open transaction holds.
- **Who can force `tenant_quota_busy` or `contended` on another tenant.** Nobody in a tenant role can. The usage row and the step-log fence are per tenant (an advisory key from a 64-bit hash of the tenant UUID, which the platform assigns). Their holders are the tenant's own transactions, the platform's purge for that tenant, and limit changes for that tenant.
- **What `tenant_quota_busy` discloses.** It is a closed enum value with no ids, sizes or holders. It is readable only on the reader's own stream with `run:logs:read` and the PII reveal grant, and it reports contention from the same tenant's own activity or an operator's limit change. It is not an oracle across tenants.
- **Telemetry.** `step_log_quota_ensures_total{outcome}`, `refills_total{kind}` (now with `ended`), `refusals_total{cause}`, `refill_budget_exceeded_total{caller}` and the lease-row gauge all have closed labels and no tenant or pid (design :1311-1317; data-model :312-316, :413-416). The structured warning per busy carry names the tenant id and nothing sensitive (design :1772-1773).
- **Definer hygiene.**
  - `refill_wait` is now `STABLE` (data-model :248-249; spike :304), so a test's `CREATE OR REPLACE` takes effect and no cached plan folds it in. The runtime role cannot replace it, and T041-5 adds a migration test that it returns 200 ms (design :2456-2457).
  - A caller's lower `lock_timeout` is kept (test `a_callers_lower_lock_timeout_is_kept`, leases_tests :1849-1893).
  - The `set_config(..., true)` inside the refill is undone when the ensure's or the carry's subtransaction aborts.
- **One privilege list.** ADR-0097 decision 1, NFR-041-SEC, the Grants paragraph, data-model §3b and T041-5's audit all agree, except for Minors 5 and 6.

The files I read, all in the snapshot under `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot/`:
- `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`
- `docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md`
- `docs/designs/041-step-logs.md`
- `specs/041-step-logs/data-model.md`
- `specs/041-step-logs/contracts.md`
- `specs/041-step-logs/research.md`
- `tests/performance/scalability-ha/step_log_spike.rs`
- `tests/performance/scalability-ha/step_log_leases_tests.rs`
- `crates/infra-postgres/src/content_usage.rs`

and the diff at `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/diff-fc4adff7-9361a112.diff`.
