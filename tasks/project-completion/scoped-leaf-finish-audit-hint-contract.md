# Scoped OrdinaryLeafV1 Finish — audit, hint and observation vocabulary

Status: reviewed bounded design prerequisite at6b34f227; no code, schema, privilege or executable guarantee.
Refs:T014-014..017/040; reviewed scoped-leaf-finish-{binding,schema,application-contract}.md and
canonical-leaf-reader-contract.md. Fixed OrdinaryLeafV1 depth1 only; no selectable source profile.
This does not design the owning transaction, nesting, creation, C0, caller intent or activation.

## Preferred vocabulary

| Concern | Exact choice |
| --- | --- |
| Audit action | Reuse `AuditAction::CapacityMemberFinished` / `capacity.member.finished` |
| Audit resource | Reuse `ResourceType::CapacityGroup` / `capacity_group`; resource_id is G's UUID |
| New hint message | `CapacityScopedMemberFinished` |
| New outbox event_type | `capacity.scoped_member.finished` |
| New AsyncAPI channel / operation | `capacityScopedMemberFinished` / `publishCapacityScopedMemberFinished` |
| New channel address | `sagat.v1.capacity.scoped_member.finished.{tenant}` |
| Observation | Reuse `CAPACITY_FINISH_SPAN` / `workflow.capacity.finish` and `CapacityFinishOutcome` |

Audit action denotes the genuine membership transition shared by both kinds; group resource remains
correct. A future vocabulary change should broaden the action's ChildRoot-only Rust doc comment to
capacity membership settlement, retaining its string and all existing ChildRoot behavior. This is
not permission to broaden the existing **wire message** `CapacityMemberFinished`: its channel,
required chainId/headRunId, component, baseline and meaning remain unchanged. Never substitute
originalRun for chain/head or add a mixed/nullable scoped variant to that message.

## Audit and durable evidence

Use `AuditRecord::engine` with the qualified tenant, G UUID, guarded postwait time and original Run's
optional persisted trace. Exact reason: **`ordinary leaf path capacity member finished`**.
This states that the selected membership finished; it does not assert a terminal Run, chain FINAL,
descendant settlement, recertification, quiescence, new execution permission or broker delivery.
No owner text, operation key, fingerprint, scheduler credentials, SQL, payload/result, secret or
caller-supplied reason enters the audit. The existing receipt binds audit_id to the exact member,
originalRun/owner/index and immutable result; no new audit DTO, resource kind or metadata bag.

Future first application commits selected member/header contribution, one receipt, one audit and
one new scoped hint atomically. Receipt audit_id/outbox_id identify those original records.
Confirmed abort/refusal adds none. Exact replay returns the original receipt/result/IDs and appends
none, including when the header has since changed. An unknown COMMIT may already contain the whole
set: reconciliation retains the original command and creates no replacement records on replay.
These are future native obligations; vocabulary or fixture-created rows do not prove atomicity.

## Closed scoped reference hint

Use application/json and payload `type: object`, `additionalProperties: false`. Exact fields:

| Field | Required type and bound |
| --- | --- |
| tenantId, namespaceId, groupId, memberId, originalRunId | Required strings, UUID format |
| rootOwnerNode | Required string; minLength1, maxLength64, pattern `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$` |
| iterationIndex | Required integer0..9999 |
| resultingRevision | Required integer2..9223372036854775807 |
| epoch | Required integer1..9223372036854775807 |
| retired | Required boolean |
| traceId | Optional string or null; non-null exactly32 lowercase hex, pattern `^[0-9a-f]{32}$` |

Identity comes from the exact retained scoped binding. Result counters and retired come from the
committed receipt result; retired means `possession == Retired`, never “this Run is terminal”.
Trace comes only from the already guarded original Run; absent trace may be omitted or null.
The future producer must enforce these bounds and never propagate arbitrary trace text. Trace is
correlation metadata, excluded from the operation fingerprint and never completion evidence.
No chainId/headRunId, profile selector, generic path JSON, certificate/body, status/reason, payload,
credential, operation key, holder identity or external-effect promise is added.

Reuse the existing dispatcher header `Nats-Msg-Id` with the stable outbox UUID across redeliveries;
aggregate_id is G and the subject tenant equals the payload/row tenant. The new channel is a dormant
reference notification of the committed receipt. Duplicates, delay, loss and out-of-order delivery
are possible; consumers must reread qualified durable state under their own authority before acting.
The hint supplies neither an execution lease nor authorization, and makes no exactly-once promise.

## Returned-attempt observations

