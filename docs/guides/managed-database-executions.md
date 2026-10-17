# Ad hoc PostgreSQL executions and connection probes

A saved namespace connection can be used by a published SDK/Studio workflow, an ad hoc step, an
ad hoc workflow, or a fixed connection probe. All four use the same durable engine, connection
revision pins, OpenBao capability resolution and isolated database runner.

## Test a saved connection

Send `POST /v1/namespaces/{namespaceId}/connections/{name}/probe` with a fresh `Idempotency-Key`
and `{}`. An optional `revision` selects an immutable revision. The server creates a fixed
`SELECT 1::bigint` query, limited to one row, 4 KiB of output and a five-second query/connect
budget. The durable run has a 30-second deadline. Connection policy can narrow these limits.
The connection must be active and the caller must be allowed to use its credential.

The 201 response contains a run ID and `origin: CONNECTION_PROBE`. Follow the normal run endpoint
or read `GET` on the probe endpoint. A durable `SUCCEEDED` is evidence for the returned revision
and timestamp. A save or an accepted probe does not prove connectivity. If a connection is edited,
the previous probe continues to identify the old revision. Failures expose a closed category,
for example `DATABASE_AUTHENTICATION_FAILED` or `DATABASE_DESTINATION_DENIED`, without provider
messages or credentials. Result cells require the separate audited database-result endpoint.

## Execute a step

Send this body to `POST /v1/namespaces/{namespaceId}/database-executions`, with a fresh
`Idempotency-Key`:

```json
{
  "mode": "step",
  "definition": {
    "apiVersion": "sagat-octopus-orchestrator.io/v1",
    "kind": "Workflow",
    "metadata": {"name": "orders-check"},
    "entrypoint": "query",
    "nodes": [{
      "id": "query",
      "type": "database",
      "connectionRef": {"name": "orders-db"},
      "operation": "query",
      "statement": "SELECT $1::bigint AS order_id",
      "parameters": [{"literal": {"type": "integer", "value": "42"}}]
    }]
  },
  "input": {},
  "timeoutMs": 300000
}
```

The response is an ordinary durable run with origin `AD_HOC_STEP`. Refreshing the client can
resume inspection by run ID. Repeating the same key and content returns the same run after
current authorization checks. Changed content or actor under that key conflicts. Cancellation,
attempt history and result access use the existing run APIs. Cancellation does not roll back a
customer write that has already committed; ambiguous writes require manual review.

Use `mode: workflow` for a graph of up to 64 database, timer, condition, parallel and end nodes,
including typed expression parameters from input or an earlier inline result. At least one
database node is required. Triggers, ordinary activities, scripts, child workflows and sagas
are outside the ad hoc subset. For larger result/blob consumption, see the
[SDK guide](managed-database-sdk.md).

## Authorization and retention

Execution requires `run:start`, `database:adhoc`, `database:use` and current secret ACL. Execute
operations also require `database:write`. Developers can execute queries when granted the secret;
operators/viewers can inspect metadata but cannot execute database operations. Results require
`database:result:read`. Every request and every later managed capability checks current authority.
Permission refusals produce bounded `database.denied` audit records without SQL or parameters.

The retained definition is private and immutable. It is absent from ordinary workflow catalogs,
publication/promotion and source APIs, and cannot be activated by ordinary run start. Identical
canonical content by the same actor in the same namespace reuses its private version. Each
execution still has its own run, audit, pins and outcome. Internal definitions count toward the
existing workflow quota (default 1,000 per namespace) and retained-content quota; distinct
content is refused when retention is full. Immutable definitions are not automatically deleted.
Reuse remains possible at the workflow-count ceiling, subject to run/content quotas.

Ad hoc execution is an explicit permission independent of workflow publication approval,
including in namespaces with protected publication. Generic rerun requires the original actor,
current `run:operate` plus execution permissions and a justification. It starts a new run with
the retained definition, original input and origin, resolving floating connection references
again. A new default five-minute deadline applies (30 seconds for probes).

Filter run metadata with `GET /v1/runs?origin=AD_HOC_STEP`, `AD_HOC_WORKFLOW` or `CONNECTION_PROBE`,
using the existing namespace header and pagination. Ordinary published runs omit `origin`.

## Operational evidence

Migration `20261030000000_database_ad_hoc` guards private identity and origin, and refuses a
downgrade while private definitions/runs remain. Retention, run/pins, initial journal/tasks/outbox,
audit and request receipt share one transaction. `database_executions_total` labels only the
closed origin/outcome set; `database_execution_denials_total` counts safe refusals. Existing
admission and runner metrics/traces describe capacity, authorization and actual execution.
The runner deployment requirements remain in the [runner runbook](../runbooks/database-runner.md).
