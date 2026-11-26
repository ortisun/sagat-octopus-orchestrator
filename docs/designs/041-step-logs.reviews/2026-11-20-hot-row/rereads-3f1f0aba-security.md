Re-read by the `security` reviewer of the fixes at `3f1f0aba` (answering its review of `4d74b27a`). Saved verbatim.

I have what I need: the resolution map, both fix diffs, the current ADR-0097, data-model §3b and §4, design 2b, the grants paragraph, NFR-041-SEC and T041-5, plus the store code the editor runs through. Here is the full re-read report.

# Security re-read: design 041 hot-row revision, fixes at 3f1f0aba

Both of my Major findings are closed. The fix adds one new Major: the reclaim that runs when the limit changes is assigned to a caller that cannot run it, and it depends on a fence it does not take itself. The new setting `sagat.step_log_charge_nowait` is safe.

Worktree: `/Users/marcelo/Documents/Projects/sagat-wt-041`; paths below are relative to it. Read:
- `docs/designs/041-step-logs.reviews/2026-11-20-hot-row/resolution.md`
- the fix diffs `fix-code.diff` and `fix-docs.diff`
- the current `docs/adr/ADR-0097-…md`
- `specs/041-step-logs/data-model.md` §3, §3b and §4
- `docs/designs/041-step-logs.md`: the grants paragraph, NFR-041-SEC, the purge and T041-5
- `crates/infra-postgres/src/quotas.rs` and `content_usage.rs`, the limit editor
- `crates/infra-postgres/migrations/20270201000000_step_log_quotas.up.sql`
- the prototype in `tests/performance/scalability-ha/step_log_spike.rs` and `step_log_leases_tests.rs`

## Status of my earlier findings

**Major 1 (a lowered limit strands lease room and refuses every charge): closed**, provided New Major 1 below is fixed.
- A charge that finds S + G > L now goes to the refill instead of raising:
  - `data-model.md:241` ("no row, or S + G > L: the refill")
  - `ADR-0097:81`
  - prototype `fix-code.diff:461-480`
- The refill reclaims idle leases whenever the free room is under max(n, 1 MiB), and that includes negative free room (prototype `fix-code.diff:425`).
- A limit or ceiling change reclaims every lease (`ADR-0097:94-97`, `data-model.md:251-253`).
- Test: `a_limit_lowered_above_the_usage_still_admits_what_fits`.
- I re-traced my scenario: lower the limit, purge to zero, then charge. The limit change leaves G = 0 and S = U. Each charge then finds no lease with room, so it refills, which grants under I4. Nothing stays stranded.

**Major 2 (the release waits for leases "under the usage row" and can deadlock): closed.**
- The release takes no lease. Under the usage row it reads Σ `used`, refuses when U would go below zero, and lowers S:
  - `ADR-0097:90-93`
  - `data-model.md:257-260`
  - prototype `fix-code.diff:482-494`
- The release takes the step-log fence shared, then the usage row. A refill needs the usage row, so it is serialized against the release. A reclaim holds the fence exclusively, so it is excluded.
- The Σ `used` the release reads can only be stale on the low side. Charges only raise `used`, and every decrease happens under the usage row. So the U ≥ 0 check is conservative.
- No transaction now waits for a lease: charges and refills only try them (`SKIP LOCKED`), the release only reads them, and the reclaim runs under the exclusive fence. The cycle I described cannot form.
- Test: `a_release_never_waits_for_a_lease_and_never_goes_below_zero`.

**My earlier Minors and Nits:**
- Taken:
  - one tenant per purge transaction (`041-step-logs.md:1258`)
  - explicit revokes (`data-model.md:286`, `:296-298`)
  - the refill reads L itself (`data-model.md:242`, prototype `fix-code.diff:411-412`)
  - NFR-041-SEC and the grants list updated (`041-step-logs.md:1817-1820`, `:1225-1230`)
  - the usage accessor `sagat_quota_step_log_usage`, with `step_log_leased_bytes` never shown (`data-model.md:254-256`)
  - names written `public.…` (`data-model.md:232`)
  - the leases' foreign key (`data-model.md:213`)
  - the enforcement switch (`data-model.md:228-229`)
  - the ADR's Evidence now says the prototype proves no privileges (`ADR-0097:32-33`)
- Not taken: inlining the charge into the trigger function. That is acceptable, because the explicit revokes and the refill reading L itself close the same risk.

## New findings

### Major 1: The limit-change reclaim is given to a caller that may not execute it, and it relies on a fence it does not take

