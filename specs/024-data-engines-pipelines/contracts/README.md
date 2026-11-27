# Feature 024 draft contracts

Draft fragments for review. None is canonical; T024-008 merges each into its canonical file in
`specs/001-platform-foundation/contracts/` and regenerates the compatibility baselines only after
reading their diffs. Every change is additive within its current version. Revised 2026-11-27 after
the correctness review (Decision 024-016: `CompleteStepRequest.data_result` is field 14, the event
payload is closed, the encoding table, the `TOMBSTONE` publication origin). Revised 2026-11-26 against
`develop` at `e5bda041`: the canonical files already carry feature 035's connection union, lake
drivers, egress ceiling and `ResolvedDatabase` fields 13 to 15, and these fragments extend them.

| Fragment | Merges into | Rule |
|---|---|---|
| [engines.md](engines.md) | OpenAPI driver catalog and connection schemas, worker protocol, runner config, docs | Driver enum grows; PostgreSQL unchanged |
| [data-assets.openapi.yaml](data-assets.openapi.yaml) | `openapi.yaml` | `/v1` additive: new paths, three variants added to the existing `DatabaseConnectionConfig` union, enum growth (drivers, quota resources, alert kinds, webhook events) |
| [workflow-data.schema.json](workflow-data.schema.json) | `workflow.schema.json` | Additive v1 (ADR-0032): three node kinds (`dataTransform`, `dataQuery`, `dataPublish`) and the `assets` property of `database` and `icebergWrite`, gated by the registry capability `data-pipelines-v1` |
| [worker-data.md](worker-data.md) | `worker.proto`, `worker.baseline.json`, the register of minors in `sdks/SPEC.md` section 9 | The next free minor when the contract merges (1.7 is feature 041's; Decision 024-014); new RPC and optional fields only |
| [events.asyncapi.yaml](events.asyncapi.yaml) | `events.asyncapi.yaml`, `events.baseline.json` | New message, enum values appended (FR-004-007 policy) |
| [data-runtime.md](data-runtime.md) | `docs/runbooks/data-runner.md`, `runners/data/README.md`, the data-mode note of Architecture decision 0079 | Operational contract of the data runner, the data mode of the sandbox runner and the data container profile |
| (no fragment) | `sdks/error-catalog.json`, `sdks/SPEC.md` section 2, each SDK's embedded copy | `DATABASE_COST_LIMIT` and the data failure codes as `worker`-origin entries (Decision 024-015) |
| (no fragment) | `specs/016-payload-classification/` carrier list | The `dataQuery` output joins the 016 carrier set with a platform-derived schema (D1265); coordinated with the 016 owner |

Clients MUST tolerate unknown `driver`, `kind`, alert `kind` and webhook event values (enum growth
rule). A server never substitutes a default for a row it cannot decode. A binary without these
drivers fails the whole connection list with an internal error on a new-driver row
(`StoreError::Internal`, FR-024-038), which is why the drivers' registry capabilities stay
unrecorded while a rollback to such a binary is planned; the registry also refuses such a process
once a capability is recorded (Architecture decision 0082).

## REST errors

The closed `Error.code` set is unchanged. New `details.reason` values, each with a remediation row
in `crates/api-http/src/error.rs` and named in the OpenAPI:

| Code | Reason | When |
|---|---|---|
| `VALIDATION_FAILED` | `DRIVER_NOT_ENABLED` | The driver is not advertised by this server, or its registry capability (`database-engines-v1`, `database-warehouses-v1`) is not recorded |
| `VALIDATION_FAILED` | `DATA_PIPELINES_NOT_ENABLED` | A definition holds a data node or an `assets` property, or an asset route is called, while the registry capability `data-pipelines-v1` is not recorded |
| `VALIDATION_FAILED` | `DRIVER_CAPABILITY_UNSUPPORTED` | The operation or a setting is not offered by the driver or the server's enabled capability |
| `VALIDATION_FAILED` | `CONNECTION_DRIVER_MISMATCH` | A node published for one driver resolves to a connection of another |
| `VALIDATION_FAILED` | `ASSET_PRODUCER_MISMATCH` | A workflow writes an asset whose producer is another workflow or node |
| `VALIDATION_FAILED` | `ASSET_CONTRACT_INCOMPATIBLE` | A revision breaks the compatibility mode, or a transform's declared output cannot match the contract |
| `VALIDATION_FAILED` | `ASSET_KIND_MISMATCH` | A transform writes an external asset, or an observation names a managed one |
| `VALIDATION_FAILED` | `DATA_NODE_SCOPE` | A data node is inside a body, a saga, or a definition with continuation, declares `destinations`, names the same output asset twice or the same input alias twice, or a `dataQuery` names a 017 database step as a step source |
| `CONFLICT` | `ASSET_NOT_ACTIVE` | The asset is paused or deleted |
| `CONFLICT` | `ASSET_PUBLISH_STALE` | `expectedPointerVersion` or the snapshot's base no longer matches |
| `CONFLICT` | `SNAPSHOT_QUARANTINED` | Publication of a quarantined snapshot |
| `CONFLICT` | `SNAPSHOT_NOT_PUBLISHABLE` | The snapshot is abandoned or retired (a rollback to a snapshot retention retired included: its metadata survives, so it is never `NOT_FOUND`), or a rollback target was never published |
| `CONFLICT` | `SNAPSHOT_RETIRED` | An 018 reset point would rematerialize a pin or reference to a retired snapshot |
| `QUOTA_EXCEEDED` | existing `limit` names plus the seven new resources | A ceiling refuses admission |

Foreign or missing assets, snapshots and connections are `NOT_FOUND` without a reason that could
reveal existence. Encoded (031) fields read by database or data-node parameter expressions are
refused with 031's codes (`VALIDATION_FAILED`, reason `ENCODING`, issue `ENCODING_READ_BY_EXPRESSION`);
this feature defines no encrypted-field code (coordinator decision C7).

## Step failure codes (closed, payload-free)

Database steps keep the `DatabaseFailure` set (feature 017's codes and feature 035's) and gain
`DATABASE_COST_LIMIT` (non-retryable); a `pii` write above a connection's ceiling is the existing
`CLASSIFICATION_EGRESS_REFUSED`. Data steps use a new closed set, mapped in
[data-runtime.md](data-runtime.md#failure-mapping):
`DATA_CONFIGURATION_INVALID`, `DATA_RUNTIME_UNAVAILABLE`, `DATA_INPUT_UNAVAILABLE`,
`DATA_INPUT_QUARANTINED`,
`DATA_STATEMENT_DENIED`, `DATA_TRANSFORM_FAILED`, `DATA_TYPE_UNSUPPORTED`, `DATA_RESOURCE_EXHAUSTED`, `DATA_OUTPUT_LIMIT`,
`DATA_CONTRACT_VIOLATION`, `DATA_AUDIT_FAILED`, `DATA_PUBLISH_STALE`, `DATA_ASSET_NOT_ACTIVE`,
`DATA_TIMEOUT`, `DATA_CANCELLED`, plus the existing `STORAGE_QUOTA_EXCEEDED` and `WORKER_ERROR`.
No code carries SQL, values, file names, provider or driver messages. Each code is a `worker`-origin
entry of `sdks/error-catalog.json` with its `retryable` flag and remediation, as feature 035's codes
are (Decision 024-015). A Polars step that design 046's block check refuses at lease fails
`DATA_STATEMENT_DENIED` with the closed reason `PACKAGE_BLOCKED` in its failure detail, in the lease
transaction, as a script step is refused (design 046 section 12).

## SDK conformance scenarios (new, shared by the six SDKs: TypeScript, Go, Python, C#, Java, Rust)

| Scenario | Covers |
|---|---|
| `database-engines` | canonical database nodes for each driver id, engine-native placeholders, typed literals and expressions, refused DDL; typed results from the MySQL/MariaDB/SQL Server fixtures and the fake warehouse servers |
| `data-transform-builders` | `dataTransform` (duckdb and polars), `dataQuery`, `dataPublish`, the `assets` property of `database` and `icebergWrite`; byte-identical canonical documents; root-only and producer refusals |
| `data-pipeline-runtime` | produce, audit, publish and read a snapshot end to end; a `dataQuery` over it with its classified output; audit failure; stale publication; publication-event trigger |
