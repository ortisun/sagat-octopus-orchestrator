# Data model — 035 Iceberg writes and step-output destinations

Status: proposed design (2026-10-27, revised after review round 1). Nothing here exists yet. It
extends the 017 model (`specs/017-database-connections/data-model.md`) and the engine tables, and every
change is additive. The decisions cited are in [research.md](research.md).

## 1. Connection revisions (017 catalog)

There is no new table. `database_connection_revisions.config` (jsonb) keeps the driver tag, and
`crates/domain/src/database.rs::ConnectionConfig` gains three variants (D035-040).

Every variant, `Postgres` included, gains `egressCeiling`:

- values `public`, `internal` or `pii`; `secret` is not a valid value;
- default `internal`, and a revision without the field reads as `internal`;
- setting `pii` requires the actor to hold the 016 reveal grant for the namespace (FR-035-005).

### `driver: iceberg`

| Field | Type and rule |
|---|---|
| `catalog.type` | `rest` \| `glue` \| `hive` \| `jdbc` |
| `catalog.rest` | `uri` (HTTPS outside development), `warehouse`, `prefix?`; `auth`: `oauth2` (`tokenUri`, `scope?`) \| `bearer` \| `sigv4` (`region`, `signingName`); `accessDelegation`: `none` \| `vended-credentials`; `remoteSigning` (default `false`, D035-050) |
| `catalog.glue` | `region`, `catalogId`, `roleArn?`, `externalId?` |
| `catalog.hive` | `uris` (1–4 entries of `thrift://host:port`); `tls`: `required` \| `none`, and `none` only when the deployment sets `allowUnauthenticatedMetastore` |
| `catalog.jdbc` | `host`, `port`, `database`, `tls` (017 `TlsPolicy`); `engine`: `postgresql` (others refused in this delivery) |
| `storage.type` | `s3` \| `gcs` \| `adls` |
| `storage.s3` | `region`, `endpoint?` (S3-compatible), `pathStyle` (bool; virtual-hosted style preferred, G035-021), `roleArn?`, `externalId?` |
| `storage.gcs` | `project?`, `endpoint?` (emulator, development only) |
| `storage.adls` | `account`, `endpointSuffix?` |
| `storage.credentials` | `static` (uses `storageSecret`) \| `vended` (REST with `vended-credentials` only) |
| `warehouseLocations` | 1–16 URI prefixes (`s3://`, `gs://`, `abfss://`); every table, data, delete and metadata location must start with one of them (FR-035-004) |
| `allowedModes` | a subset of `append`, `overwrite`, `upsert`; default `[append]` |
| `catalogSecret`, `storageSecret` | logical secret references (017); no `storageSecret` when credentials are `vended` |
| `egressCeiling`, `limits` | as above; `limits` are intersected with the platform ceilings in plan.md |

The relay allows only the endpoints declared here and the official regional endpoints derived from
the storage type and region (D035-049).

### `driver: object_storage`

| Field | Type and rule |
|---|---|
| `storage` | as for the Iceberg connection |
| `prefix` | one URI prefix, no `..` |
| `storageSecret` | logical secret reference (017) |
| `formats` | `parquet`, `ndjson` |
| `egressCeiling`, `limits` | as above |
| `conditionalCreate` | `true` \| `false` \| `unknown`; found by the probe and recorded with its result, never authored |

### `driver: clickhouse`

| Field | Type and rule |
|---|---|
| `url` | HTTPS; plain HTTP only in development |
| `database` | the only database destinations may target |
| `tls` | verify-full |
| `secret` | logical secret reference (017) |
| `egressCeiling`, `limits` | as above |

### Credential secrets (OpenBao KV v2, 017 two-phase resolution, each pinned and audited)

