# Security re-read of the fixes of the re-reads (2026-11-26, commit `9960a3c1`)

Short re-read of the security fixes alone, by the security reviewer, against
`design-fixes-rereview-security-reviewer.md` and `design-fixes-responses.md`.

Verdict: all twelve earlier items fixed (seven new findings, the four partly fixed items, the suspicion); no Blocker, no
Major. `Principal::authorize` refuses every namespace-scoped permission to a bound principal, so a route that forgets the
scope refuses instead of admitting. Six minors and four nits on the backstops remain, each to be fixed (OD-2026-11-25-2).

Checked in the code and found closed: the blob route of the original scenario (`crates/application/src/runs.rs:978-979`,
`:3421-3427`; `crates/api-http/src/blobs.rs:118-122`); `admit` (`crates/application/src/api_admission.rs:274-290`,
`:298-302`); the replay after a revocation (membership read again on every request, `crates/api-http/src/extract.rs:52-65`,
`crates/application/src/identity.rs:255`, `crates/infra-postgres/src/identity.rs:362-377`; namespace resolved at `runs.rs:1391`
before every receipt lookup, `:1424`, `:1699`, `:1768`); slot 4a before slot 6 covers both orders of a revocation and a write;
schedule firing, backfill, the batch executor, worker routes and the event stream (none fails open); switching the capability
off fails closed (`migrations/20270113000000_instance_registry.up.sql:404`); only provisioning writes memberships
(`migrations/20261008000200_runtime_role_grants.up.sql:19`).

## New findings

1. **minor: a role change leaves the bindings in place**, so they return without a new audited bind (the foreign key to
   `memberships` cascades on DELETE only; the platform fixed this for reveal grants,
   `migrations/20261220000000_reveal_grants_role_change.up.sql:1-5`, `:22-27`). Fix: an `AFTER UPDATE OF role ... WHEN
   (OLD.role IS DISTINCT FROM NEW.role)` trigger deleting the subject's bindings; `bindRunRecovery` reads the role through
   `sagat_lock_memberships` `FOR SHARE` in its own transaction.
2. **minor: the fitness rule T022-258 matches the four permission names literally**; a role-only check written with a variable
   escapes it (`crates/infra-postgres/src/execution/resets.rs:675`, `crates/application/src/batch_executor.rs:443`,
   `crates/application/src/payload_disclosure.rs:625`, `crates/infra-postgres/src/database.rs:44`, and the natural rewrite of
   `crates/application/src/runs/pending_reset.rs:486-488`); `Membership::restore` is public (one production caller,
   `crates/infra-postgres/src/identity.rs:683`); the resolvers' role-only method is not named. Fix: refuse every `.grants(` outside
   an allow-list by enclosing function; refuse `Membership::restore(` outside `crates/infra-postgres/src/identity.rs`; name the
   role-only method and restrict its callers.
3. **minor: the pending reset completion is not covered.** `complete_reset` calls `authorize` (`pending_reset.rs:486-488`), which
   for a bound principal becomes `Forbidden`, mapped to `AUTHORITY_REVOKED` (`reset_completion.rs:225`), so every `RunRecovery`
   reset fails; converted to `authorize_in`, the opaque `NotFound` maps to `ADMISSION_REFUSED` (`:255`); `recheck_requester`
   checks only the stored role (`resets.rs:655-681`). Fix: `authorize_in(entry.namespace_id, …)` in `complete_reset`, the scope
   refusal mapped to `AUTHORITY_REVOKED`, the binding read `FOR SHARE` in `recheck_requester`; evidence in T022-259 (a reset in A
   completes; revoked before the source ends, it fails `AUTHORITY_REVOKED`).
4. **minor: the battery T022-257 accepts "403 or the opaque 404"**, so it cannot catch an existence check (the cross-tenant
   battery requires the exact body of a missing resource, `tests/contract/security-multitenancy/rest/battery.rs:7-8`). Fix: for
   operations whose permissions the role holds, require status and body equal to those for an identifier that does not exist.
5. **minor: the activation trigger's function attributes are unspecified** (D022-276); the writer of memberships is not the owner
   (`reveal_grants_role_change.up.sql:7-8`) and `platform_requirements` is revoked from PUBLIC (`instance_registry.up.sql:385-404`).
   Fix: SECURITY DEFINER, `SET search_path = pg_catalog, pg_temp`, qualified names, `REVOKE ALL ... FROM PUBLIC`, `ENABLE
   ALWAYS`, as in the precedent.
6. **minor: merge order could open a window.** The role enters the catalog in slice 2 (`tasks.md:903`, `:1095`); T022-253 (the
   CHECK and the trigger) comes before T022-256 in slice 13 (`tasks.md:1129`); the capability manifest (D022-256) does not list
   the scope artefacts. Fix: land the CHECK widening and the trigger with or after T022-256; name the T022-256 tests and the
   T022-257 battery in the manifest of `failure-handler-v1`.

## Nits

1. T022-256 does not pin the scope class of the role's four permissions: a domain test that every permission a namespace-bound
   role holds is classed namespace.
2. The revised `404` row in `contracts/errors.md` says "before any other refusal of the route", but the resolvers check the role
   first: reword.
3. The denial audit is written only when the namespace or resource exists, so response time tells existing slugs (as
   `record_cross_tenant` today): write the audit off the response path, or an equal write for a miss.
4. `tasks.md:1073` (T022-228's amendment of 2026-11-25, D022-241, reversed by D022-242) still says "machine identity".

## Suspicion (not checked)

The event stream's maximum lifetime sets the revocation window for reads (`crates/api-http/src/routes/execution.rs:1304`;
D022-277 cites `:1276`, a stale line): state the value in D022-277.
