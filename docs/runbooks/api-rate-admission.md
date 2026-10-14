# Namespace API rate admission

Public namespace operations consume a durable PostgreSQL token after authentication,
operation authorization, typed path/body parsing, and canonical namespace resolution. The debit
happens before mutation, response streaming, or upload body consumption. A request that reaches the
handler consumes one token even when the later business result is an idempotent replay or conflict.
Malformed bodies, denied roles, missing resources, cross-tenant probes, and nested-resource URL
namespace mismatches consume none.

`api_requests_per_second` is a namespace quota. Its durable bucket has a two-second burst capacity,
uses database time for refill, and is shared by every control-plane replica. A depleted bucket
returns retryable `429` with `Retry-After`; a zero policy returns the quota-exceeded `429`. A storage
failure fails closed. The decision emits only
`api_namespace_rate_admission_total{outcome="admitted|refused"}`. Tenant, namespace, subject,
resource identifiers, request bodies, tokens, and error text are never metric labels or log fields.

## Charged route inventory

The operation column is the closed `ApiOperation` authorization contract. `PayloadInspect` performs
its two permissions and canonical run lookup in its `prepare` phase, then uses the resolved-namespace
admission entry point.

| Method | Route | Operation |
|---|---|---|
| GET | `/namespaces/{namespaceId}` | `NamespaceRead` |
| GET | `/workflows` | `DefinitionRead` |
| POST | `/workflows` | `DefinitionWrite` |
| GET | `/workflows/{workflowId}` | `DefinitionRead` |
| PUT | `/workflows/{workflowId}` | `DefinitionWrite` |
| POST | `/workflows/{workflowId}/publish` | `DefinitionPublish` |
| POST | `/workflows/{workflowId}/validate` | `DefinitionWrite` |
| GET | `/workflows/{workflowId}/diff` | `DefinitionRead` |
| GET | `/workflows/{workflowId}/versions` | `DefinitionRead` |
| GET | `/workflows/{workflowId}/versions/{versionNumber}` | `DefinitionRead` |
| GET | `/workflows/{workflowId}/versions/{versionNumber}/compensation-plans` | `DefinitionRead` |
| POST | `/run-input-blobs` | `RunStart` through authorized input-blob access |
| GET | `/runs` | `RunRead` |
| POST | `/runs` | `RunStart` |
| GET | `/runs/{runId}` | `RunRead` |
| POST | `/runs/{runId}/payload-inspections` | `PayloadInspect` (`RunRead` + `RunPayloadInspect`) |
| GET | `/runs/{runId}/events` | `RunRead` |
| GET | `/runs/{runId}/telemetry` | `RunRead` |
| GET | `/runs/{runId}/attempts` | `RunRead` |
| GET | `/runs/{runId}/graph` | `RunRead` |
| GET | `/runs/{runId}/graph/changes` | `RunRead` |
| GET | `/runs/{runId}/stream` | `RunRead`, once per connection or reconnect |
| POST | `/runs/{runId}/operations` (`CANCEL`, `PAUSE`, `RESUME`) | `RunOperate` |
| POST | `/runs/{runId}/operations` (`RERUN_AS_NEW`) | `RunRerun` (`RunOperate` + `RunStart`) |
| POST | `/runs/{runId}/operations` (`RETRY_STEP`, `RETRY_COMPENSATION`, `MARK_MANUAL_REVIEW`) | `RunOperate` |
| POST | `/runs/{runId}/operations` (`SKIP_COMPENSATION`, `RESOLVE_STEP`) | `SagaOverride` |
| POST | `/runs/{runId}/signals/{signalName}` | `RunSignal` |
| GET | `/namespaces/{namespaceId}/worker-bindings` | `WorkerBind` |
| PUT, DELETE | `/namespaces/{namespaceId}/worker-bindings/{subject}` | `WorkerBind` |
| GET | `/namespaces/{namespaceId}/secrets` | `SecretRead` |
| PUT, DELETE | `/namespaces/{namespaceId}/secrets/{secretName}` | `SecretManage` |
| GET | `/namespaces/{namespaceId}/secrets/{secretName}/access` | `SecretManage` |
| PUT, DELETE | `/namespaces/{namespaceId}/secrets/{secretName}/access/{subject}` | `SecretManage` |
| GET, PUT, DELETE | `/namespaces/{namespaceId}/egress-policy` | `EgressManage` |
| GET | `/namespaces/{namespaceId}/alerts` | `OpsRead` |
| GET | `/namespaces/{namespaceId}/workers` | `OpsRead` |
| POST | `/namespaces/{namespaceId}/workers/{subject}/drain` | `WorkerManage` |
| POST | `/namespaces/{namespaceId}/workers/{subject}/undrain` | `WorkerManage` |
| GET | `/namespaces/{namespaceId}/publication-policy` | `DefinitionRead` |
| PUT | `/namespaces/{namespaceId}/publication-policy` | `PublicationManage` |
| GET | `/workflows/{workflowId}/publication-requests` | `DefinitionRead` |
| POST | `/workflows/{workflowId}/publication-requests` | `DefinitionPublish` |
| GET | `/publication-requests/{requestId}` | `DefinitionRead` |
| POST | `/publication-requests/{requestId}/approve` | `PublicationApprove` |
| POST | `/publication-requests/{requestId}/reject` | `PublicationApprove` |
| POST | `/publication-requests/{requestId}/withdraw` | `DefinitionPublish` |
| POST | `/publication-requests/{requestId}/publish` | `DefinitionPublish` |
| GET | `/workflows/{workflowId}/activations` | `DefinitionRead` |
| PUT | `/workflows/{workflowId}/active-version` | `VersionActivate` |
| POST | `/workflows/{workflowId}/versions/{versionNumber}/promote` | `Promotion`; authorize source and destination, debit destination once |
| POST | `/events` | `EventPublish` |
| GET, POST | `/namespaces/{namespaceId}/event-endpoints` | `EventRead`, `EventManage` |
| GET | `/namespaces/{namespaceId}/event-endpoints/{endpointId}` | `EventRead` |
| PUT, DELETE | `/namespaces/{namespaceId}/event-endpoints/{endpointId}` | `EventManage` |
| GET | `/namespaces/{namespaceId}/events/{source}/{eventId}` | `EventRead` |
| GET, POST | `/namespaces/{namespaceId}/webhook-subscriptions` | `EventRead`, `EventManage` |
| GET | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}` | `EventRead` |
| PUT, DELETE | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}` | `EventManage` |
| GET | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries` | `EventRead` |
| GET | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries/{deliveryId}` | `EventRead` |
| POST | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries/{deliveryId}/redrive` | `EventManage` |
| POST | `/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/redrive` | `EventManage` |
| GET, POST | `/namespaces/{namespaceId}/broker-subscriptions` | `EventRead`, `EventManage` |
| GET | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}` | `EventRead` |
| PUT, DELETE | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}` | `EventManage` |
| GET | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries` | `EventRead` |
| GET | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries/{deliveryId}` | `EventRead` |
| POST | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries/{deliveryId}/redrive` | `EventManage` |
| POST | `/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/redrive` | `EventManage` |
| GET, POST | `/namespaces/{namespaceId}/connectors` | `EventRead`, `EventManage` |
| GET | `/namespaces/{namespaceId}/connectors/{connectorId}` | `EventRead` |
| PUT, DELETE | `/namespaces/{namespaceId}/connectors/{connectorId}` | `EventManage` |
| GET | `/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters` | `EventRead` |
| GET | `/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}` | `EventRead` |
| DELETE | `/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}` | `EventManage` |
| POST | `/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}/redrive` | `EventManage` |
| GET, POST | `/namespaces/{namespaceId}/scripts` | `ScriptRead`, `ScriptWrite` |
| GET | `/namespaces/{namespaceId}/scripts/{scriptId}` | `ScriptRead` |
| PATCH, DELETE | `/namespaces/{namespaceId}/scripts/{scriptId}` | `ScriptWrite` |
| GET | `/namespaces/{namespaceId}/scripts/{scriptId}/versions` | `ScriptRead` |
| POST | `/namespaces/{namespaceId}/scripts/{scriptId}/versions` | `ScriptPublish` |
| GET | `/namespaces/{namespaceId}/scripts/{scriptId}/versions/{versionNumber}` | `ScriptRead` |
| GET, POST | `/namespaces/{namespaceId}/scripts/{scriptId}/test-runs` | `ScriptWrite` |
| GET | `/namespaces/{namespaceId}/scripts/{scriptId}/test-runs/{testRunId}` | `ScriptWrite` |
| GET | `/namespaces/{namespaceId}/script-executions` | `ScriptRead` |
| GET | `/namespaces/{namespaceId}/script-executions/{executionId}` | `ScriptRead` |