| Secret of | JSON fields (all values write-only) |
|---|---|
| REST catalog | `{"clientId","clientSecret"}` (oauth2), `{"token"}` (bearer), or AWS keys (sigv4) |
| Glue, S3 | `{"accessKeyId","secretAccessKey","sessionToken"?}` |
| GCS | `{"serviceAccountJson"}` |
| ADLS | `{"accountKey"}`, `{"sasToken"}`, or `{"tenantId","clientId","clientSecret"}` |
| Hive | none (TLS or a private network) |
| JDBC catalog, ClickHouse | `{"username","password"}` |

Vended storage credentials arrive from the catalog inside the sandboxed attempt and never leave it
(FR-035-003).

## 2. Workflow IR (contracts/workflow-lake.schema.json)

### `icebergWrite` node

- **Domain shape:** `Node.iceberg: Option<IcebergWrite>`, beside `Node.database`.
- **Execution:** kind `Activity`, reserved activity `sagat.database.iceberg.write`.
- **Retries:** automatic, because recovery is lookup-backed.
- **Effect:** `high_risk` and `irreversible`, unless `compensation: revert`. A revert is a derived
  compensation `~c` running the reserved activity `sagat.database.iceberg.revert` with the source's
  connection pin (D035-056).
- **Fields:**
  - `connectionRef {name, revision?}`, `table {namespace[], name}`, `ref` (default `main`);
  - `mode`, `overwrite {scope, filter?, allowEmpty}`, `upsert {keyColumns?, deleteMarker?}`;
  - `records`: exactly one of an `inline` expression, a `blobs` expression or a `databaseResult` node
    id;
  - `mapping[] {field, column}`, `ignoreUnknownFields`, `recordSchema?` (an egress annotation,
    D035-051), `limits`, `compensation?: revert`.

### `destinations` on a node

- **Shape:** an array of at most 8 items. Each has `name`, `kind`, `connectionRef` or
  `subscriptionRef`, `target`, `mode`, `records` (`output`, an array path, `blobs`, or `inline`),
  `mapping`, `ignoreUnknownFields`, `recordSchema?`, `onFailure` (`hold` | `fail` | `continue`) and
  `limits`.
- **Allowed on:** root-level nodes that produce an output, except derived nodes and `icebergWrite`.
- **Restriction:** broker streams accept only `output`, `outputPath` and `inline` (D035-052).

### Derived destination nodes

Built at parse, before region analysis, and never stored (D035-054):

- **Identity:** id `<source>~d.<name>`, `NodeOrigin::Destination { source, name }`.
- **Activity per kind:** `sagat.database.destination.{iceberg,postgresql,object_storage,clickhouse}`;
  broker streams have none (engine-side).
- **Graph:** edges run source → each derived node → the source's original successors.
- **Saga region:** they belong to the source's region.

## 3. Engine state

| Table / column | Change |
|---|---|
| `steps.kind` | `CHECK` accepts `DESTINATION` |
| `steps.waiting_kind` | `OPERATOR` is reused for `hold` and `OUTCOME_UNKNOWN`; new `DELIVERY` for broker-stream steps waiting on deliveries |
| step output of a destination step | the receipt (contracts/receipts.schema.json), retained content under existing quotas |
| `webhook_subscriptions.purpose` | new: `RUN_EVENTS` (default for existing rows) \| `STEP_OUTPUT` |
| `webhook_subscriptions.egress_ceiling` | new, nullable: set only for `STEP_OUTPUT` (`public` \| `internal` \| `pii`) |
| `webhook_deliveries.kind` | accepts `STEP_OUTPUT` |
| `webhook_deliveries.run_id`, `.step_key`, `.payload_digest` | new, nullable, set only for `STEP_OUTPUT`; index on `(tenant_id, run_id, step_key)` where not null; `payload` keeps only the envelope, and the record data lives in `payloads` (D035-052) |
| `worker_bindings` of `DatabaseRunner` subjects | new `profile` (`database` \| `lake`; existing bindings read as `database`) (D035-057) |
| ad hoc runs | new origin `CATALOG_BROWSE`, excluded from run lists and reads (D035-065) |

**Effect keys.** An `icebergWrite` uses the existing step key. A destination step uses
`instance_destination_key = <source key>~d.<name>`, parallel to `instance_compensation_key`.

