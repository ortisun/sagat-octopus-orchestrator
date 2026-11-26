# Security review of design 041's revision (approval run, first round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: ff8033cd933d0c6adf972706eb1bb38407bdb7c6
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, read-only, given the diff of the design and of its companion documents since the approved commit f9d4a65f, on the owner's approval path "Full review, capped" (OD-2026-11-20-17). Saved verbatim.

I found no Blocker, one Major and four Nits. The change keeps the runtime role and definer-function rules of ADR-0027, and the quota stays exact by ADR-0097's I1 to I5. The one thing that needs a fix before approval is that one tenant's log activity can now tie up database connections that every tenant shares (S-1).

## Findings

**S-1 (Major): one tenant's log waits can hold the replica's shared database connections, and no test measures the effect on other tenants.**
- **Where:** `docs/designs/041-step-logs.md` section 1 "Admission" (lines 775-777: "A carry is not admitted in this class"), section 2 "What a report can wait for" (929 onward), section 2b "Room made before the report" and "The fallback's budget" (1227, 1284), and task T041-9's regimes (3073-3101).
- **What is wrong:**
  - The approved design kept log work inside the `Logs` class, with per-tenant tickets (2 per replica), so one tenant could not take the database pool through logging. The carry moves an attempt's last records onto the report, which is in the reserved `Lease` class. That class has no per-tenant bound (`crates/api-grpc/src/limits.rs:489-508`).
  - Two waits on the tenant's usage row come with the carry, and both run on the pool connection the report holds. The ensure waits up to 200 ms per lock acquisition, under no run lock. The fallback refill waits the same, under the run lock, and the statement timeout (5 s) is the hard limit for both.
  - Section 2b says fallbacks are "no longer rare" in a tenant's last `max(n, 1 MiB)` of quota. There, refills take back each other's room before the report reserves it. Behind a transaction pooler (G041-027) nearly every refill is a fallback.
- **Scenario:** tenant A runs many concurrent short steps while it stays inside that band (for example, a small limit it sets on itself while its purge or deletes free room). Every carrying report then queues on A's usage row with a pool connection held. With a pool of 20 per replica, tenant B's reports and heartbeats wait for a connection, and B's leases can expire.
  - Content commits can already convoy on a tenant's usage row (G041-024). The change adds a second, log-driven path that the Logs tickets were meant to prevent.
  - None of T041-9's regimes has a second tenant as the victim. The cold-start regime checks connections only for a single workload.
- **Recommendation:**
  - Add a two-tenant case to T041-9's "Near the limit" and "Behind a transaction pooler" regimes, with its criterion fixed before the run. Tenant A is held in the fallback regime; tenant B, far from its limit, must keep its completion p99 and its pool-acquire wait within its solo baseline, and no heartbeat or lease call may fail to get a connection.
  - Name the remedy now in case that fails: a per-tenant, per-replica permit that is tried, never waited for, before any carry the store expects to need an ensure or a fallback. With no permit free, the application plans `MarkOnly` / `tenant_quota_busy` before the transaction.
- **Recommended disposition: fix.** The isolation the Logs class was built for is bypassed by the new path, and the measurement plus a pre-decided remedy is cheap next to a release with an unmeasured cross-tenant regression.

**S-2 (Nit): the ensure writes quota state before the report's checks have run.**
- **Where:** `docs/designs/041-step-logs.md:1227-1236`; ADR-0097 decision 3b.
- **What is wrong:** the ensure runs after C1 to C3 only. That is before the lease, binding and membership are checked, and before C0 refuses the runner roles. The design says itself that "a carry C0 refuses may have cost an ensure". So a report with a forged lease, from a worker whose membership was revoked, or from a runner token, can lock and write its own tenant's usage row and park room in a lease that nothing holds.
  - The effect is limited: it stays within the caller's own tenant, and the room is bounded and can be taken back.
  - It is not a cross-tenant oracle: every caller pays the ensure before any read, and its answer never reaches the client.
- **Recommendation:** state that `authorize(Permission::WorkerExecute)` and C0's role half (known from the token) run before the ensure, and add a test that a runner-token or forged-lease carry runs no ensure. Optionally, run the ensure only after an unlocked lease read like the ingest's T2.

**S-3 (Nit): the ensure's size cap is far looser than any valid carry.**
- **Where:** `specs/041-step-logs/data-model.md:266`; the prototype `tests/performance/scalability-ha/step_log_spike.rs:386-388` uses `16777216 + 128 * 100001`.
- **What is wrong:** the definer function accepts a planned charge of about 29.6 MB. A valid carry (64 records, 8 KiB message and exception, 2 KiB fields after C3, plus 65 × 128) is about 1.13 MiB at most. Any caller running as `sagat_app` can pull up to 29.6 MB of a tenant's free room into its lease per backend, which turns into `held` refusals until a reclaim.
- **Recommendation:** derive the cap from the carry's own bounds and add a test at the boundary.

