Doc: docs/designs/041-step-logs.md
Reviewed-commit: 7d34e23b
Reviewer: correctness
Counts: Blocker 0, Major 0, Nit 1

I re-reviewed head 7d34e23b by reading the files; I could not run git or tests. There is no new Blocker or Major. All eight findings are resolved in code or in the records, and M1 leaves one documentation Nit behind.

**Callers of `QuotaStore::usage(…, TenantStepLogBytes)`.** No production path or existing test hits the new refusal.
- The only production caller is `crates/application/src/quotas.rs:655`. It runs only after the `TenantStepLogBytes` branch has returned at `:640-651`. `list` and `clear` both go through `used`.
- `PgRunStore` (`crates/infra-postgres/src/quotas.rs:745`) is the only implementation.
- The other repository callers pass fixed resources: `secrets.rs:237`, `trigger_schedules.rs:404` and `tests/performance/security-multitenancy/main.rs:243`. The one other loop over `QuotaResource::ALL` (`database/policy.rs:62`) reads no usage.
- The only call with this resource is the new test, `security-multitenancy/quotas.rs:896`.

**Status of each finding**

| Finding | Status |
|---|---|
| M1, hand-off to T041-5 | Resolved in D041-141 (a), in T041-39 (`041-step-logs.md:3592-3595`) and in data-model.md §3b (`:355-359`, `STABLE`, the call inside the one `SELECT`). One residual Nit, below. |
| N1, negative usage fails the view | Resolved as a decision, D041-141 (c). Nothing writes `step_log_bytes` yet: the migration only adds the column with default 0. |
| N2 and security Nit 2, unguarded usage read | Resolved. `quotas.rs:1237-1241` refuses with `StoreError::Internal`, the same way the admission-policy arm at `:1310` does, and `the_store_refuses_the_step_log_usage_without_the_grant` pins it. |
| N3, waiting predicate | Resolved. `:813-816` reads `pg_backend_pid()` on the revoke's own transaction connection, and `:859` adds `$1 = ANY (pg_blocking_pids(pid))`. |
| N4 and security Nit 1, "withholds" wording | Resolved. The test comment (`:772-778`), D041-141 and T041-39 now say "either consistent pair". Where contracts.md:289-290 and the runbook (`:125`) still say "withholds", they mean a read made after the commit, which is correct. |
| security Nit 3, trigger named as if a test | Resolved at `reveal_grants.rs:77`. |

**New finding**

- **Nit, the residual of M1** (`docs/designs/041-step-logs.md:2614`, `:2690-2697`, `:2730`). The hand-off was written into T041-39, D041-141 and §3b, but not into the text of T041-5, which is what its implementer reads:
  - `:2614` still says "the quota readers of `crates/infra-postgres/src/quotas.rs` (`:1228-1235` read S today) moved to `sagat_quota_step_log_usage`". After this change, `quotas.rs:1227-1234` is the `TenantLogBytes` arm, so the line reference now points at the wrong reader. Read literally, the sentence also invites replacing the new refusal at `:1237` with a call to the function, which would undo D041-141 (b).
  - The privilege-audit bullet at `:2690-2697` has no `provolatile = 's'` check.
  - The Files list at `:2730` leaves out `reveal_grants.rs`.

  Tests catch every wrong path this could lead to:
  - Undoing the refusal fails `the_store_refuses_…`.
  - Leaving the raw subquery in place fails the planned source-text check at `:2703-2706` ("no Rust source … reads `step_log_bytes` … except through `sagat_quota_step_log_usage`").
  - Declaring the function `VOLATILE` fails the race test with `(Some(7777), false)`.

  That is why I rate it a Nit. Smallest fix: rewrite the sentence at `:2614` to name `reveal_grants.rs` and point to D041-141 (a), add `provolatile = 's'` for `sagat_quota_step_log_usage` to the privilege-audit bullet, and add `reveal_grants.rs` to the Files list.

**Evidence still open.** T041-39 is not ticked. Its done condition needs these two runs, and I could not run them:
```
cargo test -p sagat-tests --test integration_security_multitenancy quotas::
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_security_multitenancy quotas::
```
The resolution's statements that the tests were "seen failing" (the race test on the two-statement read under the tightened predicate; the refusal test reading 0) are claims I have not checked.

Files:
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/reveal_grants.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/application/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/tests/integration/security-multitenancy/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/docs/designs/041-step-logs.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/data-model.md
