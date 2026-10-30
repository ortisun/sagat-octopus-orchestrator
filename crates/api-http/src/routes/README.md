# crates/api-http/src/routes

## Overview

Axum handler families mounted beneath /v1. Most handlers and route-local DTOs are internal because routes is crate-private; reveal_grants is separately exposed as a public module. Mounting depends on router composition and configured services.

## Interface

- Chains expose GET inspection/runs and POST signal/cancel under `/namespaces/{namespace_id}/chains/{chain_id}`. Declared operations expose POST run/chain query and update routes. Both require approvals-complete composition. [chains.rs:routes] [declared_operations.rs:routes]
- Optional connectors expose collection GET/POST, item GET/PUT/DELETE and dead-letter list/read/delete/redrive. [connectors.rs:create_connector]
- Optional database configuration exposes driver/connection reads and connection PUT/DELETE. Execution composition adds POST database executions and GET/POST connection probes; optional result support streams a run/node database result. [database.rs:drivers] [database_executions.rs:create] [database_results.rs:read]
- Optional events expose endpoint CRUD, event ingest/read and unauthenticated POST `/webhooks/{endpoint_id}`. [events.rs:receive_webhook] [events.rs:get_event]
- Execution handlers cover workflow CRUD/publication/validation/diff/versions/compensation plans, run start/read/journal/telemetry/attempts/graph/change-stream/SSE and run operations/signals. [execution.rs:create_workflow] [execution.rs:start_run] [execution.rs:run_stream] [execution.rs:operate_run]
- Governance handlers manage tenant/namespace/activity quotas, worker bindings and egress policy. Optional secret services add secret/pool/ACL operations and managed migration/reconciliation. [governance.rs:list_tenant_quotas] [governance.rs:put_secret] [governance.rs:get_egress_policy]
- Public probes GET `/health` and `/ready` do not extract Caller. Authenticated identity handlers expose auth context, platform version, workspace reads/creation and namespace reads/creation. [health.rs:health] [health.rs:ready] [identity.rs:auth_context] [identity.rs:create_workspace] [identity.rs:create_namespace]
- Operations handlers list namespace alerts/workers, drain/undrain workers and read tenant audit. [operations.rs:list_alerts] [operations.rs:list_audit]
- Optional outbound handlers manage webhook/broker subscriptions and delivery reads/single or bulk redrive. [outbound.rs:create_subscription] [outbound.rs:create_broker_subscription] [outbound.rs:redrive_delivery]
- Optional payload routes POST run disclosures (input/result/journal, optional PII reveal) or structural inspections (input/journal). [payload_disclosure.rs:disclose_payload] [payload_inspection.rs:inspect_payload]
- Approvals-complete composition exposes GET run approvals and POST decisions. Public reveal-grant routes expose GET collection and PUT/DELETE subject grants under the namespace. [primitives.rs:routes] [reveal_grants.rs:routes]
- Optional runner handlers claim/heartbeat/complete builds/tests, retrieve artifacts/signing keys/egress policy/pending activities, and report executions/incidents. Script handlers manage scripts/versions/tests and read executions/artifacts. [runner.rs:claim_build] [runner.rs:record_incident] [scripts.rs:create_script] [scripts.rs:update_script] [scripts.rs:get_execution]
- Optional publishing handlers manage policy, publication requests and decisions, activation and promotion. Selected `ApprovalApiState` and `DeclaredOperationApiState` are re-exported; most DTOs/handlers remain internal. [versioning.rs:get_policy] [versioning.rs:promote_version] [../lib.rs:ApprovalApiState] [../lib.rs:DeclaredOperationApiState]

## Invariants

- Authenticated handlers use Caller-derived tenant/current authority and canonical resource namespaces for admission; webhook authentication and probes are separate boundaries. [connectors.rs:admitted_connector] [events.rs:receive_webhook]
- Execution/chain mutation keys are 8–160 printable ASCII; legacy identity creation permits absence. Declared updates use updateKey and distinguish replay; approval decisions bind replay to a request hash. [execution.rs:idempotency_key] [chains.rs:idempotency_key] [identity.rs:create_workspace] [declared_operations.rs:update_operation] [primitives.rs:decide_approval_inner]
- Chain commands require expectedRunId and map moved heads to CHAIN_HEAD_CHANGED. Cancel reasons are 1–512 characters. Run starts require exactly one inline input/blob reference; operations check action fields, expected version, 8–2000-character reasons and SagaOverride authority for unknown-effect retry. [chains.rs:signal_chain] [chains.rs:cancel_chain] [execution.rs:start_run] [execution.rs:operate_run]
- SSE authorizes before opening, bounds streams/lifetime, rechecks membership, prioritizes Last-Event-ID over query after, reconnects on revoked access and emits terminal end. [execution.rs:run_stream] [execution.rs:follow]
- Duplicate webhook security headers count as absent; malformed/unknown identities share rejection, while verifier unavailability remains retryable. Event/dead-letter disclosure defaults to structure; PII requires permission. [events.rs:single] [events.rs:get_event] [connectors.rs:get_dead_letter]
- Subscription/connector/script handlers bind canonical namespaces before admission. Webhook URLs are hidden from readers lacking EventManage; managed-secret writes require requestId and expectedVersion, and DTOs do not return secret values. [outbound.rs:admitted_subscription] [outbound.rs:can_see_urls] [scripts.rs:admitted_script] [governance.rs:put_secret]
- Payload reads prepare the trusted run, charge its resolved namespace once and return bounded private/no-store responses. Database-result bodies retain a bounded blob slot and stop at a 30-second/token-expiry budget. [payload_disclosure.rs:disclose_payload] [payload_inspection.rs:inspect_payload] [database_results.rs:read]
- Request-specific ceilings include events 240 KiB plus envelope, disclosure 1 KiB, scripts approximately 1 MiB and runner 4 MiB. Readiness uses a 500-ms ping with a one-second single-flight cache; liveness performs no dependency I/O. [events.rs:EVENT_MAX_BODY_BYTES] [payload_disclosure.rs:DISCLOSURE_MAX_BODY_BYTES] [scripts.rs:SCRIPT_MAX_BODY_BYTES] [runner.rs:RUNNER_MAX_BODY_BYTES] [health.rs:ready] [health.rs:health]
- Declared queries return no-store snapshots; updates separate replay/applied and map revision rejection to 409 versus validator rejection to 422. Approval/declared-operation telemetry labels are closed. [declared_operations.rs:update_chain_operation] [declared_operations.rs:OperationTelemetry] [primitives.rs:decide_approval]

## Side Effects

- Handlers invoke application service/store ports, pass audit effects and construct Axum responses; provider/file I/O belongs to the configured services/adapters. The webhook receiver passes raw security material/body to the event port. [execution.rs:start_run] [governance.rs:put_secret] [events.rs:receive_webhook]
- SSE allocates a channel, holds a stream slot, spawns polling work and emits SSE/keepalives and stream metrics; database reads remain port-mediated. [execution.rs:run_stream]
- Payload disclosure serializes bounded JSON, inspection constructs DTOs and database results retain semaphore/deadline guards in the response body. [payload_disclosure.rs:encode_disclosure] [payload_inspection.rs:inspect_payload] [database_results.rs:ResultBody]
- Handlers emit tracing/closed-label metrics and pass audits for primitives, webhook rejection, publication and promotion. [declared_operations.rs:OperationTelemetry] [primitives.rs:decide_approval_inner] [versioning.rs:promote_version]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-http --locked
```

These commands were not run in this documentation-only pass.
