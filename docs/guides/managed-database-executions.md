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

## Use the Studio

1. In **Connections**, create or select a PostgreSQL connection and its OpenBao secret. Activate
   the connection when it is ready for execution. **Probe / status** shows the latest tested
   revision; **Start probe** creates a durable read-only check.
2. In a normal workflow editor, add a **Database** node. Select the connection, optional revision,
   query/write operation, SQL and typed parameter bindings. Publish through the usual workflow
   controls. The resulting definition uses the same contract as the five SDKs.
3. In **Ad hoc database**, choose a single step or workflow. Workflows support the existing node
   list, connection controls, rename, undo/redo and inspector. **Add node** offers only the supported
   ad hoc kinds. Provide JSON input and run timeout, then choose **Run execution**.
4. Open the accepted run. In its graph or table, select a succeeded database node and choose
   **Read database result**. This button requires explicit result-reading permission; run metadata
   permission alone does not show rows. **Origin** in the Runs list finds ad hoc/probe executions.

Parameters are an ordered JSON array. Use quoted strings for exact integers and decimals:

```json
[
  {"literal": {"type": "integer", "value": "9223372036854775807"}},
  {"type": "text", "expression": "input.customerName"}
]
```

The editor refuses numeric JSON tokens it cannot represent exactly. Existing SDK/API definitions
with such numbers are refused before editing/autosave; adjust their exact numeric representation
through the SDK/API first. Database result cells retain their numeric text, including JSON values.
The result table pages through 25 rows and 12 columns; long cell previews explicitly show their
2,000-character display limit. This does not indicate server truncation of the retained result.
There is no export action. Closing the view or changing session/namespace clears its displayed
contents; result rows are not stored in the query cache or browser storage.

If an acknowledgement is uncertain, **Retry same execution** resends the original body and key.
The fields remain fixed. The request survives navigation and namespace switches within the same
session, with at most eight pending requests of 2 MiB each. A later permission/rate refusal cannot
unlock a potentially committed write. After successful recovery, **New execution** explicitly
starts a new intent. Signing out or reloading clears session memory; use Runs to locate/reconcile
an earlier execution before submitting another write. Credentials, SQL and input are not persisted
in browser storage for recovery.
