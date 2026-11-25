Re-read of design 041 at `4d6fe4cc` (approval path OD-2026-11-20-17). **Verdict: not ready yet.** There is no Blocker, but seven Majors are open: three first-round findings are only partly fixed, and the fixes added four new problems.

I read the persona file, the first-round report, `resolution.md`, decisions D041-124 to D041-133, the design from line 2188 to the end, the relevant parts of sections 2a/2b, and data-model.md §1, §3b, §4, §7 and §8a. I checked code references in the repository where they mattered. No target standard was supplied beyond the persona.

## First-round findings

- **R-1 (Blocker): resolved.** The "Tests first" rule at 2209-2210 says no task tests a carry before T041-36. T041-5 tests only plain inserts and direct calls (2456-2457) and hands mutations 18 and 20 to T041-36 (2520). T041-6 hands over the carrying-report half (2568-2569). T041-36 takes those cases (2819-2826), and Verification row 3258 matches.
- **R-2: resolved.** T041-10 now has `reclaim_idle_step_log_leases` with its bound (2631-2636), the ported tests (2647-2651) and its files (2656-2659). T041-5 now says "a `DELETE`" (2485). A gap in the skip count is new finding N-2 below.
- **R-3: resolved** (2419-2420).
- **R-4: resolved.** The migration files, the oracle move and its three files, and the privilege audit are at 2730-2741 and 2885-2896; the Done-when is at 2881-2882.
- **R-5: resolved.** The SQL assertion is at 2852-2853; the route assertion moved to T041-11 (2986-2987).
- **R-6: partly resolved (Major still open).** The ensure and the room map are gone, and the seam is named at 2742-2751. But the mechanism as written cannot be built against the code:
  - `begin_safely` exists only for `PgPool` (`crates/infra-postgres/src/pool.rs:112-133`). It returns a `Transaction<'static, Postgres>` that gives its connection back to the pool when it commits.
  - `PgRunTx` and `PgPathExecutionTx` hold exactly such a `tx` (`crates/infra-postgres/src/execution.rs:3825` and `:942`).
  - So "own their `PoolConnection` and begin on it by `begin_safely`" leaves the implementer to design the change of connection ownership for every run and path transaction. That includes a begin on an owned connection that is safe against cancellation (the hazard `pool.rs:99-111` documents) and the rollback-on-drop rule.
  - Fix: state the mechanism, for example acquire a `PoolConnection` in a spawned task, drive the transaction manager's begin, commit and start-rollback on it, and keep the rollback on drop.
