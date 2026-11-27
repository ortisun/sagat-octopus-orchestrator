# Security review of design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Reviewer: security (full round; the approval's security report is the re-read that follows the fixes)
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing. Line numbers are of the reviewed
commit; `mig` is `crates/infra-postgres/migrations`.

## Major

1. **Row locks need UPDATE, but the design grants none.** PostgreSQL requires UPDATE privilege for `FOR SHARE` and `FOR
   KEY SHARE`. Run creation reads `journal_bucket_switch FOR SHARE` (049:184-185), and the reference triggers take `FOR
   KEY SHARE` on `payloads`/`run_payloads` (049:135-136). `sagat_app` has only SELECT, INSERT there
   (mig/20261009000100_execution.up.sql:272; 049:112, 409). The obvious fix, granting UPDATE, would let the runtime
   rewrite the switch row. Fix: make the reference and bucket triggers SECURITY DEFINER with search_path `pg_catalog,
   pg_temp`, and serialise the switch with `pg_advisory_xact_lock_shared` (creation) against an exclusive advisory lock
   (switch); grant no UPDATE.
2. **An unattended job holds the full owner credential.** The chart holds no owner credential today
   (infra/helm/values.yaml:21). The design adds one in a Secret for a scheduled job (049:429-431, 527-529). Anyone who can
   read Secrets or create pods in that Kubernetes namespace gets owner rights: disable the ALWAYS triggers, read every
   tenant's data, alter audit. Fix: make `run_events`/`run_payloads` owned by a NOLOGIN `sagat_journal_owner`; give the job
   a login role that is a member of it, with EXECUTE only on the purge and release functions; use short-lived credentials;
   put the job in a separate namespace.
3. **The quota release trusts recorded amounts.** Step 5 records per-tenant bytes in `journal_retirements`; step 6
   releases them through `sagat_quota_release_bytes` (049:172-176, 443-446). Grants on `journal_retirements` are not
   specified. If the runtime can write it, it can inflate a release (a quota bypass, release-blocking). If it can read it,
   it sees every tenant's byte counts. Fix: no runtime grant (as `platform_requirement_history`,
   mig/20270113000000_instance_registry.up.sql:400); expose only "retirement active" through a definer function; have the
   release function sum `octet_length(content)` from the detached table itself and refuse a relation that is still
   attached.
4. **Erasing one run in a tree either survives or dangles.** `run_payloads` is keyed by the root (049:120, ADR:25-27). The
   purge deletes "by `(tenant_id, run_id, journal_bucket)`" (049:205-207), but `run_payloads` has no `run_id`. Erasure
   applies to one terminal run (specs/020-visibility-analytics-archival/spec.md:530-536). Erasing a child therefore leaves
   its content (personal data) in the root's rows; erasing the root deletes payloads its retained children still
   reference. Fix: define erasure and purge of a dated run as a whole-tree operation, or as a reference check within the
   tree; report surviving content as FR-020-037 requires.
5. **Content survives an abandoned retirement.** A per-run purge finishes a `CONTENT_PENDING_DROP` run "whenever no
   retirement of its bucket is active" (049:177-180). The retirement's lock is a session advisory lock, released if it
   crashes after a completed detach. Rows in a detached table are not reachable through the parent, so a later erasure
   deletes nothing and reports success. Fix: decide "active" from `journal_retirements` plus `pg_inherits`, never the
   advisory lock alone; a stopped retirement must reattach or drop before it releases the bucket; erasure finishes the
   drop or refuses.
6. **Purged runs stay readable until the drop.** The mark-only form writes the stub but leaves the `run_events` rows
   (`actor`, `detail`) until the drop (049:208-211). If the retirement stalls (049:231-232), that can be indefinitely,
   while the run already shows as purged. Fix: every journal read path refuses on `purged_at`/`archive_state`, with a test
   in the suite after the switch; the purge and erasure report says the content is deleted only after the drop commits.

## Nit

1. **Function grants are not explicit.** For every new function (`ensure_journal_buckets`, `sagat_quota_release_bytes`,
   the trigger functions): `REVOKE ALL ... FROM PUBLIC`, search_path `pg_catalog, pg_temp` with `public.`-qualified
   names, not `pg_catalog, public, pg_temp` (mig/20261017000100_active_run_counts.up.sql:33). Have Requirement 049-013's
   audit check `proacl` and `prosecdef` (049:148, 527).
2. **The reference triggers fire only on the digest column** (049:133). The foreign keys they replace also re-checked on a
   change of `tenant_id` or the run columns. Fix: `UPDATE OF <digest>, tenant_id, run_id, journal_bucket`.
3. **The `run_payloads` bucket check speaks of `run_id`** (049:102-105). Fix: require `root_run_id` to be a root
   (`parent_run_id IS NULL`) of the same tenant and bucket. Otherwise a row keyed by a child's id passes the check and is
   missed by the root's purge.
4. **Dropping the legacy partition has no procedure** (049:448-449): no check, no audit, no command. Fix: route it
   through `journal retire` with step 4 and the audit.
5. **The mark-only form must be feature 020's path.** It must run through `purge_run_content`, with its authorization row
   and the `run.purged` audit per run, and never `ALTER TABLE ... DISABLE TRIGGER` (049:205-211;
   specs/020-visibility-analytics-archival/data-model.md:200-212).
6. **Retirement audit placement is unspecified.** `audit_records.tenant_id` is NOT NULL
   (mig/20261008000000_identity.up.sql:57), so the global record must go in `journal_retirements`. Each per-tenant record
   should carry only that tenant's counts. The recorded actor should be the database user and the job's identity
   (049:172-174).
7. **The long-transaction report** (049:231-232) should print pid, user, application name and `xact_start` only, never
   `pg_stat_activity.query`.
8. **`--bucket` builds partition names** (049:361, 429). Parse it to a date, quote names with `%I`, and refuse the legacy
   bucket, the current month and future months.
9. **Trigger error messages** (23503, 23514) must carry no digest, tenant or run values. Digests of low-entropy personal
   data are a confirmation oracle in logs (049:137, 155).
10. **Suspicion: cloned triggers on new partitions.** Partitions attached by `ensure_journal_buckets()` may get cloned
    triggers that are not ENABLE ALWAYS, and the audit runs only at startup (049:150-154). Fix: set ALWAYS inside the
    function and assert it there.
11. **Advisory-lock denial of service.** The retirement and bucket advisory-lock keys can be taken by `sagat_app`
    (advisory locks need no privilege), which would stall retirement and `ensure_journal_buckets` (049:166-168). Fix:
    count and alert on the wait, and log who holds the lock.

Counts: Blocker 0, Major 6, Nit 11
