# Scoped Finish writer-order draft handoff

## Deliverables

- Draft test module: `/tmp/sagat-scoped-finish-writer-order.rs`
- This handoff: `/tmp/sagat-scoped-finish-writer-order-notes.md`
- Repository was read only. No repository file, commit, test evidence, formatting result, or gate result was produced.

The worktree started at the requested `c332bffc`. Root concurrently integrated the entry-barrier unit and a scheduler-expiry unit; the final source inventory below was read at `76df372bd5019012360288270a26b9031fa1d392`.

## Registration

Copy the draft to:

`tests/integration/workflow-primitives/scoped_leaf_finish_writer_order.rs`

Register it as a direct child of `scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_writer_order.rs"]
mod writer_order;
```

This nesting is intentional:

- `use super::*` reaches the scoped fixture helpers and types (`certified_flow`, `seed_group`, `finish_command`, source/capacity snapshots, linked-effect assertions, and the private fixture structs).
- `super::super::late_reports::{observe_first, observe_order}` reaches the existing `pub(super)` lock-queue observers in `leaf_certification_native.rs`.
- `RunHold` is imported from `crate::support::scope_flow`.

No production registration or caller is included.

## Drafted controls

### 1. First native certifier versus scoped Finish

`actual_first_certifier_and_scoped_finish_obey_both_original_run_orders`

- Builds a real public flat flow and closes the selected `lines/0` source without first creating a certificate.
- Does not delete, replace, or mutate an immutable certificate.
- Queues the actual native certifier and actual scoped Finish behind `RunHold`, then observes both queue directions through the existing `pg_blocking_pids` helpers.
- Certifier first: the certificate commits, then Finish applies.
- Finish first: Finish returns `SourceEvidenceMissing`, leaves the exact capacity snapshot and operation receipt absent, the certifier commits, then the unchanged Finish command applies.
- Revokes source INSERT after the pair, proves the actual certifier replays with INSERT denied, proves Finish replays its original result, and keeps one certificate/one Finish receipt/audit/hint.
- Compares the exact selected source, Run/journal, steps, and topology across the pair. The certificate digest is expected to change from absent to present.

The temporary source INSERT grant necessarily overlaps the first-creation race: the genuine certifier cannot insert after its Run wait if the grant is revoked before Finish is invoked. This is confined to the disposable fixture. Existing native tests already prove scoped Finish apply/replay with certificate INSERT revoked; this draft additionally checks revoked-source certifier replay after the race. It does not claim that Finish required the INSERT grant.

### 2. Actual later worker versus scoped Finish

`actual_later_worker_and_scoped_finish_obey_both_original_run_orders`

- Uses the existing genuinely certified public flow and a genuinely leased `later/0` activity.
- Exercises both actual worker completion and actual worker failure, each with both observed Run queue orders.
- Finish first: Finish applies; the later report advances the Run; the old Finish key replays its retained result.
- Writer first: the old Finish returns `StaleRunRevision` with no capacity change or receipt. Rediscovery supplies the new Run revision and a fresh logical operation key; that fresh command applies. The original stale key remains receipt free.
- Compares only `selected_source` and `certificates` across the race and later scheduler drive. Run/journal/steps are deliberately excluded because the real later report and planning legitimately change them.
- Proves one linked Finish transaction and the expected terminal Run status (`SUCCEEDED` for completion, `FAILED` for failure).

### 3. Two distinct genuinely certified members

`two_genuine_scoped_members_complete_in_either_observed_run_queue_order`

- Uses two real certified source indices and two matching genuine scoped members in one group.
- Forces each member to lead the Run queue once.
- The leader applies at group revision 10; the follower's original key gets `StaleGroupRevision` and no receipt.
- Fresh discovery for the independent second completion uses a new key and revision 11, then retires the group at revision 12.
- Both orders end with exactly two receipts/audits/hints and unchanged selected-source/certificate digests.

The two Finish SQL texts are identical, so the existing queue observer identifies direction by the already observed leader and its soft-blocked follower. This is deterministic in the isolated database but does not distinguish them from bind values in `pg_stat_activity`.

## Intended verification by root

Per assignment, this author ran no tests, build, Clippy, formatter, architecture check, Docker command, or agent review. Root should first copy/register the module, then run the focused native target under the repository's private external `CARGO_TARGET_DIR` and machine limits. The expected new test names are the three names above; their loops cover two, four, and two order cases respectively.

The draft may need ordinary `cargo fmt` output after integration. Any compilation or runtime failure remains unevaluated and must be reported as such until root runs it.

## Source inventory at handoff

HEAD: `76df372bd5019012360288270a26b9031fa1d392`

```text
a77f9def254ba2bedb9a9023651790dd12e8f0a9df665a6d0974ab11d2e4a3c3  tests/integration/workflow-primitives/scoped_leaf_finish_native.rs
e12945e2290d18c1a217b4a4058c0c852104ff1d0476038f9f331c8d4aeac31f  tests/integration/workflow-primitives/scoped_leaf_finish_entry_barriers.rs
4d4fba7f2e0e8647b220678890eba55c74095b704136cd71fc6a8d2d81f75e7e  tests/integration/workflow-primitives/leaf_certification_late_reports.rs
e00184bf3cd342965923aeeb9574ab4b69c57e69c4f67d1952ca07991a5bad1c  tests/integration/workflow-primitives/leaf_certification_later_work.rs
468e0505b935e3696e5ee0aa28d32f51d338e23eda5241b0d5bb7361b454f14d  tests/integration/workflow-primitives/leaf_certification_native.rs
c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821  tests/support/scope_flow.rs
832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780  crates/infra-postgres/src/scoped_leaf_finish.rs
2fb6e26bdf16d176bc35c95be7d3edb86746a0eb1e0e909fae727e7d8342d0f5  /tmp/sagat-scoped-finish-writer-order.rs
```

Relevant contract read in full: root `AGENTS.md`, `.specify/memory/constitution.md`, feature 014 `spec.md`, `plan.md`, `tasks.md`, and `tasks/project-completion/scoped-leaf-finish-native-contract.md`. `.specify/feature.json` was absent in this worktree; the assignment and active files identify feature 014.

## Scope limits

- No contextual source-closure claim: the selected source is the existing restricted public flat OrdinaryLeafV1 path.
- No certificate deletion or guard disabling.
- No production source or receipt privilege grant.
- No cancellation writer case; actual later completion and failure are covered. Cancellation would add a third writer family and is unnecessary for the requested later worker control.
- No whole-execution equality after a legitimate later writer.
- No retained-quota wait, nested source, child/chain source, production caller, C0 retention claim, admission/accounting claim, or broader B1/B2 closure.
- No test result is claimed.
