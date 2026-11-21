Doc: docs/designs/041-step-logs.md
Reviewed-commit: 543417134ff7ebda70a96690329bb95ddde55c8c
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9

**Verdict:** both third-round Majors (N-2, N-3) and all four Nits are resolved. The owned-connection mechanism can be built against sqlx 0.9.0 as written, with one exception covered in Nit 2. The fix for N-3 brings in one new Major: the server-pool sizing rule that the bound now depends on behind a transaction pooler. It is a text fix and not a Blocker, but it should be fixed, or the owner should accept it explicitly, before approval. There are four new Nits. I had no shell, so this comes from reading the files at the commit (the branch ref resolves to `54341713`) and the sqlx-core and sqlx-postgres 0.9.0 sources.

## Earlier findings

- **N-2 (connection hand-off): resolved.**
  - `docs/designs/041-step-logs.md:1474-1486` and T041-36 `:2899-2924` replace "clear `close_on_drop`" with an `OwnedTransaction` that tracks `idle`, `open` or `unknown`. Its `Drop` sends `ROLLBACK` in a spawned task and marks the connection `close_on_drop()` only when that `ROLLBACK` fails.
  - The scope now covers every `PgRunTx` and `PgPathExecutionTx` command (`:2914-2918`). The cited sites are correct: `execution.rs:447`, `:1513`, `:280-293`, `:867-878`, `:912-923`, `:1606-1616`, `chain_progress.rs:2346-2376` and the `begin_in_transaction` call at `primitives.rs:3762`.
  - The fault test asserts the same `pg_backend_pid()` across two commits, a contended claim and a refused report on a pool of one, with a hand mutation for "close on every drop" (`:3057-3066`, Verification Plan `:3532`).
  - Checked against sqlx:
    - `wait_until_ready` (`sqlx-postgres-0.9.0/src/connection/mod.rs:94-108`) flushes and drains, so the "drain first" claim holds.
    - The return path still pings (`sqlx-core-0.9.0/src/pool/connection.rs:314`).
    - Sending `BEGIN` directly leaves sqlx's `transaction_depth` at 0. That is safe only because no code in infra-postgres calls `.begin()` on a run transaction to open a savepoint. I found none.
  - Decision D041-137 is at `specs/041-step-logs/research.md:148`.
- **N-3 (wait slots behind a transaction pooler): resolved.**
  - *Q* is now set per transaction, transaction-locally, in statements the store already sends:
    - the carry's `p_wait_slots` (`specs/041-step-logs/data-model.md:623-624`, `:652`);
    - the batch's T1 (`docs/designs/041-step-logs.md:941`);
    - the marker's first statement (`:1981-1982`);
    - the rule itself in section 2b (`:1404-1411`).
  - Tests cover it: T041-5 at `:2600` and its transaction-local check, and T041-36's "next transaction reads 1".
  - T041-37 checks *Q* and the timeouts through the pooler and records `SHOW POOLS` (`:3138-3144`). The runbook line is corrected (`docs/runbooks/step-logs.md:72-73`, `:95-100`).
- **Nit 1 (old top-up wording): resolved.** L4 `:1021-1024`; section 2a row `:1295`; `data-model.md:248` and `:294` (the `bounded` parameter is gone); `docs/adr/README.md:39`; `held` removed (ADR-0097 `:138-142`, design `:1493`, data-model §7).
- **Nit 2 (T041-36 Files): resolved** (`:3102`, `:2919-2924`). One more file is missing; see Nit 3.
- **Nit 3 (ADR-0097 header): resolved** (ADR-0097 `:12-18`; it names D041-134 and D041-137 and says how they take effect).
- **Nit 4 (pool defaults): resolved.** Design `:1400-1402` and `:876-879`, runbook `:67-68`, data-model §6. The stale `cli.rs:55` help is in T041-7's Files (`:2838`).

## New findings

**1. (Major, fix) The pooler sizing rule cannot be met where a pooler is needed, and the benchmark was changed to fit it.**
- **Where:** design `:1422-1425`; ADR-0097 decision 4 `:166-167`; runbook `:95-99`; T041-37 `:3129-3130`.
- **What is wrong:**
  - The rule asks for Σ(⌊N / 2⌋ + *Q*) + R server backends: 128 for 160 client connections, so 80% of them.
  - Operators add a pooler because replicas × pool approaches `max_connections` (`docs/runbooks/scaling.md:36`), and that is exactly when this rule cannot be met. Nothing is said about what happens under it beyond "heartbeats then queue in the pooler".
  - The rule uses the default Logs cap ⌊N / 2⌋, not the configured `SAGAT_GRPC_MAX_LOG_CALLS`. That hides the one knob an operator has.
  - `scaling.md` is unchanged and does not point to the rule.
  - T041-37's pooler `default_pool_size` moved from 40 (four clients to one backend) to 128.
- **Consequence:**
  - The pooler rows (option 1c's criterion, and the two- and three-tenant rows "behind the pooler") are no longer measured in the multiplexed setup the risk row and G041-027's acceptance rely on.
  - Under the rule, how far heartbeats and task leases degrade is neither bounded nor measured.
- **Smallest correction:**
  - Write the rule with the configured cap: Σ(cap + *Q*) + R.
  - Add a pointer from `scaling.md`'s `SAGAT_DB_MAX_CONNECTIONS` row to the rule.
  - Do one of these:
    - keep one pooler row at 40 backends that records heartbeat and task-lease `maxwait`, with a criterion and a consequence;
    - or state the behaviour under the rule as a residual and put it to the owner.

**2. (Nit, fix) The no-runtime branch would panic.**
- **Where:** `:1482`, `:2906`.
- **What is wrong:** `PoolConnection::drop` calls `crate::rt::spawn` in both branches (`sqlx-core-0.9.0/src/pool/connection.rs:200-209`). With no Tokio runtime that panics (`rt/mod.rs:76-88`, `:164-166`), so "mark `close_on_drop()` and drop" does not avoid it.
- **Correction:** with no runtime, call `detach()` (`connection.rs:113-115`, which spawns nothing) and drop the raw connection.

**3. (Nit, fix) One more helper is outside T041-36's list.**
- **Where:** `crates/infra-postgres/src/definitions.rs:164-165`.
- **What is wrong:** `verify_declared_operations` takes `&mut sqlx::Transaction<'_, sqlx::Postgres>`. It is called with admission's transaction (`execution.rs:1257`), which becomes the `PgRunTx` (`:1513`).
- **Correction:** add `definitions.rs` to the helper list (`:2919-2924`) and to the Files (`:3102-3104`).

**4. (Nit, fix) ADR-0097's "Enforced by" does not cover D041-137.**
- **Where:** ADR-0097 `:42-52`.
- **What is wrong:** it does not list the owned-connection fault test, the per-transaction *Q* tests or T041-37's pooler checks. It also does not say that decision 4's new MUST on server backends is an operator rule (review only, runbook), as README rule 3 requires.

**5. (Nit, fix) The design header misses the new owner rows.**
- **Where:** `docs/designs/041-step-logs.md:6`.
- **What is wrong:** the Owner-acceptance line does not list OD-2026-11-21-1 to -3. OD-2026-11-21-1 relaxes an owner criterion.

Counts: Blocker 0, Major 1, Nit 4