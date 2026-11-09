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
  supported catalog/storage types and modes, so the Studio and SDKs never hard-code the list. **What it lists today** (built in slice S6c): `postgresql`
  always, and `iceberg` **when the deployment has `lake.enabled`**, with what the platform serves end to end and not with everything a connection
  document may say: `catalogTypes: [rest]`, `storageTypes: [s3]`, `modes: [append]`, `formats: [parquet, ndjson]`. A driver is added to the list when it
  is built (`object_storage` and `clickhouse` with their tasks, and each further catalog, storage or mode with its own), so that the list never
  advertises a capability that is not there.
- `ConnectionProbeResult.checks[]`: `{check, outcome, code?}` with checks `CATALOG_AUTH`,
  `WAREHOUSE`, `STORAGE_ACCESS`, `CONDITIONAL_CREATE`, `TABLE_DEDUPLICATION`, `DATABASE_AUTH`; outcome
  `PASSED` | `FAILED` | `SKIPPED`; codes from [errors.md](errors.md).
- **What `POST …/connections/{name}/probe` does for an Iceberg connection** (D035-150, built in slice S6c): the kind of the connection **at the revision
  asked for** (the current one when none is) chooses the probe, so one endpoint serves both, and a connection that is missing, paused, deleted,
  another tenant's or of a kind a probe does not test is answered as it has always been (the fixed `SELECT 1`, refused by admission). The start is
  authorised and admitted as any ad hoc start, and it is refused as one when `lake.enabled` is off (otherwise the probe would wait its 30 seconds for
  a runner that is not there). **At most one probe of an Iceberg connection is in flight for a connection and a revision:** another start is a
  `409` (`STATE_CONFLICT`, "a probe of this connection and revision is in flight") and makes no run, a revision is its own (another revision starts
  while this one is in flight), and a replay of the `Idempotency-Key` that started the probe is that run and not a conflict. A SQL probe is not held
  to this, as it never was. The probe waits for a runner that offers `sagat.database.lake.probe`, and has the budget of its own (one in flight in a
  namespace, four in a tenant) at the claim; the answer is read with `GET …/probe` as above. **The rule is against duplicates and not the bound of
  the cost:** a revision is its own key, so one probe of each revision of a connection may be in flight, and the cost of probes is bounded by the claim
  budget and the active-run quota. At the active-run limit the start is refused for the limit before the conflict is looked at, and a probe that
  ended, in any way, holds nothing.
- **What a reader of `GET …/connections/{name}/probe` sees for an Iceberg connection** (D035-150, built in slice S4b; `DatabaseConnectionProbe`
  in the OpenAPI): `checks[]` beside the `failureCode` every reader already had. A failed check is the answer and not a fault of the platform, so
  the run of the probe is `SUCCEEDED`; `run.status` says that **the probe ran**, and what it found is `failureCode` and `checks[]`.
  **`failureCode` is the code of the first failed check**, so a reader that knows no checks still sees that something is wrong; a fault of the
  platform (the step failed) keeps its own code and has no checks. An Iceberg probe that succeeded and whose result can no longer be read (the
  payload was purged, it is not the probe's, or the server has no codec) says `DATABASE_RESULT_UNAVAILABLE`: **no checks never means a healthy
  connection**. A `SKIPPED` check is not confirmed and is never `PASSED`; a connection is confirmed only when every check is `PASSED`. A probe
  that has not ended says nothing yet. The member is absent for any other probe. Every closed list in the schema (the checks, the outcomes, the
  codes) is held equal to the domain's by a contract test.

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