**Evidence**
- The reclaim is a separate `SECURITY DEFINER` function, revoked from `sagat_app`:
  - `data-model.md:251-253`: "`sagat_quota_step_log_reclaim(tenant)` … Called by the change of `tenant_step_log_bytes` or its ceiling, which holds the step-log fence exclusively"
  - `data-model.md:296-297`: "`sagat_quota_step_log_charge`, `sagat_quota_step_log_refill` and `sagat_quota_step_log_reclaim` (`SECURITY DEFINER` … `FROM sagat_app`)"
  - `041-step-logs.md:1228-1229`: "executable by neither `PUBLIC` nor `sagat_app`, revoked explicitly"
- T041-5 has the store call it: `041-step-logs.md:2229-2231` modifies `crates/infra-postgres/src/quotas.rs` for "the change of that limit calling `sagat_quota_step_log_reclaim` under its exclusive fence".
- That editor runs as the runtime role, in a transaction of its own pool: `quotas.rs:905` (`self.pool.begin_safely()`), with the fence taken at `:913` and the writes at `:920-936` and `:1030-1061`.
- In the prototype, the reclaim is instead an unguarded row trigger on `quota_limits` and `quota_ceilings`. It takes no fence and locks no lease before it sums, and it sums and zeroes in two statements (`fix-code.diff:497-514`):
  ```sql
  PERFORM 1 FROM public.tenant_retained_usage WHERE tenant_id = v_tenant FOR UPDATE;
  SELECT coalesce(sum(used), 0)::bigint INTO v_used FROM spike_041.step_log_quota_leases WHERE tenant_id = v_tenant;
  UPDATE public.tenant_retained_usage SET step_log_bytes = step_log_bytes + v_used, step_log_leased_bytes = 0 …;
  UPDATE spike_041.step_log_quota_leases SET granted = 0, used = 0 WHERE tenant_id = v_tenant;
  ```

**Scenario**
- As specified, T041-5 cannot be built: `sagat_app` gets "permission denied" on the reclaim.
- The obvious way out is to grant EXECUTE. That gives the runtime role a callable `reclaim(tenant)` for any tenant, with no check that the fence is held.
- The prototype's way out is a trigger. It runs on any write to a `tenant_step_log_bytes` row of `quota_limits` or `quota_ceilings`, whatever fence that write holds.
- On any call without the exclusive fence, the following happens:
  1. A charge C of n bytes is inside lease 3 and uncommitted.
  2. The reclaim sums `used`, which does not include C's n.
  3. C commits.
  4. The reclaim's `UPDATE … SET used = 0` waited for lease 3's row lock. It now re-reads the row and zeroes it.
  5. C's n bytes are stored and counted nowhere, so U is understated by n for good. The tenant can store n more bytes than its limit: a quota bypass, which is release-blocking under constitution V.
  6. Later, the purge of C's lines finds U would drop below zero and raises an underflow. That tenant's purge then fails on every pass, so its step logs outlive their retention.

**Fix**
- Make the reclaim an `ENABLE ALWAYS`, `SECURITY DEFINER` row trigger on `quota_limits` and `quota_ceilings`, for `resource = 'tenant_step_log_bytes'`, with no EXECUTE for anyone.
- The trigger itself takes `pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || tenant, 0))`. This is re-entrant when the editor already holds it, and waits when it does not.
- Then it locks the usage row, locks the tenant's leases `FOR UPDATE`, and folds and zeroes them, so it never depends on its caller.
- Remove "`quotas.rs` … calling `sagat_quota_step_log_reclaim`" from T041-5, and correct `data-model.md:233` ("the limit editor") to match.
- Add a T041-5 test: with a lease charge held by an open transaction, write `quota_limits` for the tenant without the editor's fence (as the owner). The write waits for the fence; after both commit, S + Σ `used` equals what the stored rows measure.

### Minor 1: T041-4's non-negative CHECK on S is still in its migration, and the release now needs S below zero

**Evidence**
- The constraint is still there: `crates/infra-postgres/migrations/20270201000000_step_log_quotas.up.sql:100`: `ADD CONSTRAINT tenant_retained_usage_step_log_nonnegative CHECK (step_log_bytes >= 0) NOT VALID`.
- Its removal exists only as prose: `041-step-logs.md:2192` and `:2231-2232` ("dropped before it merges"), and `data-model.md:210-211`.
- The prototype drops the constraint (`fix-code.diff:381`), so its tests cannot catch a migration that keeps it.

**Impact**
- If T041-4 merges as it is, every release that takes S below zero violates the CHECK (23514). That happens whenever bytes charged in leases are purged, which is routine.
- The purge transaction then fails on every pass for that tenant, so step logs, which can hold personal data, stay past their retention.

