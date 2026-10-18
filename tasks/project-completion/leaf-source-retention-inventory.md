# Current leaf source retention and implicit locks

Source inventory at c94d957e, not a new retention policy or deadlock proof.
Refs:T014-014/017/040, B1b.2a.1c.3. D298 remains the owner decision: nothing is deleted;
a future finished-run retention must preserve chain, receipt and blob references.

## Current source identities and lifetime

- runs: primary key id, UNIQUE(tenant_id,id) and UNIQUE(tenant_id,namespace_id,id), with
  namespace/version FKs. The always-enabled
  runs_private_definition_scope trigger freezes tenant/namespace/workflow/version admission identity
  on UPDATE. This does not forbid run deletion or changing the id column by itself. Ordinary FK
  checks do not cover privileged session_replication_role=replica bypass.
- workflow_versions: primary key id, UNIQUE(tenant_id,id), immutable UPDATE/DELETE/TRUNCATE via
  always-enabled triggers. The run pins one actual immutable version.
- run_events: primary key(tenant_id,run_id,seq), same-run/payload NO ACTION FKs, positive sequence;
  always-enabled append-only/no-TRUNCATE triggers. Existing row immutability does not prove journal
  EOF: the schema alone permits INSERT at an unused positive sequence. Normal producers serialize
  on own run and atomically update run_version; certification must check exact committed sources.
- iteration_scopes: primary key(tenant_id,run_id,scope_id), unique qualified owner. Its run FK is
  ON DELETE CASCADE. Namespace/payload FKs have NO ACTION; blob source reference is RESTRICT.
  Scope has no independent version pin: certification must resolve through its actual locked run.
- collection_iterations: primary key(tenant_id,run_id,scope_id,index), scope FK CASCADE and item/
  payload references NO ACTION. Admission/checkpoint sequence scalars do not reference run_events.
- iteration_checkpoints: same four-part primary key, iteration FK CASCADE, payload reference
  NO ACTION. journal_sequence>=2 is not a source-event FK; return/digest/sequence agreement must
  be verified by the writer. No separate reduction or per-step terminal/routing sequence exists.
- steps and collection projections allow runtime SELECT/INSERT/UPDATE, no DELETE grant. These
  privileges do not make terminal source fields database-immutable. No current source projection
  trigger makes successful scopes/checkpointed iterations/checkpoints absorbing immutable rows.
- scope_sagas is keyed by tenant/run/owner path/node, with a same-run NO ACTION FK and runtime
  SELECT/INSERT/UPDATE. Its entries have a qualified saga NO ACTION FK and runtime SELECT/INSERT.
  Neither table currently has DELETE/TRUNCATE rejection triggers.

A retained run does not pin its child projections: child-to-parent FKs prevent deleting the parent
while referenced, not deleting the child. A privileged projection DELETE can remove a checkpoint
or cascade scope children while leaving the run. Existing runtime privileges and the absence of
production cleanup are not a certificate lifetime contract or source UPDATE preservation proof.

## Current producers and locks

Collection transactions take applicable chain, exact own run FOR UPDATE, scheduler guard FOR SHARE
and actual postwait DB time. Checkpoint then takes tenant usage, run usage, scope, iteration and
local-step mutation guards before atomic projection/checkpoint/journal/run-head writes. Successful
finish takes usage/scope and checkpoint guards before committed success and owner routing.
Admission journals collection.iteration_admitted. Checkpoint journals returned then checkpointed
at adjacent sequences. Finish journals collection.finished then step.routed at adjacent sequences.
Their payload digests differ legitimately: checkpoint return and aggregate reduction are distinct.

Source preservation remains a producer proof. consume_scoped_reservation (execution.rs:5611)
and consume_path_reservation (execution/path.rs:937) permit CHECKPOINTED/DRAINED iterations when
remaining_reservations>0; they do not check the enclosing scope lifecycle. The future ordinary-leaf
predicate must explicitly verify zero remaining reservations rather than relying only on terminal
labels. Full/exact addressed persistence can update existing steps without old-status SQL guards
(execution.rs:5538, execution/path.rs:1105); current domain reducer guards, exact complete local
set and routed=true are essential. Projection retention guards alone do not establish preservation.

New certificate FKs to scope/iteration/checkpoint/event/step rows would introduce referenced-key
locks after its explicit own-run prefix; they cannot be ignored in a lock graph. A run-only FK
would recheck its already held own-run identity, not protect child projection lifetime or source
semantics. No new edge is accepted solely by this inventory.

## Cleanup is currently absent

maintenance.rs purges command dedupe, delivered outbox, expired sessions and script records; it
has no finished-run/collection projection purge port. Production Rust source searches found no
DELETE FROM runs/run_events/iteration_scopes/collection_iterations/iteration_checkpoints.
The no_maintenance_job_removes_primitive_data test documents and tests the D298 no-delete policy
at zero housekeeping windows. Existing operations tests that remove journals explicitly disable
always-enabled guards as privileged fault injection; they are not a supported purge protocol.

## Anchors and limits

crates/infra-postgres/migrations:
20261008000000_identity.up.sql:28–43 (namespace),
20261009000000_definitions.up.sql:34–78,99–106 (version),
20261009000100_execution.up.sql:20–106,229–275 (run/journal/runtime grants),
20261027000000_scoped_instance_identity.up.sql:33–47 (event qualified path),
20261029000000_collection_progress.up.sql:122–309 (collection identity/FKs/grants),
20261102000000_execution_chains.up.sql:3–4 (qualified run unique key),
20261112000000_scope_compensation.up.sql:26–100 (scope saga/entry identity and privileges),
20261121000000_collection_blob_source.up.sql:1–12 (blob reference),
20261125000000_database_ad_hoc.up.sql:61–86 (immutable run admission identity).

crates/infra-postgres/src/primitives.rs:262–343,634–651,1245–1412,1419–1733,2550–2955;
execution.rs:371–384,497–528; maintenance.rs:603–669.
Tests: tests/integration/workflow-primitives/security.rs:832–964;
chains.rs:1094–1197; tests/integration/observability-operations/migration.rs:177–225.

This is read-only source evidence, not newly executed test evidence. Schema/privileges/immutable
certificate storage, explicit retention enforcement, actual producer preservation, rollback and
unknown-COMMIT/restart tests remain separate work. DDL owners can deliberately remove guards;
no claim covers arbitrary superuser schema changes or old-writer activation compatibility.
