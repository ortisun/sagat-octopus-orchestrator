# Security review: design 041, revision of 2026-11-20 (step-log quota leases, ADR-0097)

Reviewer: `security-reviewer` subagent, read-only, on `4d74b27a`. Saved verbatim.

Read-only review of the worktree `/Users/marcelo/Documents/Projects/sagat-wt-041`. All paths below are relative to that root. Inputs: the docs and prototype diffs; `docs/designs/041-step-logs.md` (sections 2, 2a, 2b, 3, 4, NFR-041-SEC, T041-5); `specs/041-step-logs/data-model.md` §3, §3b, §4 and §8a; ADR-0097; ADR-0027 (accounting rules, lines 47-57, Amendment); `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql`; `crates/infra-postgres/src/quotas.rs`; the prototype under `tests/performance/scalability-ha/`; and constitution principles V and VI.

## Verified findings

### Major 1: A lowered limit can refuse every step-log charge for good, even after usage drops to zero

**Evidence**
- Every charge refuses when S + G > L, before it tries a lease or a refill:
  - `specs/041-step-logs/data-model.md:233`: "`S + G > L` raises `tenant_step_log_quota_exceeded`"
  - the prototype, `tests/performance/scalability-ha/step_log_spike.rs:346` (raise), then `:356` (the refill is reached only after that check).
- Room in a lease is only taken back by the refill's reclaim (`data-model.md:237-238`), and the refill is unreachable once S + G > L.
- The release only *folds* each lease (`data-model.md:245`: "S += used, G −= used, granted −= used, used = 0"). A fold keeps the lease's unused room in G.
- ADR-0097 decision 6 (`docs/adr/ADR-0097-…:78-80`) promises otherwise: "leases are not revoked … and the next refill or release reclaims their room". Neither path does that as specified.

**Scenario**
1. Tenant T has the default 1 GiB limit and logs steadily. Its 256 leases each hold up to 1 MiB of granted, unused room, so G is about S + 256 MiB.
2. An admin lowers `tenant_step_log_bytes` to 100 MiB, or an operator lowers the ceiling.
3. Every charge now sees S + G > L and is refused. That is correct so far.
4. The purge then deletes all of T's lines. Each release folds the leases and lowers S to 0, but G stays at about 256 MiB of idle room.
5. S + G stays above 100 MiB for good. Every batch and carry of T is dropped and counted `tenant_quota`, while the quota view shows a usage U near 0.
6. The only way out is to raise the limit above about 256 MiB.

**Impact**
- The quota fails closed, so there is no overshoot. But step-log capture for a tenant is denied indefinitely after a legitimate policy change.
- The quota view contradicts the refusals, so operators cannot diagnose it.
- The T041-5 test list (`docs/designs/041-step-logs.md:2142`) and the prototype test `step_log_leases_tests.rs:229` only check that charges *stop*. Nothing checks recovery.

**Fix**, either of:
- (a) In the release branch, reclaim each lease it has locked (S += used, G −= granted, both set to 0) instead of folding it. The release already waits for every lease, so this costs nothing more.
- (b) When S + G > L, have the charge call the refill instead of raising. The refill locks the usage row, reclaims idle leases and applies I4. Refuse only if S + G + n > L still holds after that.

Add a test: lower L below S + G while idle room is leased, purge to zero, then assert that a charge of n ≤ L is admitted.

### Major 2: ADR-0097 places the release's lease waits under the usage-row lock, which contradicts the data model and can deadlock

**Evidence**
- `docs/adr/ADR-0097-…:72-74`: "A release … folds, in the finalizer's step-log branch **under the usage row**, every lease of the tenant, waiting for each in lease order".
- The same ADR at `:71`: "No transaction ever waits for a lease, so no lock cycle can contain one". This is false given decision 5.
- The data model and the design say the opposite:
  - `specs/041-step-logs/data-model.md:244-245`: "after the fences and **before the usage row**, every lease … `FOR UPDATE` … (waiting)"
  - `docs/designs/041-step-logs.md:866-867`: "only a release waits for one, holding no usage row while it does".