**Fix**
- Remove the constraint from the T041-4 migration file now; it has not merged (D041-091).
- Add a T041-5 test against the real migrations: charge into a lease, purge those lines, and assert the release commits with S < 0 and U = 0.

### Nit 1: T041-5's checks lag the revision

- The source-text check at `041-step-logs.md:2200-2201` still reads "only the refill and the release branch lock `tenant_retained_usage` for the class `step_log`". It omits the limit-change reclaim, which locks that row (`data-model.md:251`; `ADR-0097:28-29` includes it). Written as it reads, the check fails, or gets loosened without notice.
- `041-step-logs.md:2214-2215` says no function is executable by `sagat_app` "except `sagat_quota_step_log_allowance`". But `sagat_quota_step_log_usage` is granted (`data-model.md:254`, `:297`).
- Fix: add the reclaim, wherever it lives after New Major 1, to the source-text check, and add `sagat_quota_step_log_usage` to the allowed list.

### Nit 2: The prototype carry catches every SQLSTATE 54000 as a quota refusal

**Evidence**
- `fix-code.diff:652`: `WHEN program_limit_exceeded THEN … ceiling_reason … 'tenant_quota' … RETURN 'mark_only'`.
- The design gives each raise a constraint name of its own (`041-step-logs.md:2181`, `data-model.md:249-250`).

**Impact**
- Any other 54000 inside the block is recorded as the tenant's quota and counted `tenant_quota`. Instead of failing, the carry reports a false reason.
- This is telemetry integrity, not a bypass.

**Fix**
- In the built `sagat_step_log_carry`, read `GET STACKED DIAGNOSTICS … CONSTRAINT_NAME`. Answer `mark_only` only for `tenant_step_log_quota_exceeded` and `tenant_step_log_quota_held`, and re-raise anything else.

## The new caller-settable input `sagat.step_log_charge_nowait`: safe

- **Who can set it.** Any session can set a custom (dotted) setting without privilege, including `sagat_app`, connection options, or `ALTER ROLE`.
- **What reads it.** Only the refill reads it, and only to choose between `FOR UPDATE` and `FOR UPDATE NOWAIT` on the usage row (`data-model.md:243`, prototype `fix-code.diff:418-424`).
- **It decides nothing about quota.** L is read before the setting is consulted (`fix-code.diff:412`). S, G, the reclaim, the grant, I4 and both refusals do not depend on it. ADR-0027 line 53 forbids a "caller-controlled session flag [that] authorizes accounting bypass"; this flag authorizes none.
- **`on` only makes things fail sooner.** Set it on its own transaction and a caller's own refill fails sooner with 55P03. That is an earlier refusal or `contended`, never a charge skipped or another tenant affected.
- **`off` cannot be forced on a carry.** The carry sets the value itself, transaction-local, before its charge (`fix-code.diff:532`). That overrides whatever the caller set.
- **No leak through the connection pool.** `set_config(…, true)` reverts at the end of the transaction, rollback included, so the next transaction on a pooled connection never sees it.
- **Residual risk.** If some code or a connection string set it at session level, ingests would fail fast instead of waiting their 200 ms for a refill. That is availability only.
- **Optional hardening.** State in §3b that the store only ever sets it transaction-local inside `sagat_step_log_carry`. Make sure the ingest maps a 55P03 from its refill to `RETRY_LATER`.

## Classes re-checked with no finding

- **Tenant boundary.** The new functions and the prototype filter every statement on the tenant, and the charge's tenant still comes from `inserted`. The new accessor `sagat_quota_step_log_usage(tenant)` exposes nothing the runtime role could not already read: it has `SELECT` on every tenant's usage row (`durable_quotas.up.sql:566`). The application must pass the principal's tenant and keep D041-066's gate (G041-018 is still open, unchanged).
- **Deadlocks.** None in the new lock order. The limit editor takes the step-log fence exclusively, then the usage row, then the leases. Every lease holder holds that fence shared, so it cannot interleave. A content-only finalizer holding the usage row waits for nothing on the step-log side.
- **Errors and metrics.** The new raises carry constraint names only. The metrics `step_log_quota_refills_total{kind}`, `step_log_quota_refusals_total{cause}` and the gauge `step_log_quota_lease_rows` have closed labels and no tenant (`data-model.md:266-268`, `ADR-0097:102-104`).
- **Test code.** The new prototype tests and helpers hold no secrets and no unsafe patterns. Their scratch trigger without a `search_path` is test-only, on the throwaway database.

## Count

- Blocker: 0
- Major: 1 (new); both earlier Majors closed
- Minor: 1 (new)
- Nit: 2 (new)
