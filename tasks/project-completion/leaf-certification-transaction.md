# Dormant ordinary-leaf source transaction — B1b.2a.1d.1 candidate

Status: reviewed contract and first dormant native implementation; producer/recovery acceptance
and release activation remain pending. Evidence: evidence/leaf-certification-native-20261018.
Refs:T014-014/017/040; reviewed leaf-certification-port.md and leaf-certification-storage.md.
No production caller or released INSERT grant; native tests explicitly opt in via disposable owner.

## Transaction and replay

PgLeafCollectionSuccessStore requires PgPool and DefinitionCache. Validate command, begin_safely,
lock_member_chain on qualified original run, then SELECT exact tenant/namespace/id runs FOR UPDATE.
Return Missing for absent qualified original run. Read actual version/revision/trace; do not require
success/current-head or redirect. Retain locked_scheduler_time's scheduler SHARE guard and actual
postwait clock; None is SchedulerFenceLost. Do not use primitives::lock_run's terminal restrictions.
The shared guard compares expiry against one materialized postlock clock in PostgreSQL, decoding
only the returned clock. A valid finite lease expiry beyond Rust's timestamp range stays live.

Canonical lookup follows authority. Strictly decode closed profile, qualified identity, nodes,
digests/counters/time. Reconstruct depth1 return path, validate_structure and actual immutable
run pin; certified_run_revision must not exceed current run revision. Unsupported stored profile
is UnsupportedStoredProfile, malformed/disagreeing record is ContradictorySource. Return original
record/replayed=true without creation CAS, shape/source queries, current run outcome or new audit.

On miss, creation revision must equal locked current revision before definition/source reads.
Resolve actual workflow_versions document through injected cache; missing pinned row is required
SourceEvidenceMissing, invalid stored definition is ContradictorySource. Apply fixed pure shape.

## Bounded source proof on miss

Read one root-owner iteration_scope by exact qualified owner and empty owner-path arrays. Require
actual namespace, COLLECTION kind, index<item_count, successful counters/admission/completion and
zero active count, valid aggregate digest/finish sequence and no drain fields. Check one exact
scope/index iteration: CHECKPOINTED, slot not held, remaining_reservations=0, no drain receipt;
entry equals checked first activity and return equals checked Return, with exact depth1 path and
instance_step_id(actual_version,run,instance) UUIDs. Check one checkpoint with equal return UUID,
digest and sequence. Compare item digest with admission metadata; no payload values are loaded.

Check one exact root owner step and the complete selected-path step set ordered by node LIMIT10.
Exactly the checked activities1..8 plus return must exist; all are FORWARD/SUCCEEDED/routed with
actual namespace and deterministic UUID. No live lease_token_hash/lease_expires_at/leased_by/
attempt_deadline_at, next_attempt_at or waiting_kind. Historical reported_lease_token_hash and
nonzero activity attempt/generation are allowed. Compare activity names with the pinned body;
SQL must project node names at most64bytes and activity names at most128bytes, with explicit
valid-length flags; oversized metadata is ContradictorySource, never silently NULL-compatible.
Project lease/retry/wait absence as booleans rather than transferring capability hashes or fields.
current node_kind may remain legacyControl and is not semantic proof. No expected-node/status
filter may hide extra rows; a tenth/unexpected row refuses. No extra row FOR UPDATE/usage/G locks.
Two exact EXISTS probes refuse scope_saga for root owner or saga entry at selected owner/index.

Read only five journal keys: admitted, checkpoint-1, checkpoint, finish, finish+1; all are root-owner
addressed with exact node/path/instance UUID. Enforce admitted<returned, checkpoint=returned+1,
checkpoint<finish, routed=finish+1, routed<=current_revision with checked arithmetic. Expected:
admitted(index,entry,item_digest), returned(index,return), checkpointed(index,return,result_digest),
collection.finished(succeeded,aggregate_digest), step.routed(activated exactly pinned successor).
There is no separate reduction/per-activity sequence. Checkpoint and aggregate digests may differ.

