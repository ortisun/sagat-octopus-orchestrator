Doc: docs/designs/041-step-logs.md
Reviewed-commit: c07372fd
Reviewer: security
Counts: Blocker 0, Major 0, Nit 3

I found no Blocker or Major, and three Nits. One statement is one snapshot here, and nothing fails open. The test catches a regression back to two reads, but it asserts less than the design promises.

## Verified in the code

**Tenant boundary: no finding.** `crates/infra-postgres/src/reveal_grants.rs:85-92` filters the membership by `$1`/`$2`, where `$1` is `principal.tenant_id()`. The namespace list, the grant rows and the usage are all tied to `m.tenant_id`, and the grant subquery also matches `g.tenant_id = n.tenant_id`. Usage and grants from tenant B cannot reach a caller in tenant A. A member of both tenants is judged only against the tenant on their token, and the "stranger" case in the first test pins this.

**Fail-closed paths: no finding.**
- No membership: no row, so `None`.
- No namespace: `EXISTS(...)` is false, so `None`. `EXISTS` is never NULL, so the decoded bool cannot be absent.
- A membership role without `payload:read-internal`: `None` (line 105).
- A role that fails to parse, or a negative usage: an error, and `?` at `crates/application/src/quotas.rs:646` fails the whole `list` or `view`.
- A token without `run:read` or `run:logs:read`: no query runs, and the usage is withheld.

Every role that can use a grant (admin, developer, operator) also holds `run:read` and `run:logs:read` (`crates/domain/src/identity.rs:630-688`), so the membership check and the token check cannot disagree in a way that opens anything. `Principal::authorize` has no side effects, so calling it with `.is_ok()` writes no denial audit. The `list` loop skips tenant-scoped resources in a namespace view, so `TenantStepLogBytes` is judged only by this path.

**One snapshot under READ COMMITTED, including after a lock wait: no finding.** The statement calls no functions, so all its subqueries share one snapshot. PostgreSQL takes relation locks during parse or analysis, or when a cached plan is revalidated in Bind. It takes the execution snapshot afterwards, in `PortalStart` (with `InvalidSnapshot`, so a fresh one under READ COMMITTED). So a statement that waited behind `ACCESS EXCLUSIVE` sees whatever committed during the wait, as D041-141 claims. In production the statement takes no row or relation locks that a revoke conflicts with, so the property is plain MVCC.

**Dropping `FOR SHARE` on the membership: no finding.**
- Nothing is written or disclosed after the read, unlike `has_effective_payload_reveal_grant`, which must keep its fence until the disclosure audit commits.
- A role change and its grant deletion happen in one transaction (`20261220000000_reveal_grants_role_change.up.sql:22-27`). A membership delete cascades to the grants (`20261202000000_reveal_grants.up.sql:20-22`). The snapshot sees either state whole.
- With the old lock, the database commits would be ordered, but the response would still reach the client after the change. The lock added no real security.
- The window that remains is inherent and accepted by the contract wording: a statement whose snapshot was taken before a revoke commits can return its answer after the commit.

**Runtime-role privilege: no finding.** `sagat_app` has SELECT on:
- `memberships` (`20261008000200_runtime_role_grants.up.sql:19`)
- `namespaces` (line 20)
- `payload_reveal_grants` (`20261202000000_reveal_grants.up.sql:28`)
- `tenant_retained_usage` (`20261024000000_durable_quotas.up.sql:566`)

The statement uses no row locks, no SECURITY DEFINER function and no new grant. None of these tables has row-level security.

**Injection and bounds: no finding.** All inputs are bound as parameters. The anti-join is bounded by the tenant's namespaces and uses the primary key of `payload_reveal_grants`.

**Does the test catch a regression to two reads? Yes, structurally.** The old grant read touches only `memberships` (`FOR SHARE`, which does not conflict with the test's `DELETE`) and the grants table. So it would finish with the grant still visible. The separate usage read would then block on the `ACCESS EXCLUSIVE` lock and read 7,777 after the commit, which the `assert_ne!` at `tests/integration/security-multitenancy/quotas.rs:871-875` rejects. The detection pattern `'%step_log%'` matches only the usage statement: the revoke session is idle in transaction, and the observer excludes its own pid. If the view blocks anywhere else, the test times out and fails rather than passing.

## Findings

**Nit 1: the test accepts a weaker answer than the design promises.** `tests/integration/security-multitenancy/quotas.rs:876-879` accepts `(Some(0), false)` or `(None, true)`. D041-141 and T041-39 (`docs/designs/041-step-logs.md:3594-3595`) say "the view withholds". With the snapshot taken after the lock wait, only `(None, true)` is reachable. As written, the test would also pass a regression that keeps two reads but opens one REPEATABLE READ transaction before the wait. That regression is safe, but it contradicts the claim the documents rest on. Fix: replace both assertions with `assert_eq!(answer, (None, true), ...)`, or reword D041-141 and T041-39 to "together or neither".

**Nit 2: an unguarded usage read is still in the store.** `crates/infra-postgres/src/quotas.rs:1236-1243`: `QuotaStore::usage(TenantStepLogBytes)` still returns the raw figure with no grant check. Today it cannot be reached, because `used` branches off first (`crates/application/src/quotas.rs:640`). Any future caller of `store.usage` (another view, an exporter, a Studio route) would bypass D041-133 without a test noticing. Fix: make that arm return `StoreError::Internal` ("read through step_log_usage_if_revealed") and add one unit or integration test that pins it.

**Nit 3: a doc comment cites the wrong test name.** At `crates/infra-postgres/src/reveal_grants.rs:69-79`, "each read as `has_effective_payload_reveal_grant` reads it" is accurate apart from the fence. But the sentence about the role-change trigger names it as if it were a test (`memberships_role_change_revokes_reveal_grants`); it is the trigger. This is cosmetic, and only matters if you want the reference to say exactly what it is.

## Suspicions and claims I could not check

- **"Seen failing on the two-transaction read"** (D041-141). I cannot run git or tests. To confirm it, apply the new test to the code of `28fe969e` (`git -C /Users/marcelo/Documents/Projects/sagat-wt-041 show 28fe969e:crates/application/src/quotas.rs` and the two infra files) and run `cargo test -p sagat-tests --test integration_security_multitenancy quotas::a_view_that_reads_after_a_revoke_commits_never_shows_the_step_log_usage`. It should fail with the "shown with the grant from before it" message.
- **Passes as the runtime role** (T041-39's done condition). Run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_security_multitenancy quotas::` and the same command without the variable. Under the runtime role, the wait detection works only if the owner pool's role can see `sagat_app` sessions in `pg_stat_activity` (superuser or `pg_read_all_stats`). If it cannot, the test times out loudly; it does not pass falsely.
- **Out of scope, not a defect of this slice:** if `tenant_log_bytes` usage, which viewers can read, includes step-log bytes, it remains an aggregate channel. G041-015 and ADR-0085 rule 6 record this as accepted.

Files reviewed:
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/reveal_grants.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/application/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/tests/integration/security-multitenancy/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/migrations/20261126000000_authority_read_locks.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/migrations/20261220000000_reveal_grants_role_change.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/migrations/20261202000000_reveal_grants.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/migrations/20261008000200_runtime_role_grants.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/contracts.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/docs/designs/041-step-logs.md
