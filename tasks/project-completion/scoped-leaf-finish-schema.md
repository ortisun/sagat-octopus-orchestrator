# Dormant scoped Finish schema extension

Refs:T014-014..017/040; local B1b.2b.2. Dependency: reviewed scoped-leaf-finish-binding.md.
Status: dormant schema implemented in migration20261212000000 with30actual PostgreSQL tests.
Conditional downgrade evidence remains a separate bounded unit. No executable scoped writer,
certificate consumption, caller, creation/activation, release permission or nested authority.

## Discriminated identity

Keep capacity_groups unchanged. Add original_run_id UUID, root_owner_node TEXT and iteration_index
INTEGER to capacity_group_members. Add the same three fields and member_kind TEXT NOT NULL
DEFAULT CHILD_ROOT to capacity_group_finish_receipts. Member chain_id becomes nullable; receipt
chain_id/head_run_id/accepted_chain_revision become nullable. Do not edit existing migrations or
backfill immutable receipts with UPDATE. The constant discriminator default preserves old inserts.

Closed total CHECKs require exactly one complete variant:

- CHILD_ROOT: existing chain fields required; all three new scoped fields NULL. Preserve existing
  nonnegative accepted chain/run revisions and every common counter/result-possession constraint.
- SCOPED_PATH: chain_id NULL (also receipt head_run_id/accepted_chain_revision NULL); all scoped
  identity fields required. Root owner uses the existing ordinary-leaf ASCII NodeId bound
  ^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$; index is0..9999. Scoped receipt accepted_run_revision >=1.

Every nullable field has explicit IS NULL/IS NOT NULL tests; SQL UNKNOWN must not admit a partial
variant. This SCOPED_PATH is only the fixed OrdinaryLeafV1 depth1 identity, not generic path JSON.
The schema proves binding shape, not successful source, certificate validity or quiescence.

## Keys, FKs and lifetime

Preserve group-wide receipt keys: (tenant,G,operationKey), (tenant,G,resultingRevision),
(tenant,G,member). They never gain a kind suffix. Preserve ChildRoot chain uniqueness/binding FKs,
deferred active-holder FK and historical result-holder identity FK.

Members add UNIQUE(tenant,G,namespace,originalRun,owner,index) and
UNIQUE(tenant,G,member,namespace,originalRun,owner,index). Add member FK
(tenant,namespace,originalRun) -> runs(tenant,namespace,id), and receipt exact scoped binding FK
(tenant,G,member,namespace,originalRun,owner,index) -> that new member unique key. Use MATCH SIMPLE
with total discriminated checks: the applicable binding is complete; the other is entirely NULL.
Do not replace both bindings with one nullable combined FK that permits both variants to escape.

The sole new business-row reference is to the original Run, already explicitly locked first by
the proposed native transaction. Add no certificate/scope/iteration/checkpoint/event/step/version/
audit/outbox FK and no source-querying trigger. Member/holder keys remain under guarded G; DDL,
uniqueness and reverse-FK waits still need focused real verification. No global lock proof follows.

Extend the existing immutable member tuple with all scoped fields using IS DISTINCT FROM.
Preserve irreversible ACTIVE->FINISHED, finished/retired immutability, append-only receipts,
DELETE/TRUNCATE rejection and ALWAYS triggers. Retain members, tombstones and receipts without
expiry; source/certificate retention remains unchanged. C0/old binary compatibility stays open.

## Preserve ChildRoot permissions; keep scoped INSERT closed

The current table-level receipt INSERT would automatically cover new columns. Revoke that table
grant and grant INSERT only on the exact original20 columns named by the existing ChildRoot
writer: tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,
expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,
resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,
resulting_holder_member_id,finished_at,audit_id,outbox_id.

Grant no new receipt discriminator/scoped fields, group/member INSERT or certificate INSERT.
Preserve SELECT and existing header/member state-update columns. An omitted discriminator defaults
to ChildRoot and cannot bind to a scoped member. This is trusted server SQL authorization, not RLS:
existing state UPDATE still covers owner-fixture rows. Actual old native apply/replay must remain
successful through sagat_app, and actual scoped receipt/certificate INSERT must remain denied.

## Conditional downgrade

Use a separate new migration pair. In one transaction acquire relation locks before inspection,
starting runs -> capacity_groups -> capacity_group_members -> capacity_group_finish_receipts,
so removing the original-Run FK does not acquire a late reverse relation edge. Exact DDL lock
strength/behavior is a required real PostgreSQL verification, not accepted solely by this text.
Refuse55000 if any scoped member or receipt exists, including finished tombstones. Never delete,
rewrite, convert or unbind them. With ChildRoot rows only, restore the previous guard, closed
ChildRoot checks, NOT NULLs and original receipt INSERT grant; remove only extension FKs/unique
keys/columns. Preserve all existing ChildRoot rows and retention guards. A retained certificate
alone does not block this extension's reversal; its own migration retains its existing guard.

## Implementation and evidence units

Schema increment owns at most five source files: up/down pair, storage-classification.json,
focused schema test module and module registration. Add protected_system tenant-keyed bounded
root-owner columns and closed discriminator vocabulary to classification. No content-accounting
increase or new payload field follows from this fixed metadata.

Actual schema tests need valid positive scoped and ChildRoot fixtures before named negative
constraints: partial/mixed variants, bounds, wrong tenant/namespace/run/binding, duplicate path,
cross-kind operation/revision uniqueness, identity/kind rewriting, finished reopening and receipt
mutation/retention. Inspect unchanged global receipt unique keys. Execute effective privileges
and denied INSERT probes under sagat_app; preserve actual native ChildRoot apply/replay. Run
classification and current runtime SQL privilege regressions.

Separate downgrade increment must prove ChildRoot-only down/up preserves data, active/finished
scoped rows block, and an actual held writer is observed behind lock-before-inspection barriers.
No sleep-only lock oracle. Schema-only implementation does not close actual scoped port, native
canonical source read, receipt fingerprint, truthful audit/hint, caller intent, C0 or B1 authority.

## Executed schema increment — 2026-10-18

The new migration pair, classification and five schema tests implement this contract. The complete
capacity Finish regression passed30/30 under sagat_app in75.28s (harness39.41s) on PostgreSQL16.15.
Before migration, the initial five tests all failed with42703; the first schema snapshot passed5/5.
Subsequent bounded test reviews strengthened independent nullable fields, qualified flat Run and
receipt binding, exact effective20-column grant, ACTIVE identity and timestamp-only FINISHED
immutability. Cross-group/tenant receipt probes and both grammar endpoints now execute successfully.
The cycle3 destination fixture correction uses a valid empty RETIRED group.

A preceding regression attempt passed13 then failed PostgreSQL startup (SSLRequest0x48); its
16subsequent shared-mutex failures are retained separately. The successful run used a dedicated
held test server and unique database per World. No production server/configuration was changed.
Classification completeness, runtime SQL privilege audit and execution-plane retention gates all
passed against the same SQL/catalog. Source hashes and all logs are retained under
`evidence/scoped-finish-schema-20261018`. No native scoped source consumption, release INSERT,
caller, C0 or nesting acceptance follows. The conditional down SQL still needs its dedicated
legacy/native and held-writer verification before the parent binding unit closes.
