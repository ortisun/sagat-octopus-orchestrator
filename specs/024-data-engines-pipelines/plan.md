# Implementation Plan: Data engines and pipelines

**Branch**: `feature/024-data-engines-pipelines` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/024-data-engines-pipelines/spec.md`

Status: Proposed — not started.

## Summary

Extend the feature-017 database model with five drivers and add a data-pipeline layer. Remote
engines (MySQL, MariaDB, SQL Server, BigQuery, Snowflake) become new `ConnectionConfig` variants,
driver-catalog entries and reserved managed activities executed by `apps/database-runner` under the
existing lease/capability protocol — the private deployment for MySQL, MariaDB and SQL Server, a
separate warehouse deployment for BigQuery and Snowflake — with per-engine network profiles,
credential codecs, query-mode enforcement and typed cell mapping. Engine-local computation (DuckDB
SQL, Polars) runs only in data nodes (`dataTransform`, `dataQuery`), in a per-attempt container
started by `apps/sandbox-runner` over immutable Parquet snapshots of declared data assets; DuckDB is
not a 017 driver (D1279); the engine verifies outputs itself, audits run in a second fresh container, and
publication is a compare-and-set of a pointer row committed with the step, a `dataPublish` node or
an API call (ADR-0058). Lineage, freshness, alerts and a publication event complete the layer.

## Technical Context

**Language/Version**: Rust (pinned `rust-toolchain.toml`), TypeScript (Studio), the five SDK
languages; Python 3.12 inside the data runtime image only.
**Primary Dependencies**: existing Tokio/Axum/tonic/SQLx stack; driver candidates `mysql_async` and
`tiberius` with rustls (D1252); warehouses over a guarded HTTPS client; `parquet` metadata reader in
a new adapter crate; DuckDB only inside the pinned SQL data runtime image, Polars only inside 023's
`python-data` runtime image, which this feature ships on 023's image-catalog mechanism (D1242, D1270).
**Storage**: PostgreSQL (authority: catalog, pointer, pins, publications); ADR-0023 blob backend for
snapshot parts; OpenBao KV v2 for credentials (017).
**Testing**: `cargo test -p sagat-tests` targets with Testcontainers (MySQL 8.4, MariaDB 11, SQL
Server 2022, PostgreSQL, OpenBao), fake BigQuery/Snowflake HTTP servers, sandbox containers;
five-SDK conformance; Playwright for the Studio.
**Target Platform**: Linux containers; SQL Server test image is x86-64 only (D1245).
**Project Type**: modular monolith + isolated executors + Studio + five SDKs.
**Performance Goals**: none promised; NFR-024-005 measurements on a declared environment.
**Constraints**: ADR-0023 mediated bytes (G1247), finite ceilings below, root-only data nodes.
**Scale/Scope**: 41 FRs, 8 user stories; delivery in eight increments.

## Constitution Check

Design checks, not approval of an implementation or a test result.

| Principle | Before research | After design | Evidence / future gate |
| --- | --- | --- | --- |
| I — durable atomic transitions | Pass | Pass | Pointer CAS, pins, snapshot rows, journal, next tasks and outbox commit in the completion/node/API transactions of the data model; T024-034, T024-060. |
| II — pure domain, inward dependencies | Pass | Pass | Drivers, assets, contracts, freshness and publication rules in `crates/domain`; drivers and Parquet only in `infra-*`; new architecture check T024-005. |
| III — at-least-once effects, honest compensation | Pass | Pass | Remote writes keep one attempt, lookup within the attempt or manual review (FR-024-010); pointer moves are internal state; rollback is explicit. T024-024, T024-061. |
| IV — pinned versions, five SDKs | Pass | Pass | Revisions pinned at admission, snapshots at first lease; additive IR/OpenAPI/proto/AsyncAPI; conformance T024-064. |
| V — isolated code, tenant and secret boundary | Justified | Justified | DuckDB/Polars only in per-attempt containers; the data profile relaxes ADR-0013's "no mounts" with a read-only input bind and sized `noexec` tmpfs mounts (ADR-0058 supersedes ADR-0013 in part; Complexity Tracking); warehouse egress in a separate runner deployment; codecs refuse secret-directed endpoints. T024-028, T024-039, T024-062. |
| VI — OTel, audit, typed errors | Pass | Pass | Closed labels, audit actions and codes below; T024-063. |
| VII — inbox/outbox, reconciliation | Pass | Pass | Publication event written to the inbox and delivery rows in the causing transaction; triggers deduplicate by inbox key. T024-046. |
| VIII — failure tests, verified contracts | Pass | Pass | Failure matrix below; rule-9 tasks T024-060..062; fake-server and real-service evidence separated (D1272). |

**Governance gate**: ADR-0058 (proposed; it supersedes ADR-0013, ADR-0023 and ADR-0027 in part) must
be accepted and O1240 answered before T024-010 (its migration already creates the
`DATA_SNAPSHOT_PART` purpose, the guarded `COMMITTED → DELETING` trigger and the `data` quota class);
O1243 before retention may delete bytes (T024-038; until then it only marks `RETIRED`); O1241 before
T024-027; O1242 and 023's `python-data` runtime before T024-051. Constitution Governance also
requires benchmark data and operational cost for the ADR-0023/0027 refinements before ADR-0058 is
accepted as implemented (T024-066).

## Project Structure

### Documentation (this feature)

```text
specs/024-data-engines-pipelines/
├── spec.md  research.md  plan.md  data-model.md  quickstart.md  tasks.md
├── checklists/requirements.md
└── contracts/  README.md  engines.md  data-runtime.md  worker-data.md
                data-assets.openapi.yaml  workflow-data.schema.json  events.asyncapi.yaml
