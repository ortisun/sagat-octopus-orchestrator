# Security review: feature 022 rate gate served and listed

Reviewer: `security-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted change later committed on
`feature/022-throughput-control`. Text as returned; resolutions in `rate-gate-responses.md`.

**Verdict:** no Blocker. Two Majors, both resource exhaustion that crosses tenants through the two new passes that run over every tenant. Nothing was run; every finding comes from reading the code.

## Major 1: a slow purge keeps other tenants' expired windows counted against their `rate_limit_live_keys`

Where: `maintenance.rs:1256` (candidate query); `passes.rs:93`, `:4294`; `readiness_gate.rs:611` (`live_keys`).

- A window row is deleted only when `live_grants = 0 AND waiters = 0`; `live_grants` falls only when the purge deletes grants. A window whose grants all aged out stays until the purge reaches it.
- `live_keys` counts every window row under the versions in force whether any grant still counts or not; `admits_rate_key` judges against it.
- The purge picks `SELECT DISTINCT ... LIMIT 64` keys with no order and no fairness between tenants, at most 20 rounds per housekeeping (1,280 keys every 300 s per replica).
- Scenario: tenant A keeps a backlog of aged per-second keys above 1,280 per housekeeping; tenant B near its own ceiling sees its aged windows stay for hours and its new keys refused `FLOW_CONTROL_LIMIT`, although none of its grants counts. Admission of one tenant depends on another's writes.
- Second cost: the candidate filter is an expression with no index, a full scan of `rate_limit_grants` 20 times per housekeeping on every replica.
- Fix: (a) count only windows that still matter (`waiters > 0 OR` a grant unleased or within `max_period`), in `live_keys` and in the usage query — the actual fix; (b) purge oldest first with an index and a time budget — bounds the cost.

## Major 2: the lease pass does work proportional to every unleased step grant, every tick, and a tenant can inflate it

Where: `maintenance.rs:295-322`; `passes.rs:1039`.

- The per-shard lateral orders and limits after the join to `steps` and the lateral into `run_events`, so both journal probes run for every unleased grant on the shard.
- Scenario: a definition with a rate-limited root step on a queue no worker polls; up to `tenant_active_runs` steps granted and never leased; about 100k index probes every tick, delaying the passes of every tenant on those shards.
- Fix: `AND st.attempt >= u.attempt` on the `steps` join (the domain's `leased_this`), so a never-leased grant costs no journal probe; index `(shard, granted_at) WHERE leased_at IS NULL`; longer term record the lease in the lease transaction.

## Minor 3: the runbook still says rate rows never drain and points operators to the emergency force

`docs/runbooks/flow-control.md:48-50` ("No process of this release lists `flow-control-rate-v1`") and `:88-91` ("Rate-limit rows do not drain in this build … a rotation with rate-limit rows ends only by the force of section 3") are false now and contradict section 6; an operator would force-retire needlessly.

## Minor 4: a new test-only door in production code, with no fitness rule

`maintenance.rs:1249`: `pub async fn purge_rate_grants_as_of(now, ...)` is callable from any crate; with a future `now` it deletes grants that still count and the prune trigger removes the window (over-delivery). The diff removes `serving_rate_gates_for_tests` and its rule and adds a door of the same kind with none. Fix: `pub(crate)` with tests seeding rows, or a dev-only feature, or a call-site rule.

## Minor 5: the architecture-checker test file holds a duplicated block and a test of the removed rule

`scripts/tests/test_check_architecture.py:966-1325`: a second copy of several classes plus a second `if __name__ == "__main__"`; the stale rate-gates test fails. Pre-existing: `unittest.main()` mid-file skips later classes when run as a script.

## Minor 6: the purge queues behind admissions and holds the key's lock during a long delete

`maintenance.rs:1285`: `FOR UPDATE` without `SKIP LOCKED`; every replica's housekeeping waits on the same keys; up to 5,000 rows deleted under the lock, each firing the count trigger on the window. Fix: `SKIP LOCKED` and a smaller per-key batch (about 500).

## Nit 7: the usage endpoint counts differently from enforcement

`quotas.rs:1369-1376`: usage counts windows of every key version; the gate only versions in force.

## Nit 8: migration has no lock timeout

`20270206000000…up.sql` takes ACCESS EXCLUSIVE on `quota_limits` and `quota_ceilings` with no `lock_timeout`; same as earlier migrations.

## Suspicions (not confirmed)

- **S1:** unleased grants are never reclaimed except through the reducer's `RateReturned`; a terminal path that bypasses the reducer would leak a slot that counts forever. Safety net: purge unleased grants of terminal runs; test cancel and terminate of a run with a granted unleased rate step.
- **S2:** the lease instant may be earlier than the lease commit (`run_events.occurred_at` is the transaction's clock read); a small over-delivery for per-second keys.
- **S3:** a key-existence oracle within a namespace at its `rate_limit_live_keys` ceiling (the G022-014 class, as for concurrency); record it beside G022-002 and G022-014.
- **S4:** the owner's acceptance of an unserialized live-keys count (OD-2026-11-19-4, OD-2026-11-20-3) names the concurrency check; extend it to `rate_limit_live_keys` in writing.

## Classes checked with no finding

Tenant boundary in every new statement (tenant carried in every join and lock; cross-tenant effects only those of Majors 1 and 2); privileges of `sagat_app` (by reading; run the rate suite as the runtime role); the migration (NOT VALID then validated; the down refuses retained rows; lists match `20270130000300`); no key values in logs, spans, metrics or errors; failing closed (no ring, switch off, reset of a rate-limited start, collections and loops); removing the test-only door is safe (served = built ∩ switched ∩ keys); lock-order races (lease and purge touch disjoint rows; a return deletes only unleased rows).

## Test gaps worth adding

The purge or lease pass over tenant A leaves tenant B unchanged; the rate suite as the runtime role; `rate_limit_live_keys` admits a new key once the namespace's windows aged but before the purge.
