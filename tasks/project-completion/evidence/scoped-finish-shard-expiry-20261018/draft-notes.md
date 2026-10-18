# Scoped Finish post-shard-wait expiry handoff

Read-only draft for the root writer. This author made no repository edit and ran no test, build,
Cargo, formatter, Clippy, architecture, agent, commit or external-write operation.

## Registration

Copy `/tmp/sagat-scoped-finish-shard-expiry.rs` to:

`tests/integration/workflow-primitives/scoped_leaf_finish_shard_expiry.rs`

Register it as a direct child of
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_shard_expiry.rs"]
mod shard_expiry;
```

It can follow the current `receipts` child or a separately installed `entry_barriers` child. It
depends only on the existing private native helpers through `use super::*`; no helper visibility,
application API or production code change is required.

## Exact test/API shape

Two tests call one bounded helper, once with no receipt and once after a genuine first application:

- `native_first_apply_revalidates_expiry_after_unchanged_shard_wait`
- `native_exact_replay_revalidates_expiry_after_unchanged_shard_wait`

The helper uses these exact existing interfaces and database witnesses:

1. `certified_flow`, `seed_group`, `finish_command`, `source_execution_snapshot`,
   `capacity_snapshot`, `finish_counts` and `assert_linked_effect` from the native parent.
2. `ShardStore::renew(owner, &[lease], Duration::from_secs(3))` to shorten the genuine current
   scheduler authority without changing its owner or generation.
3. Owner SQL `SELECT expires_at FROM scheduler_shards WHERE shard=run_shard($1) FOR UPDATE` to
   hold the committed row unchanged. There is no `UPDATE` during the wait.
4. `PgScopedLeafFinishStore::finish_scoped_leaf` polled locally. `Flow` is never spawned.
5. `pg_stat_activity`, `wait_event_type='Lock'`, `pg_blocking_pids` and the actual
   `FROM scheduler_shards` query text to prove the native `FOR SHARE` is waiting directly behind
   the owner gate while the database says the lease is still live.
6. Independent `FOR UPDATE NOWAIT` probes on the original Run and selected G, both expecting
   SQLSTATE `55P03`, to distinguish the native transaction's already retained guards from the
   scheduler row that it is waiting to share-lock.
7. A database-clock polling loop, bounded by a ten-second timeout, to observe
   `clock_timestamp() >= expires_at` while the row remains locked and unchanged. It uses yields,
   with no sleep as ordering evidence.
8. After releasing the unchanged row, the native call must return `SchedulerFenceLost`. Exact
   source/capacity snapshots and receipt/audit/hint counts must equal their pre-attempt values.
9. `ShardStore::acquire` takes the now-expired shard with a fresh owner/generation, and
   `SchedulerAuthority::for_run` replaces only renewable authority on the same command. The same
   operation key/fingerprint then applies once or replays the exact original result, according to
   the case, ending with one receipt/audit/hint set.

The explicit native pool is `sagat_app`; the parent fixture continues to grant only the four
test-only scoped receipt extension columns after genuine source certification. Scheduler renewal
and fresh take are outside the asserted source/capacity snapshots.

## Frozen source

Requested HEAD: `211394282795bc295318a302d3ec30b71df2791d`.

The scoped adapter remains the implementation introduced by
`7b0ced121d5c2cb4bbc735d513be4894bebd5923`. Relevant inspected SHA-256 values:

| Input | SHA-256 |
| --- | --- |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `tasks/project-completion/scoped-leaf-finish-native-contract.md` | `b7e983ace83cbb96109afadd3dc688eeec14e680cc985859868b2689675dbacb` |
| `crates/application/src/shards.rs` | `831d8fbca5a2f26c24648499aab5e93150bf5595c2d678d2a0c4bfb958d590aa` |
| `crates/application/src/primitives.rs` | `bd58299fa2104543cb4d84d529e1d105c2ee92254fe76ca444319d3b91a617ea` |
| `crates/infra-postgres/src/partitioning.rs` | `7d1229004cd83440f2a6c6e22a9c8ea88b0708bfddfd0233c7799b522deb2f1d` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` | `edf76249c4564832039e392d9f6ce281a0e05c6b190d085c2d42f62c10ea5eb1` |
| `tests/integration/workflow-primitives/capacity_finish_barriers.rs` | `285f073e0dec5e4b512be4542b297b8f7f1b6a1cb0697017a6e70c40dc1adc31` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |

The worktree already contained unrelated root-owned qualification/decoder changes during this
draft. They were neither read as authority for this slice nor modified.

## Limits and remaining gates

This draft establishes no result until the root installs and executes it. Root must review the
installed diff, run both focused cases on a private `CARGO_TARGET_DIR`, rerun the affected native
family and current qualification/receipt/entry controls, and record real outputs. Proportional fmt,
strict Clippy and architecture checks plus final review/evidence updates remain pending.

This slice excludes chain/child/historical/full-context matrices, source-writer ordering,
audit/outbox/receipt/deferred-COMMIT injection, unknown COMMIT and process recovery, returned
observations/OTLP, caller reconstruction, release privileges, C0, B1/B2, resource ceilings and the
shared T014-040/T014-043/T014-044 release gates.