**Scenario (if built as the ADR reads)**
1. A purge P of tenant T reaches its finalizer, locks T's `tenant_retained_usage` row, then waits for lease 7.
2. Lease 7 is held by a carrying report R of T: CD charged lease 7, and R also stores output.
3. R's commit runs the content finalizer, which waits for T's usage row, held by P. That is a cycle.
4. The same cycle forms with any transaction that holds a lease from one statement and needs a refill in a later one.

**Impact**
- PostgreSQL breaks the cycle only after `deadlock_timeout`, with the report holding the run row the whole time.
- If R is the victim, it fails at commit with a deadlock. That is not a carry error, so there is no downgrade re-run, and the report is answered `Unavailable`.
- The "Enforced by" tests are listed against the ADR, so they could be written to the wrong order.

**Fix**
- Reword decision 5 to "after the policy fences and before the usage row, holding no usage row while it waits".
- Replace the sentence at `:71` with "only a release waits for a lease, and it holds no usage row while it does".
- Make the T041-5 lock-order test hold a lease, start a release, and assert in `pg_stat_activity` that the release does not hold the usage row while it waits.

### Minor 1: The release's wait has no bound and can span tenants, which delays retention

**Evidence**
- The purge deletes "at most 500 lines per transaction" from `step_log_streams_removal_idx (tenant_id, stream_id)`, with no rule of one tenant per transaction (`docs/designs/041-step-logs.md:1230-1232`).
- The finalizer handles tenants `ORDER BY tenant_id` (`durable_quotas.up.sql:305-308`) and keeps each tenant's usage row `FOR UPDATE` until commit (`:348`).
- The release now waits for up to 256 leases per tenant inside `COMMIT`.
- ADR-0097 decision 5 (`:72`) says the delete route also releases. The design says the delete route only marks streams and the purge deletes (`041-step-logs.md:1229`, `:1240-1242`).

**Scenario**
1. A purge transaction holds lines of tenants A < B.
2. Its finalizer locks A's usage row, then waits for B's leases one at a time. Each wait is up to one B transaction's time from charge to commit.
3. Meanwhile A's completions with output and A's refills queue on A's row because of B's load.
4. Under sustained load, 256 waits in a row inside `COMMIT` can run past a pool `statement_timeout` (D041-110 cites 5 s). The purge then fails and retries.

**Impact**
- Cross-tenant latency coupling.
- Under sustained load the purge may never finish for a busy tenant, so personal data in step logs outlives its retention.

**Fix**
- Have the purge handle one tenant per transaction, as ADR-0027 line 57 allows.
- Give the release a bounded `lock_timeout` with a retry.
- Add a tenant-free metric for release waits.
- Make ADR-0097 and the design agree on whether the delete route deletes lines.

### Minor 2: The private lease table and the new functions are not revoked explicitly from `sagat_app`

**Evidence**
- `data-model.md:265` gives `step_log_quota_leases` "none" for `sagat_app`, and `:275-276` says "no `EXECUTE` to `sagat_app`". Both rely on absence, not on a `REVOKE`.
- The existing pattern revokes explicitly: `REVOKE ALL ON tenant_retained_usage, quota_transaction_deltas, quota_accounting_control FROM sagat_app` and the function list (`durable_quotas.up.sql:564-571`).

**Impact**
- In a deployment that set `ALTER DEFAULT PRIVILEGES … GRANT … ON TABLES/FUNCTIONS TO sagat_app`, the runtime role could write leases (a quota bypass, release-blocking under constitution V).
- It could also call the charge function with another tenant's id (cross-tenant quota exhaustion).

**Fix**
- Put `REVOKE ALL ON step_log_quota_leases FROM sagat_app` and `REVOKE ALL ON FUNCTION sagat_quota_step_log_charge(uuid,bigint), sagat_quota_step_log_refill(uuid,bigint,bigint) FROM sagat_app` in the `DO $grants$` block. Keep the unconditional `REVOKE … FROM PUBLIC`.
- In the T041-5 bypass tests, run as `sagat_app` and expect 42501 for: SELECT, INSERT, UPDATE and DELETE on the leases; `UPDATE tenant_retained_usage SET step_log_leased_bytes`; and EXECUTE of both functions.

