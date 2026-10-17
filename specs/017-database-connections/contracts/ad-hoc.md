# Managed database execution admission

All routes use authenticated tenant/namespace scope and the existing finite API quota. No route
executes SQL in the API process. Responses containing execution or probe metadata use `no-store`.

## Create an ad hoc execution

`POST /namespaces/{namespaceId}/database-executions`, required `Idempotency-Key` (8–160 printable
ASCII characters), closed JSON body:

```json
{"mode":"step","definition":{"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow","metadata":{"name":"database-ad-hoc"},"entrypoint":"query","nodes":[{"id":"query","type":"database","connectionRef":{"name":"orders-db"},"operation":"query","statement":"SELECT $1::bigint","parameters":[{"literal":{"type":"integer","value":"42"}}]}]},"input":{},"timeoutMs":300000}
```

`mode` is `step` or `workflow`; input defaults to `{}`, timeout to 300,000 ms (1–3,600,000).
Definition uses the canonical workflow IR contract. Step mode requires one database node. Workflow
mode allows database/timer/condition/parallel/end only, 1–64 nodes, at least one database node and
no triggers. No scripts, arbitrary activities, child workflows or sagas in this delivery.

Returns 201 with the ordinary `Run` body and optional origin `AD_HOC_STEP`/`AD_HOC_WORKFLOW`.
The same key/body returns the same run after current authorization. A changed actor/body/origin
under the key conflicts. Generated IDs do not affect fingerprints. Admission needs current
`run:start`, `database:adhoc`, `database:use`, secret ACL and `database:write` for execute nodes.
Private definitions remain finite retained content and count toward workflow quota; identical
content by the same actor reuses its definition even when that quota is full.

## Probe a connection

`POST /namespaces/{namespaceId}/connections/{name}/probe`, required `Idempotency-Key`, closed body
`{}` or `{"revision":1}`. Returns 201 `Run` with origin `CONNECTION_PROBE`. The server builds the
fixed one-node query `SELECT 1::bigint`; SQL/parameters/origin cannot be provided. Same authorization
as an ad hoc query, including current active connection and secret ACL. Small result/deadline
limits apply. Probe success means durable `SUCCEEDED`, never mere admission or save success.

`GET /namespaces/{namespaceId}/connections/{name}/probe` returns the latest durable probe metadata
or `null` when none exists. It requires namespace/run metadata read authority; it contains run,
tested revision, timestamps, durable status and a closed failure code, never SQL, result cells or
credentials. An edit does not move evidence to the new revision.

`GET /runs?origin=AD_HOC_STEP|AD_HOC_WORKFLOW|CONNECTION_PROBE` retains existing namespace header,
pagination and authorization. Run cancellation, attempts and result reads use existing routes.
Rerun requires the original initiating actor, current ad hoc permissions and a justification;
it retains origin and resolves floating connection revisions at its new admission. A rerun starts
with a fresh default five-minute deadline (30 seconds for a fixed probe).
