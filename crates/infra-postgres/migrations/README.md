# crates/infra-postgres/migrations

## Overview

Ordered PostgreSQL up/down migrations for authority tables, projections, accounting, operational metadata, indexes, mutation guards and runtime grants. Historical migration comments describe their original checkpoint, not current endpoint availability.

## Interface

- Tables: `tenants`, `workspaces`, `namespaces`, `memberships`, `audit_records`. [20261008000000_identity.up.sql:tenants]
- Tables: `workflows`, `workflow_versions`, `command_dedupe`. [20261009000000_definitions.up.sql:workflows]
- Tables: `payloads`, `runs`, `run_events`, `steps`, `worker_sessions`, `event_outbox`. [20261009000100_execution.up.sql:payloads]
- Tables: `quota_limits`. [20261010000000_quotas.up.sql:quota_limits]
- Tables: `worker_bindings`. [20261010000100_worker_bindings.up.sql:worker_bindings]
- Tables: `secrets`, `secret_versions`, `secret_access`. [20261010000200_secrets.up.sql:secrets]
- Tables: `egress_policies`. [20261010000300_egress_policies.up.sql:egress_policies]
- Tables: `quota_ceilings`. [20261010000400_quota_ceilings.up.sql:quota_ceilings]
- Tables: `run_sagas`, `run_saga_entries`, `saga_plans`, `saga_plan_entries`, `compensation_breakers`. [20261011000000_sagas.up.sql:run_sagas]
- Tables: `event_endpoints`, `inbox_events`, `event_deliveries`, `workflow_triggers`, `webhook_subscriptions`, `webhook_deliveries`, `webhook_attempts`, `event_connectors`, `connector_offsets`, `connector_leases`, `connector_dead_letters`. [20261012000000_events.up.sql:event_endpoints]
- Tables: `scripts`, `script_artifacts`, `script_versions`, `script_build_jobs`, `script_test_runs`, `script_executions`. [20261014000000_scripts.up.sql:scripts]
- Tables: `telemetry_pending`, `engine_leases`. [20261015000100_telemetry_projection.up.sql:telemetry_pending]
- Tables: `operational_alerts`. [20261015000200_operational_alerts.up.sql:operational_alerts]
- Tables: `worker_drains`. [20261015000300_operator_actions.up.sql:worker_drains]
- Tables: `publication_policies`, `publication_requests`, `publication_decisions`, `workflow_activations`, `workflow_version_scripts`. [20261016000000_versioning.up.sql:publication_policies]
- Tables: `scheduler_shards`, `scheduler_members`, `tenant_scheduling`. [20261017000000_scheduler_shards.up.sql:scheduler_shards]
- Tables: `run_active_counts`. [20261017000100_active_run_counts.up.sql:run_active_counts]
- Tables: `blobs`. [20261018000000_blobs.up.sql:blobs]
- Tables: `activity_quota_limits`. [20261019000000_activity_quotas.up.sql:activity_quota_limits]
- Tables: `tenant_retained_usage`, `quota_transaction_deltas`, `quota_accounting_control`, `quota_rate_buckets`. [20261024000000_durable_quotas.up.sql:tenant_retained_usage]
- Tables: `run_instance_usage`, `tenant_iteration_usage`, `tenant_collection_policies`, `iteration_scopes`, `collection_scope_items`, `collection_iterations`, `iteration_checkpoints`. [20261029000000_collection_progress.up.sql:run_instance_usage]
- Tables: `declared_operation_definitions`, `workflow_operation_states`, `operation_update_receipts`. [20261030000000_declared_operations.up.sql:declared_operation_definitions]
- Tables: `loop_scopes`, `loop_iterations`, `loop_checkpoints`, `loop_timers`. [20261031000000_loop_progress.up.sql:loop_scopes]
- Tables: `approval_requests`, `approval_decision_receipts`, `approval_timers`. [20261101000000_approval_progress.up.sql:approval_requests]
- Tables: `execution_chains`, `chain_segments`, `child_chain_links`. [20261102000000_execution_chains.up.sql:execution_chains]
- Tables: `secret_pools`, `worker_binding_pools`. [20261109000000_secret_worker_pools.up.sql:secret_pools]
- Tables: `scope_sagas`, `scope_saga_entries`. [20261112000000_scope_compensation.up.sql:scope_sagas]
- Tables: `secret_provider_operations`. [20261122000100_openbao_secrets.up.sql:secret_provider_operations]
- Tables: `database_connections`, `database_connection_revisions`, `run_database_connections`. [20261123000000_database_connections.up.sql:database_connections]
- Tables: `step_database_credentials`. [20261124000000_database_executor.up.sql:step_database_credentials]
- Tables: `private_execution_definitions`. [20261125000000_database_ad_hoc.up.sql:private_execution_definitions]
- Tables: `payload_reveal_grants`. [20261202000000_reveal_grants.up.sql:payload_reveal_grants]
- Tables: `capacity_groups`, `capacity_group_members`, `capacity_group_finish_receipts`. [20261210000000_capacity_finish_storage.up.sql:capacity_groups]
- Tables: `leaf_collection_success_certifications`. [20261211000000_leaf_success_certification.up.sql:leaf_collection_success_certifications]
- Tables: `capacity_group_transfer_receipts`. [20261226000000_capacity_transfer_receipts.up.sql:capacity_group_transfer_receipts]