**S-4 (Nit): the carry function takes its lock keys and mark reason as free text from the caller.**
- **Where:** `specs/041-step-logs/data-model.md:526-531, 551-552`.
- **What is wrong:** `sagat_step_log_carry` takes `p_fence_keys`, `p_stream_key` and `p_mark_reason` as caller-built text. If the Rust code that builds the keys drifts from the ingest's, the policy read goes unfenced and the `last_stored_xid` order guard breaks silently. An unknown `p_mark_reason` only fails on the `ceiling_reason` CHECK and costs a re-run of the report.
  - There is no privilege gain, because the function is `SECURITY INVOKER`.
- **Recommendation:** build the keys inside the function (or in one SQL helper the ingest also calls) from `p_tenant`, `p_namespace`, `p_run`, `p_step`, `p_attempt` and `p_generation`, and check `p_mark_reason` against the closed set at the top.

**S-5 (Nit): the runbook's PostgreSQL logging settings should be zero, not "bounded".**
- **Where:** `docs/designs/041-step-logs.md:1854-1855` (unchanged text, but the change adds new exposure).
- **What is wrong:** the carry puts line text into the parameters of a call on the report path, and that call can be slow by design (200 ms budget, 5 s timeout). A "bounded" nonzero `log_parameter_max_length` logs a prefix of every parameter. `log_error_verbosity = terse` drops the DETAIL lines, but only if the operator sets it.
- **Recommendation:** have the runbook ask for `log_parameter_max_length = 0` and `log_parameter_max_length_on_error = 0` for `sagat_app`.

## Classes with no finding (each was checked)

- **Tenant boundary:**
  - The ensure uses the principal's tenant.
  - The carry's identity is the attempt the report's fence verified.
  - Leases are keyed `(tenant_id, writer)`.
  - The fence and stream-lock keys include the tenant.
  - The new `ceiling` value `tenant_quota_busy` is shown only to readers who already hold `run:logs:read` and the PII grant.
  - Metric labels are closed sets with no tenant.
- **Runtime role and definer functions:**
  - `sagat_app` has only `SELECT` on `tenant_retained_usage` (`crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:564-572`). It gets no privilege on `step_log_quota_leases` or the counter sequences, and no `UPDATE` on lines.
  - Row locks it takes (`FOR UPDATE SKIP LOCKED` on streams) are covered by its `UPDATE` grant.
  - The definer functions pin `search_path = pg_catalog, pg_temp`, qualify names and revoke from `PUBLIC`; charge, refill and the limit-change trigger are revoked from `sagat_app`.
  - The 200 ms budget comes from a function, not a session setting. A caller's `lock_timeout = 0` does not lengthen it (prototype `step_log_spike.rs:322-336`, test `a_refill_past_its_budget_answers_busy_and_the_report_lands`).
- **Quota bypass:**
  - The charge is measured by the database from the transition table in an `ENABLE ALWAYS` trigger, never passed by a caller.
  - `ON CONFLICT DO NOTHING` rows are not in `NEW TABLE`.
  - A reserved carry's charge holds the step-log fence shared, so a limit change waits behind it.
  - With enforcement off, usage stays exact.
- **Secrets:**
  - `StepLogCarry` is in `skip_debug`.
  - A failed carry keeps only its SQLSTATE (`store_error_without_text`).
  - The raise texts are fixed.
  - A report whose output or failure code holds a declared secret writes a mark-only carry.
  - One suggestion for the owner, not a defect: the report path already resolves the attempt's declared secrets to scan its output (`crates/application/src/worker/secrets.rs:265-292`). Running the carry's ≤16 KiB through the same scan would shrink G041-001 for carries at almost no cost, but ADR-0085 rule 4 currently forbids it, so it would need that ADR amended.
- **Injection and parsing:** no dynamic SQL; arrays are bound parameters; fields are flat scalar maps of at most 2 KiB; the carry is validated (64 records, 16 KiB, sequence range) before any statement.
- **Races:**
  - A carry is written only when the report is recorded now, and replays are answered before the carry is read.
  - A re-run is fenced again (lease, cancel, stop, binding and membership under their locks).
  - The idle and ended-backend reclaims fold under the usage row with leases taken `SKIP LOCKED`, so a pid reused in between is still accounted correctly.

## Suspicion, not verified

I did not confirm that a tenant admin may lower their own `tenant_step_log_bytes` freely enough to stay inside the fallback band at will. S-1 holds without it, through the transaction-pooler regime or a tenant whose quota is simply near full.

Counts: Blocker 0, Major 1, Nit 4
