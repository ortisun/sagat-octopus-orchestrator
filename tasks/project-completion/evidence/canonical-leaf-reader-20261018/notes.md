# Canonical metadata reader control inventory

Reviewed against repository revision `8f0c125e` on branch
`feature/016-project-completion`. This is a read-only inventory and an unexecuted test draft; no
repository file, test, Cargo command, commit, or external action was performed.

## Exact existing coverage

- `native_certification_is_metadata_only_and_replays_after_later_scope_failure` creates a
  certificate from actual native source evidence, returns the complete unchanged record on replay,
  replays before a stale creation CAS, replays again after later work has failed, and retains one
  certificate/audit. This already exercises equal revision replay and the older-certificate path
  after the Run advances; the latter inequality is implicit in the executed later transition.
- `current_authority_precedes_canonical_replay_and_fresh_owner_keeps_original_record` proves the
  scheduler guard precedes canonical replay and a fresh current owner receives the original record.
- `original_qualified_identity_and_creation_cas_refuse_without_metadata_writes` proves the exact
  Run guard returns `Missing` for wrong tenant/namespace/run, an absent iteration without a
  neighboring certificate reaches `SourceEvidenceMissing`, and a receipt miss reaches stale CAS
  before source proof.
- `native_certification_refuses_unrepresentable_canonical_time` covers the guarded timestamp
  projection at year 9999, year 10000, a far-future finite value, and a representable BC value.
- `dormant_snapshot_constraints_and_namespace_binding_are_real` already covers profile,
  owner/body/return node, index, digest, sequence, finite-time, namespace/Run FK, primary-key and
  audit uniqueness constraints. Repeating those schema cases would add no reader oracle.
- The other native replay/fault tests repeatedly compare the replayed `record` with the first
  committed record. None changes `definition_version_id`, stores
  `certified_run_revision > runs.run_version`, or leaves an exact owner/index absent while a
  neighboring certificate exists.

## Concrete missing controls represented by `controls.rs`

1. A native certificate is created first, then an explicitly owner-only metadata-decoding fixture
   changes only the stored definition pin or certified revision. A schema-valid wrong UUID and a
   certified revision one above the exact guarded Run must each return exactly
   `ContradictorySource`. The test does not present either row as native source proof and does not
   disable a constraint already covered by the schema suite.
2. A native `lines/0` certificate is created first. A stale expected revision is then used as the
   miss sentinel: exact `lines/0` must replay before creation CAS, while absent `missing/0` and
   `lines/1` must not select that neighbor and must reach exactly `StaleRunRevision`. This avoids a
   fake namespace: a wrong namespace would fail the exact Run guard with `Missing` before the
   canonical reader and therefore would not test the five-key lookup.

The public native certifier remains the only production entry point used. There is no public reader
hook, fake authority, fabricated `LockedSourceRun`, or direct call to the crate-private reader.

## Exact registration

Append the contents of `controls.rs` to
`tests/integration/workflow-primitives/leaf_certification_refusals.rs`. No new `mod` declaration is
needed: `tests/integration/workflow-primitives/leaf_certification_native.rs` already registers that
file as `mod refusals`, and the draft intentionally uses its existing `use super::*`, `closed`,
`counts`, and `refusal_state` scope.

If a separate file is preferred, imports and helper visibility must be adjusted; that variant was
not drafted because appending to the existing refusal module has fewer fixture assumptions.

## Remaining fixture assumptions to verify during integration

- The disposable database owner can disable and restore the `ENABLE ALWAYS` trigger
  `leaf_success_append_only`. Existing retention tests already use the same owner capability to
  disable an ALWAYS source trigger, but this exact certificate trigger has not been disabled in an
  existing test.
- `closed()` continues to leave the scheduler authority current and the Run revision stable across
  certification/replay. Existing native tests depend on both properties.
- The trigger restore executes in the same committed transaction and the helper asserts its catalog
  state is `tgenabled = 'A'`; integration still needs to confirm that exact owner/DDL path works in
  the disposable fixture.
- `missing` remains a valid bounded `NodeId`; the stale CAS sentinel ensures neither missing-owner
  nor missing-index control depends on definition-shape or source-proof refusal ordering.

No tests were run and no pass/fail result is claimed.
