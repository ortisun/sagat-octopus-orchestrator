# Scoped Finish 16b process-recovery draft handoff

## Artifacts

- Draft source: `/tmp/sagat-scoped-finish-process.rs`
- Draft SHA-256: `9a89ac5fb3f5facc77704103e528be86a67aa18089756260f1c3a052bfb83cf4`
- Draft length: 598 lines
- This handoff: `/tmp/sagat-scoped-finish-process-notes.md`
- Sealed 16a input: `/tmp/sagat-scoped-finish-commit-loss.rs`, SHA-256
  `4574812672dfe1cfde15088e066f81ab02205113442581c262fdb95d701959a6`

Only these `/tmp` 16b artifacts were written. The repository was read only for this draft. I did
not run tests, build, Clippy, rustfmt, or any executable verification, and I did not commit.

## Install and registration

Copy the source to:

`tests/integration/workflow-primitives/scoped_leaf_finish_process.rs`

Register it in `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` immediately
after the existing commit-loss child:

```rust
#[path = "scoped_leaf_finish_commit_loss.rs"]
mod commit_loss;
#[path = "scoped_leaf_finish_process.rs"]
mod process;
```

The private re-exec name is already fixed to the resulting integration-test path:

`leaf_certification_native::scoped_finish::process::native_reconciliation_probe`

The draft imports the four `pub(super)` 16a helpers through `super::commit_loss` and the shared
proxy through `crate::capacity_finish::commit_proxy`; it contains no duplicate proxy or fault
trigger implementation.

## Covered proof

The public test loops over actual `Committed` and deferred `23514` abort resolutions. For each
isolated real flow it:

1. Builds a genuine flat certified source with `certified_flow`, seeds one selected holder, and
   persists the complete immutable command as an eleven-field private JSON object: tenant,
   namespace, Run, path owner/index, G, member, operation key, expected G revision/epoch, and
   expected Run revision. Scheduler owner/generation are deliberately absent because they are
   renewable authority.
2. Starts the real 16a COMMIT-ack-loss proxy as `sagat_app`, holds its deferred advisory COMMIT
   gate, drives the original writer to typed `CommitOutcomeUnknown { operation_key }`, and keeps
   the original server transaction unresolved.
3. Re-execs the integration binary as a separate OS process. Its database URL exists only in a
   child environment variable; argv contains only the exact probe selector, and stdio is null.
   The command and result files contain no credential. The child owns a one-connection `sagat_app`
   pool and an actual `PgScopedLeafFinishStore`.
4. Observes that child's backend blocked on the original Run, records its PID and backend PID,
   checks native Run/G/shard NOWAIT guards plus zero effects and unchanged source/capacity, kills
   and reaps the child while the original server outcome is unresolved, and proves the killed
   child produced no result.
5. Releases only the original advisory gate, observes the configured real server outcome, waits
   for the killed child's old backend to disappear, removes the test trigger, and preserves the
   exact durable command bytes.
6. Releases the old scheduler lease and acquires a fresh owner/generation. A new OS process reads
   the same file and queries only current scheduler owner/generation before reconstructing the
   command. It does not rediscover or refresh any CAS expectation.
7. For committed resolution, checks exact original receipt replay including audit ID, outbox ID,
   timestamp, counters, and result. For aborted resolution, checks the old transaction left no
   receipt/effects and the same old CAS command applies once with fresh nonnil IDs. Both paths end
   with one linked audit/receipt/hint set, retired G `(11,5,0)`, unchanged genuine source snapshot,
   unchanged fingerprint, unchanged command bytes, a new PID, and no killed-child result.
8. Reconstructs the same durable command with the already discovered fresh scheduler authority in
   a new one-connection runtime pool and performs an exact post-recovery replay. This binds the
   aborted branch's fresh application to a stable receipt/result and rechecks that replay changes
   no capacity projection; for the committed branch it is a second exact replay of the receipt
   captured before the replacement was launched.

The second test is the ignored private subprocess probe. It rejects execution without its private
marker, validates the closed eleven-field file, asserts `current_user = sagat_app`, discovers only
qualified current scheduler owner/generation, reconstructs the command, calls the real store, and
writes a closed result file.

## Current API plausibility (read only)

Observed at repository HEAD `80fc39176e3440d351eb4d8fc1179c8047a090aa` while drafting:

- `tests/integration/workflow-primitives/scoped_leaf_finish_commit_loss.rs` SHA-256
  `46a978de20bdec45c6cafd5caf946ec4ce559f5f31644b1df5dd5971c375939d`; its four helper signatures
  match the imports used here. Its content differs from the sealed `/tmp` input only by root's
  formatting and claimant-cleanup integration changes observed during concurrent work.
- `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` SHA-256
  `d14d3907ed41fde38b7ea2bd0847309f9e821daa73a8df10987e92ce4a2b450f`; it exposes the fixture,
  snapshot, receipt-effect, and assertion helpers used by the child module through `super::*`.
- `crates/application/src/scoped_leaf_finish.rs` SHA-256
  `f389cf29a48684f24fa76d9cd4da567b1bea6a81ae52bc1f0a7a06ce0038e2e3`; command/result/error and
  fingerprint fields match the draft.
- `crates/application/src/primitives.rs` SHA-256
  `bd58299fa2104543cb4d84d529e1d105c2ee92254fe76ca444319d3b91a617ea`; current
  `SchedulerAuthority::for_run(owner, run, generation)` accepts the reconstructed authority.
- `crates/infra-postgres/src/scoped_leaf_finish.rs` remains from commit
  `7b0ced121d5c2cb4bbc735d513be4894bebd5923`, git blob
  `a0bd57ac23bbd40c055c7df7a18c820830ccb9ef`, SHA-256
  `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780`; its real Run entry query,
  receipt fields, replay precedence, and `PgScopedLeafFinishStore::new(PgPool)` match this draft.
- Reference process test `tests/integration/workflow-primitives/leaf_certification_process.rs`
  SHA-256 `65e1222b205b288e2194249668c993b14f5400b61f3150448e85180b4c14fe33` supplied the actual OS
  process, backend observation, kill/reap, and fresh-process shape. This draft intentionally does
  not copy its fresh Run-CAS discovery.
- Reviewed native contract SHA-256
  `c7f4b2b7d278acda07b0fcbb33f31c3c6b16c4454bfb7cb068e7839f84d5a81d`.

Because the repository changed concurrently during drafting, these are observed source identities,
not a frozen-tree claim. Root should inspect the installation diff and run formatting/tests on its
final tree.

## Suggested root verification

After installation and formatting, the focused commands are:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_workflow_primitives \
  leaf_certification_native::scoped_finish::process::killed_scoped_reconciler_restarts_complete_old_command_after_real_commit_or_abort \
  --locked -- --exact --nocapture

SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_workflow_primitives \
  leaf_certification_native::scoped_finish --locked -- --nocapture
```

Then apply the repository's normal fmt, strict Clippy, architecture, and frozen-input gates. No
result is claimed here for any command.

## Honest boundary

This proves death and cold restart of the reconciliation client process while a separate original
database writer remains alive until PostgreSQL resolves its deferred COMMIT. It does not crash or
restart the original writer process, execution engine, PostgreSQL, test harness, or control plane.
It does not prove production caller wiring, production grants, command-file durability across host
or storage failure, crash-safe/atomic file writes, secret-manager delivery, C0 retention, group
lifecycle/admission/accounting, non-flat source closure, nested paths, or general process recovery.
