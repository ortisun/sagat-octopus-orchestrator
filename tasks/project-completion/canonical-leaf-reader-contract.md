# Canonical ordinary-leaf metadata reader extraction

Reviewed contract at8d15bacc; bounded prerequisite for the future scoped Finish adapter.
Decision only: no repository changes, tests, native Finish transaction, grants or production caller.
Reuse the existing reader in infra-postgres::leaf_certification; do not create a certification
command, fabricate LockedSourceRun, invoke the certifier from Finish, or duplicate its decoder.

```rust
pub(crate) async fn read_canonical_leaf_success(
    tx: &mut Transaction<'_, Postgres>,
    identity: &LeafCollectionPathIdentity,
    actual_definition_version: WorkflowVersionId,
    actual_run_revision: u64,
) -> Result<Option<LeafCollectionSuccessRecord>, LeafCollectionSuccessError>;
```

Keep this function in leaf_certification.rs; its crate-private visibility permits a future sibling
adapter. No public re-export, application port, generic reader, new identity/snapshot VO or DTO.
Use the genuine application identity and record directly. Keep LockedSourceRun private to the
certifier: its trace and clock are required for creation, not for canonical metadata retrieval.
The certifier passes `&c.identity, run.version, run.revision` from its existing locked read.

Caller preconditions, held throughout the same transaction: applicable own-chain ordering,
exact `(tenant, namespace, originalRun)` Run guard, actual pin/revision from that guarded row,
and current original-run scheduler fence with the existing postwait check. Never pass expected
CAS counters, caller claims, another run's pin or values read in another transaction.
Future Finish also retains its already specified selected-G/binding guards before this lookup.
The helper supplies no explicit row locks, scheduler check, authority capability or chain validity;
ordinary SELECT relation locks are not a replacement for these caller obligations. It neither
begins nor commits/rolls back a transaction and cannot establish that its preconditions were met.

Move the existing canonical() body without changing its SQL selection or decode/error order:
one fixed fetch_optional on leaf_collection_success_certifications, qualified by all five keys
`tenant_id, namespace_id, run_id, owner_node, iteration_index` derived from identity depth1.
No wildcard projection, join, source-table lookup, payload, JSON, sibling/history scan or FOR UPDATE.
Project the existing profile-equality boolean and exact record metadata columns only. Cardinality
is at most one under the retained primary key; node/digest sizes rely on the existing schema
constraints plus the existing NodeId/32-byte decoder checks. This is not a corruption-hardening
expansion that bounds arbitrary owner-written oversized values after constraints are removed.
Preserve this SQL projection before SQLx can decode certified_at:
`CASE WHEN isfinite(certified_at) AND certified_at<TIMESTAMPTZ '10000-01-01 00:00:00+00' THEN certified_at END AS certified_at`.
NULL from that guard is ContradictorySource; do not fetch the raw timestamp and guard afterward,
clamp it, convert it to an unchecked integer, or reject the existing representable BC control.

On no row return Ok(None). On a row check the closed profile first; then decode UUIDs, parsed nodes,
nonnegative i64-to-u64 counters, exactly 32-byte digests and the guarded timestamp with fallible
access. Reconstruct the record identity from the exact key and its return instance in that path;
reuse validate_structure for positive signed-range counters, ordering and adjacency relationships.
Finally require record.definition_version == actual_definition_version and
record.certified_run_revision <= actual_run_revision. Return the unchanged original record,
including its original timestamp/audit ID; do not substitute current revision/time or require equality.

The helper emits only UnsupportedStoredProfile, ContradictorySource or Storage(StoreError).
Reuse existing safe storage classification: SQLSTATE only if five ASCII alphanumerics; otherwise
fixed diagnostic. Preserve existing safe Display/Debug; never attach raw driver text, SQL, bind
values, records or owner text. Existing application error reuse avoids a new empty error wrapper.
Future Finish maps None to SourceEvidenceMissing and these three variants to its corresponding
semantic errors, preserving the sanitized Storage value. It must not map None to Missing or
an empty success, or turn malformed source into an unfinished/source-creation condition. Any new
helper error variant requires an explicit consumer-contract change; no silent catch-all downgrade.
No Finish translator is implemented in this extraction.

Certifier behavior remains command validation -> begin -> applicable chain -> exact Run -> current
fence -> canonical read. Some(record) replays before creation CAS, definition/shape/source proof,
current run outcome or a new audit. None continues the unchanged creation-CAS/prove/insert/audit
path. All rollback, COMMIT uncertainty and observations stay in the existing transaction wrapper.
This changes only reader inputs/name/visibility and its call site, not the source writer's proof.
Future Finish's approved order remains authority -> validated canonical metadata -> operation
receipt -> creation CAS/member checks. Receipt replay cannot bypass this read or its refusal.

