# Scoped Finish entry-barrier handoff

Read-only draft prepared for the root writer. No repository file was edited, and no Cargo, test,
formatter, Clippy, architecture, commit or external-write command was run by this drafter.

## Exact registration

Copy `/tmp/sagat-scoped-finish-entry-barriers.rs` to:

`tests/integration/workflow-primitives/scoped_leaf_finish_entry_barriers.rs`

Append this child registration to
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` after the existing
`qualification` and `receipts` children:

```rust
#[path = "scoped_leaf_finish_entry_barriers.rs"]
mod entry_barriers;
```

No helper visibility or production API change is needed: the child uses `super::*` and the current
private native fixtures unchanged.

## Bounded contents

Exactly three tests are drafted:

1. Hold selected G and the original Run, observe the real native Run query blocked behind
   `RunHold`, release the Run, receive `Busy` from G's `NOWAIT`, and prove rollback releases the Run
   while G remains held. Source/capacity snapshots and receipt/audit/hint counts stay unchanged;
   releasing G lets the unchanged command apply once.
2. Hold the original Run and poll two exact same-command native futures. Observe the first actual
   Run query and then a two-waiter recursive `pg_blocking_pids` graph, allowing PostgreSQL's second
   waiter to soft-block behind the first. Release yields exactly one apply and one replay with the
   same original result, one receipt/audit/hint fact set, and unchanged source state.
3. Create the genuine source first, then add the existing explicitly labelled owner chain-membership
   context fixture. Revoke source-certificate and receipt SELECT from the explicit `sagat_app` pool,
   prove both direct probes receive `42501`, check stale authority still refuses earlier, hold the
   applicable chain and observe the native chain query, and prove the original Run remains independently
   lockable `NOWAIT`. Release yields `UnsupportedRunContext` with source/capacity snapshots unchanged
   and zero effects.

All ordering evidence comes from actual PostgreSQL query text, `wait_event_type='Lock'`,
`pg_blocking_pids`, row-lock probes and bounded timeouts. There is no sleep-based ordering claim.
Only owned store/command futures are polled; `Flow` is not spawned. The tests reuse genuine native
source certification, explicit `sagat_app`, and the current fixture's four test-only scoped receipt
column grants. They add no source writer, production grant, non-flat source claim or broader matrix.

## Frozen inputs

The requested starting HEAD was observed as
`cbfb620ddb160ed6439db29ffd147f52c7ee8be6`. During this read-only task the root concurrently
committed the already supplied qualification/receipt controls as
`211394282795bc295318a302d3ec30b71df2791d`; the worktree was clean when rechecked. The draft is
anchored to the requested base plus those named controls. SHA-256 values of the exact inspected
contents are:

| Input | SHA-256 |
| --- | --- |
| `tasks/project-completion/scoped-leaf-finish-native-contract.md` | `b7e983ace83cbb96109afadd3dc688eeec14e680cc985859868b2689675dbacb` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` | `edf76249c4564832039e392d9f6ce281a0e05c6b190d085c2d42f62c10ea5eb1` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_source_controls.rs` | `a05dc4cd31be2a9a35ba4231eda276d8d8ff4f5179a7c74573bb371c93bd1cfd` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_qualification.rs` | `d566473414d330ff8b4eb8090d181f32a933aca47eb063f49b54fa1663a0bf44` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_receipts.rs` | `c87a64561f92abc33aceb35a487b7d28351e39c906110919e52444a972026026` |
| `tests/integration/workflow-primitives/capacity_finish_barriers.rs` | `285f073e0dec5e4b512be4542b297b8f7f1b6a1cb0697017a6e70c40dc1adc31` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |

The existing full-12 qualification/receipt pass is root-supplied prior evidence. This draft neither
reran nor re-claims it.

## Remaining root gates

- Review the draft against the now-frozen merged tree and adjust only concrete compile/test findings.
- Install the child and registration, then run the focused three tests under the explicit private
  `CARGO_TARGET_DIR` and machine limits required by `AGENTS.md`.
- Re-run the affected native family and the existing full-12 qualification/receipt controls on the
  installed tree; record actual outputs and any failure without retrying it away.
- Run the proportional fmt, strict Clippy and architecture gates selected by the root, then perform
  the pending final review and evidence update.
- Keep the later unit-4 items separate: post-shard-wait expiry, source-writer races, injected
  audit/outbox/receipt/deferred-commit failures, unknown COMMIT/process recovery, observations,
  historical/full context matrices, parent/child cases and broader runtime/release gates.

This slice does not close T014-040, T014-043, T014-044, nested runtime, production ceilings, C0,
caller reconstruction, release privileges, B1/B2, or any production/non-flat capacity guarantee.
