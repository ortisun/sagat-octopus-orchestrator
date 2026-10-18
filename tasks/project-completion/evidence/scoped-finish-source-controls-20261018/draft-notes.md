# Scoped Finish source controls draft notes

Base: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion` at
`7b0ced121d5c2cb4bbc735d513be4894bebd5923`.

Draft: `/tmp/sagat-scoped-finish-source-controls.rs`.

Frozen source SHA-256: `7238e75fd7e04c5bef294c21c95931354596fc01166bff631d6b007c55360622`.

## Exact parent registration

Install the draft as:

`tests/integration/workflow-primitives/scoped_leaf_finish_source_controls.rs`

Then add this registration to
`tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_source_controls.rs"]
mod source_controls;
```

The child deliberately uses `use super::*;` to reuse the parent's private current-flow fixtures and
oracles: `certified_flow`, `seed_group`, `active_member`, `finish_command`,
`source_execution_snapshot`, `capacity_snapshot`, `finish_counts`, and `assert_linked_effect`.
No parent helper was copied or changed.

## Bounded coverage

The draft contains three real-PostgreSQL tests:

1. `native_scoped_finish_validates_certificate_pin_before_matching_receipt`
   creates genuine certificates for a two-item current Flow, applies one genuine Finish and keeps
   its receipt. Labelled owner-only fixtures then install a schema-valid wrong definition pin and a
   future certified revision. The matching command and a same-key different-fingerprint command
   must both return exactly `ContradictorySource` before receipt decoding. After exact restoration,
   the different fingerprint returns `FingerprintConflict` and the original receipt replays. Owner
   source/Run/history/certificate and definition snapshots plus the full capacity transaction
   snapshot prove the refused calls added no mutation.
2. `native_scoped_finish_requires_retained_certificate_before_receipt_and_after_fence`
   applies a genuine Finish, removes the actual selected certificate only through a labelled owner
   immutable-guard fixture, and expects `SourceEvidenceMissing` before the matching receipt. A stale
   scheduler authority still returns `SchedulerFenceLost` before missing-source inspection. The
   exact certificate is restored, the guard is checked as ALWAYS, source and capacity snapshots are
   unchanged, and the original receipt replays without new effects.
3. `native_scoped_finish_applies_and_replays_from_certificate_metadata_only`
   certifies first, then revokes runtime-role SELECT from `iteration_scopes`,
   `collection_iterations`, `iteration_checkpoints`, `steps`, `scope_sagas`,
   `scope_saga_entries`, `run_events`, `workflow_versions`, and `audit_records`. Direct sagat_app
   probes require SQLSTATE 42501 for every table. The same explicit sagat_app pool retains
   certificate SELECT, has no certificate INSERT, and retains audit INSERT without audit SELECT.
   Genuine first application and exact replay must succeed using retained certificate metadata;
   owner snapshots show the source, Run, certificate and history do not change, while only the
   exact capacity member/header, one receipt, one audit and one hint appear on first application.
   No workflow runtime is driven after revocation.

The multi-row shared snapshot preserves the first review's C1 correction; the tests do not use the
older scalar `refusal_state` helper. Existing exact linked-time and full capacity/source snapshot
oracles preserve the other C1 corrections. The integrated C2 review reported no issue and is not
reinterpreted here.

## Fixture boundary

The wrong-pin update and actual certificate removal are explicitly labelled owner-only disposable
fault fixtures. Each transaction restores `leaf_success_append_only` with `ENABLE ALWAYS` before
the sagat_app consumer can observe the corrupted or missing row. The exact genuine row is restored
before final source assertions. The disposable wrong workflow version is removed under its owner
guard and that guard is restored with `ENABLE ALWAYS` in the same cleanup transaction. These tests
do not claim that a native/runtime writer can corrupt immutable source evidence.

## Deliberate gaps

This unit covers only source-before-receipt ordering and metadata-only evidence. It does not add the
unsupported-profile or malformed decoder matrix, exhaustive qualification permutations,
cross-kind receipts, parent/chain context closure, out-of-range receipt time controls, raw diagnostic
canaries, lock/wait barriers, transactional fault injection, unknown-COMMIT/process recovery, or the
returned-observation matrix. Those remain separate contract units 14 through 17. It adds no caller,
production privilege, production migration, source writer, capacity lifecycle, admission path, or
release claim.

No repository file was modified. Per assignment, no formatter, Cargo command, test, database run,
commit, or external write was performed. The draft therefore makes no compilation or execution
claim; the root agent remains the sole installer, formatter and test executor.
