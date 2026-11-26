Doc: docs/designs/041-step-logs.md
Reviewed-commit: c1fc1206
Reviewer: security
Counts: Blocker 0, Major 0, Nit 5

All paths below are under `/Users/marcelo/Documents/Projects/sagat-wt-041/`. I read the diff, the spec sections you named and the surrounding code. I ran no tests, so the T041-4 evidence lines (30 passed as the runtime role, and so on) are the author's claims and I have not reproduced them. To reproduce them: `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_security_multitenancy -- quota step_log` and `cargo test -p sagat-domain --locked step_logs`.

## Classes checked with no finding

- **Tenant boundary (verified).** The new query binds the tenant only from `principal.tenant_id()`. It links `g.tenant_id = n.tenant_id AND g.namespace_id = n.id`, and the foreign key `payload_reveal_grants_namespace_same_tenant_fk` makes grant and namespace agree on tenant. The same subject string holding grants in a second tenant cannot satisfy the first tenant's check. In a namespace view, `tenant_step_log_bytes` (tenant scope) is skipped before the check runs, so it never reaches that view.
- **Admission and the oracle (verified).**
  - The permission check runs before any store call, so a viewer causes no database round trip.
  - The timing difference only tells callers about their own permission.
  - A missing membership row, a role that cannot use a grant, or a tenant with no namespace all answer false.
  - A store error fails the whole request rather than showing the usage.
  - `QuotaBody` serialises `usageWithheld` only when it is true, and `used` is absent then, which matches the `openapi.yaml` `Quota` schema.
  - The `set` and `clear` views go through the same check.
  - No refusal path (`classify`, `put_setting`) reports a usage figure.
- **Least privilege (verified).**
  - `sagat_app` has SELECT on `namespaces` (`20261008000200_runtime_role_grants.up.sql:20`) and SELECT on `payload_reveal_grants` (`20261202000000_reveal_grants.up.sql:28`).
  - The row lock goes through `sagat_lock_memberships`, which is SECURITY DEFINER with `search_path = pg_catalog, pg_temp` and EXECUTE granted only to `sagat_app` (`20261126000000_authority_read_locks.up.sql:4-35`).
  - The slice adds no new function, no new grant and no dynamic SQL.
- **Secrets and personal data (verified).** No new log line, audit record, metric label or error text. The usage stays out of responses for callers without the grants.
- **Carry validation (verified).**
  - `validate_carry` checks the length and byte bounds before it walks the records.
  - In every planned path, the total added to `Drops` equals `mark - last_sequence`, which is at most 2^62. So `Drops::add` and `by_server` cannot overflow.
  - The mark-only path only counts distinct sequences in (`last_sequence`, `highest`]. That keeps `sent` at or below `mark - last_sequence`, so the gap subtraction cannot underflow.
  - `HashSet` uses the default SipHash, so crafted sequences cannot cause hash flooding.
  - The work is bounded by the gRPC decode cap of 320 KiB (256 KiB plus 64 KiB, `crates/api-grpc/src/server.rs:14`), which is about 80k records at most.

## Nits (verified in the code)

1. **Nit: the fail-closed branches are untested, and their mutations would survive.**
   - Where: `crates/infra-postgres/src/reveal_grants.rs:80-87`; `tests/integration/security-multitenancy/quotas.rs:1338-1431`.
   - Scenario:
     - Removing `EXISTS (SELECT 1 FROM namespaces WHERE tenant_id = $1) AND` makes a tenant with no namespace show `used: 0` instead of `usageWithheld: true`, and no test fails.
     - Removing the `role_can_use_reveal_grant` check at :80-82 also passes. Its only protection is the role-change cascade (`20261220000000_reveal_grants_role_change.up.sql`), which deletes the grants of a demoted subject.
     - The case "the token still says Developer, the membership was demoted to Viewer" and the cross-tenant same-subject case are not tested either.
   - Fix: add three cases to the test:
     - a tenant with no namespace, which must be withheld;
     - a token role that holds `run:logs:read` while the membership role cannot use a grant;
     - subject S with grants in every namespace of tenant B, which must still be withheld in tenant A.