```

### Source boundaries and planned additions

| Path | Change |
| --- | --- |
| `crates/domain/src/database.rs`, `crates/domain/src/database/failure.rs` | `DatabaseDriver` enum, `ConnectionConfig` variants, per-driver `DatabaseLimits::max_for`, reserved activity per driver, new closed failure codes |
| `crates/domain/src/data.rs` (new), `crates/domain/src/data/` | asset identity/kind/status, `SchemaContract`, column types and compatibility, `AuditCheck`, `FreshnessPolicy` and pure status evaluation, snapshot state transitions, publication CAS rule, retention selection |
| `crates/domain/src/workflow/` | `dataTransform` and `dataPublish` node kinds, `database.assets` annotation, root-only validation |
| `crates/domain/src/identity.rs`, `audit.rs`, `quota.rs` | four permissions, `runner` grant, audit actions, seven quota resources |
| `crates/application/src/database.rs` | per-driver credential codec port; capability resolution unchanged in shape |
| `crates/application/src/data_assets.rs` (new) | catalog service and ports (declaration write, read, lineage, freshness) |
| `crates/application/src/data_execution.rs` (new) | first-lease pins, `ResolveData`, completion acceptance, publication (node/API), events |
| `crates/application/src/data_retention.rs` (new) | retention selection and two-phase deletion port |
| `crates/infra-database/src/{mysql.rs,sqlserver.rs,bigquery.rs,snowflake.rs,endpoints.rs}` | adapters behind cargo features `mysql`, `sqlserver`, `bigquery`, `snowflake`; public-endpoint profile |
| `crates/application/src/blobs.rs`, `crates/infra-blobstore/src/backend.rs` | additive ranged read on `BlobBackend` for bounded footer reads |
| `crates/infra-data/` (new adapter crate), `crates/infra-data/fuzz/` | bounded, panic-isolated Parquet footer reader and contract verifier with a fuzz target; DuckDB statement pre-check (sqlparser `DuckDbDialect`) |
| `crates/infra-ir/src/{database.rs,data.rs}` | codecs for the new node kinds and driver references |
| `crates/infra-postgres/src/{data_assets.rs,data_execution.rs,data_retention.rs}`, `migrations/` | stores and additive migrations (data model) |
| `crates/infra-sandbox/src/{docker.rs,data.rs}` | data runtime profile (mounts, quotas, two-container flow) |
| `crates/api-grpc/src/{database.rs,data.rs,service.rs}` | `ResolveData`, data completion, new connection messages, minor negotiation |
| `crates/api-http/src/{dto_database.rs,dto_data.rs,routes/}` | connection union, asset/lineage/freshness/publication routes |
| `crates/scheduler/` | `dataPublish` activation in the reducer path; retention loop on `engine_leases` |
| `crates/telemetry/src/{metrics.rs,span.rs}` | closed vocabulary below |
| `apps/database-runner/src/`, `infra/helm/` | per-driver registration, config of enabled drivers and endpoint profile; a second (warehouse) deployment with its own NetworkPolicy |
| `apps/sandbox-runner/src/` | data task claim, staging, transform and audit containers, part upload |
| `runners/data/` (new) | SQL data runtime (`Dockerfile.runtime`, host program, README); `runners/data/python-data/` (`Dockerfile.runtime`, `catalog.json` under 023's `catalog.schema.json`) |
| `frontend/apps/studio/src/features/{connections,data-assets,workflows,runs}` | forms, catalog, lineage, editors, runtime graph |
| `sdks/{typescript,go,python,dotnet,java}` | builders for the new nodes and driver references |
| `tests/{contract,integration,fault,e2e}/data-engines-pipelines/`, `tests/contract/sdk-conformance/scenarios/` | suites and shared scenarios |
| `docs/runbooks/{database-engines.md,data-runner.md,data-assets.md}`, `docs/guides/data-pipelines.md` | operations and usage |

## Architecture decisions

1. **Drivers in the domain, adapters behind features.** `ConnectionConfig` becomes an enum of six
   variants (no DuckDB variant, D1279); every variant validates its closed settings; `DatabaseOperation` gains the driver it
   was published for so a connection revision of another driver is refused at admission. The
   runner dispatches by variant to the adapter compiled in; an adapter not compiled or not enabled
   makes the runner omit that activity from registration.
2. **Credential codecs per driver.** `DatabaseCredentialsCodec` gains a driver argument and an enum
   of credential kinds. Each codec rejects unknown fields, zeroizes buffers and lists its
   secret-bearing fields for the output scanner (FR-024-013). The BigQuery codec never reads an
   endpoint from the key; the Snowflake codec computes the public-key fingerprint itself.
3. **Network.** Two deployments of the same runner binary (D1243): the private profile (MySQL,
   MariaDB, SQL Server, PostgreSQL) keeps the 017 policy and has no public egress; the warehouse
   profile (BigQuery, Snowflake) has its own NetworkPolicy allowing DNS, the engine and TCP 443 to
   non-private ranges or listed private-endpoint networks, and no route to private customer
   networks. Private engines reuse `NetworkPolicy`. `endpoints.rs` adds the public-endpoint
   profile: host form fixed by driver, deployment host allowlist (exact or suffix) that namespace
   policy can only narrow, resolution inside
   the connector, every address checked against the reserved/metadata/platform deny list and
   required to be public unless inside an operator-listed private-endpoint network; one TLS identity
   = the original host; no redirects, no environment proxies, bounded response sizes.
4. **Statement handling.** Publication validates single-statement shape per dialect with sqlparser
   (`MySqlDialect`, `MsSqlDialect`, `BigQueryDialect`, `SnowflakeDialect`, `DuckDbDialect`); the
   adapter repeats it. Query mode is enforced at the engine (D1249) and the parser is never the
   boundary. Placeholders are engine-native (`?`, `@P1`, `?`/`@name`, `?`, `$1`).
5. **Warehouse protocol.** BigQuery: OAuth2 JWT-bearer token from the service-account key at the
   fixed token endpoint, `jobs.insert`/`jobs.query` with job ID = keyed hash of (tenant, run, node,
   attempt), `maximumBytesBilled`, dry run for query mode, `jobs.getQueryResults` pages within
   row/byte bounds, `jobs.cancel`, lookup by `jobs.get`. Snowflake: key-pair JWT (≤ 1 h, regenerated
   per attempt, never persisted), SQL API with `requestId` and `retry=true` on resubmission,
   `MULTI_STATEMENT_COUNT=1`, statement `timeout`, polling of 202 within the lease, partitions within
   bounds, cancel. Identifiers in labels/query tags are fixed strings only.
6. **Data runtime.** `runners/data/Dockerfile.runtime` (the SQL data runtime) pins a Python 3.12
   base by digest and the DuckDB package by hash, and bakes the Parquet and JSON extensions; it
   carries no tenant package catalog. Polars runs on the `python-data` image-catalog runtime: 023 owns
   the artifact mechanism (FR-023-027), this feature ships the image and catalog (D1270).
   The host program reads a bounded spec on standard input and writes one result line, as the
   feature-005 runners do. The sandbox runner stages inputs through `ReadBlob` of exactly the pinned
   parts into a per-attempt directory it bounds, bound read-only at `/in` (`ro,noexec,nosuid,nodev`);
   `/out` and `/spill` are tmpfs `noexec,nosuid,nodev` with `size=`, charged to the container memory
   cgroup, so the effective DuckDB `memory_limit` is the container memory minus `/out`, `/spill` and
   headroom (admission refuses limits that leave less than 256 MiB). `crates/infra-sandbox` gains a
   unit test over the generated container specification: every writable mount `noexec` and sized,
   every bind read-only, nothing else mounted. The runner then starts the transform container. Its
   `/out` tmpfs is discarded with it, so before exiting the host program streams each part to the
   runner over standard output in bounded length-prefixed frames; the runner uploads each part as it
   arrives (`WriteBlob`, digest computed while streaming) and keeps no copy. It then reads its own
   uploaded parts back with `ReadBlob` into a new staged directory, starts a fresh audit container
   with that directory bound read-only at `/in`, and reports the audited digests, which the engine
   compares with the committed parts (D1276). Step sources (D1274) are staged
   the same way: a committed Parquet step-output blob as one part, an inline or blob 017 database
   result as a typed JSON table. See [data runtime](contracts/data-runtime.md).
7. **Output acceptance.** The runner uploads parts with `WriteBlob` purpose `DATA_SNAPSHOT_PART`.
   The engine, outside any transaction, reads each part's footer through the blob backend (bounded:
   footer ≤ 1 MiB, 32 parts; the `BlobBackend` port gains an additive ranged read, implemented with
   `object_store` range requests in `crates/infra-blobstore`, so a footer is read without streaming
   the whole part), verifies schema/row counts/encryption flag against the pinned contract
   with `crates/infra-data` in `apps/execution-engine` (bounded decoder limits, a blocking task under
   `catch_unwind`, a fuzz target; G1256), then opens the completion transaction, rechecks the fence
   and the blob rows, and commits (data model). A mismatch, limit hit or decoder panic is
   `DATA_CONTRACT_VIOLATION`.
8. **Publication.** One application function performs the CAS for the three origins: lock the asset
   head(s) in id order, check status `ACTIVE`, current producer binding (run origins), snapshot
   state `AUDITED` or (rollback) previously `PUBLISHED`, and the expected version; then write the
   pointer, the publication record, abandon passed-over `AUDITED` snapshots, the inbox event, the
   webhook delivery rows and the audit. A snapshot equal to the current pointer returns
   `alreadyPublished` without writing.
9. **`dataPublish` in the reducer.** The node activates like `end`/`timer` (no lease); the scheduler
   path that applies the reducer's commands calls the publication function in the same transaction.
   Run cancellation, deadline and a concurrent API publication are decided by first commit.
10. **Freshness and alerts.** The 008 monitor gains two conditions computed in SQL with the database
    clock, bounded to 25 alerts per kind per namespace per look, with the existing fire/resolve rows
    and `ops.alert.*` deliveries.
11. **Retention.** A loop on `engine_leases` processes one asset per transaction in UUID order, bounded
    batches and wall time; mark phase commits before deletes; confirm phase follows ADR-0023. Byte
    deletion is enabled only after O1243; run purge (020, ADR-0054) never touches
    `DATA_SNAPSHOT_PART` blobs.
12. **`dataQuery`.** A data node with inputs, one `SELECT`, typed parameters and limits, claimed as
    `sagat.data.query` by a runner holding `data:execute`; it pins inputs at first lease, runs one
    container, encodes the result into the 017 typed-cell shape (≤ 256 KiB inline, validated by the
    017 result validator as a reusable function) and completes with it as its output. The output is a
    016 carrier: its schema is platform-derived and annotated with the highest level among the
    pinned input columns, so the existing payload disclosure decides who sees it. No 017 driver,
    role rule or result endpoint changes.
13. **Studio and SDKs.** The Studio follows the generated client; the driver form is driven by
    `listDatabaseDrivers`; lineage uses the graph component with a table alternative. SDK builders
    emit canonical IR; no SDK talks to an engine.

## Transaction boundaries

The complete list is in [data-model.md](data-model.md#transactions-on-these-records), with the
journal events this feature adds and their 018 reset rules (C1). Run admission follows the merged admission lock order of ADR-0050 (018): asset heads sit in slot 4 (catalog heads
and pins: 019 fences → 017 connection heads → 021 resource and variable heads → **024 asset heads** →
025 → 030), after authority, receipt and run-creating aggregates, before secrets, run rows,
semaphores and usage (D1273). The two critical ones:

```text
-- Data completion (CompleteStep with data outputs), after footer verification outside locks
BEGIN;
  lock run; existing managed completion guards; check lease/generation/receipt (replay returns
  the stored result);
  lock the output asset heads ascending BEFORE inserting rows that reference them: FOR NO KEY
     UPDATE when any output auto-publishes, otherwise FOR SHARE (taking the key-share of a foreign
     key first and upgrading later would let two completions deadlock); recheck ACTIVE and the
     producer binding under that lock (DATA_ASSET_NOT_ACTIVE / refusal otherwise);
  for each output: check every part blob is UPLOADED with this exact fence, size and sha256, and
     that the audited digests equal the parts' digests;
  insert data_snapshots (AUDITED|QUARANTINED), parts (blobs -> COMMITTED), inputs, audits;
  if publish = auto and AUDITED: CAS(pointer_version = base);
     write pointer, publication, abandon passed-over AUDITED, inbox event (protected_system:
     no content charge, no events_per_second spend), deliveries, audit;
     on stale: fail -> step FAILED DATA_PUBLISH_STALE | skip -> published=false;
  no new tenant_data_bytes charge: parts were charged at upload reservation (ADR-0027, D1278);
  update step, journal, next tasks/timers, outbox, completion receipt;
