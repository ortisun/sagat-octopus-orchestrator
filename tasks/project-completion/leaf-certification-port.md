# Immutable leaf-success certification port — B1b.2a.1c.1 candidate

Status: reviewed API/idempotence boundary and application types/structural checks implemented;
no schema or source writer implemented.
Refs:T014-014..017/040, ADR-0030§15, capacity-leaf-success-certification.md.
This operation certifies one existing original-run path and never acquires G.

## Command and immutable result

LeafCollectionPathIdentity contains TenantId, NamespaceId, original RunId and InstancePath.
Validate exactly one path segment. Derive root owner and index from that path; accept no duplicate
owner/index fields. The command CertifyLeafCollectionSuccess contains that identity,
expected_run_revision and existing primitives::SchedulerAuthority only. The method is fixed V1;
there are no caller-supplied source sequences/digests/version/shape/success/closure flags or group.

LeafCollectionSuccessStore::certify_leaf_collection_success returns a record plus replayed, or a
typed error. The immutable record contains identity, closed OrdinaryLeafV1 profile,
actual locked-run WorkflowVersionId, existing primitives::CollectionScopeId, qualified body NodeId,
exact return ExecutionInstanceId, admitted/returned/checkpoint/finished/owner-routed sequences,
checkpoint and aggregate workflow::Digest, certified_run_revision, certified_at and audit_id.
The profile binds source rules and LeafCollectionSuccessShape contract1. It expresses successful
closure under those rules, not mutable no_effects/producers_closed booleans. The writer resolves
all source facts itself; the pure classifier cannot establish a version pin or source finality.

Existing collection.finished supplies the reduction outcome/digest and step.routed follows it.
Do not invent a reduction_sequence or per-activity terminal/routing sequence columns. V1 verifies
the complete bounded local set under own-run guard; returned_sequence is checkpoint_sequence-1
only after verifying that exact V1 source event. The record retains actual references/disposition.
A stable audit resource identity preventing generic repeat-collapse must be frozen with the audit/
storage contract before implementation; it cannot reuse capacity.member.finished semantics.

## Canonical identity is the receipt

One immutable record per tenant/namespace/original-run/path is the canonical result and receipt.
The caller supplies no mutable business parameters, and V1 has no selectable profile. There is no
separate operation key/fingerprint for this metadata fact. Creation CAS and renewable authority
are preconditions, not identity. This differs from a G mutation: scoped Finish still obeys B1a's
same-key/fingerprint receipt contract. ADR-0030§15 requires applicable transactional evidence, not
an additional key-shaped row for every immutable fact.

The record cannot be overwritten/reused or silently replaced by another profile. A future profile
or receipt/source retirement requires an explicit compatible schema/reader contract; changing
this method's interpretation is insufficient. Receipt lifetime, immutable identity and non-reuse
are required storage prerequisites, not guarantees established by this API document.

## Exact order and recovery

1. Validate structural command bounds and supported revision range.
2. Begin safely. Retain applicable own-chain → exact original own-run, rechecking tenant/namespace/
   run association. Never resolve to a successor or require this original run to be current head.
3. Retain current own-run scheduler guard; sample DB time after its actual wait. Authority is
   required for first creation and replay, including a terminal original run.
4. Look up the canonical record. Validate its supported profile, qualified identity, actual immutable
   run pin and structural integrity. If present, return the original record. Do not compare creation
   CAS, rerun source certification, require current run success or append another certification audit.
5. If absent, require expected_run_revision == actual current revision. Resolve the pinned definition,
   check OrdinaryLeafV1 shape and every exact bounded committed source fact from the source contract.
6. Insert the canonical certification and truthful audit atomically. Return success after confirmed
   commit only; leave execution revision/journal/projection unchanged.

Unknown COMMIT requires fresh invocation for the same identity under current authority. Acquiring
that original-run guard waits for the first TX to resolve. Presence returns original record even
if creation CAS is now stale. Absence permits creation only with current CAS and full source proof;
absence plus stale revision rejects until fresh discovery. Timeout/cancellation cannot establish
rollback. The caller must retain identity/intent across uncertain response; durable production
intent/reconstruction is a separate activation prerequisite.

## Closed errors

UnsupportedPath, Missing, UnsupportedShape(CollectionSuccessShapeError), UnsupportedStoredProfile,
SchedulerFenceLost, StaleRunRevision{current}, SourceNotClosed, SourceEvidenceMissing,
ContradictorySource, InvalidRevision, Storage(StoreError) for confirmed failure, and
CommitOutcomeUnknown{identity}. Tenant/namespace absence stays Missing. Present unfinished source
is SourceNotClosed; missing required retained evidence must not silently become that lifecycle
state. There is no G Busy, capacity revision, fingerprint conflict or guaranteed rollback after
COMMIT dispatch. Error/span/log details contain no business values or raw command/driver text.

## Remaining persistence and evidence gates

Before SQL: define immutable canonical identity/schema/uniqueness/privileges, source retention and
FK/implicit-lock inventory; exact record validation and supported profile tags; audit vocabulary/
resource identity/atomicity and closed observations. No new broker hint is required to define this
metadata-only port. If the persistence decision adds one, it must commit atomically and replay must
preserve its original identity without emitting another transition. No execution event is invented.

Prove source preservation, exact pin/scope/path/source references, live/stale authority, creation
CAS versus replay, duplicates, contradictory/missing evidence, first abort, actual unknown COMMIT
and fresh reconciliation/restart before acceptance. Keep source-certified rows/inputs available
for the stated replay/downstream verification window. A separate operation key cannot compensate
for lost evidence or an identity that can be reused.

This candidate decides only the port/receipt boundary. Source schema/retention/audit implementation,
producer tests, production caller and every scoped/group/nested runtime acceptance remain open.


Independent fresh Astra/xhigh bounded review cycle1 found no concrete issue in receipt identity,
authority/replay/CAS order or unknown recovery, after checking ADR-0030 and reviewed prerequisites.
This is an API decision only, no source writer/schema/test guarantee. Existing user choice keeps
current review; no external review CLI.

Application types now live in application::leaf_certification. Identity enforces depth1 and retains
all four qualifiers; command revision checks enforce positive signed-journal range. Record checks
enforce exact return-path binding and V1 source-sequence ordering without pretending to establish
the existence or finality of those sources. Error Display/Debug use safe messages; the diagnostic
StoreError remains available through explicit source-chain inspection, which requires caller
redaction. No database adapter, source writer or production caller is supplied by these types.