- **R-7: resolved** (2774-2777, 2872-2875). Reports do insert into `run_events`, so the trigger point exists.
- **R-8: resolved.** Each metric has one producer: T041-6 at 2547-2552, T041-10 at 2629-2642, T041-36 at 2750 and 2767-2773. The alerts at 1951-1959 name only emitted metrics.
- **R-9: resolved** (2781-2790, 2885-2899; T041-35's Files at 2292-2297).
- **R-10: resolved apart from the pooler setup** (2902-2915). The pooler gap is new finding N-3.
- **R-11: partly resolved (Major still open).** `holdAnswer` and `releaseAnswer` are defined (3086-3091) and named in the preamble (3072). Two things remain:
  - The fault is written `{"fault": "holdAnswer", "rpc": …, "nth": …}`, but the harness's `faults[]` schema is `channel`, `match`, `action`, `times`, `after` (`tests/contract/sdk-conformance/README.md:558-570`).
  - "The fault proxy" exists only in Rust (`sdks/rust/conformance/src/faults/grpc.rs`). In the other five SDKs faults run in each runner's interceptors (design 1792-1796). The `out.json` field that carries "before or after the release, how long after the batch" is never named, nor how it is normalised for a golden.
  - So the six SDKs would still each invent the evidence format. Fix: write the fault in the `faults[]` form and name the observation field and how it is normalised.
- **R-12: resolved** (2560-2562). The limit-change quota reclaim zeroes the writer's quota lease (data-model.md §3b), so T8 refills.
- **R-13: partly resolved (Major still open).** The field, the value and a test are named (2447-2455). Two gaps remain:
  - The execution engine opens its own pool (`apps/execution-engine/src/lib.rs:95`) and is the process that charges quota leases, since it hosts `WorkerGrpc`. It is not in Files (2529-2530).
  - In both the control plane and the execution engine, the instance id is created inside `registry::member` (`apps/control-plane/src/registry.rs:71`), after the pool is opened (`apps/control-plane/src/lib.rs:523` then `:545`; `apps/execution-engine/src/lib.rs:95` then `:195`). The id must be created first and passed to both `open_pool`/`pool_config` and `member`, and those signature changes are not stated. One-shot processes (migrate, inventory, handover, cleanup) have no rule for their name.
  - Without this, every engine replica keeps one shared name, so the seven wait slots become cluster-wide, and T041-37's per-replica criteria (2947-2948) cannot be measured.
- **R-14: resolved** (2369-2372, 2398, 2407, 2506-2507, 2534, 2602-2603).
- **R-15: resolved** (3258).
- **R-16: resolved** (2303-2305, 2309-2310, 2363, 2412-2414).
- **R-17: resolved** (2367-2369; T041-5's Files at 2527-2535 no longer list T041-4's migrations).
- **R-18: resolved** (2514-2520). One mapping wrinkle is Nit 7 below.
- **R-19: resolved** (2577-2580).
- **R-20: resolved.** The terms are defined at 2223-2226, and every later use in the range is qualified.
- **R-21: resolved** (2249-2251).

## New Majors

- **N-1 (Major, fix): the privilege list disagrees in three places.**
  - T041-5 (2493-2499) and Verification row 3283 say `sagat_app` may execute exactly the allowance plus six quota-lease functions, "the privilege audit checking exactly that list".
  - Section 3's single grants list (1553-1555) and data-model.md §3b/§4 (lines 248-251, 395-397) also grant `EXECUTE` to `sagat_app` on `sagat_quota_step_log_top_up_mark()` and `sagat_quota_step_log_wait_slots()`. The carry, a `SECURITY INVOKER` function, calls `top_up_mark()` (data-model.md:605), so it needs that grant.
  - Section 3 also places the two constants among the definer functions, while data-model.md calls them `IMMUTABLE` and `SECURITY INVOKER`.
  - T041-5's list of functions (2430-2438) leaves out the constants and the counter sequences that data-model.md requires in the audit.
  - Fix: add both constants and the sequences to T041-5's migration and to the audit list, and correct row 3283.
- **N-2 (Major, fix): a skipped tenant cannot be told apart.** T041-10 counts tenants "skipped because their step-log fence was held" and tests that they are counted (2634-2635, 2650). But `sagat_quota_step_log_reclaim_idle` returns 0 both when the fence is held and when nothing is idle (data-model.md:311-315), so the store cannot tell the two apart.
  - Changing that after T041-5 merges needs a new migration.
  - Fix: give the function a distinct answer in data-model.md §3b and T041-5, for example a `(reclaimed, skipped)` row.
- **N-3 (Major, fix): the pooler regime's PgBouncer config cannot carry the platform's statements.** `pgbouncer.ini` is fixed at three settings (2913-2915). sqlx uses named prepared statements by default. PgBouncer in transaction mode needs `max_prepared_statements` set above 0; as I recall, its default was still 0 in the version Ubuntu 24.04 packages (1.22), though I did not confirm that version. No auth settings are given either.
  - Run as written, the regime would fail on statement errors rather than measure anything, and a miss reopens ADR-0097 option 1c for the owner.
  - Fix: add `max_prepared_statements`, `auth_type`/`auth_file` and the user file to the ini spec and its Files.
- **N-4 (Major, fix): a security guard that no task builds.** Section 2b (1411-1413) says "a source-text check of task T041-36 fails when anything but the store's top-up calls `sagat_quota_step_log_top_up`". T041-36's tests (2791-2880) do not include it. The function is executable by `sagat_app` for any tenant, so this check is the only guard behind the claim at 1411. Fix: add it to T041-36's first test list.

## Nits

From the first round, still open:
1. Nit 8: the path globs in the evidence at 2239 and 2242 are not in backticks.
2. Nit 17: lines over the length limit remain, for example 2469, 2577, 2580, 2987, 3050, 3071 and 3091.
3. Nit 27: "row" still means a benchmark measurement in T041-37 (2907, 2950, 2946) beside "usage row".

New:
4. data-model.md:19 gives `p_mark_reason` three values; §8a (590) and T041-36 (2734) give four, including `failed`.
5. data-model.md:356 publishes "raw, monotonic values", while T041-10 (2638-2642) publishes deltas. A replica that loses shard 0 also keeps exporting a stale `step_log_quota_lease_rows` gauge.
6. T041-10 does not say how the purge pass learns that it holds shard 0. `Keeper::shards` in `crates/scheduler/src/shards.rs:77` is the likely source.
7. Mutation 18 is mapped two ways: T041-5 at 2519 ("the top-up charging what it grants") and T041-36 at 2825-2826 ("a top-up that answers without granting").
8. `pool.rs:30-52` (2449) points at `new` and the `Debug` impl; the struct is at `:11-24`.
9. T041-6's lock-order slots (2556) leave out L4, which its hook names (2560).
10. `.github/workflows/perf-spike.yml:63` still calls `carry_diag_rounds.sh`, which T041-36 deletes, behind `SPIKE_DIAG`. T041-36 does not say to remove that branch.

Counts: Blocker 0, Major 7, Nit 10