Project only fixed metadata fields, not unrestricted event detail: guard JSON scalar kinds, bound
identifier/digest/number text sizes and require exact path/activated array cardinalities before
reconstructing a small typed-decoder input. Wrong event types expose no unrelated detail fields.
Reuse event_codec::decode for event semantics; compare the reconstructed typed events with the
exact expected facts. Never hydrate payload contents, result/aggregate values, sibling paths or
whole history/plan. LIMIT10 and exact primary/qualified keys bound runtime source reads; static
pinned-definition work is bounded by definition limits, not iteration history.

## Closed error precedence

Missing means absent qualified original run. Scope OPEN/DRAINING/FAILED/CANCELLED is SourceNotClosed:
not closed under V1 successful-source rules, with no promise it can become certifiable later.
After a claimed successful scope, an incompatible iteration/step state, positive remaining reserve,
held slot, drain reference, unexpected row/saga, identity/digest/sequence disagreement or malformed
metadata is ContradictorySource. An absent required scope/iteration/checkpoint/step/journal is
SourceEvidenceMissing, not silently an unfinished lifecycle (including never-admitted requested
indices). Unsupported definition is UnsupportedShape. Stale creation CAS precedes source reads;
replay skips it. Database transport/query errors remain Storage with safe Display/Debug.

## Atomic metadata write and uncertain COMMIT

Use AuditRecord::engine with locked original-run trace and fixed reason
"ordinary leaf collection path success certified". Resource is namespace/run/owner/canonicalIndex;
existing dedicated noncollapsing action/resource tags apply. Construct record with fresh DB fence
time/current revision and original audit ID. Insert record and audit in one TX. Execution revision,
steps, journal, usage, G and outbox remain unchanged. No broker hint or inline activity executes.

Explicitly rollback precommit refusal. Confirmed commit returns success. Only server-confirmed
SQLSTATE23/40001/40P01 COMMIT aborts become Storage; other commit failures are
CommitOutcomeUnknown{original identity}. Cancellation returns no outcome and cannot prove abort.
No automatic retry in adapter. Fresh same-identity invocation waits on original run, requires
current authority and reconciles original record before stale creation CAS. Absence needs fresh
CAS and complete source proof. Retained sources/identity obey the already implemented D298 guards.

## Increment gates

The first native increment tests actual HTTP/gRPC source creation, before-closure refusal,
metadata-only audit/record atomicity and replay while a later scope is active/failed. Explicit
owner GRANT only in disposable native tests keeps the released schema SELECT-only. These tests
are not sufficient producer preservation/recovery acceptance. Additional bounded source refusal,
identity/CAS/current-authority and run/journal nonmutation evidence is recorded in
evidence/leaf-certification-refusals-20261018. Owner corruption fixtures do not prove producer
reachability. Every producer's both-order barrier, real unknown-COMMIT/reconciliation/
restart and closed observations remain required before release INSERT or a production caller.
One bounded late-report unit is implemented: new-key stale completion/failure versus first
certification in both observed flat original-Run lock queue directions. It does not prove an
in-flight successful writer, chain-member source or other producer families. Evidence:
evidence/leaf-certification-late-reports-20261018.
A separate actual successful closure writer starts from a production scheduler checkpoint and
races certification in both observed flat Run queue orders. Certification first refuses OPEN;
closure first makes the queued creation CAS stale. Fresh CAS certifies, and the original closure
descriptor replays without changing the certified source. Evidence:
evidence/leaf-certification-closure-20261018. This does not close other producer families.
Actual dispatched-COMMIT response loss and fresh direct pool/cache/store reconstruction are
covered in evidence/leaf-certification-commit-loss-20261018 for both confirmed backend outcomes.
The original qualified Run and shared scheduler fence remain held while the client sees Unknown;
cancellation of a blocked reconciler proves no original rollback. This is not process restart.
No source UPDATE immutability, old-writer/C0, scoped Finish or nesting acceptance follows.

Fresh Astra/xhigh cycle1 found that row count did not bound steps.activity bytes; explicit SQL
metadata length/validity projections now cover steps too. Cycle2 found no concrete issue in this
bounded contract. This review supplies no executed SQL or producer/recovery acceptance.
