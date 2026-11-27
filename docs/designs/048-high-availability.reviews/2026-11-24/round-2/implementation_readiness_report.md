# Implementation readiness review of design 048, round 2 (2026-11-24, stage 9 of egm-review)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: 70c9f3f7464087d31e7cc0dc62966570be38f6f1
Reviewer: egm-implementation-readiness
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer confirmed the
design's factual claims (the pool's settings and missing hooks, `25006` as internal, sqlx 0.9, the lease and keeper
defaults, `ScheduleAnyway`, the single PostgreSQL, the drill's constant, the SDK retry settings, the upgrade text, the
test targets and CI jobs, the 0.2.0 upgrade script, six SDKs, `binary_version`, the arithmetic).

## Blockers

1. No mechanism for a client bound on every statement: `begin_safely` only moves `BEGIN` into a task; statements run on
   borrowed executors at about 233 commit sites in 61 files; how keepalive is set in sqlx is not said.
2. The commit's bound contradicts itself (timeout 0, and no bound at 0); its value is not given; no inventory or mechanism
   for "every transaction"; autocommit writes; where `client_connection_check_interval` is set.
3. "A process MUST stay ready" contradicts Architecture decision 0082 decisions 3 and 5 and the code (`health.rs`,
   `apps/execution-engine/src/lib.rs`).

## Majors

4. `57P04` today is retryable through class 57; `registry_error` in `instances.rs` maps only 08 and 57P01-03 (a
   read-only primary would close the gate after 30 s); `scoped_leaf_finish.rs` maps everything to internal; how a `25006`
   connection is closed is not chosen.
5. Which pools run the recovery check, beacon and continuity check (request, registry, migration, read); the migration
   pool would fail on a fresh database.
6. `durability_beacons` has no schema, cleanup or grants.
7. The extension names no tables (steps, script build and test jobs, connector and engine leases), has no schema for
   `database_continuity`, no function or grants, unclear batching and timeouts, no keeper port changes; the fixed 6 s
   threshold ignores a keeper round of up to 120 s; with no engine the row is never written.
8. The error type needs a variant, its word in `kind()`, an arm in `record()`, a reason on `AppError::Unavailable`, the
   mapping files, and its relation to `CommitOutcomeUnknown`.
9. The chart hard-codes the spread in `_helpers.tpl`, so an example cannot set it; `ha.yaml` exists; disruption budgets
   exist.
10. No versions for CloudNativePG, Barman Cloud, Patroni, etcd, HAProxy; the PostgreSQL major is not chosen
    (`transaction_timeout` needs 17); the Compose image has no digest.
11. "A heartbeat is not retried" changes `sdks/SPEC.md`; the window's setting name; its interaction with the attempt
    count; how the conformance scenario avoids a 310 s run.
12. The two reads' routes, the pool type's name and module, the checker rule, the header's format, the configuration
    files, and how the read pool skips the beacon.
13. The lint's cut-off value and where it lives, how each rule is detected, whether downs are linted, the checksum
    baseline.
14. The release manifest's format, the table's schema, the function's name, how a binary knows its release, how "the last
    migration of a release" is enforced; adding the check to the heartbeat replaces a function, which the rule forbids.
15. "New write behaviour" names none; per-statement retry and the census refusal need a custom migration runner;
    changing the activation function hits the no-replace rule; capability lists are closed checks; too much in one task.
16. The earlier 0.3.0 commit is not pinned; how N-1 and design 051's driver are wired; where the latency threshold comes
    from.
17. The tool for reading WAL after the fork, the ledger's format, the rolling-upgrade fault procedure, pass and fail in
    script terms; whether `infra/dr/drill.sh`'s constant changes.
18. No fault test is named; the harness for two-node tests is not specified.
19. Architecture decisions 0130 and 0131 exist: "write" should be "revise"; 0130 is stale (the waited replay).

## Nits

20. Gap 247 is resolved for its task; node drain is its residual.
21. The engine's settings are in `apps/execution-engine/src/settings.rs`; the existing variable is `DATABASE_URL`, so
    `SAGAT_DATABASE_READ_URL` is inconsistent.
22. Metrics deferred to Task 048-11 leave Tasks 048-4 to 048-8 without counters; the dependency on design 051 is only
    in prose.

Counts: Blocker 3, Major 16, Nit 3
