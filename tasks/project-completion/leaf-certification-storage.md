# Dormant ordinary-leaf certification storage — B1b.2a.1c.3

Status: reviewed bounded decision; no schema or source adapter implemented.
Refs:T014-014/017/040, ADR-0030, D298, leaf-certification-port.md,
leaf-source-retention-inventory.md. Fixed OrdinaryLeafV1 only; no capacity mutation/caller.

## Canonical snapshot

Add leaf_collection_success_certifications, a retained immutable metadata table. Store exactly:
tenant_id, namespace_id, run_id, owner_node, iteration_index, profile, definition_version_id,
scope_id, body_node, return_node, admitted_sequence, returned_sequence, checkpoint_sequence,
checkpoint_result_digest, finished_sequence, aggregate_result_digest, owner_routed_sequence,
certified_run_revision, certified_at, audit_id. UUIDs/scalars only, no values/JSON/path arrays.
The depth1 path is represented losslessly by root owner_node plus iteration_index; return_node
is implicitly qualified by that exact same path. Profile is exactly ORDINARY_LEAF_V1.

Primary key(tenant_id,namespace_id,run_id,owner_node,iteration_index), plus UNIQUE
(tenant_id,run_id,owner_node,iteration_index) prevents a second namespace alias for one actual
run/path. audit_id is UNIQUE. Node identifiers obey the existing ASCII NodeId grammar/64 limit;
index is0..9999; both digests are32bytes. All six sequence/revision counters are positive bigint;
use subtraction checks to avoid overflow: returned-admitted>0, checkpoint-returned=1,
finished-checkpoint>0, owner_routed-finished=1, certified_revision>=owner_routed.
certified_at is a finite timestamp. No separate certification UUID, operation key or mutable flag.

Only FK: (tenant_id,namespace_id,run_id) REFERENCES runs(tenant_id,namespace_id,id), NO ACTION,
not deferred, using the existing runs_tenant_namespace_id_uniq key. It protects original-run
identity and namespace association; it does not prove version/scope/source semantics. The adapter
must check actual namespace and immutable run pin for creation AND existing replay.
No FKs to mutable scope/iteration/checkpoint/steps, journal/payload, version, audit or outbox rows.
Exact source agreement is established by the reviewed writer, not by schema-shaped assertions.
Avoiding child FKs does not excuse source retention: it is enforced separately below.

Immutable UPDATE/DELETE and TRUNCATE rejection uses always-enabled triggers, including owner
sessions and replication-role bypass. No overwrite/upsert/retirement/TTL. Revoke PUBLIC and direct
runtime default privileges; the dormant schema grants sagat_app SELECT only, with INSERT added
alongside the separately proved adapter, never UPDATE/DELETE/TRUNCATE. Add the certificate table
to the startup append-only privilege check. Runtime INSERT will be an adapter privilege, not a
public proof API. Owner fixtures can validate schema
shape only; they must never be claimed as successful source certification.

## Explicit lifetime and retained original sources

D298 already requires no deletion. This unit does not choose a finished-run expiry policy.
Certificates and their exact required sources have no retirement path. A later D298 decision must
retain or compatibly transfer original identity/source/replay/downstream-verification obligations
before it may delete them. Unsupported profile replacement or run/path identity reuse is forbidden.

