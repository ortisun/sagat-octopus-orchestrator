# Re-read of design 041 for approval: implementation readiness at c52b5d37

I checked the commit by reading the worktree's branch ref. I read the persona, the target from "## Detailed Implementation Plan" to the end, sections 2a and 2b, data-model.md §1, §3b, §4, §6 to §8a, the resolution's "Second round" section, D041-134 and D041-135, ADR-0097 decision 1 and 3b, and the code the tasks name. No target standard was supplied, so I audited against the persona's criteria.

Two limits on this review:
- The second-round readiness report is not in the review folder. I could only check its Nits that the resolution names (5, 6 and 8), and all three are fixed.
- Two findings rest on behaviour outside the repository, sqlx-core 0.9.0's `PoolConnection` API and PgBouncer's handling of startup parameters. I am working from what I know of them; I did not re-read their sources.

## Status of the items left open at 4d6fe4cc

| Item | Status |
|---|---|
| R-6 | Still open. The connection seam is now named (2b lines 1413-1418, T041-36 lines 2807-2828), but the mechanism as written cannot be built (M-1). The fix also brings a new problem (M-2). |
| R-11 | Resolved as a Major. `holdAnswer` is in the `faults[]` form, and `heldAnswers` and its normalisation are named (lines 3201-3210). Nit 1 is what remains. |
| R-13 | Resolved. The instance id is created before the pool, `registry::member` takes it, and the files are listed (lines 2499-2508). Nit 2 is what remains. |
| N-1 | Resolved. §3b (lines 243-254), §4 (399-412), T041-5 (2550-2560), the Verification row (3408) and ADR-0097 decision 1 (lines 96-103) all give the same list: six definer functions, two invoker helpers, sequences revoked. |
| N-2 | Resolved. `(reclaimed integer, skipped boolean)` is consistent across §3b, 2b, T041-5 and T041-10. |
| N-3 | Partly resolved. `max_prepared_statements`, `auth_type`/`auth_file` and the version pin are now there (lines 3007-3017). The pooler still does not carry the pool's session settings (M-3). |
| N-4 | Resolved (line 2952). Nit 5 is a placement detail. |

## Still open

### M-1. A connection's `close_on_drop` cannot be cleared (R-6 remainder) - Major
- **Scope:** T041-36 lines 2813-2818; section 2b lines 1416-1418.
- **Problem:**
  - The plan marks the connection `close_on_drop()` at `BEGIN` and has `commit` "clear" it once `COMMIT` succeeds.
  - In sqlx 0.9, `PoolConnection::close_on_drop(&mut self)` only sets the flag. No public method clears it.
  - So as written, either every report connection closes after a successful commit, or the implementer has to invent a different mechanism.
- **Guidance:** pick the construction and write it down. Two options:
  - Keep an `Option<PoolConnection<Postgres>>` with a `finished` flag. Call `close_on_drop()` in the wrapper's `Drop` only when the flag is unset, and call it explicitly when `COMMIT` or `ROLLBACK` fails.
  - Drive sqlx's own Postgres transaction manager on the owned connection.

### M-2. "A dropped transaction closes its connection" hits every uncommitted run and path transaction, and the change reaches files the plan does not name - Major
- **Scope:** T041-36 lines 2810-2820 and its Files (2980-2994); 2b line 1417.
- **Churn on the hot path:**
  - The ports have no rollback method. `crates/application/src/execution.rs:1649-1650` says "Dropping the transaction instead rolls everything back".
  - The store returns `Ok(None)` after `BEGIN` on contention: `crates/infra-postgres/src/execution.rs:867-878`, `:912-923` and `:1606-1616` (claims under `SkipLocked`).
  - The poll path returns on that: `crates/application/src/worker.rs:1117-1122`.
  - So every contended claim, every refused or replayed report and every early return would close a backend.
  - Each new backend is a new quota-lease writer, so its next carry falls back. It also leaves an ended-backend quota lease for the purge to reclaim.
  - None of this is weighed, and no test or regime covers it.
- **Unnamed scope:**
  - `begin_in_transaction` (`execution.rs:280-293`) builds `PgRunTx` from a `Transaction`. Its callers are `crates/infra-postgres/src/primitives.rs:3551-3846` and `crates/infra-postgres/src/chain_progress.rs:2346-2376`.
  - These helpers outside `execution.rs` take `&mut Transaction` and are called with `&mut self.tx`: `crates/infra-postgres/src/breaker.rs:74, 135, 161`, `crates/infra-postgres/src/declared_operations.rs:1230` and `crates/infra-postgres/src/execution/scoped_events.rs:102`.
  - None of these files is in T041-36's Files.
- **Guidance:**
  - Close a connection only when its state is unknown: a statement was in flight, or `COMMIT`/`ROLLBACK` failed.
  - On a clean drop, roll back and return the connection, for example from a spawned task.
  - Add a test that a contended claim does not change the backend's `pg_backend_pid()`.
  - List the files above and give `begin_in_transaction`'s new parameter type.