2. **Nit: the grant check and the usage read do not happen at the same moment.**
   - Where: `crates/application/src/quotas.rs:409` and `:417` (list), `:552` and `:560` (view); `crates/infra-postgres/src/quotas.rs:1265-1269`.
   - Scenario: `reveals_in_every_namespace` commits its own transaction without the grant-set fence. `store.usage` then reads on a different pool connection. If an admin's `revoke_reveal` commits between the two (they do not conflict: both lock memberships FOR SHARE, and the revoke's advisory lock is not waited on), the response still carries `used` for a subject that no longer holds the grant. That is one figure per race, for a subject who held the grant a moment earlier.
   - This is weaker than ADR-0085 rule 6's wording ("MUST [not] disclose ... how large it is, to a caller without both") and than D041-133's "read as `has_effective_payload_reveal_grant` reads them". So far the gap is accepted only in a code comment.
   - Fix (either one):
     - record it as a decision in `specs/041-step-logs/research.md`;
     - or read the usage inside the same transaction after `lock_grant_set_shared` for each namespace, which costs one advisory lock per namespace.

3. **Nit: `run:read` is not checked.**
   - Where: `crates/application/src/quotas.rs:665`.
   - Scenario: ADR-0085 rule 6 requires `run:read` and `run:logs:read`, but the check covers only `RunLogsRead`. This cannot be exploited today, because every role that holds `RunLogsRead` also holds `RunRead` (`crates/domain/src/identity.rs:630-688`). A later role that has the first without the second would see the usage.
   - Fix (either one):
     - add `&& principal.authorize(Permission::RunRead).is_ok()`;
     - or assert in the role-matrix test (`crates/domain/tests/identity.rs`) that `RunLogsRead` implies `RunRead`.

4. **Nit: `plan_batch` assumes valid input without checking it.**
   - Where: `crates/domain/src/step_logs.rs:607` (also :583 and :604).
   - Scenario: `gap = mark - stream.last_sequence - sent` relies on the caller having run `validate_batch`. The release profile has no `overflow-checks` (`Cargo.toml:94-96`). A batch with repeated sequences such as `[3,3,3,3]` on a new stream gives `sent = 4` and `mark = 3`. Without validation the subtraction wraps to about 2^64. The counters saturate at 2^53, but `Drops` carries about 1.8e19. A debug build panics instead.
   - Separately, a mark-only `CarryMark` whose `highest_sequence` is above `MAX_WORKER_SEQUENCE` would move `last_sequence` into the marker range (2^62 and up).
   - No caller exists in this slice (T041-6 and T041-36 add them), so neither case is reachable yet.
   - Fix: add `debug_assert!`s for both preconditions, and use `checked_sub` with a typed refusal or saturation. Better still, have `plan_batch` take a validated newtype that only `validate_batch` and `validate_carry` can build.

5. **Nit (suspicion, for T041-6 and T041-36): one tenant can inflate a platform-wide metric.**
   - Where: `crates/domain/src/step_logs.rs:293-326` (`Drops`, "before saturation"); data-model.md §7 (`step_log_lines_dropped_total{reason}`, no tenant label).
   - Scenario: a worker of any tenant sends a valid carry, or a batch, with `highest_sequence = 2^62 − 1`. That puts about 4.6e18 into `Drops[WorkerGap]` in one plan. If the application adds `plan.drops` to the counter as it stands, a few such reports from one tenant dominate the shared counter, or wrap a u64 counter. Operators then lose the signal for every tenant.
   - Fix: increase the metric by the saturated change of the stream counters (`after.dropped_* - before.dropped_*`), split by reason, not by the raw `Drops`.

## Suspicions not verified

None besides Nit 5.