The existing six outcomes fit scoped Finish without a new enum or metrics: `applied` after confirmed
first COMMIT; `replayed` for a confirmed matching receipt; `busy` for selected-G contention;
`storage_error` for Storage; `unknown` for CommitOutcomeUnknown; `refused` for every remaining
explicit ScopedLeafFinishError variant. Keep the match exhaustive, including structural/path/counter,
fence/binding, canonical-source/profile/contradiction, fingerprint, stale-CAS and inactive refusals.
An unknown prior operation remains unresolved when a later attempt refuses; metrics do not resolve it.

Reuse `workflow_capacity_finish_attempts_total{outcome}` and
`workflow_capacity_finish_duration_seconds{outcome}` once per returned attempt. Reuse unlabeled
`workflow_capacity_member_finished_total` only on acknowledged first application; replay, refusal,
abort and unknown add zero. Cancellation returns no outcome and is uncounted. A committed unknown
followed by replay can remain uncounted: receipts/audit/outbox are authoritative, process metrics are not.
No ID, node/index, namespace, tenant, kind, error text or operation key becomes a metric label.

Scoped span fields are exactly sagat.tenant.id, sagat.namespace.id, sagat.capacity.group_id,
sagat.capacity.member_id, sagat.run.id=originalRun and sagat.outcome, plus fixed otel.status_code
`error` only for storage_error/unknown. No chain/head or owner text is recorded. Retain the exact
created span handle, instrument its clone and record on that handle; never mutate Span::current(),
which can be the caller when this span is filtered. Inherit valid caller trace; an untraced call
still records metrics but has no promised OTLP export. Add no reads or authority for observation.
Reuse fixed log `capacity Finish attempt completed` with finite outcome only. Never format command,
raw StoreError/driver errors, SQL/binds or source records; explicit diagnostics retain existing
SQLSTATE-only sanitization and safe Display/Debug. No new observation DTO or reason-label taxonomy.

## Separate future verification

Vocabulary-only tests must add the new channel/operation/message, validate a full positive payload,
each required omission, wrong types, UUID formats, both owner/index/counter boundaries, optional
trace/null and malformed trace, forbidden chain/head, payload/credentials and any extra property.
Keep `capacity_finish_hint_is_closed_control_metadata` unchanged. Assert old channel/message and
CapacityMemberFinished baseline subtree unchanged; add only the new message baseline entry after
reviewing its diff, then run compatibility without SAGAT_UPDATE_BASELINE. The baseline checker tracks
types/enums/required only: explicit schema tests must enforce bounds and additionalProperties:false.
Later native tests must separately validate **actual produced** scoped outbox rows against this
canonical message; assert exact audit text/actor/resource/tenant, original receipt IDs, replay/refusal
counts, atomic rollback and unknown reconciliation. Native observation tests must exercise all six
real outcomes, filtered-span parent safety, redaction/no ID labels and cancellation accounting.
Keep existing ChildRoot native event/observation regressions. No tests were run for this draft.

Source anchors at6b34f227: crates/domain/src/audit.rs:34,274,385,427,544;
crates/infra-postgres/src/capacity_finish.rs:53,68,90,290,313,321;
crates/telemetry/src/capacity.rs:5,9,31; crates/infra-postgres/src/leaf_certification.rs:217;
specs/001-platform-foundation/contracts/events.asyncapi.yaml:8,67,260,415 and events.baseline.json:217;
tests/integration/workflow-primitives/hint_contract.rs:15,53,108 and capacity_finish_observations.rs:29,77;
tests/contract/events-webhooks-messaging/asyncapi/{main.rs:34,compatibility.rs:8};
reviewed scoped-leaf-finish-application-contract.md:72,88,103,122,132 and scoped-leaf-finish-schema.md:16.
No necessary unresolved vocabulary choice identified; native authority and evidence remain separate.

One fresh independent Astra/xhigh bounded read-only review found no actionable inconsistency or
missing vocabulary choice. No tests, native producer, privilege or production caller are claimed.


## Implemented vocabulary evidence — 2026-10-18

The canonical channel/operation/message and hand-reviewed additive baseline entry are integrated.
All20 old parsed baseline entries and the entire old ChildRoot message remain unchanged. New
closed-schema tests first failed because the new channel was absent, then all3 hint tests passed.
They enforce UUID formats, required fields, forbidden extras and exact owner/index/counter/trace
bounds, including i64::MAX acceptance and its successor refusal. AsyncAPI4/4 tests and pinned
CLI3.6.0 validation pass (0errors/0warnings, one pre-existing-version informational notice).
The shared test validator now enables format checks; actual existing capacity Finish33/33 passes
with its outbox, observation, uncertainty, schema/privilege and downgrade regressions. Strict
Clippy/fmt/architecture pass. One fresh bounded implementation review found no issue.
This adds no scoped producer, actual scoped outbox/audit evidence, permissions or caller.
Evidence: evidence/scoped-finish-hint-20261018/manifest.json.