### M-3. The wait-slot setting has no stated mechanism and does not reach the backends behind a transaction pooler (N-3 remainder, from D041-134) - Major
- **Scope:** T041-5 lines 2495-2498; 2b lines 1366-1368 and 1439-1446; T041-37 lines 3007-3017 and 3045-3051.
- **Problem:**
  - T041-5 says the pool sets `sagat.step_log_wait_slots` "when it connects", but not how. It could go in the startup `options`, as `pool.rs:59-71` does for the timeouts, or in a statement after connect.
  - PgBouncer in transaction mode does not carry either kind of setting to the server backends. As far as I know, it also refuses an unknown startup parameter unless `ignore_startup_parameters` names it. `pgbouncer.ini` sets neither.
  - So the pooler regimes may not connect at all. If they do, `sagat_quota_step_log_wait_slots()` reads 1 on most backends (*Q* = *P* = 1) and the statement timeouts are lost. The two-tenant "behind the pooler" criterion and the busy rates would then measure a configuration that is not the shipped one.
- **Guidance:**
  - State the mechanism in T041-5.
  - Say in 2b what *Q* is behind a transaction pooler.
  - Give `pgbouncer.ini` (or the benchmark role) a way to apply the settings on the server side, and have T041-37 assert the setting on a pooled backend before each row.

### M-4. The benchmark's in-process replicas share one `application_name`, so they share wait slots (new, from D041-134) - Major
- **Scope:** T041-37's regimes (lines 3018-3063); the T041-36 driver rename (2863-2867).
- **Problem:**
  - `tests/support/engine.rs:756-765` builds each replica's pool with `PoolConfig::new`. T041-5 does not give that default a per-replica name.
  - The slot keys come from `application_name`, so the 8 replicas would share one tenant's 3 slots and the 5 replica slots across 160 connections. Production has 8 times as many.
  - That skews the 0.1% busy criteria, the per-replica slot counts read by `application_name` (lines 3049-3054) and the B-victim criteria. A run could trigger owner questions or design changes because of how the benchmark is set up, not because of the design.
- **Guidance:** have `replica_with_pool` or the driver name each replica `sagat-octopus-orchestrator/<uuid>`. Have T041-37's setup assert distinct names in `pg_stat_activity`, and state `PoolConfig::new`'s default name.

### Nits
1. **R-11 remainder (3205-3210):**
   - The plan does not say what `releaseAnswer` does when no answer is held yet (wait until one is held, or fail).
   - It does not say whether `heldAnswers` sits at the top level beside `faultsApplied` or inside `observations[]` (`tests/contract/sdk-conformance/README.md:574-578`).
   - A golden matches exactly, so "`afterRelease`, or `beforeRelease` with `atLeast2000`" has to become the cases the scenario actually plays.
2. **R-13 remainder:**
   - The list of one-shot processes leaves out `seed_dev`, `seed_dev_namespace`, `provision_tenant`, `grant_quota_ceiling`, `set_scheduling_weight` and `reseal_secrets` (`apps/control-plane/src/lib.rs:171-417`).
   - "The one-shot processes' pool builders" in Files means `apps/control-plane/src/{cleanup.rs,inventory.rs,handover.rs}`, which should be named.
   - The "`instance_id` argument" also receives a process kind, so its type is unclear.
   - The registry pool's name (`apps/control-plane/src/registry.rs:53-58`) is not given.
   - The plan does not say whether the `step_log_wait_slots` field overrides the computed ⌊N / 4⌋.
3. **The top-up's `held` answer has no condition** since D041-134. §3b (lines 303-318) defines only `contended`, `room`, `busy`, `limit` and `made`, yet T041-5 (line 2525) and T041-36 (line 2923) expect `held`.
4. **Stale text in data-model.md §3b:** line 247 still says "the top-up the refill", and the refill's `bounded` parameter (lines 286-287) has no unbounded caller left.
5. **Where the top-up call lives:** T041-36 runs the top-up statement inside the transactions' `commit` (2821-2824), while the source-text check (2952) allows it only in `crates/infra-postgres/src/step_logs.rs`. Say that `commit` calls a helper there.
6. **SCRAM verifier:** the "SCRAM verifier generated by `run.sh`" (3015-3017) must equal the server role's own verifier for pass-through to work. Say that `run.sh` sets the role's password to that verifier, or use the disposable role's plaintext password.
7. **Mark-only reasons:** `CarryPlan::MarkOnly` needs to carry its reason (2781-2782). The plan also does not say which reason a content fallback records after a `MarkOnly(failed)` re-run (lines 1113-1114, 1245).
8. **T041-9 (3284-3286)** does not say what happens when fewer than six of the twelve rounds are valid.
9. **Misplaced test:** the connection fault test "beside `tests/fault/events-webhooks-messaging/cancellation.rs`" (line 2951) is listed under the `integration_step_logs` tests, and that file is not in T041-36's Files.

Counts: Blocker 0, Major 4, Nit 9