I found no new Blocker and no new Major. All four third-round Majors are resolved. Seven of the nine Nits are fixed. The other two were deferred to the build and are still open; I recommend accepting them. The fixes leave six small Nits. Each changed task can be built as written.

I read the persona, my third-round report, the resolution's "Third round" section, D041-137 and the documents diff from `c52b5d37` to `543417134ff7`. On the design side I read the changed texts of T041-5, T041-6's Builds line, T041-36 and its Files, T041-37, T041-9 and section 2b. In `specs/041-step-logs/data-model.md` I read §3b, §6, §7 and §8a. I checked the code the tasks name in `crates/infra-postgres/src` and `apps/control-plane/src`, plus `tests/support/engine.rs` and `tests/fault/events-webhooks-messaging/cancellation.rs`. I also checked the sqlx 0.9.0 sources `sqlx-core/src/pool/connection.rs`, `sqlx-core/src/rt/mod.rs` and `sqlx-postgres/src/transaction.rs`. No target standard was supplied, so I audited against the persona's criteria.

## Status of the third-round items

| Item | Status |
|---|---|
| M-1 (`close_on_drop` cannot be cleared) | **Resolved.** `OwnedTransaction` is now named, with its states `idle`, `open` and `unknown` (T041-36, lines 2899-2912). sqlx 0.9.0 confirms the premise: `close_on_drop()` only sets the flag (`pool/connection.rs:97-99`). Draining an in-flight statement before `ROLLBACK` is real sqlx behaviour (`wait_until_ready` at the start of every execute). No code in `crates/infra-postgres/src` calls sqlx's own `.begin()` on a connection a run or path transaction would hold. That matters because a raw `BEGIN` leaves sqlx's `transaction_depth` at 0, so a nested `begin()` would send `BEGIN`/`COMMIT` instead of a savepoint. |
| M-2 (blast radius) | **Resolved.** An ordinary drop now rolls back on the same backend and keeps it. `begin_in_transaction` takes an `OwnedTransaction`. The named files are in Files, and the helper sites I checked are listed (`breaker.rs:74/135/161`, `declared_operations.rs:1230`, `execution/scoped_events.rs:102`). The test on a pool of one, with two hand mutations, can fail. Finding 1 below is a small leftover. |
| M-3 (*Q* behind a pooler) | **Resolved.** *Q* is set per transaction with `set_config(…, true)` in statements the store already sends. The carry's `p_wait_slots` is in §8a. Section 2b states the server-pool sizing rule. `pgbouncer.ini` uses `default_pool_size = 128`. The handling of `options` has both branches with a check through the pooler, and T041-37 checks *Q* before each pooler row. |
| M-4 (replicas share one `application_name`) | **Resolved.** `replica_with_pool` (`tests/support/engine.rs:756`) names each pool with a fresh UUID. `PoolConfig::new` defaults to `…/unnamed`. T041-37's setup asserts distinct names. |
| Nit 2 | Resolved: the one-shot processes are listed, `PoolName` is given, and the registry pool's name is set. I confirmed the call sites in `apps/control-plane/src/{lib.rs,cleanup.rs,inventory.rs,handover.rs,registry.rs}` and in `apps/execution-engine/src/lib.rs:95` and `:191`. |
| Nits 3, 4, 5, 6, 8, 9 | Resolved. |
| Nits 1, 7 | Not resolved. The resolution defers them to T041-14 and T041-36 (findings 7 and 8). |

## Open findings

1. **Nit, fix: `primitives.rs`'s own helpers are not in T041-36's helper list.**
   - T041-36 (line 2919) converts the helpers "called with the run or path transaction".
   - In `finish_collection` (`crates/infra-postgres/src/primitives.rs:3551`), the variant's `OwnedTransaction` is used long before it becomes a run transaction. It is passed to helpers that take `&mut Transaction<'_, Postgres>`: `validate_finish_replay` (`:220`), `lock_drain_run` (`:649`), `check_scheduler_fence` (`:1287`) and `crate::execution::charge_collection_owner_output` (`execution.rs:4092`).
   - The compiler will point at them, but the rule as written does not cover them. Widen it to "called with an `OwnedTransaction`" and name these.

2. **Nit, fix: the batch's and the marker's *Q* have no test that can fail, and data-model §8 is stale.**
   - Nothing in T041-6 fails if T1's select list omits *Q*. Nothing in T041-13 fails if the marker's first statement omits it. T041-5's tests set *Q* themselves, and T041-36 covers only the carry. An omission would quietly run batches at *Q* = *P* = 1.
   - `specs/041-step-logs/data-model.md` still gives T1 as `SELECT pg_current_xact_id()` (line 548), and the marker's first statement the same way (line 742).
   - Fix: add a T041-6 test that a batch's transaction (for example, held after T1) reads `sagat_quota_step_log_wait_slots()` as the store's *Q*, with the same test for the marker in T041-13, and update both §8 lines.

3. **Nit, fix: where *Q* is computed is not named, and T041-5's test has no store to run on.**
   - T041-5 (line 2588) tests that "a store over a pool of 20 computes *Q* = 5". But T041-5 builds no store that sends *Q*; those are T041-6 and T041-36.
   - Name one function, for example in `pool.rs`, from `PgPool::options().get_max_connections()`. Test that function in T041-5, and have both stores call it.
   - Also name the `begin_safely` variant.

4. **Nit, fix: the "no Tokio runtime" branch of `OwnedTransaction`'s `Drop` (lines 2904-2907) panics instead of closing.**
   - Marking `close_on_drop()` and dropping still goes through `PoolConnection::drop`, which calls `rt::spawn`. Without a runtime, that panics (`sqlx-core-0.9.0/src/pool/connection.rs:199-210`, `rt/mod.rs:70-91`).
   - Fix: use `detach()` and drop the raw connection on that path, or state that the branch is unreachable.

5. **Nit, fix: the owned-connection test can race.**
   - T041-36 (line 3057) checks "no backend of the pool is `idle in transaction`" after each drop. The `ROLLBACK` runs in a spawned task, so a check from a separate connection made right after the drop can see the backend before the rollback lands.
   - Say that the check follows a fresh acquire of the pool's only connection, which waits for the task, or that it polls with `eventually_within`.

6. **Nit, fix: "B's `maxwait`" cannot be measured.**
   - The three-tenant criterion behind the pooler (line 3185) uses B's `maxwait`. `SHOW POOLS` reports per (database, user) pool, and every tenant uses the benchmark's one role, so there is no B-specific figure.
   - State that the pool's `maxwait` is used; it is an upper bound on B's.

7. **Nit, accept: third-round Nit 1** (`releaseAnswer` when nothing is held, where `heldAnswers` sits, the golden's exact cases). It is deferred to T041-14. A golden that does not match fails loudly when it is written.

8. **Nit, accept: third-round Nit 7** (the reason `MarkOnly` carries, and the reason after a `MarkOnly(failed)` re-run meets the content fallback). It is deferred to T041-36. Section 2a's "which keeps the downgraded carry" (line 1263) already implies that `failed` is kept.

Counts: Blocker 0, Major 0, Nit 8