What a successful read establishes: an exact persisted certificate exists, its supported metadata
is structurally decodable and agrees with the supplied actual pin/revision. Its SQL key binds the
returned identity; validate_structure alone is not that database lookup. It does not re-prove the
scope, checkpoint, step set, journal events, digests' underlying content or audit-row existence.
A schema-valid coherent owner-inserted certificate can pass; this is not arbitrary database-owner
corruption detection. Original source proof belongs to the certifier's prove() and checked writers;
retention keeps evidence available and does not prove producer UPDATE preservation. Do not add
source revalidation on replay to this refactor. No chain-source acceptance, topology closure,
current-head/finality proof, release permission or full scoped Finish guarantee follows.
D298 retention, append-only certificate guards, runtime SELECT-only privilege and pending producer/
release gates remain prerequisites; extraction creates no INSERT, migration or production caller.

Verification for a future implementation (not executed for this draft): establish the current
baseline and rerun the same existing native cases after extraction, using real PostgreSQL fixtures:
- native_certification_is_metadata_only_and_replays_after_later_scope_failure: actual native source,
  unchanged record/source/revision, stale-CAS replay, later failed Run, one audit and rollback.
- current_authority_precedes_canonical_replay_and_fresh_owner_keeps_original_record: authority first.
- original_qualified_identity_and_creation_cas_refuse_without_metadata_writes and
  outstanding_reservation_refuses_and_creation_cas_precedes_source_corruption: unchanged miss/CAS.
- native_certification_refuses_unrepresentable_canonical_time: 9999, 10000, far-future and BC controls;
  native_certification_guard_failure_redacts_driver_diagnostics: preserved safe errors.
Add only missing metadata-reader controls using the existing disposable native fixture/entry point:
schema-valid wrong stored definition pin and certified revision above the actual Run must refuse;
a matching exact key with equal or older certified revision must return the complete original row;
an absent owner/index must not select a neighboring certificate. Keep unrelated source proof out
of these controls and label owner-created rows as decoding fixtures, never native source evidence.
The existing schema suite already checks unsupported-profile, node/index/digest/sequence/infinity
constraints; do not bypass production guards just to duplicate those tests. If the decoder/query
is changed beyond extraction, add a targeted malformed/profile/null decode control for that change.
Keep helper crate-private; do not add a public test hook. Existing behavior may pass before this
extraction; retain that baseline honestly. Add new controls only for concrete missing oracles.
These tests plus the unchanged existing source assertions substantiate extraction, not Finish.

Anchors at this revision: infra-postgres/src/leaf_certification.rs:38,51,74,80,196,217,236,258,302;
application/src/leaf_certification.rs:25,100,122,159; all under crates/.
Migration: crates/infra-postgres/migrations/20261211000000_leaf_success_certification.up.sql:24,46,48,55,85.
Reviewed contracts: tasks/project-completion/scoped-leaf-finish-application-contract.md:92,103,126;
leaf-certification-transaction.md:10,18,28,65,84; leaf-certification-storage.md:26,42,65 in the same dir.
Native tests: tests/integration/workflow-primitives/leaf_certification_native.rs:139,188,241;
leaf_certification_refusals.rs:57,114,182 in the same dir. No unresolved extraction decision identified.

One fresh independent Astra/xhigh bounded read-only review found no actionable issue. Root
reconciled test guidance to retain actual passing baselines for behavior-preserving extraction;
no artificial RED failure is required. No tests, executable extraction or Finish authority claimed.


## Implemented bounded extraction — 2026-10-18

The genuine identity/locked pin/revision signature is crate-private in leaf_certification.rs;
its exact SQL, strict metadata decoding and existing certifier ordering are preserved. Two
new real PostgreSQL controls passed before extraction, then the complete current native
certifier suite passed21/21 (one private subprocess probe ignored in direct enumeration).
Those controls inject owner-only schema-valid wrong pin/future revision, restore ALWAYS
append-only enforcement, and distinguish exact owner/index misses from neighboring replay
before stale creation CAS. They are decoder tests, not native production mutation proofs.
Strict targeted Clippy/fmt/architecture pass. Two fresh bounded reviews found no issue.
No new authority, certificate writer/INSERT permission, scoped Finish consumer or caller follows.
Evidence: evidence/canonical-leaf-reader-20261018/manifest.json.
