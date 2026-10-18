# Scoped leaf Finish unknown-COMMIT draft handoff

Drafted read-only against `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
The only authored artifacts are this note and `/tmp/sagat-scoped-finish-commit-loss.rs`.

## Install and registration

Copy the Rust draft to:

`tests/integration/workflow-primitives/scoped_leaf_finish_commit_loss.rs`

Register it near the other scoped native children at the bottom of
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_commit_loss.rs"]
mod commit_loss;
```

The current root worktree is concurrently adding `writer_order`, `atomic_failures`, and `contexts`
children after `shard_expiry`; adding `commit_loss` after those entries is mechanical. Do not replace
or discard those root-owned edits.

The proxy import is intentionally the current crate path:

```rust
use crate::capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution};
```

`capacity_finish.rs` declares `pub(crate) mod commit_proxy`, so the nested leaf/scoped test can use
the already reviewed one-shot wire relay without a second proxy implementation.

The test-only `install_commit_probe`, `remove_commit_probe`, `waiting_commit`, and
`own_guards_held` helpers are `pub(super)`. A later sibling 16b process probe can reuse them through
`super::commit_loss`. That later probe must persist and reconstruct the complete original scoped
command: group, member, operation key, leaf identity, expected group revision, expected group epoch,
and expected Run revision. It may rediscover only renewable current scheduler authority. It must not
copy the leaf-certifier process test's fresh-CAS command discovery.

## Covered behavior

The single parameterized test executes both backend resolutions:

- actual COMMIT with its reply lost;
- actual deferred `23514` abort with its reply lost.

For each resolution it:

- creates genuine flat `OrdinaryLeafV1` source evidence through `certified_flow`;
- retains the existing disposable four-column scoped receipt grant and proves the proxied connection
  is really `sagat_app`, while certificate INSERT remains denied;
- starts and admits a distinct real public root collection Run but never completes or certifies its
  member;
- seeds both exact bindings into the same owner-created group;
- blocks the deferred receipt trigger on an advisory transaction gate after COMMIT dispatch;
- proves the original Run, selected G, and scheduler shard still reject conflicting NOWAIT probes;
- closes only the client reply direction and checks the typed
  `CommitOutcomeUnknown { operation_key }`;
- sends the distinct public Run to G and requires `Busy` before that Run's absent certificate could
  be consulted;
- locally polls the complete original command until it is visibly waiting at the original Run,
  verifies no claimant result exists, and cancels only that unresolved claimant future;
- observes the proxy's actual backend C/E plus idle resolution before interpreting state;
- uses a fresh pool/store with the unchanged complete original command, key, identity, expectations,
  and fingerprint;
- requires replay after the actual commit, first application after the actual abort, then one stable
  replay in both cases;
- keeps the genuine source snapshot unchanged and finishes with exactly one receipt, audit, and
  scoped hint linked to the selected member.

No `tokio::spawn` captures `Flow`; operation futures are boxed and polled locally. The relay's own
existing internal task remains inside `CommitAckLossProxy`.

## Source identity and hashes

Repository HEAD observed: `76df372bd5019012360288270a26b9031fa1d392`.

The production adapter remained unchanged. Its last modifying commit is
`7b0ced121d5c2cb4bbc735d513be4894bebd5923`; current Git blob is
`a0bd57ac23bbd40c055c7df7a18c820830ccb9ef`.

SHA-256 inputs observed while drafting:

| File | SHA-256 |
| --- | --- |
| `/tmp/sagat-scoped-finish-commit-loss.rs` | `4574812672dfe1cfde15088e066f81ab02205113442581c262fdb95d701959a6` |
| `capacity_finish_proxy.rs` | `98838f211342a30320300786c474e46f0cd87005a9d5674e1787d07a5b662b23` |
| `capacity_finish_commit_loss.rs` | `6c0624fadac91b3cb456faa5da43ba78f0906cc6533aa8880c1088c5162a1424` |
| `leaf_certification_commit_loss.rs` | `ab20f174accfc5d0db192ce64b2bc29b5fc81c572bfbfc2a5119a6155b4a2e8b` |
| HEAD `scoped_leaf_finish_native.rs` | `a77f9def254ba2bedb9a9023651790dd12e8f0a9df665a6d0974ab11d2e4a3c3` |
| concurrently edited working-tree `scoped_leaf_finish_native.rs` | `2591eb3d678bd26a61d8b3ef099b9dcdfdbb8b0f9afd43a0007d524efaef826b` |
| `crates/application/src/scoped_leaf_finish.rs` | `f389cf29a48684f24fa76d9cd4da567b1bea6a81ae52bc1f0a7a06ce0038e2e3` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |

The worktree became dirty during drafting from root-owned unit 15 files and registration. This
draft did not alter any repository file.

## Current API plausibility

The draft follows current observed signatures:

- `certified_flow`, `seed_group`, `finish_command`, `source_execution_snapshot`,
  `capacity_snapshot`, `finish_counts`, `assert_linked_effect`, and `assert_header` are private
  ancestor helpers available to a child module through `use super::*`;
- `PgScopedLeafFinishStore::new(PgPool)` and
  `ScopedLeafFinishStore::finish_scoped_leaf(&ScopedLeafFinishCommand)` match the current adapter;
- `ScopedLeafFinishError::CommitOutcomeUnknown` carries only `operation_key`;
- `CommitAckLossProxy::start`, `arm`, `wait_sent`, `lose_reply`, `resolution`, and `close` match the
  current proxy;
- `Flow.api`, `Flow.scheduler`, `TestApi::as_user`, `TestDb::runtime_pool_with`, scheduler `tick`, and
  `ShardStore::renew` match current fixtures;
- `super::super::document` addresses the existing private public-workflow document helper from the
  nested `commit_loss -> scoped_finish -> leaf_certification_native` module path.

Likely compile-sensitive points for the installer to inspect after concurrent merges are the nested
module path to `document`, ancestor helper visibility, and any newly introduced child-module import
collision. No production API or adapter edit should be needed.

## Verification still required by root

No build, format, test, Docker, or Clippy command was run, as requested. The draft therefore claims
no passing result. Root should first inspect the installed diff, then follow the repository's disk
and private `CARGO_TARGET_DIR` rules and run the focused exact test, followed by the scoped family and
required gates. The focused selector should be:

```text
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  leaf_certification_native::scoped_finish::commit_loss::unknown_scoped_commit_reconciles_real_commit_and_abort_with_the_complete_old_command \
  -- --exact --nocapture
```

This unit is intentionally limited to in-process client acknowledgement loss and cold pool/store
reconstruction. Actual process death belongs to separate unit 16b. Returned observation coverage,
production caller intent, production grants, C0 retention/mixed writers, lifecycle creation and
admission, nesting, quota accounting, and broader B1/B2/runtime acceptance remain outside it.
