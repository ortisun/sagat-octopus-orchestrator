# Connection kinds and REST additions (draft, feature 035)

Merged into `specs/001-platform-foundation/contracts/openapi.yaml` at implementation. Field rules are in
[data-model.md](../data-model.md) §1. All changes are additive; existing PostgreSQL connections keep
their shape and read `egressCeiling` as `internal`.

## Schemas

- `DatabaseConnection.config` becomes `oneOf` `PostgresConnectionConfig` (existing, plus optional
  `egressCeiling`), `IcebergConnectionConfig` (`driver: const iceberg`), `ObjectStorageConnectionConfig`
  (`driver: const object_storage`), `ClickHouseConnectionConfig` (`driver: const clickhouse`), with
  `discriminator.propertyName: driver`. Every config is `additionalProperties: false`; URIs carrying
  user info or query credentials are refused (`VALIDATION_FAILED`, `reason: CREDENTIAL_IN_URI`).
- `GET /database-drivers` lists `postgresql`, `iceberg`, `object_storage`, `clickhouse`, each with its
  supported catalog/storage types and modes, so the Studio and SDKs never hard-code the list.
- `ConnectionProbeResult.checks[]`: `{check, outcome, code?}` with checks `CATALOG_AUTH`,
  `WAREHOUSE`, `STORAGE_ACCESS`, `CONDITIONAL_CREATE`, `TABLE_DEDUPLICATION`, `DATABASE_AUTH`; outcome
  `PASSED` | `FAILED` | `SKIPPED`; codes from [errors.md](errors.md).

## Catalog browsing (FR-035-007)

Browsing needs I/O in the managed executor, so it follows the probe's admission model (017 ad hoc
admission of a fixed, platform-defined operation):

| Operation | Method and path | Permission | Result |
|---|---|---|---|
| Start a browse | `POST /namespaces/{ns}/connections/{name}/catalog-browse` body `{level: NAMESPACES \| TABLES \| TABLE, parent?: string[], table?: {namespace, name}, pageToken?}` | `database:browse` | `202` with `{browseId}`; same-key replay returns the same id |
| Read a browse | `GET /namespaces/{ns}/catalog-browses/{browseId}` | `database:browse`, same actor | `{state: PENDING \| DONE \| FAILED, page?: {items[], nextPageToken?}, table?: {schema, partitionSpec, identifierFields, refs, formatVersion}, failure?: {code}}` |

Responses are `Cache-Control: no-store`, and pages hold at most 200 entries. Every browse is
audited (`DatabaseCatalogBrowsed`) without naming tables beyond the requested level. Another tenant's
connection is `404`.

Browse executions are ad hoc runs with origin `CATALOG_BROWSE` (D035-065):

- Run lists and run reads exclude that origin for every role.
- `GET …/catalog-browses/{browseId}` answers only the actor who started the browse; anyone else gets
  the same `404` as a missing browse.
- A same-key replay is keyed by tenant, namespace and actor.
- Results expire 10 minutes after completion with the run's retained output; the ad hoc run itself
  follows the existing retention.

## Run operations

`POST /runs/{id}/operations` on a step of kind `DESTINATION` (D035-055):

- `RETRY_STEP` from `WAITING(OPERATOR)` after a failure requires `run:operate`; after an unknown
  outcome (`OUTCOME_UNKNOWN`) it requires `saga:override`, as `runs.rs` already does for outcomes that
  may have happened;
- `RESOLVE_STEP` after an unknown outcome requires `saga:override`;
- the existing skip review action, widened from compensation steps to destination steps, requires
  `saga:override`;
- on a broker-stream destination step, `RETRY_STEP` returns it to `WAITING(DELIVERY)`
  ([destinations.md](destinations.md#broker_stream)).

A `SKIPPED` destination step is terminal and never reopened. A later redrive of an Iceberg,
PostgreSQL, object-storage or ClickHouse destination is an ad hoc destination execution:
`POST /namespaces/{ns}/database-executions` with `{runId, step, destination}`, reusing the
destination's pinned definition and effect key, so it cannot duplicate; it needs `database:adhoc` and
`database:write`. A broker-stream destination is redriven by the ADR-0025 delivery redrive.

## Subscriptions

`webhook_subscriptions` gain two fields in the create and read shapes:

- `purpose`: `RUN_EVENTS` (the default) or `STEP_OUTPUT`. A `STEP_OUTPUT` subscription selects no
  run-event kinds.
- `egressCeiling`: `public`, `internal` or `pii`. Only `STEP_OUTPUT` subscriptions carry it; setting
  `pii` requires the 016 reveal grant.

Changing either field is audited.