**Attempt input.** The input sent to the runner carries `effectKey`, `firstAttemptStartedAt`
(database clock), the pinned connection binding and the record references. A ClickHouse destination
records its baseline block numbers durably with the attempt (D035-060).

### Destination step lifecycle

```text
source completion accepted ──► READY ──► RUNNING ──► SUCCEEDED (receipt)
                                           │
                                           ├─ refusal / retries exhausted ─► onFailure:
                                           │     hold     ─► WAITING(OPERATOR) ─► RETRY_STEP (run:operate) ─► READY (broker: WAITING(DELIVERY))
                                           │                                   └► skip action (saga:override) ─► SKIPPED
                                           │     fail     ─► FAILED ─► forward-step failure in the source's saga region
                                           │     continue ─► SKIPPED (failure receipt; terminal; redrive = ad hoc execution)
                                           └─ OUTCOME_UNKNOWN ─► WAITING(OPERATOR) ─► RETRY_STEP | RESOLVE_STEP (SUCCEEDED | FAILED), both saga:override
broker stream: completion stores payloads and envelopes ─► WAITING(DELIVERY)
                 ─► shard owner sees all DELIVERED ─► SUCCEEDED | any DEAD ─► onFailure
```

The source's successors become ready once every destination step is `SUCCEEDED` or `SKIPPED`.

## 4. Lake runtime attempt (not persisted)

```text
load ─► compatible? ─no─► ICEBERG_UNSUPPORTED_TABLE
  └► search(effect key, from head H) ─found─► receipt (foundByLookup)
        │ inconclusive ─► OUTCOME_UNKNOWN
        └ absent ─► write files ─► commit(assert H) ─ok─► receipt
                                      │ conflict ─► search new snapshots ─► revalidate ─► retry (bounded)
                                      └ unknown  ─► search ─► found: receipt | absent: retry | inconclusive: OUTCOME_UNKNOWN
```

## 5. Permissions, quotas, audit (D035-066)

**Permissions.** One new permission, `database:browse`, for developers and admins. The catalog grows
from 48 to 49 entries, a change ADR-0071 records against ADR-0036 decision 5. The other permissions
reuse the code's existing grants:

| Action | Permission | Roles |
|---|---|---|
| Publish or run Iceberg writes and database-backed destinations | `database:use` + `database:write` | admin only, like 017 writes |
| Reference a broker destination | `event:manage` | developer, admin |
| Retry a destination that failed | `run:operate` | operator, admin |
| Retry or resolve a destination whose outcome is unknown | `saga:override` | admin |
| Skip a destination | `saga:override` | admin |
| Redrive a skipped destination (ad hoc) | `database:adhoc` + `database:write` | admin |

**Quota resources:**

- concurrent lake attempts per namespace and per tenant;
- record bytes per step;
- delivery rows and delivery bytes per completion;
- existing content quotas, which cover receipts and the `STEP_OUTPUT` payloads.

**Audit actions.** These reuse or add the following, none carrying values, paths or table contents:

- `DatabaseConnectionChanged` (reused, now for the new kinds), `DatabaseCatalogBrowsed`,
  `DestinationDecided` (retry, skip, resolve), `IcebergRevertRequested`;
- subscription purpose and ceiling changes;
- relay denials, reported in the result.

## 6. Storage classification (`crates/infra-postgres/storage-classification.json`)

| Column | Classification |
|---|---|
| `webhook_deliveries.payload` | stays `protected_system`: for `STEP_OUTPUT` it holds only the envelope |
| `webhook_deliveries.payload_digest`, `.step_key` | `protected_system` (identifiers, as for `node_id`) |
| `webhook_subscriptions.purpose`, `.egress_ceiling`; `worker_bindings.profile` | `protected_system` (closed enums) |

The record data of broker destinations, and every receipt, are run content in `payloads`. Retention,
erasure, quotas and payload inspection apply to them.