## Invariants

- Composite tenant/scope foreign keys, uniqueness and bounded checks constrain cross-tenant links, states, sizes and projection identities. [20261008000000_identity.up.sql:namespaces_workspace_same_tenant_fk] [20261009000100_execution.up.sql:runs_namespace_same_tenant_fk]
- ENABLE ALWAYS guards constrain protected audit/journal/payload/version/plan/artifact/activation/receipt/proof mutations. Guard behavior is table-specific; script versions permit a guarded BUILDING-to-terminal update rather than unrestricted rewriting. [20261008000200_runtime_role_grants.up.sql:audit_records_append_only] [20261009000100_execution.up.sql:execution_reject_mutation] [20261014000000_scripts.up.sql:script_versions_guard] [20261211000000_leaf_success_certification.up.sql:leaf_success_append_only]
- Undelivered outbox obligations cannot be deleted. Partial/covering indexes support ready/due/actionable/path/capacity discovery but are not workload-performance evidence. [20261009000100_execution.up.sql:outbox_reject_undelivered_delete] [20261205000000_bounded_execution_discovery.up.sql:steps_path_actionable_idx] [20261208000000_path_global_work_indexes.up.sql:steps_path_live_idx] [20261225000000_capacity_discovery_index.up.sql:capacity_members_active_discovery_idx]
- Runtime grants run only when sagat_app exists and differ by table; private accounting tables/functions remain restricted. Trigger-driven byte enforcement is activated by a later migration after initial disabled setup. [20261008000200_runtime_role_grants.up.sql:sagat_app] [20261024000000_durable_quotas.up.sql:quota_accounting_control] [20261026000000_activate_durable_byte_quotas.up.sql:content_enforcement_enabled]
- Many later down migrations refuse to discard retained feature state; early identity/core migrations contain direct DROP statements, so rollback is not universally data-preserving. [20261020000000_run_input_blobs.down.sql:RUN_INPUT] [20261029000000_collection_progress.down.sql:iteration_scopes] [20261122000100_openbao_secrets.down.sql:secret_provider_operations] [20261210000000_capacity_finish_storage.down.sql:capacity_groups] [20261008000000_identity.down.sql:tenants]
- Capacity groups/receipts and leaf certifications are narrow dormant metadata/proof boundaries; table existence does not enable a production creator/caller, general nesting or workload acceptance. [20261210000000_capacity_finish_storage.up.sql:capacity_groups] [20261226000000_capacity_transfer_receipts.up.sql:capacity_group_transfer_receipts] [20261211000000_leaf_success_certification.up.sql:leaf_collection_success_certifications]

## Side Effects

- Application executes DDL, functions/triggers/indexes/constraints, backfills and schema-state changes; examples backfill activations, retained usage, instance usage, node kinds and projected charges. [20261016000000_versioning.up.sql:workflow_activations] [20261024000000_durable_quotas.up.sql:tenant_retained_usage] [20261029000000_collection_progress.up.sql:run_instance_usage] [20261205000000_bounded_execution_discovery.up.sql:node_kind] [20261207000000_projected_instance_charge.up.sql:charged_instances]
- Some migrations acquire strong table locks and grant/revoke role privileges. Installed triggers later update accounting/projections or reject writes during application transactions. [20261017000100_active_run_counts.up.sql:LOCK TABLE] [20261210000000_capacity_finish_storage.down.sql:ACCESS EXCLUSIVE] [20261008000200_runtime_role_grants.up.sql:sagat_app] [20261024000000_durable_quotas.up.sql:sagat_quota_flush_deltas] [20261220000000_reveal_grants_role_change.up.sql:sagat_revoke_reveal_grants_on_role_change]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-postgres --locked
```

These commands were not run in this documentation-only pass.
