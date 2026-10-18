# Scoped native Finish context-refusal draft handoff

## Artifact and registration

- Draft source: `/tmp/sagat-scoped-finish-contexts.rs`
- Intended repository path: `tests/integration/workflow-primitives/scoped_leaf_finish_contexts.rs`
- Register at the end of `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_contexts.rs"]
mod contexts;
```

The draft adds tests only. It does not change production code or make a new semantic decision.

## Bounded matrix

The adapter's complete context predicate is `(parent_run_id IS NULL) && !exact_chain_membership`.
The draft keeps the matrix small and uses public runtime paths for every new Run or chain fact.

| selected original Run | exact chain membership | evidence |
|---|---:|---|
| parentless | absent | existing positive native Finish family |
| parentless | present, historical | new actual public `continueAsNew` handoff test; original segment is `CONTINUED`, successor is current, chain remains unfinished |
| child | absent | new actual ordinary collection child, first live and then publicly detached from a cancelled parent |
| child | present | new actual continuation child driven to a settled `FINAL`/`SUCCEEDED` one-segment chain |

The existing entry-barrier test already covers owner-seeded parentless chain membership and the
actual applicable-chain wait. The new historical test supplies the missing public handoff case and
detects an erroneous guard restricted to the current segment.

## Tests

1. `native_scoped_finish_refuses_actual_public_child_before_and_after_detachment`
   - Publishes and starts an ordinary collection as a real child Run.
   - Proves `parent_run_id` is present, exact chain membership is absent, and no child certificate
     exists.
   - Checks refusal while the child is live, then cancels the parent through the public operation
     API with `detach`, proves `parent_settled=true` and the child remains unfinished, and checks the
     same refusal again.

2. `native_scoped_finish_refuses_actual_settled_final_child_chain_segment`
   - Publishes a real continuation child, completes its activity through the worker path, and drives
     its terminal branch.
   - Proves child parentage plus `FINAL` segment, `SUCCEEDED` chain outcome, and settled child-chain
     link before invoking Finish.

3. `native_scoped_finish_refuses_actual_parentless_historical_chain_segment`
   - Starts a parentless continuation workflow through HTTP, completes the initial segment, and
     drives an actual handoff.
   - Proves the selected original Run has no parent, is finished `SUCCEEDED`, its segment is
     `CONTINUED`, `successor_run_id` is the chain's distinct current Run, and the chain is not final.
   - Therefore only historical exact chain membership can cause `UnsupportedRunContext`.

Every case owner-seeds only the capacity registry member needed to reach the adapter. The synthetic
path `lines/0` is explicitly labelled as registry identity. It is not a canonical certification.

## Ordering and rollback assertions

The shared assertion:

- derives current scheduler owner/generation from the actual selected Run and renews that lease;
- submits stale authority first and requires `SchedulerFenceLost`;
- uses a pool whose `current_user` is explicitly asserted as `sagat_app`;
- revokes certificate and receipt `SELECT` from `sagat_app` in the disposable database;
- checks effective privileges are false and executes direct probes that must return SQLSTATE
  `42501` for both tables;
- submits current authority and requires `UnsupportedRunContext`, proving the context gate occurs
  before source absence and receipt lookup;
- compares exact source/execution/topology and capacity snapshots and asserts zero receipt, audit,
  or outbox effects.

The genuine certificate made by `certified_flow` remains attached only to the separate flat Run.
Each test proves certificate counts `(flat=1, selected context Run=0)`. No fixture claims child or
historical-segment source closure.

## Honest limits

- This agent did not copy the file into the repository and did not run formatting, compilation,
  tests, builds, or architecture checks, per the read-only assignment. The root agent must run all
  gates after integration.
- The draft proves the existing context predicate and its ordering. It does not establish a
  production caller, grants, nested leaf-source certification, or non-flat Finish support.
- Parent settlement, Run terminal status, segment completion kind, chain final outcome, and detach
  state are overlays on the two predicate dimensions, not additional adapter gates.
- No schema surgery or owner-written chain topology is used. Owner writes are limited to the
  existing capacity fixture and test-only privilege changes.
- The live child-chain test covers a terminal one-segment child chain. The parentless historical
  test covers public rollover to a distinct current successor. Broader lifecycle combinations are
  intentionally outside this bounded slice.

## Source baseline

- Worktree HEAD observed: `76df372bd5019012360288270a26b9031fa1d392`
- `crates/infra-postgres/src/scoped_leaf_finish.rs`
  - git blob `a0bd57ac23bbd40c055c7df7a18c820830ccb9ef`
  - SHA-256 `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780`
- `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`
  - git blob `44bd2a9ab840d6c57f56497bc91aba6a250dc356`
  - SHA-256 `a77f9def254ba2bedb9a9023651790dd12e8f0a9df665a6d0974ab11d2e4a3c3`
- `tests/integration/workflow-primitives/scoped_leaf_finish_entry_barriers.rs`
  - git blob `d9e9b4370c41c8d4681d781f39fbf238041217b4`
  - SHA-256 `e12945e2290d18c1a217b4a4058c0c852104ff1d0476038f9f331c8d4aeac31f`
- `tests/integration/workflow-primitives/leaf_certification_native.rs`
  - git blob `ed1fe34f93882318c8eee6f231b8f2155f427f2e`
  - SHA-256 `468e0505b935e3696e5ee0aa28d32f51d338e23eda5241b0d5bb7361b454f14d`
- `specs/014-workflow-primitives/contracts/README.md`
  - git blob `7883f80d4af06dd3f41bd1c54ae89d5ff270af78`
  - SHA-256 `ad180cd616caec3fba70e62ad4272f8a09bf5728c81a0432c8ddecbd182c8c41`
- Draft SHA-256: `1d35fe2191efd3ed0c756ca583b151355e606b743bb006a60f2ef60f68c818bf`

Recheck these hashes if the root worktree advances before integration.
