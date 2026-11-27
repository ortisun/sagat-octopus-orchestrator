# Architecture review of design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Reviewer: architecture (full round; the approval's architecture report is the re-read that follows the fixes)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing.

## Blocker

1. Reading the unlogged `server_incarnation` in `after_connect` fails on every hot standby (no query on an unlogged table
   during recovery), so the read pool and feature 032's passive processes never connect, and a demoted primary shows as
   an error. Fix: a function that returns a null token in recovery; exempt the read pool and passive processes; a test
   on a standby.
2. Feature 032 relies on a mechanism design 048 rejects: its plan drops ADR-0067 decision 6(b) (a waited commit per
   replay) because "design 048 makes every new connection commit a waited beacon", which 048 rejects, while ADR-0130
   rejects per-replay waits and supersedes nothing. Fix: ADR-0130 supersedes ADR-0067 decisions 6(b)/(d) in part with
   the residual argued for `sync_quorum`, and FR-032-009 and T032-058 follow; or 032 keeps the replay commits.

## Major

3. The unlogged table contradicts ADR-0067 ("no unlogged tables"), D1587 and T032-009's contract test. Fix: ADR-0130
   amends that bullet; T032-009 allows it by name.
4. The lease arithmetic is wrong: a failed heartbeat is retried for about 1.4 s and then waits for the next beat, so the
   tolerance is about TTL − 2 × 10 = 10 s, not 20 s. Fix: state 10 s, or retry a heartbeat until the lease's remainder.
5. Class 40 includes 40003; ADR-0078's classifier accepts 23, 40001 and 40P01; ADR-0078 and ADR-0014 are not listed as
   refined. Fix: reuse `confirmed_abort`; list both ADRs.
6. sqlx 0.9 gives neither `COMMIT`'s command tag nor the transaction's `PoolConnection`. Fix: raw `BEGIN`/`COMMIT` on an
   owned connection, or a vendored `sqlx-postgres` patch; or drop the tag.
7. The interplay with the registry is unspecified: whether its connection is a writer connection, how `registry_error`
   classifies an unknown outcome and an `after_connect` refusal, and whether confirmation or registration comes first.
8. `DATABASE_UNCONFIRMED` with `Retry-After: 2` is added by no task to the OpenAPI reasons, the gRPC error value or
   `sdks/SPEC.md`.
9. Duplicates with feature 032: `DURABILITY_UNCONFIRMED` against `DATABASE_UNCONFIRMED`; `SAGAT_COMMIT_DEADLINE_MS`
   against `SAGAT_DB_COMMIT_DEADLINE_MS`; T032-024's second commit guard and metric. Fix: one name each; T032-024 uses
   Task 048-6.
10. `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY=false` does not choose the preferred mode; ADR-0130 decision 1 states no
    exception; nothing logs or exports the loss of RPO 0 as the Delivery Constraints require; the alert says "writes
    stopped" in a mode where they are not.
11. An autocommit write on the pool bypasses `commit_safely`; the planned check catches only strings that call writing
    functions. Fix: in the test profile, writer pools default to read-only and the helpers send `BEGIN READ WRITE`.
12. The application crate has no sqlx, so the confirmation needs a port; no type owns the identity state shared by the
    pool hooks and the gate; one-shot commands and test fixtures get pools that refuse until confirmed.

## Nit

13. `begin_safely` does not record the mode: read-only is set afterwards, and `classify` has no transaction context.
14. `durability_beacons` is unbounded; delete on deregister and cap it.
15. Concurrent first calls of `sagat_server_identity()` collide on `id = 1`; use `ON CONFLICT DO NOTHING`.
16. Scope the architecture rules (`pg_terminate_backend` in about 20 test files; the application port's `tx.commit()`);
    scan migrations for `synchronous_commit` and `set_config`.
17. ADR-0130's "Enforced by" lists unbuilt tests without "pending"; its Context says 1.4 s, the design 3 s.
18. Design 053 depends on Tasks 048-7 and 048-8, unnamed in its build order.
19. The verification plan runs feature 010's fault suite on every pull request, against the selection rule; state the
    Compose drill's job timeout (180 minutes).
20. Say that the reveal and redaction decisions are taken on the primary with the authorization.

Counts: Blocker 2, Major 10, Nit 8