Entity routes for endpoints, subscriptions, connectors, scripts, and script executions resolve the
stored entity and compare its namespace with the URL before debiting. An attacker cannot spend a
victim namespace's token by combining a foreign entity identifier with that namespace's URL.

## Explicit exemptions

| Method | Route | Reason |
|---|---|---|
| GET | `/health` | Process probes must remain available under tenant load. |
| GET | `/ready` | Dependency probes must remain available under tenant load. |
| GET | `/auth/context` | Tenant-wide authenticated metadata. |
| GET | `/version` | Tenant-wide authenticated metadata. |
| GET, POST | `/workspaces` | Tenant-wide identity inventory and provisioning. |
| GET | `/workspaces/{workspaceId}` | Tenant-wide identity inventory. |
| POST | `/workspaces/{workspaceId}/namespaces` | Namespace provisioning has no existing namespace bucket. |
| GET | `/namespaces` | Tenant-wide namespace inventory. |
| GET | `/quotas` | Tenant-wide quota administration. |
| PUT, DELETE | `/quotas/{resource}` | Tenant-wide quota administration. |
| GET | `/audit` | The current query is tenant-wide and has no canonical namespace filter. A future namespace filter must be charged. |
| GET | `/namespaces/{namespaceId}/quotas` | Authorized `QuotaRead` recovery view; canonical tenant/namespace lookup remains enforced. |
| PUT, DELETE | `/namespaces/{namespaceId}/quotas/{resource}` | Authorized `QuotaManage` recovery control; a zero API rate must not lock administrators out of raising or resetting it. |
| GET | `/namespaces/{namespaceId}/activity-quotas` | Authorized `QuotaRead` recovery view; canonical tenant/namespace lookup remains enforced. |
| PUT, DELETE | `/namespaces/{namespaceId}/activity-quotas/{activity}` | Authorized `QuotaManage` recovery control. |
| POST | `/webhooks/{endpointId}` | External webhook authentication and persisted endpoint event-rate admission apply instead. |
| GET | `/script-artifacts/{digest}` | Artifact identity is tenant-deduplicated, not namespace-owned. |
| POST | `/script-runner/builds/claim` | Runner claims must remain available to settle existing work. |
| POST | `/script-runner/builds/{jobId}/heartbeat` | Runner heartbeats must remain available to settle existing work. |
| POST | `/script-runner/builds/{jobId}/complete` | Runner terminal reports must remain available to settle existing work. |
| POST | `/script-runner/test-runs/claim` | Test-run claims must remain available to settle existing work. |
| POST | `/script-runner/test-runs/{testRunId}/heartbeat` | Test-run heartbeats must remain available to settle existing work. |
| POST | `/script-runner/test-runs/{testRunId}/complete` | Test-run terminal reports must remain available to settle existing work. |
| GET | `/script-runner/artifacts/{digest}` | Runners need artifacts to settle existing work. |
| GET | `/script-runner/signing-key` | Runner trust bootstrap must remain available. |
| GET | `/script-runner/namespaces/{namespaceId}/egress-policy` | Runners need the policy required to settle existing work. |
| GET | `/script-runner/namespaces/{namespaceId}/pending-activities` | Runner recovery must remain available to settle existing work. |
| POST | `/script-runner/executions` | Runner execution reports must remain available to settle existing work. |
| POST | `/script-runner/incidents` | Runner incident reports must remain available to settle existing work. |
| All worker gRPC methods | Worker lease heartbeats and terminal completion must remain available to settle existing work. |

## Operational checks

When changing a public route:

1. Add it to exactly one inventory above.
2. For a charged route, select one closed `ApiOperation`, resolve the stored namespace, and invoke
   admission once. Do not add admission inside nested application services.
3. Parse the body, path, query, and required idempotency headers before debit. For upload routes,
   resolve and debit before polling the streaming body.
4. Verify denied, missing, cross-tenant, and URL-mismatch requests leave the bucket unchanged.
5. Verify a successful request and any later conflict/replay each consume exactly one token.
6. Keep worker and runner terminal paths exempt so a zero API rate cannot strand leased work.

The focused live contract is
`cargo test -p sagat-tests --test contract_rest_security_multitenancy api_rate_admission`.