### Minor 3: Privileged helpers take the tenant, the byte count and (for the refill) the limit as arguments

**Evidence**
- `data-model.md:231` defines `sagat_quota_step_log_charge(tenant, bytes)`, and `:236` defines `sagat_quota_step_log_refill(tenant, bytes, L)`. Both are `SECURITY DEFINER`.
- ADR-0027 line 53 requires "no runtime ability to … call a privileged mutation helper directly". Line 55 says "Do not reuse application-pre-read limits as authority".

**Impact**
- The revoke is the only guard. If EXECUTE ever leaks (see Minor 2), the refill grants room against whatever L the caller passes.
- A `pg_trigger_depth()` guard would not help: `sagat_app` can create a temporary table with a trigger that calls the function.

**Fix**
- Inline the charge and the refill in the statement trigger function, which cannot be called directly.
- Failing that, have the refill read L itself after the fence, which the transaction already holds.

### Minor 4: The security NFR and §4 of the data model still list only the earlier SECURITY DEFINER functions

**Evidence**
- `docs/designs/041-step-logs.md:1782-1785` (NFR-041-SEC): "no new `SECURITY DEFINER` function beyond the accessor … and the statement-level quota trigger function". The revision did not touch this.
- `data-model.md:267-268`: "The only new `SECURITY DEFINER` function is the accessor".
- `data-model.md:228` announces "Three" functions but lists two plus a branch of the finalizer. ADR-0097 `:111` also says "three functions".

**Impact**
- The binding security requirement now forbids what §3b requires.
- Whoever writes the NFR's check either fails on the lease functions or loosens the check without noticing.

**Fix**
- Name `sagat_quota_step_log_charge` and `sagat_quota_step_log_refill` in NFR-041-SEC and in §4's sentence.
- Correct "Three" to two functions plus the finalizer branch in both places.

### Minor 5: How the quota API reads the usage S + Σ `used` is not specified, and the current code reads S only

**Evidence**
- `data-model.md:248-249` and ADR-0097 `:77-78` say the quota API shows S + Σ `used`.
- `sagat_app` has no privilege on the lease table (`data-model.md:265`).
- `crates/infra-postgres/src/quotas.rs:1228-1235` reads `step_log_bytes` only.

**Impact**
- As built, the view would understate usage and move in lease-sized steps, while refusals already apply.
- The obvious fix, granting SELECT on the leases to `sagat_app`, would break §4.

**Fix**
- Read through a `SECURITY DEFINER` accessor: `sagat_quota_step_log_allowance` or a usage-only sibling, with the same pattern and an `EXECUTE` grant.
- Keep D041-066's gate (and G041-018's open residual) on the `used` value.
- Never expose `step_log_leased_bytes`. It reveals how fast a tenant logs.

### Nit 1: §3b names its objects without a schema under `search_path = pg_catalog, pg_temp`

**Evidence**
- `data-model.md:233` names `UPDATE step_log_quota_leases …`, and `:235` calls `sagat_quota_step_log_refill(…)`, both without `public.`.
- The existing finalizer qualifies every name: `public.tenant_retained_usage` and `public.quota_limits` (`durable_quotas.up.sql:317-348`).

**Impact**
- With that path, an unqualified *relation* that is not in `pg_catalog` resolves from `pg_temp`, which the runtime role can populate.
- Normal operation would fail first, so the real risk is someone "fixing" it by widening the `search_path`.

**Fix**
- Write `public.` names in §3b.
- Add a bypass test: `sagat_app` creates `TEMP TABLE step_log_quota_leases`, inserts lines, and the real lease is still charged.

### Nit 2: ADR-0097's "Enforced by" and "Evidence" overstate what the prototype proves

**Evidence**
- `step_log_spike.rs` `install_leases` creates plain functions (not `SECURITY DEFINER`) in `spike_041`, with no `REVOKE`.
- `step_log_leases_tests.rs` runs as the owning role.

**Impact**
- Decision 1 (privileges) has no evidence yet. The ADR's wording suggests it does.

**Fix**
- Say in the ADR that the prototype proves exactness and lock behaviour only, and that the privilege half waits for T041-5.

