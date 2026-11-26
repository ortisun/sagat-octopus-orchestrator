# Security re-read of decision D041-119 (writer leases, the ensure, the 200 ms fallback)

Reviewer: `security-reviewer` subagent, read-only, on `fc4adff7`. Saved verbatim.

I re-read the design at fc4adff7 and found 2 Major, 3 minor and 1 nit. Most earlier findings are closed; the old Major 1 is only partly closed and now sits on the ensure. I could not run anything, so the pool-starvation effect in Major 2 is my reasoning, not a measurement.

## Status of the earlier findings

| Earlier finding | Status | Where checked |
|---|---|---|
| Major 1 (waits on the usage row, the bound *b*) | Partly closed. The wait under the run lock is now 200 ms and turns into a mark-only answer (spike :322-335, carry block :645-668). The wait on the shared pool is not closed: it moved to the ensure and grew to the 5 s statement timeout (Major 2). | |
| Major 2 (the session setting) | Closed. No custom setting is read, and a caller's `lock_timeout = 0` or `1h` cannot lengthen the wait. | spike :321-326; test `a_refill_past_its_budget_answers_busy_and_the_report_lands` (leases_tests :1441-1454) |
| minor 3 (idle reclaim without the fence) | Closed. | spike :398-401; test at :1556 |
| minor 4 (privilege lists, the caller's interval) | Partly closed. The interval is fixed (spike :404) and data-model §4 is updated. The other lists are stale (minor 3 below), and the purge still has no way to find tenants (minor 2 below). | |
| minor 5, minor 7, minor 8, nit 9 | Closed. | data-model :342-345; :300-303 and design :1275; data-model :223-224; research :128 suffix |
| minor 6 (`lock_timeout` is per acquisition) | Accepted as stated: the statement timeout is the outer guard (ADR-0097 :115-116). | |
| Suspicion S1 (prepared transactions) | Closed by the `NOWAIT` on the writer's own lease. | spike :349 |
| Suspicion S3 (retry storms) | Closed for the fallback. It comes back if the ensure's failure fails the report (Major 1). | |

## Findings I verified in the text and prototype

**Major 1. Nothing says what happens when the ensure fails, and at the quota limit it always fails.**
- **Where:** data-model.md:261-264; docs/designs/041-step-logs.md:1181-1188 and :2565-2572 (T041-36's ensure item); ADR-0097:100-108; spike `step_log_ensure` :381-390, calling the refill, which raises at :361-370.
- **Scenario:**
  - A tenant's step-log usage reaches its limit. The ensure calls the refill with `charge` false. The refill folds the lease, finds that g = max(n, …) is more than F, and raises `tenant_step_log_quota_exceeded`.
  - The carry then reports no room left. So every later carry on that connection calls the ensure again, and every one raises.
  - The ensure is its own statement before the report transaction. It sits outside the carry's block and outside the downgrade re-run, and no `StoreError` mapping is given for it.
  - Built the obvious way (`?`), every report that carries a log fails while the tenant is at its quota. The worker resends, fails again, and runs out of retries or its lease. A full step-log quota then changes step outcomes for the whole tenant, which FR-041-005, ADR-0084 decision 7 and ADR-0095 decision 4 forbid.
  - The same happens on other errors: `57014` from the statement timeout, `55P03` on the own lease (`NOWAIT`), a deadlock the database breaks, a foreign-key error.
- **What it is:** FR-041-005 (:150-157) forbids this in general, but T041-36 neither maps the ensure's errors nor tests them. Its tests cover only "room made", "falls back" and "busy".
- **Fix:** State in 2b, ADR-0097 3b and T041-36 that any ensure error is caught, counted (e.g. `step_log_quota_refills_total{kind="ensure"}` plus an outcome label, or a closed `ensure_failed` cause), and the report proceeds as if no ensure ran. The carry's reservation then fails and the block answers. Add tests: an ensure at the limit, an ensure past the statement timeout, and an ensure hitting `NOWAIT` on its own lease. Each report must still land.

**Major 2. The ensure's wait has no budget, no admission cap and no allowance check, and it holds a shared pool connection the whole time.**
- **Where:** ADR-0097:104-105 and :120-121 ("The ensure … waits without that budget"); data-model.md:262-264 (`bounded` false); spike :384 (fence waited on, not tried) and :389; refill near the limit: :337-346, then raises at :361-370; resolution.md:62 (cap not taken).
- **Scenario:**
  - Tenant A has filled its step-log quota, which is cheap to do, and runs many sequential workers.
  - Every carrying report first runs an ensure on the report's pooled connection.
  - Each ensure: (1) waits on A's usage row for up to the 5 s statement timeout, with no 200 ms budget; (2) under the row, because F < max(n, 1 MiB), reclaims every other lease nobody holds (up to the number of backends that charged in 10 minutes); (3) then raises, which rolls the reclaim back.
  - So every carry holds the row for a time proportional to the lease count, does no useful work, and the next ensure repeats it.
  - Holders A controls stretch the queue further: its `sagat_quota_lock_log_usage` holders (scripts, dispatch, connectors), its delete route's 500-row purge transactions, its outputs' finalizers.
  - Each waiting ensure holds a connection from the replica's pool of 20, and other tenants share that pool. The per-tenant admission caps (`SAGAT_STEP_LOGS_TENANT_CALLS`, data-model :385-387) apply to ingest calls only, not to reports.
  - The ensure also holds the step-log fence shared while it waits. A limit change then waits up to 5 s, and meanwhile every carry of the tenant is `contended` and loses its records.
  - What an attacker gets: a tenant at its own quota can tie up other tenants' pool connections for seconds per report, which shows up as pool-acquire failures for those tenants. This part is inferred, not measured.
- **Smallest fix:** In `sagat_quota_step_log_ensure`, read the allowance (L − U, unlocked) first and return at once when the planned charge does not fit. The carry's own plan already refuses that case (design :1008-1010), so the ensure can never help there. Try the step-log fence instead of waiting for it. Call the refill with `bounded` true, or a fixed ensure budget, and treat a timeout as "skip". Have the ensure return a result instead of raising, so a reclaim it did is not rolled back.
- **How to confirm:** a held-lock test with 20 ensures of one tenant at its limit, checking that a second tenant's pool acquire never waits past one report's p99.

**minor 1. The design does not say where the ensure's tenant comes from, and the ensure runs before checks it should follow.**
- **Where:** design :1181-1186 and :2565-2568 say "for the tenant" / "the report's tenant", but the ensure runs before the report's fence is verified. Compare the reservation, which is bound (data-model :278-279; design :986-988: "never from the request").
- **State today:** the report path uses `principal.tenant_id()` (`crates/application/src/worker.rs:2556`), so there is no cross-tenant path now. Because the ensure depends only on the principal's tenant, a missing run and another tenant's run look the same in timing.
- **Gaps:** Nothing prevents a builder from taking the tenant from the run or the request. The ensure runs before C0, so runner and database-runner tokens and managed steps get an ensure for a carry C0 then refuses. It also runs for replays. `p_bytes` has no upper bound in SQL. One call can lease all of a tenant's free room, and the report's reservation then holds it, so the tenant's other writers are refused as `held`. This is within one tenant only.
- **Fix:** State that the tenant is `principal.tenant_id()` only. Run the ensure after C0 to C2, and only for a `Full` carry. In SQL, clamp or refuse `p_bytes` above the largest planned charge a valid carry can have. Add a cross-tenant test: a worker of tenant A reports with a carry on tenant B's run, and B's usage row and leases are never touched.

**minor 2. The idle reclaim still cannot find its tenants, and it locks the usage row before checking anything.**
- **Where:** data-model.md:265-269; design :1239-1245; spike :402 (`FOR UPDATE` on the usage row comes before the idle scan).
- **Scenario:** `sagat_app` cannot read the lease table, so the purge pass has to call `reclaim_idle` for every tenant in `tenant_retained_usage` on every pass. Each call waits on that tenant's usage row with no time limit, even when the tenant has no leases.
- **Fix:** add a definer function that returns the tenant ids with leases idle past the interval (ids only, bounded count). In `reclaim_idle`, check for an idle lease without a lock before taking the row.

**minor 3. The privilege lists still contradict each other.**
- **Where:** ADR-0097:78-81: the runtime role has no privilege on "the functions"; data-model.md:235-238: "no `EXECUTE` to `sagat_app` except the accessor"; design :1330-1337 (Grants) and NFR-041-SEC :1932-1937: list only charge, refill, reclaim and usage, and say there is "no new `SECURITY DEFINER` function beyond" those; data-model :261, :270 and :339-340: grant `EXECUTE` to `sagat_app` on ensure, reserve, reclaim_idle, usage and counters.
- **Scenario:** T041-5's bypass and privilege-audit tests can be written from either list. One reading revokes ensure and reserve, so the store fails at runtime. The other builds definers that NFR-041-SEC forbids, and the audit does not catch it.
- **Fix:** Put one list, with each function's grant, in ADR-0097 decision 1, NFR-041-SEC, the design's Grants paragraph and data-model §3b. Include `refill_wait` and the counter sequences. Make the privilege audit check exactly that list.

**nit 1. The public text for `tenant_quota_busy` claims the quota had room, which is not proven.**
- **Where:** contracts.md:233 ("the quota had room but could not be granted"); design :1211 and :1729-1731 ("while the tenant had room").
- **Why it is wrong:** the `55P03` comes from waiting for the usage row before S, G and L are read under the lock. A busy answer can therefore occur when the quota is actually full.
- **Fix:** say "the quota could not be checked within its budget". The Studio text "was busy" (design :1714) is fine as it is.

## Suspicions I did not verify

- **S2 (earlier review), now more likely.** Suppose a transaction holds the usage row (for example through `sagat_quota_lock_log_usage`) and then takes the step-log fence shared for the first time. It queues behind a limit change waiting for the fence exclusively, and that limit change waits for an ensure holding the fence shared while it waits for the usage row. That is a three-way deadlock. The ensure's long hold on the fence widens the window, and if the database aborts the ensure, Major 1 applies. To check: grep the paths that call `lock_log_usage` for any step-log insert in the same transaction.
- **Transaction pooler.** ADR-0097:71-72 claims the design works behind a transaction pooler. There, the ensure's transaction and the report's transaction can run on different server backends, so the ensure fills another backend's lease and the fallback stops being rare. This means more `tenant_quota_busy` losses and room parked in idle leases. To check: confirm whether any target deploys PgBouncer in transaction mode.

## Classes checked with no finding

- **Room taken for another tenant:** every lease statement filters by `p_tenant` and `writer = pg_backend_pid()` (spike :339-373, :385-386, :403-409, :430-432, :449-453). A charge only spends the room of its own (tenant, backend) lease, so no function moves room between tenants. A random tenant id fails on the `tenants` foreign key (`20261024000000_durable_quotas.up.sql:51`), so no rows can be created for a tenant that does not exist.
- **Information exposure of the new reason:** `tenant_quota_busy` is a closed enum value with no ids or values. Its contention can only come from the same tenant's own transactions, the reader needs `run:logs:read` and the PII reveal grant, and the raised messages carry no ids.
- **Metrics, alert and audit:** `caller`, `cause` and `kind` are closed label sets with no tenant label (data-model :300-308, :389-402). Counting happens in the store or through sequences revoked from `sagat_app`. Ensures are internal, and limit changes stay audited.
- **Definer hygiene in the specification:** pinned `search_path`, names qualified with `public.`, the budget read from a definer function the runtime role cannot replace, the caller's lower `lock_timeout` kept and restored. The test `a_refill_restores_the_callers_lock_timeout` (leases_tests :1482-1507) covers the restore.