COMMIT;

-- API publish / rollback
BEGIN;
  receipt lookup (same key+fingerprint -> replay after current authorization);
  current membership guard (before any asset lock, D1273);
  lock asset head FOR NO KEY UPDATE; check ACTIVE, snapshot eligibility, expectedPointerVersion;
  write pointer, publication, receipt, inbox event, deliveries, audit;
COMMIT;
```

Provider and object-store I/O never runs inside these transactions (ADR-0014 begin rule; 017
two-phase capability pattern). Every new adapter transaction starts with `begin_safely`.

## Authorization matrix

| Action | Permission(s), checked on current membership | Built-in roles by default |
| --- | --- | --- |
| List drivers, read connections | `namespace:read` | admin, developer, operator, viewer |
| Create/revise/pause/delete a connection of any driver | `database:manage` + secret management/ACL | admin |
| Use a connection in a published workflow | `database:use` (+ `database:write` for `execute`) | admin, developer (write: admin) |
| Ad hoc execution on a remote driver | `run:start` + `database:adhoc` + `database:use` (+ `database:write` for execute) | admin, developer |
| Read a remote database result | `run:read` + `database:result:read` (017, unchanged) | admin |
| Run a `dataQuery` (published, or as a 022 standalone step) | `run:start` + `data:asset:use` | admin, developer |
| Read a `dataQuery` output | 016 payload disclosure at the highest pinned column level (`payload:read-internal`; the namespace `payload:reveal-pii` grant) | per 016 |
| Read assets, revisions, snapshots, audits, publications, freshness | `namespace:read` | admin, developer, operator, viewer |
| Read lineage | `namespace:read` + `definition:read` | admin, developer, operator, viewer |
| Declare, revise, pause or delete an asset | `data:asset:manage` | admin |
| Publish a workflow version that reads or writes assets | `definition:publish` + `data:asset:use`; a write (managed transform output or external `assets.writes`) also needs the producer binding, and an external write the asset's connection | admin, developer |
| Publish or roll back through the API | `data:asset:publish` | admin, operator |
| Claim a remote database task, `ResolveDatabase` | `worker:execute` + `database:execute`, current binding | `database-runner` (private deployment: mysql, mariadb, sqlserver, postgresql; warehouse deployment: bigquery, snowflake) |
| Claim a data task, `ResolveData`, `ReadBlob` of pinned parts, `WriteBlob` of parts | `worker:execute` + `data:execute`, current binding | `runner` |

Generic `ReadBlob` of a `DATA_SNAPSHOT_PART` blob is refused unless the part belongs to a snapshot
pinned for the calling attempt. Foreign resources are opaque `NOT_FOUND`; denials are audited with
the existing bounded deduplication. Four permissions are added (the total depends on the parallel
drafts that also add permissions). This feature adds no built-in role; tests assert the agreed set of
roles (021 adds `integration-runner`, 025 `agent-runner`), not a count.

## Finite ceilings

Values are provisional engineering bounds pending NFR-024-005 measurements; a ceiling is published
lower if a measured run does not support it.

| Resource | Default | Hard ceiling | Enforced by |
| --- | ---: | ---: | --- |
| Statement, parameters, rows, result bytes (all drivers) | 017 values | 017 values | existing `database_*` quotas |
| Query deadline — postgresql, mysql, mariadb, sqlserver | 10 s | 30 s | `database_timeout_ms` |
| Query deadline — warehouse drivers bigquery, snowflake | 60 s | 600 s | `database_analytic_timeout_ms` |
| BigQuery maximum bytes billed per statement | 10 GiB | 1 TiB | connection + `database_bytes_billed` |
| Warehouse lookups after an ambiguous response | 3 / 30 s | 5 / 60 s | runner, within the lease |
| Snowflake result partitions read per statement | 16 | 64 | runner, plus result bytes |
| Connections per namespace (all drivers, paused included) | 100 | 1,000 | `database_connections` |
| Managed database concurrency namespace / tenant | 16 / 64 | 64 / 256 | existing |
| Data assets per namespace | 200 | 2,000 | `data_assets` |
| Revisions per asset | — | 1,000 | store |
| Declaration bytes | — | 128 KiB | store check |
| Columns per contract | — | 256 | domain |
| Custom audit checks per asset / statement bytes | 8 / 16 KiB | 32 / 16 KiB | domain |
| Accepted values per check | — | 64 | domain |
| Inputs / outputs per transform | 8 / 1 | 32 / 8 | domain |
| Input bytes staged per attempt | 1 GiB | 4 GiB | runner, before start |
| Snapshot bytes | 256 MiB | 2 GiB | `data_snapshot_bytes`; `/out` tmpfs `size=` |
| Parts per snapshot (each ≤ 64 MiB blob ceiling) | 8 | 32 | engine |
| Parquet footer read per part | — | 1 MiB | engine |
| Container memory (includes the `/out` and `/spill` tmpfs) / CPUs | 3 GiB / 1 | 16 GiB / 4 | memory cgroup |
| `/spill` tmpfs | 1 GiB | 4 GiB | `size=`, DuckDB `max_temp_directory_size` |
| `dataQuery` result (inline output) / rows / wall time | 256 KiB / 1,000 / 2 min | 256 KiB / 10,000 / 10 min | engine, container |
| Transform wall time / audit wall time | 10 min / 2 min | 60 min / 10 min | lease deadline, container |
| Transform attempts | 2 | 5 | retry policy |
| Concurrent data attempts namespace / tenant | 4 / 16 | 32 / 128 | `data_concurrency`, `tenant_data_concurrency` |
| Pending data part uploads per tenant | 128 | 512 | blob catalog |
| Retained published snapshots per asset | 5 | 50 | retention |
| Unpublished / quarantined retention | 7 days | 30 days | retention |
| Terminal-run pin grace | 24 h | 24 h | retention |
| `tenant_data_bytes` | 10 GiB | 2^50 bytes | ADR-0027 accounting, class `data` |
| Lineage response | 200 nodes, depth 4 | 500 nodes, 2,000 edges, depth 8 | API |
| Freshness `maxLagSeconds` | — | 60 s .. 31,536,000 s | domain |
| Publication justification | — | 1,024 bytes | API |

Effective = min(request, namespace, deployment). Zero refuses new admission and is never
unlimited. Lowering a ceiling never rewrites a retained revision, kills dispatched work or deletes
retained snapshots; it refuses new growth.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
| --- | --- | --- |
| Runner crash after uploading parts, before completion | Parts `UPLOADED`, no snapshot | Lease expiry → retry with new generation and same pins; parts swept by ADR-0023 (fault) |
| Crash after completion commit, before acknowledgement | Snapshot and pointer committed | Receipt replay returns the same outcome; no second snapshot (fault) |
| Duplicate `CompleteStep` / stale generation | — | Receipt replay / `STALE_ATTEMPT`; stale parts never committed (integration) |
| Lost broker hint for a data task | Task `READY` | Reconciler re-emits; runner polls (fault) |
| Run cancellation races an auto-publishing completion | First commit wins | Loser is an audited no-op; never half-published (race test with barrier) |
| Run deadline races an approval decision before `dataPublish` | First commit wins (014) | Audited no-op for the loser (race test) |
| API publication races a run publication | Asset row lock serializes | CAS loser `DATA_PUBLISH_STALE` / `CONFLICT` (race test) |
| Pointer moves after a step pinned its input | Step reads its pinned snapshot | Retention keeps it (integration) |
| Retention races a new pin | Pin takes asset head `FOR SHARE`; retention `FOR NO KEY UPDATE` and recheck | Pinned snapshot never deleted (race test) |
| Object store unavailable during staging or footer read | Attempt fails retryably | Nothing published (fault) |
| Sandbox reports success for a bad file | Engine refuses | `DATA_CONTRACT_VIOLATION` (security) |
| Audit container crash or timeout | Snapshot `QUARANTINED` with `ERROR` outcome | Step fails `DATA_AUDIT_FAILED`; retry policy applies to a new attempt (fault) |
| A tenant file written to `/out` or `/spill` is executed | `EACCES` (`noexec`) | Step fails `DATA_TRANSFORM_FAILED`; mount-profile unit test and container canary (security) |
| `/out` filled past `size=` | `ENOSPC` in the container | `DATA_OUTPUT_LIMIT`; nothing committed (security) |
| Publication while content quota and `events_per_second` are exhausted | Commits: the event is `protected_system` and not rate-charged | Publication and completion succeed (integration at full quota) |
| An 018 reset point after a data node | Per the registry rules: pins and snapshot references rematerialized, publications and observations copied, never re-executed | Reset refused (`SNAPSHOT_RETIRED`) when a referenced snapshot is retired (integration with 018) |
| Binary rollback to 0.2.0 after a new-driver connection exists | 0.2.0 store raises an internal error on the whole connection list | Prevented operationally: new drivers stay disabled until rollback is no longer planned; schema downgrade refuses (integration on a 0.2.0 binary) |
| `tenant_data_bytes` exhausted during part upload | Reservation refused; attempt fails `STORAGE_QUOTA_EXCEEDED` (ADR-0027 path) | Earlier parts swept; pointer unchanged (integration) |
| Quarantined snapshot routed by `onError` to a `from` input | Input refused | `DATA_INPUT_QUARANTINED`; nothing derived from it published (integration) |
| Audit container reports digests of other files | Engine refuses | `DATA_CONTRACT_VIOLATION`; nothing committed (security) |
| Warehouse write, ambiguous response, lease held | Lookup decides | Known outcome reported; else `OUTCOME_UNKNOWN` (fake-server fault) |
| MySQL/MariaDB/SQL Server write, ambiguous response | `OUTCOME_UNKNOWN`, manual review | No automatic retry (real-container SIGKILL fault) |
| Failed compensation: the explicit compensating write on the error path fails | Run records both outcomes; review if unknown | Never assumed undone (fault) |
| Engine or runner restart mid-transform | Lease expiry, retry | Pins reused; old parts unused (restart test) |
| Publication event redelivered | Inbox key dedupe | One run per trigger per snapshot (fault) |
| OpenBao unavailable | Fail closed | `DATABASE_CREDENTIAL_UNAVAILABLE`, no fallback (integration) |
| Monitor lease lost mid-look | Alerts unchanged | Next look recomputes (integration) |

## Observability

**Spans** (attributes `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.node`,
`sagat.attempt`, `sagat.database.driver` or `sagat.data.engine`, `sagat.outcome`; never SQL, values,
locators or credentials): `database.execute` (existing, driver attribute added),
`database.warehouse.lookup`, `data.capability.resolve`, `data.stage`, `data.transform`, `data.audit`,
`data.footer.verify`, `data.snapshot.commit`, `data.publish`, `data.retention`, `data.freshness.look`.

**Metrics** (closed labels only): `database_executions_total{driver,operation,outcome}` and
`database_execution_seconds{driver,operation}` (the `driver` label is added; dashboards aggregate),
`database_connection_seconds{driver,outcome}`, `database_warehouse_lookups_total{driver,outcome}`
(`outcome` ∈ `succeeded|failed|unknown`), `database_cost_refusals_total{driver}`,
`data_transform_attempts_total{engine,outcome}`, `data_transform_seconds{engine}`,
`data_query_attempts_total{outcome}`, `data_footer_decoder_panics_total`,
`data_stage_bytes_total`, `data_footer_verifications_total{outcome}`,
`data_audit_checks_total{outcome}`, `data_publications_total{origin,outcome}`,
`data_snapshot_bytes` (histogram), `data_retention_operations_total{outcome}`, and the existing
`alerts_firing{kind}` with the two new kinds. `outcome`, `origin`, `driver` and `engine` are closed
enums in `crates/telemetry/src/metrics.rs`.

**Audit actions** (bounded metadata, written in the deciding transaction): `data.asset_changed`,
`data.snapshot_published` (origin run/node/api), `data.snapshot_rolled_back`,
`data.snapshot_quarantined`, `data.snapshot_read` (each `ResolveData`, collapsed per attempt/generation),
`data.snapshot_retired`, `data.denied`; existing `database.connection_changed`, `database.denied`,
`database.result_read` and `secret.resolved` cover the new drivers. An audit failure rolls back its
decision.

## Migration, compatibility and rollback

- **Contracts**: OpenAPI `/v1` additive (connection config becomes a `oneOf` by `driver`; the driver
  list grows; new asset routes; seven quota resources; two alert kinds; one webhook event kind).
  Clients must tolerate unknown `driver` values (enum growth); a contract test reads a list holding
  a MySQL connection with the previous generated Studio client. IR additive in `v1` (ADR-0032):
  three node kinds (`dataTransform`, `dataQuery`, `dataPublish`) and one optional `database.assets`
  field, capability-gated; the 016 carrier set gains the `dataQuery` output. AsyncAPI grows (new message
  `DataAssetPublished`, alert kind values); baseline regenerated after reading the diff. Worker
  protocol: new minor after 1.6 (D1271), new RPC and optional fields only.
- **Database**: additive migrations; downgrade refuses while assets, snapshots or new-driver
  connections exist (017 precedent) rather than deleting authority or evidence. A 0.2.0 *binary*
  cannot decode a new-driver connection row (`StoreError::Internal`, the whole list fails), so new
  drivers stay disabled until binary rollback is no longer planned, and the release notes say so.
- **Rollout**: one capability at a time (ADR-0032 decision 8): driver framework → each driver →
  data runtime → assets/transforms → lineage/freshness → Polars → external assets. Disabling a
  capability stops new publication and admission; in-flight runs drain with compatible readers.
- **Rollback**: disabling a driver hides it and refuses new admission; existing revisions and pins
  stay readable. Disabling data pipelines stops new transforms; pointers, snapshots and retention
  keep working so readers are not stranded.

## Delivery sequence

1. Driver framework with PostgreSQL only (no behaviour change; compatibility proof).
2. MySQL/MariaDB adapter, codecs, type table, probes; then SQL Server.
3. Public-endpoint profile, BigQuery and Snowflake against fake servers; real-service gate (O1241).
4. SQL data runtime image, sandbox data profile with its mount-profile test, `dataQuery` (US04 foundation).
5. Asset catalog, contracts, snapshots, transforms, engine verification, audits, auto publication.
6. `dataPublish`, API publish/rollback, publication event and triggers, lineage, freshness, alerts.
7. Polars catalog (O1242) and external assets.
8. Studio, five SDKs, conformance, runbooks, demo and release gates.

## Verification plan

- **Unit**: domain validation of every driver config, contract compatibility pairs, freshness
  evaluation over DST and boundaries, snapshot transitions, retention selection, CAS rule.
- **Integration (real PostgreSQL, OpenBao, engines)**: catalog/admission/pins; each driver against
  its container; footer verification; publication transactions; retention; runtime-role sweep.
- **Contract**: OpenAPI/IR/proto/AsyncAPI compatibility baselines; error catalogue; fake-server
  protocol fixtures; five-SDK canonical documents.
- **Security**: tenant battery over every new operation; credential and private-key canaries;
  sandbox canaries; secret-directed endpoint refusal; DNS rebinding and metadata address denial.
- **Fault (rule 9)**: crash at each commit point (staging, completion, publication, rollback,
  retention mark/confirm), duplicate completion and duplicate event, timeout vs decision race,
  lost broker hint, failed compensation write, engine/runner restart, quota exhaustion.
- **E2E**: Studio flows per driver and for assets; quickstart demo.
- **Performance**: NFR-024-005 report under `tests/performance/reports/data-engines-pipelines.md`
  with environment, failures and measured values only.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
| --- | --- | --- |
| Second executor class (sandbox runner) for DuckDB/Polars | Tenant computation must not share a process with other attempts' credentials | Running DuckDB inside the database runner exposes concurrent attempts' credentials to one engine bug or hostile query |
| Two containers per transform attempt (transform, audit) | Asset-owner audits must not be influenced by producer code | One container lets the transform tamper with its own audit |
| Engine-side Parquet footer verification | The contract must not rest on the sandbox's word | Trusting the report makes the contract advisory |
| New quota class `data` | Pipelines must not exhaust run payload capacity | Sharing `tenant_content_bytes` lets one large snapshot block run admission |
| Three new node kinds | Transform, query and publish are distinct durable steps (publication can wait on approval; a query writes nothing) | One node with an implicit publish cannot express audit-then-approve-then-publish; DuckDB as a 017 driver would change ADR-0036 decision 1 and the 017 result view |
| Data-profile mounts beyond ADR-0013 (read-only `/in` bind, sized `noexec` tmpfs `/out`, `/spill`) | Inputs and outputs of up to GiBs cannot travel on standard input and one stdout line | ADR-0013's stdin/stdout-only profile cannot carry datasets; an unbounded writable disk is what ADR-0013 rejected, so the mounts are sized tmpfs charged to memory and `noexec` |
| A second deployment of the database runner (warehouse profile): same app, separate process and NetworkPolicy | Principle V (isolation of tenant-authored effects): warehouse attempts send tenant SQL to public endpoints and must not share a pod, credentials in memory or network routes with attempts that reach private customer databases; 017's `apps/database-runner` is the precedent for a process isolated by its capability | One deployment with both would let a compromised warehouse attempt reach private networks (ADR-0055 option 4); a new app would duplicate the managed protocol with no extra isolation |

## Risks

- Mediated bytes may make large snapshots slow (G1247): ceilings start low and are measured.
- Driver crates may log SQL or allocate unbounded frames (017 found both in `tokio-postgres`):
  inspected before adoption; findings can block a driver (D1252).
- Warehouse behaviour without emulators (G1241): the real-service gate is mandatory.
- The curated Python catalog adds supply-chain upkeep (O1242).