### Nit 3: Lease rows outlive their tenant

**Evidence**
- `step_log_quota_leases` has no foreign key (`data-model.md:211-219`).
- `tenant_retained_usage` cascades from `tenants` (`durable_quotas.up.sql:51`).

**Fix**
- Add `REFERENCES tenant_retained_usage (tenant_id) ON DELETE CASCADE`. The lease updates in the charge do not change the key, so this adds no `KEY SHARE` on the usage row.

### Nit 4: §3b does not say how the charge treats `step_log_enforcement_enabled`

**Evidence**
- The ADR-0027 Amendment defines the flag (`ADR-0027…:128`), and `sagat_quota_step_log_allowance` reports it (`:131`).
- §3b's charge never mentions it.

**Impact**
- The ingest's early check (T6) and the charge can disagree. This is not a security issue.

**Fix**
- State the charge's rule for the flag, the same one the allowance reports.

## Suspicions (not verified)

- **S1. COPY and MERGE insert paths.** `sagat_app` has INSERT on `step_log_lines`, so it can run `COPY … FROM STDIN` and `MERGE … WHEN NOT MATCHED THEN INSERT`. Check on the CI PostgreSQL that the statement-level `AFTER INSERT` trigger with `REFERENCING NEW TABLE` fires and charges on both paths. Add both to the runtime-role bypass tests of T041-5.
- **S2. A purge transaction that already holds the usage row.** If T041-10's purge transaction also purges log-class rows, or takes the usage row through `sagat_quota_lock_log_usage` earlier in the same transaction, then even the data model's order makes the release wait for leases while holding the usage row. That is Major 2's cycle again. Check when the purge is built.
- **S3. `statement_timeout` on the scheduler pool's COMMIT.** Minor 1's liveness concern depends on it. Measure the release's wait at 8 × 128 in T041-9.

## Classes checked with no finding

- **Tenant boundary: no finding.**
  - The charge's tenant comes from the stored rows: the transition table `inserted` grouped by `tenant_id` (`data-model.md:165`). The release uses `OLD.tenant_id`.
  - Every lease statement filters on `tenant_id` (`step_log_spike.rs:296`, `:346-356`).
  - Fences are per tenant (`quota-policy:<resource>:<tenant>`, `crates/infra-postgres/src/content_usage.rs:54-58`). One tenant cannot touch, read, starve or observe another's leases.
  - A tenant holding all 256 of its leases only falls back to its own usage row. `pg_backend_pid()` steers nothing that matters for exactness.
- **Information leaks through errors: no finding.** Refusals carry SQLSTATE 54000 and a constraint name, with no tenant id or byte values. `StepLogCarry` keeps only the SQLSTATE.
- **Exactness (I1 to I5) under concurrency: no finding.** I checked the charge, refill, reclaim, fold and the all-held fallback against a concurrent refill, reclaim and lowered limit. U ≤ S + G ≤ L holds. A lowered limit cannot slip between the charge's check and its commit, because the charge holds the step-log fence shared until commit.
- **Storage changes: no finding.** The streams' expiry index on `expires_day`, the fill factor of 70, and dropping the run index (reads go through the `tenant_id`-led attempt key) keep reads tenant-scoped and are no oracle.
- **Metrics: no finding.** The revision adds no metric. Any later lease, refill or release-wait metric must carry no tenant or namespace label (constitution VI).
- **Secrets and unsafe patterns in the prototype and tests: no finding.**
  - No credentials or secrets anywhere.
  - `failure()` prints the `Debug` of benchmark errors to stderr.
  - `SAGAT_PERF_PGLOG` keeps the disposable server's log of synthetic data. Statement text is logged without bound parameters by default.
  - The workflow's `SPIKE_DIAG` expression yields only `'1'` or `'0'`.
  - `GITHUB_REF_NAME` appears only in a `case` pattern.
  - `SAGAT_BENCH_PG_EXTRA` is set by the script itself.
  - The scratch triggers and columns on `public.tenant_retained_usage` exist only behind `perf-spike`, on a throwaway database.

## Count

- Blocker: 0
- Major: 2
- Minor: 5
- Nit: 4
- Suspicions: 3, not counted