Enforce current retained source lifetime with unconditional always-enabled BEFORE DELETE and
BEFORE TRUNCATE rejection on iteration_scopes, collection_scope_items, collection_iterations,
iteration_checkpoints, steps, scope_sagas, scope_saga_entries and runs. Explicitly revoke DELETE and
TRUNCATE from runtime/PUBLIC. Reuse the existing execution rejection function. These triggers
perform no queries and introduce no conditional certificate lookup/snapshot race or row-lock edge.
Current production code has no DELETE on these tables; ordinary INSERT/UPDATE remains unchanged.
Existing always-enabled guards already retain run_events, payloads and workflow_versions. A
normal FK alone does not retain the original run under session_replication_role=replica: ordinary
FK checks are disabled there. The always-enabled runs DELETE/TRUNCATE guards therefore retain
it independently. Add an always-enabled runs BEFORE UPDATE OF id guard rejecting an actual id
change; existing admission-identity guard already freezes tenant/namespace/workflow/version.
Together they preserve the actual original-run lookup and pin even under replication-role bypass,
without treating mutable projection UPDATE fields as immutable. PostgreSQL16 documents this
[FK bypass](https://www.postgresql.org/docs/16/runtime-config-client.html#GUC-SESSION-REPLICATION-ROLE).
No outbox FK: delivered hint retention remains independent. Certificate records copy only digests.

These are lifetime guards, NOT UPDATE immutability or closure proof. Current successful scope/
iteration/local-row UPDATE preservation must be established by actual source-writer/producer
barriers before any adapter acceptance. The immutable copied record does not make false source
assertions true. Arbitrary privileged DDL removing guards is outside the runtime trust boundary.
No source row is treated as terminal merely because it is retained.

## Audit and transaction edges

Add truthful AuditAction::LeafCollectionSuccessCertified -> collection.leaf_success_certified and
ResourceType::LeafCollectionSuccess -> leaf_collection_success. Resource ID is the injective
namespaceUUID/runUUID/rootOwnerNode/decimalIndex string; NodeId forbids slash and numeric index
is canonical unsigned decimal. Tenant remains the audit's separate qualified tenant. No payload,
secret, result digest or business data appears in reason. Fixed engine actor/reason, inherited
traceID. This action does not collapse repeats and takes no dedupe advisory lock.

The first canonical insert and this one audit commit atomically; record.audit_id is the original
audit ID. Replay returns the original record and writes no new audit. No broker hint/execution
journal event is added; certification is metadata only. No capacity.member.finished reuse.

Future writer prefix remains applicable chain -> exact original own run -> own scheduler SHARE
and fresh postwait DB time, followed by canonical lookup/replay or CAS plus bounded source proof.
Canonical INSERT's run FK takes only a referenced-key guard on its already held own run. It does
not take G, tenant/run usage, foreign run, source-row key guards or outbox locks via added FKs.
Audit INSERT retains its existing tenant FK check; no new audit FK/implicit source edge is added.
PK/unique index waits remain possible, but supported writers serialize the same fact on own run.
No global deadlock-freedom guarantee follows from this inventory; actual barriers remain required.

## Down migration and implementation gates

Down acquires ACCESS EXCLUSIVE on runs before the certificate table in its migration transaction,
matching the writer's earlier own-run guard before certificate access. Before removing any object,
the empty check rejects55000 if any certificate exists. Only then remove table and added source guards.
A racing insert either precedes the check (refuse down) or cannot proceed against removed schema;
no nonempty certificate store is silently dropped. No live downgrade traffic is promised, and
DDL relation-lock ordering is separate from the source transaction proof. Compatible readers and
a reviewed retention/migration path are required once any certificate exists.

Before source adapter: implement dormant schema/classification and real PostgreSQL constraint,
privilege, immutability, retention/cascade/TRUNCATE and empty/nonempty down tests. Track every
variable-length column as bounded protected metadata with its actual CHECK. Then separately
prove actual pinned source facts, all preserved producers, atomic audit/refusal/rollback,
unknown-COMMIT reconciliation and restart. No source SQL or production caller is accepted by this
decision. B1b.2a.1/B1b.2/B1/B2, old-writer fencing/C0, scoped Finish and nesting remain open.

Two fresh Astra/xhigh review cycles: cycle1 found the ordinary-FK replication-role retention gap;
the contract now includes always-enabled run DELETE/TRUNCATE and id immutability guards.
Cycle2 found no concrete issue in this bounded decision. This is no SQL/test acceptance.
