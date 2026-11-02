# Feature 024 draft contracts

Draft fragments for review. None is canonical; T024-008 merges each into its canonical file in
`specs/001-platform-foundation/contracts/` and regenerates the compatibility baselines only after
reading their diffs. Every change is additive within its current version.

| Fragment | Merges into | Rule |
|---|---|---|
| [engines.md](engines.md) | OpenAPI driver catalog and connection schemas, worker protocol, runner config, docs | Driver enum grows; PostgreSQL unchanged |
| [data-assets.openapi.yaml](data-assets.openapi.yaml) | `openapi.yaml` | `/v1` additive: new paths, `oneOf` connection config, enum growth (drivers, quota resources, alert kinds, webhook events) |
| [workflow-data.schema.json](workflow-data.schema.json) | `workflow.schema.json` | Additive v1 (ADR-0032): two node kinds and `database.assets`, capability-gated |
| [worker-data.md](worker-data.md) | `worker.proto`, `worker.baseline.json` | One new minor after 1.6 (D1271); new RPC and optional fields only |
| [events.asyncapi.yaml](events.asyncapi.yaml) | `events.asyncapi.yaml`, `events.baseline.json` | New message, enum values appended (FR-004-007 policy) |
| [data-runtime.md](data-runtime.md) | `docs/runbooks/data-runner.md`, `runners/data/README.md` | Operational contract of the sandbox data profile |
| (no fragment) | `specs/016-payload-classification/` carrier list | The `dataQuery` output joins the 016 carrier set with a platform-derived schema (D1265); coordinated with the 016 owner |

Clients MUST tolerate unknown `driver`, `kind`, alert `kind` and webhook event values (enum growth
rule). Older servers that read a connection or asset row they cannot decode answer `UNAVAILABLE`
and never substitute a default.

## REST errors

The closed `Error.code` set is unchanged. New `details.reason` values, each with a remediation row
in `crates/api-http/src/error.rs` and named in the OpenAPI:

| Code | Reason | When |
|---|---|---|
| `VALIDATION_FAILED` | `DRIVER_NOT_ENABLED` | The driver is not advertised by this server |
| `VALIDATION_FAILED` | `DRIVER_CAPABILITY_UNSUPPORTED` | The operation or a setting is not offered by the driver or the server's enabled capability |
| `VALIDATION_FAILED` | `CONNECTION_DRIVER_MISMATCH` | A node published for one driver resolves to a connection of another |
| `VALIDATION_FAILED` | `ASSET_PRODUCER_MISMATCH` | A workflow writes an asset whose producer is another workflow or node |
| `VALIDATION_FAILED` | `ASSET_CONTRACT_INCOMPATIBLE` | A revision breaks the compatibility mode, or a transform's declared output cannot match the contract |
| `VALIDATION_FAILED` | `ASSET_KIND_MISMATCH` | A transform writes an external asset, or an observation names a managed one |
| `VALIDATION_FAILED` | `DATA_NODE_SCOPE` | A data node is inside a body, a saga, or a definition with continuation |
| `CONFLICT` | `ASSET_NOT_ACTIVE` | The asset is paused or deleted |
| `CONFLICT` | `ASSET_PUBLISH_STALE` | `expectedPointerVersion` or the snapshot's base no longer matches |
| `CONFLICT` | `SNAPSHOT_QUARANTINED` | Publication of a quarantined snapshot |
| `CONFLICT` | `SNAPSHOT_NOT_PUBLISHABLE` | The snapshot is abandoned or retired, or a rollback target was never published |
| `CONFLICT` | `SNAPSHOT_RETIRED` | An 018 reset point would rematerialize a pin or reference to a retired snapshot |
| `QUOTA_EXCEEDED` | existing `limit` names plus the seven new resources | A ceiling refuses admission |

Foreign or missing assets, snapshots and connections are `NOT_FOUND` without a reason that could
reveal existence. Encoded (031) fields read by database or data-node parameter expressions are
refused with 031's codes (`VALIDATION_FAILED`, reason `ENCODING`, issue `ENCODING_READ_BY_EXPRESSION`);
this feature defines no encrypted-field code (coordinator decision C7).

## Step failure codes (closed, payload-free)

Database steps keep the 017 `DatabaseFailure` set and gain `DATABASE_COST_LIMIT` (non-retryable).
Data steps use a new closed set, mapped in [data-runtime.md](data-runtime.md#failure-mapping):
`DATA_CONFIGURATION_INVALID`, `DATA_RUNTIME_UNAVAILABLE`, `DATA_INPUT_UNAVAILABLE`,
`DATA_INPUT_QUARANTINED`,
`DATA_STATEMENT_DENIED`, `DATA_TRANSFORM_FAILED`, `DATA_TYPE_UNSUPPORTED`, `DATA_RESOURCE_EXHAUSTED`, `DATA_OUTPUT_LIMIT`,
`DATA_CONTRACT_VIOLATION`, `DATA_AUDIT_FAILED`, `DATA_PUBLISH_STALE`, `DATA_ASSET_NOT_ACTIVE`,
`DATA_TIMEOUT`, `DATA_CANCELLED`, plus the existing `STORAGE_QUOTA_EXCEEDED` and `WORKER_ERROR`.
No code carries SQL, values, file names, provider or driver messages.

## SDK conformance scenarios (new, shared by the five SDKs)

| Scenario | Covers |
|---|---|
| `database-engines` | canonical database nodes for each driver id, engine-native placeholders, typed literals and expressions, refused DDL; typed results from the MySQL/MariaDB/SQL Server fixtures and the fake warehouse servers |
| `data-transform-builders` | `dataTransform` (duckdb and polars), `dataQuery`, `dataPublish`, `database.assets`; byte-identical canonical documents; root-only and producer refusals |
| `data-pipeline-runtime` | produce, audit, publish and read a snapshot end to end; a `dataQuery` over it with its classified output; audit failure; stale publication; publication-event trigger |
