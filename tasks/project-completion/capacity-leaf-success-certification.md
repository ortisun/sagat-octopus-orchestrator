# Successful leaf collection path certification — B1b.2a.1a candidate

Status: reviewed source-writer choice; pure shape/application port/dormant schema/audit vocabulary
implemented separately. Source adapter and actual producer/recovery proofs remain pending.
Refs:T014-014..017/040, ADR-0030, B1a, capacity-path-settlement.md.
No failed/drained path, loops, nested controllers, children or capacity mutation.

## First source writer

Use a separate bounded own-run transaction after committed successful CollectionFinished to
certify one admitted depth-1 iteration path. It retains applicable chain → exact original own-run,
current own scheduler guard and the actual postwait DB-time check. Discovery and supplied revisions
are not proof. It takes no G lock and changes no execution state or terminal journal.

The pinned definition must satisfy a versioned checked shape: the current public root collection
chain, optionally its supported ordinary source prefix, and selected body with one to eight
sequential ordinary activities then return. This first case excludes body branches/error edges,
controllers/children, scripts/managed operations, compensation/reconciliation declarations and any effect-policy settings. A missing or broader shape refuses certification. The implementation
must define the exact typed predicate before storage; a publisher capability flag is insufficient.
The path has exactly one segment naming the actual root ForEach owner and admitted index.

## Required exact committed source facts

Under the own-run guard, validate namespace/run/pinned-version/scope/owner/index/path together:

1. Selected collection_iterations admission identity and admitted_sequence match the exact
   collection.iteration_admitted source event. Its lifecycle is CHECKPOINTED and slot_held=false.
2. Exact iteration_checkpoints identity, return node and digest/sequence match the iteration row
   and exact collection.iteration_checkpointed event. That event addresses the owner and carries
   the selected index in its detail. No result/input/aggregate value is loaded.
3. Owning iteration_scopes is SUCCEEDED, with no drain, and its exact finish_sequence references
   collection.finished for this owner and successful aggregate digest. Validate the successful
   reduction reference and owner-step successful routing. Require admission < checkpoint < finish
   <= current own-run revision, plus the exact return source sequence required by checkpoint.
4. The complete selected local row set equals the pinned body's ordinary activities plus return;
   every row is SUCCEEDED and routed, with no live lease/retry/wait. No unexpected derived row or
   scope-saga entry exists for this path/owner. The definition's explicit absence of compensation/
   reconciliation producers is required, not inferred from missing rows.

Any missing, contradictory, unsupported or unresolved obligation refuses. The writer may read only
these bounded selected-path rows and exact referenced journal rows, selecting identity/digest/
outcome fields; no sibling iteration, aggregate value, whole plan or history hydration. The record
must retain exact identity, definition/shape-contract version, source references and disposition.
Schema, uniqueness, source retention, durable replay and atomic audit are the next reviewed unit;
this decision does not supply them or authorize a caller-asserted closure record.

## Preservation obligation

Own-run serialization must cover every current producer. Current checkpoint closes the return
and requires other local steps already terminal/routed. Successful collection cannot enter drain:
mark_collection_draining accepts OPEN only, and saga planning requires DRAINING. Selected ordinary
steps have no declared derived producers. Completion/failure/retry/claim/reconciliation/review
require their respective nonterminal attempt/wait states and cannot revive these SUCCEEDED rows.
Full addressed routing does not consult ancestor inactivity, so routed=true is essential here.
A later collection's activity/failure/cancellation must not reopen this earlier successful scope.
Rerun creates a different run identity and cannot reuse the proof. These source invariants need
actual writer tests before accepting this port; no certification is derived from run terminality.

A future writer/shape change capable of reopening these states must fail closed for certified
paths until a new preservation contract is proven. Old-writer/migration fencing remains C0;
this document assumes no old binary reads a new flag. No public activation follows.

## Required evidence and dependencies

Prove before-closure refusal and successful certification while a later collection is active,
then after that later scope fails/cancels. Retain both-order lock barriers against actual checkpoint,
finish, stale worker reports, retry/review/reconciliation attempts and cancellation. Validate exact
source sequences/digests/identities, missing/extra local rows, unsupported definitions, stale fence,
expiry, duplicate/conflicting intent, rollback and restart. Show bounded selected reads with many
sibling iterations; source facts must survive for the stated proof/replay window.

Next: review the typed source predicate/port, immutable record/receipt/audit schema and real source
writer proof. Failed/drained paths, declared effects/derived companions, loops, nesting, child
relations and scoped capacity Finish require separate decisions. This candidate closes no B1b.2a.1,
B1b.2/B1/B2 or canonical task and enables no publication, creator or nested execution.

Source anchors: infra-postgres/primitives.rs checkpoint1567-1633, exact journals1367/1709,
drain1806-1830 and finish2630-2859; domain/execution/state.rs lifecycle1316 and compensation4191;
state_effects.rs reconciliation86 and review193-239; domain/execution/plan.rs routing79/166;
infra-ir/convert.rs public shape646/701/776. Paths are under crates unless noted.


Independent fresh Astra/xhigh bounded review cycle1 found no concrete issue after checking
routing/lifecycle/effect/drain/compensation producers. This is source review, not a writer proof.
Root narrows the broader draft to successful ordinary leaf bodies; all drain/effect cases remain
separate prerequisites. Existing user choice retains current review; no external review CLI.


The pure LeafCollectionSuccessShape definition predicate is implemented as version1. It qualifies
ordered activity/return IDs by root owner/body and checks the frozen surrounding collection graph.
It rejects all selected-body EffectPolicy settings, including otherwise inert compensation budget
metadata. Other referenced linear bodies may retain their own declared effects. This is static
shape only: it deliberately carries no workflow-version/run/settled flag and gives no pin/source
proof. Source certification/API/storage/actual producer barriers remain pending. Evidence:
evidence/collection-success-shape-20261018.
