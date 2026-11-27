# Implementation Plan: Data engines and pipelines

**Branch**: `feature/024-data-engines-pipelines` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/024-data-engines-pipelines/spec.md`

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decision 0058 accepted (delegate, Owner decision 2026-11-26-164); not started.
Revised 2026-11-26 against `develop` at `e5bda041` after an independent audit (research Decisions
024-001 to 024-015), and on 2026-11-27 after the correctness review (Decision 024-016).

## Summary

Extend the feature-017 database model with five drivers and add a data-pipeline layer. Remote
engines (MySQL, MariaDB, SQL Server, BigQuery, Snowflake) become new `ConnectionConfig` variants
beside `Postgres` and feature 035's `Iceberg`, driver-catalog entries and reserved managed activities
(`ManagedKind::Sql` per driver) executed by `apps/database-runner` under the existing
lease/capability protocol — the private deployment (profile `database`) for MySQL, MariaDB and SQL
Server, a warehouse deployment bound to a new profile `warehouse` for BigQuery and Snowflake — with
per-engine network profiles, credential codecs, query-mode enforcement, typed cell mapping and
feature 035's egress ceiling. Engine-local computation (DuckDB SQL, Polars) runs only in the data
nodes `dataTransform` and `dataQuery`, claimed by a new data runner (`apps/data-runner`, role
`data-runner`) that asks a data-mode sandbox runner for a per-attempt container through Architecture
decision 0079's attempt API, over immutable Parquet snapshots of declared data assets; DuckDB is not
a 017 driver (D1279); the engine verifies outputs itself, audits run in a second fresh attempt, and
publication is a compare-and-set of a pointer row committed with the step, a `dataPublish` engine
node or an API call (ADR-0058). Lineage, freshness, alerts and a publication event complete the
layer. Activation is by instance-registry capabilities.

## Technical Context

**Language/Version**: Rust (pinned `rust-toolchain.toml`), TypeScript (Studio), the six SDK
languages (TypeScript, Go, Python, C#, Java, Rust); Python inside the data runtime image only.
**Primary Dependencies**: existing Tokio/Axum/tonic/SQLx stack; driver candidates `mysql_async` and
`tiberius` with rustls (D1252); warehouses over a guarded HTTPS client; `parquet` metadata reader in
a new adapter crate; feature 035's attempt API, staging and relay modules (`crates/infra-lake`) and
container engine (`crates/infra-sandbox/src/lake/engine.rs`, generalized); DuckDB only inside the
pinned data runtime image; Polars and PyArrow only from a `python-data` script version's own lock
and design 046's signed layer (Decision 024-008; Owner decision 2026-11-26-162).
**Storage**: PostgreSQL (authority: catalog, pointer, pins, publications); the platform's ADR-0023
blob backend under the namespace for snapshot parts (Owner decision 2026-11-26-160); OpenBao KV v2
for credentials (feature 017's custody, which design 015 keeps).
**Testing**: `cargo test -p sagat-tests` targets with Testcontainers (MySQL 8.4, MariaDB 11, SQL
Server 2022, PostgreSQL, OpenBao), fake BigQuery/Snowflake HTTP servers, sandbox containers;
six-SDK conformance; Playwright for the Studio; the budget target
`integration_claim_transition_efficiency`.
**Target Platform**: Linux containers; SQL Server test image is x86-64 only (D1245).
**Project Type**: modular monolith + isolated executors + Studio + six SDKs.
**Performance Goals**: none promised; NFR-024-005 measurements on a declared environment.
**Constraints**: ADR-0023 mediated bytes (G1247), finite ceilings below, root-only data nodes.
**Scale/Scope**: 42 requirements, 8 user stories; delivery in a foundation slice and two lanes (see
[tasks](tasks.md#build-slices)).

## Constitution Check

Design checks, not approval of an implementation or a test result.

| Principle | Before research | After design | Evidence / future gate |
| --- | --- | --- | --- |
| I — durable atomic transitions | Pass | Pass | Pointer CAS, pins, snapshot rows, journal, next tasks and outbox commit in the completion/node/API transactions of the data model; the data lease and completion have budget shapes of their own and ordinary budgets do not grow (design 038); T024-034, T024-060, T024-074. |
| II — pure domain, inward dependencies | Pass | Pass | Drivers, assets, contracts, freshness and publication rules in `crates/domain`; drivers and Parquet only in `infra-*`; the existing package rules of `scripts/check-architecture.py` extended (T024-005). |
| III — at-least-once effects, honest compensation | Pass | Pass | Remote writes keep one attempt, lookup within the attempt or manual review (FR-024-010), each driver's recovery rule named in Architecture decision 0071 decision 7's vocabulary; pointer moves are internal state; rollback is explicit. T024-024, T024-061. |
| IV — pinned versions, six SDKs, graph and procedural models | Pass | Pass | Revisions pinned at admission, snapshots at first lease; additive IR/OpenAPI/proto/AsyncAPI at the next free protocol minor; six-SDK conformance (TypeScript, Go, Python, C#, Java, Rust) T024-064; data nodes are graph-model only and a procedural workflow reaches them through a child graph workflow (Decision 024-010, Gap 024-003). |
| V — isolated code, tenant and secret boundary | Pass | Pass | Untrusted DuckDB/Polars code runs only in per-attempt containers that a data-mode sandbox runner starts at the data runner's request (Architecture decisions 0058 and 0079); the data runner holds no engine access, and neither it nor the containers receive any secret or credential (they read only pinned blobs through lease-authorized reads), so no disclosure set applies to them. The five remote drivers are managed database capabilities whose selected credential is disclosed only to the lease-owning `database-runner` with `database:execute`: Principle V's **second** declared set, unchanged, with the profile binding of Architecture decision 0071 keeping the private and warehouse deployments apart. No constitution amendment is needed. The data profile's mounts refine ADR-0013 as the lake profile did (Complexity Tracking); codecs refuse secret-directed endpoints. T024-028, T024-039, T024-062, T024-071, T024-073. |
| VI — OTel, audit, typed errors | Pass | Pass | Closed labels, audit actions and codes below; step failure codes are `worker`-origin entries of the SDK error catalog (Decision 024-015); T024-012, T024-063. |
| VII — inbox/outbox, reconciliation | Pass | Pass | Publication event written to the inbox and delivery rows in the causing transaction; triggers deduplicate by inbox key. T024-046. |
| VIII — failure tests, verified contracts | Pass | Pass | Failure matrix below; rule-9 tasks T024-060..062; fake-server and real-service evidence separated (D1272). |

**Governance gate**: ADR-0058 is accepted (Owner decision 2026-11-26-164) and Owner questions 1240 to
1243 are decided (Owner decisions 2026-11-26-160 to 2026-11-26-163), so T024-010 creates the
`DATA_SNAPSHOT_PART` purpose, the guarded `COMMITTED → DELETING` trigger (enabled) and the `data`
quota class, and retention deletes bytes under the declared retention (T024-038). What still gates
work: the real-service suite on owner-provided accounts before `database-warehouses-v1` is recorded
(T024-027, Owner decision 2026-11-26-161); design 046 built and Gap 024-002 designed (T024-076)
before T024-051; Architecture decision 0079's acceptance and its Kubernetes half before the data
deployment ships on Kubernetes (Gap 024-001; Compose is not blocked); feature 020's run purge built
before T024-077 proves that the purge leaves snapshot parts and metadata alone (owner row
OD-2026-11-27-240, requested). Constitution Governance also
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
| `crates/domain/src/database.rs`, `crates/domain/src/database/failure.rs` | `SqlDriver` enum; four `ConnectionConfig` variants for the five drivers (MySQL and MariaDB share one) beside `Postgres` and `Iceberg`, each with `egress_ceiling`; `driver()`, `sql_secret()`, `sql_limits()` and `secret_slots()` (`SecretRole::Primary`) for them; per-driver `DatabaseLimits::max_for`; `ManagedKind::Sql(SqlDriver)`, `ManagedKind::{DataTransform, DataQuery}` with `activity()` and `profile()`; `DATABASE_COST_LIMIT` |
| `crates/domain/src/binding.rs` | `BindingProfile::{Warehouse, Data}` |
| `crates/domain/src/data.rs` (new), `crates/domain/src/data/` | asset identity/kind/status, `SchemaContract`, column types and compatibility, `AuditCheck`, `FreshnessPolicy` and pure status evaluation, snapshot state transitions, publication CAS rule, retention selection, the data failure set |
| `crates/domain/src/workflow/`, `crates/domain/src/workflow.rs` | `dataTransform`, `dataQuery` and `dataPublish` node kinds, the `assets` property of `database` and `icebergWrite`, root-only and graph-only validation, `Node::managed_kind` for data nodes |
| `crates/domain/src/execution/reset.rs` | the reset rule of each new journal event type (exhaustive match) |
| `crates/domain/src/identity.rs`, `audit.rs`, `quota.rs` | four permissions, the machine role `data-runner`, audit actions, seven quota resources |
| `crates/application/src/database.rs` | `DatabaseCredentialsCodec` generalized per driver (port); driver catalog and the `DRIVER_NOT_ENABLED` gate; capability resolution unchanged in shape |
| `crates/application/src/instances.rs` | `Capability::{DatabaseEnginesV1, DatabaseWarehousesV1, DataPipelinesV1}` in `Capability::ALL` |
| `crates/application/tests/managed_kind_sites.rs` | extended to the new managed kinds |
| `crates/infra-secrets/src/database.rs` | the per-driver credential codecs beside `JsonDatabaseCredentials` (password, service-account key with fixed token URI/universe, key pair) |
| `crates/application/src/data_assets.rs` (new) | catalog service and ports (declaration write, read, lineage, freshness) |
| `crates/application/src/data_execution.rs` (new) | first-lease pins, `ResolveData`, completion acceptance, publication (node/API), events |
| `crates/application/src/data_retention.rs` (new) | retention selection and two-phase deletion port |
| `crates/infra-database/src/{mysql.rs,sqlserver.rs,bigquery.rs,snowflake.rs,endpoints.rs}` | adapters behind cargo features `mysql`, `sqlserver`, `bigquery`, `snowflake`; public-endpoint profile |
| `crates/application/src/blobs.rs`, `crates/infra-blobstore/src/backend.rs` | additive ranged read on `BlobBackend` for bounded footer reads |
| `crates/infra-data/` (new adapter crate), `crates/infra-data/fuzz/` | bounded, panic-isolated Parquet footer reader and contract verifier with a fuzz target; DuckDB statement pre-check (sqlparser `DuckDbDialect`) |
| `crates/infra-ir/src/{database.rs,data.rs}` | codecs for the new node kinds and driver references |
| `crates/infra-postgres/src/{data_assets.rs,data_execution.rs,data_retention.rs}`, `crates/infra-postgres/src/database.rs`, `migrations/` | stores; `config_from` decodes the new driver configurations; additive migrations (data model, registry capabilities, binding profiles) |
| `crates/infra-sandbox/src/data.rs`, `crates/infra-sandbox/src/lake/engine.rs` | the data container profile beside `lake.rs` (mounts, class budgets, mount-profile unit test); the lake engine's list, inspect, wait and kill by label generalized to the data mode |
| `crates/infra-lake/src/{sandbox_api.rs,staging.rs,relay.rs,supervisor.rs}` | reused by the data runner: attempt API wire types and client, `uid_for_slot`, attempt directory, peer check, table of endings; extended with the data mode's request member `layer` and its own slot range |
| `crates/api-grpc/src/{database.rs,data.rs,service.rs}` | `ResolveData`, data completion, new connection messages, minor negotiation |
| `crates/api-http/src/{dto_database.rs,dto_data.rs,routes/}` | connection union, asset/lineage/freshness/publication routes |
| `crates/scheduler/` | `dataPublish` activation in the reducer path; retention loop on `engine_leases` |
| `crates/telemetry/src/{metrics.rs,span.rs}` | closed vocabulary below |
| `apps/database-runner/src/`, `infra/helm/` | per-driver registration, config of enabled drivers and endpoint profile; a deployment bound to the `warehouse` profile with its own NetworkPolicy |
| `apps/data-runner/` (new, package `sagat-data-runner`) | data task claim, `ResolveData`, staging into the attempt directory, the attempt socket, transform and audit attempts through the attempt API, part upload, report; no container-engine dependency |
| `apps/sandbox-runner/src/data/` (new) | the exclusive data mode: the three calls of Architecture decision 0079 over a node-local Unix socket, class budgets, label state, sweep and drain, as `src/lake/` does |
| `runners/data/` (new) | data runtime (`Dockerfile.runtime`: Python line of design 046's dependency runtime, DuckDB, the host program for the data contract, README); `runners/data/python-data/starter/` (the reviewed starter lock for Polars and PyArrow, a template, not a catalog) |
| `frontend/apps/studio/src/features/{connections,data-assets,workflows,runs}` | forms, catalog, lineage, editors, runtime graph |
| `sdks/{typescript,go,python,dotnet,java,rust}`, `sdks/error-catalog.json` and the embedded copies | builders for the new nodes and the `assets` property; the `worker`-origin catalog entries |
| `scripts/check-architecture.py` (+ `scripts/tests/`) | `sagat-data-runner` joins the packages without container-engine access; `parquet` only in `sagat-infra-data`; target drivers only in `sagat-infra-database` |
| `tests/{contract,integration,fault,e2e}/data-engines-pipelines/`, `tests/contract/sdk-conformance/scenarios/` | suites and shared scenarios |
| `docs/runbooks/{database-engines.md,data-runner.md,data-assets.md}`, `docs/guides/data-pipelines.md` | operations and usage |

## Architecture decisions

1. **Drivers in the domain, adapters behind features.** `ConnectionConfig` gains four variants for
   the five drivers (`Mysql` shared by `mysql` and `mariadb` with a `SqlDriver` field, `SqlServer`, `BigQuery`,
   `Snowflake`) beside `Postgres` and feature 035's boxed `Iceberg` (no DuckDB variant, D1279); every
   variant validates its closed settings and carries `egress_ceiling`; `driver()` answers each id;
   `sql_secret()`, `sql_limits()` and `secret_slots()` treat every SQL variant as `Postgres` is
   treated. `DatabaseOperation` gains the driver it was published for so a connection revision of
   another driver is refused at admission. `ManagedKind::Sql(SqlDriver)` names the reserved activity
   `sagat.database.<driver>` and the profile (`Database` or `Warehouse`); the runner dispatches by
   variant to the adapter compiled in; an adapter not compiled or not enabled makes the runner omit
   that activity from registration, and the server hands a step only to a runner that offers its
   activity (D035-150).
2. **Credential codecs per driver.** `DatabaseCredentialsCodec` gains a driver argument and an enum
   of credential kinds. Each codec rejects unknown fields, zeroizes buffers and lists its
   secret-bearing fields for the output scanner (FR-024-013). The BigQuery codec never reads an
   endpoint from the key; the Snowflake codec computes the public-key fingerprint itself.
3. **Network.** Two deployments of the same runner binary (D1243), each bound to its
   `BindingProfile` (`database`, and the new `warehouse`; `worker_bindings_profile_valid` widened):
   the private profile (MySQL, MariaDB, SQL Server, PostgreSQL) keeps the 017 policy and has no public
   egress; the warehouse profile (BigQuery, Snowflake) has its own NetworkPolicy allowing DNS, the engine and TCP 443 to
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
6. **Data runtime and the attempt path** (Decision 024-001, Architecture decisions 0058 and 0079).
   `runners/data/Dockerfile.runtime` pins by digest the Python line of design 046's dependency
   runtime and the DuckDB package by hash, bakes the Parquet and JSON extensions, and carries the
   host program for the data contract; it carries no package catalog. A Polars attempt additionally
   gets its version's signed design 046 layer read-only at `/opt/sagat/deps` (Decision 024-008).
   The **data runner** (`apps/data-runner`) claims the data activity, calls `ResolveData`, creates an
   attempt directory in the work volume it shares with the node's data-mode sandbox runner (the lake
   runner's `AttemptDir`), stages the pinned parts into its `in/` through `ReadBlob` (allowed by the
   lease's `step_lease_blobs` rows, each part's size and SHA-256 checked while reading, the total
   bounded before any download), writes the request file, binds the attempt socket for the slot's
   peer user, and asks the **data-mode sandbox runner** for an attempt (`POST /v1/attempts/{id}`
   with slot, class and deadline; `wait`; `DELETE`). The data mode builds the container from its own
   configuration: the image by digest, the class budget (memory including the `/out` and `/spill`
   tmpfs, CPUs, processes), the attempt directory read-only, `/out` and `/spill` tmpfs
   `noexec,nosuid,nodev` with `size=`, nothing else. An author's `limits` select the smallest
   configured class that covers them and lower what is enforced in software (DuckDB `memory_limit`
   is the class memory minus `/out`, `/spill` and headroom; admission refuses limits no class covers
   or that leave DuckDB less than 256 MiB). `crates/infra-sandbox` gains a unit test over the
   generated container specification: every writable mount `noexec` and sized, every bind read-only,
   the signed layer the only executable mount, nothing else mounted. The host program connects to
   the attempt socket, reads the request, runs the statement or script and, before it exits, streams
   each part of `/out` in bounded length-prefixed frames; the data runner uploads each part as it
   arrives (`WriteBlob`, digest computed while streaming) and keeps no copy. It then reads its own
   uploaded parts back with `ReadBlob` into a new attempt directory, checking each against the
   SHA-256 the server measured at upload (this read-back is what binds the audit to the uploaded
   bytes), asks for a fresh audit attempt in the transform's class with that directory read-only, and
   reports the digests of what it staged, which the engine compares with the committed parts: that
   comparison catches a data-runner fault that staged other bytes, not a lying audit container, whose
   outcomes rest on the sandbox (Gap 1249; Decision 024-016). A memory kill of the transform reported
   by `DELETE` (`memory_killed`) is `DATA_RESOURCE_EXHAUSTED`; a refusal or an unavailable sandbox is
   `DATA_RUNTIME_UNAVAILABLE` (retryable, not counted against `maxAttempts` when no container of the
   attempt started, at most 10 such attempts per step); any failure of the audit attempt itself
   (refusal, unavailable, memory kill, timeout, crash, `ERROR` of a platform-generated check) is
   `DATA_RUNTIME_UNAVAILABLE` with no snapshot committed, never a quarantine. On start-up the data
   runner ends, through `DELETE`, every attempt whose directory it finds in its work volume before it
   offers the activities, so an attempt orphaned by its crash does not hold a slot until its label
   deadline. Step sources (D1274) are staged the same way: a committed Parquet step-output blob
   as one part, an inline or blob 017 database result as a typed JSON table. See
   [data runtime](contracts/data-runtime.md).
7. **Output acceptance.** The runner uploads parts with `WriteBlob` purpose `DATA_SNAPSHOT_PART`.
   The engine, outside any transaction, reads each part's footer through the blob backend (bounded:
   footer ≤ 1 MiB, 32 parts; the `BlobBackend` port gains an additive ranged read, implemented with
   `object_store` range requests in `crates/infra-blobstore`, so a footer is read without streaming
   the whole part), verifies schema/row counts/encryption flag against the pinned contract
   with `crates/infra-data` in `apps/execution-engine` (bounded decoder limits, a blocking task under
   `catch_unwind`, a fuzz target; G1256) under the normative encoding table of the
   [data runtime contract](contracts/data-runtime.md#contract-types-and-parquet-encodings), then
   opens the completion transaction, rechecks the fence and the blob rows, and commits (data model).
   A mismatch, limit hit or decoder panic is `DATA_CONTRACT_VIOLATION`. An object-store error while
   reading a footer (`BlobReadError::Unavailable`) is not a mismatch: `CompleteStep` answers the
   transient gRPC status `UNAVAILABLE` with no state change, and the data runner sends the same
   completion again with the same idempotency key while its lease holds.
8. **Publication.** One application function performs the CAS for the three origins: lock the asset
   head(s) in id order, check status `ACTIVE`, current producer binding (run origins; for
   `RUN_NODE` against the `dataTransform` node the `dataPublish` names in `from`), snapshot
   state `AUDITED` or (rollback) previously `PUBLISHED`, and the expected version; then write the
   pointer, the publication record, abandon passed-over `AUDITED` snapshots that no live run pins,
   the inbox event, the webhook delivery rows and the audit. A snapshot equal to the current pointer
   returns `alreadyPublished` without writing. An `ABANDONED` snapshot met by a `dataPublish` is
   stale and follows `onStale`; a stale auto-publication commits its snapshot directly as `ABANDONED`
   when no `from` or `dataPublish` of the definition names its node. The output documents of the
   transform and of `dataPublish` are `protected_system` records, so a run publication never meets a
   content-quota refusal (Decision 024-016); the API justification is charged `content`, so an API
   publication at a full content quota is refused `QUOTA_EXCEEDED` with everything rolled back
   (Decision 024-017).
9. **`dataPublish` in the reducer.** The node activates like `end`/`timer` (no lease); the scheduler
   path that applies the reducer's commands calls the publication function in the same transaction.
   Run cancellation, deadline and a concurrent API publication are decided by first commit.
10. **Freshness and alerts.** The 008 monitor gains two conditions computed in SQL with the database
    clock, bounded to 25 alerts per kind per namespace per look, with the existing fire/resolve rows
    and `ops.alert.*` deliveries. Lag is measured from the current snapshot's `created_at` (managed)
    or `last_observed_at` (external), never from `last_published_at`, so a rollback does not refresh
    an asset. `DATA_SNAPSHOT_QUARANTINED` is seen while an unretired quarantined snapshot newer than
    the current one exists (or any, before the first publication).
11. **Retention.** A loop on `engine_leases` processes one asset per transaction in UUID order, bounded
    batches and wall time; mark phase commits before deletes; confirm phase follows ADR-0023. Byte
    deletion follows the retention the asset manager declared (Owner decision 2026-11-26-163) and
    waits until no `step_lease_blobs` row names the part; run purge (020, ADR-0054) never touches
    `DATA_SNAPSHOT_PART` blobs.
12. **`dataQuery`.** A data node with inputs, one `SELECT`, typed parameters and limits, claimed as
    `sagat.data.query` by the data runner (`data:execute`); it pins inputs at first lease, runs one
    container, encodes the result into the 017 typed-cell shape (≤ 256 KiB inline, validated by the
    017 result validator as a reusable function) and completes with it as its output. The output is a
    016 carrier: its schema is platform-derived and annotated with the highest level among the
    pinned inputs, an asset input at its highest pinned column level and a step source at `pii`
    unless its output is a 016 carrier that declares a level for every field; publication refuses a
    step source that is a 017 database step (`DATA_NODE_SCOPE`), so a database result keeps the
    017 audience (`database:result:read`); the first-lease transaction records each input's level in `step_data_inputs`, so the
    existing payload disclosure decides who sees it and never meets an undefined level. No 017 driver,
    role rule or result endpoint changes.
13. **Studio and SDKs.** The Studio follows the generated client; the driver form is driven by
    `listDatabaseDrivers`; lineage uses the graph component with a table alternative. The six SDK
    builders emit canonical IR; no SDK talks to an engine.
14. **Activation** (Decision 024-003). One migration adds `database-engines-v1`,
    `database-warehouses-v1` and `data-pipelines-v1` to the three closed capability CHECKs of the
    instance registry and `Capability::ALL`; each is listed by a control plane or engine only when
    its build serves it, and recorded by the operator (Architecture decision 0082). Catalog writes,
    publication and admission that need an unrecorded capability are refused before any work.
15. **Merged features reused, not re-invented.** Runs started by `data.asset.published` triggers pass
    design 043's gate at row 9 and probes or ad hoc executions at row 13 (Decision 024-011); the data
    lease and completion get budget shapes in `integration_claim_transition_efficiency` (Decision
    024-012); data steps are refused step logs (Decision 024-013); new-driver connections carry the
    egress ceiling under the uniform `pii`-egress rule (Decision 024-005); every driver declares its
    recovery rule (Decision 024-006); an Iceberg table is an external asset with an `icebergWrite`
    producer (Decision 024-009).

## Transaction boundaries

The complete list is in [data-model.md](data-model.md#transactions-on-these-records), with the
journal events this feature adds and their 018 reset rules (C1). Run admission follows the merged admission lock order of ADR-0050 (018): asset heads sit in slot 4b (catalog heads
and pins: 019 fences → 017 connection heads → 021 resource and variable heads → **024 asset heads** →
025 → 030), and a writer of the asset catalog takes the 024 data-catalog fence in slot 4a, after
authority, receipt and run-creating aggregates, before secrets, run rows, semaphores and usage
(D1273). The two critical ones:

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
     that the digests the data runner staged for the audit equal the parts' digests (a data-runner
     self-check; the audit outcomes rest on the sandbox, Gap 1249);
  insert data_snapshots (AUDITED|QUARANTINED, or ABANDONED for a stale auto-publication no later
     node reads), parts (blobs -> COMMITTED), inputs, audits;
  if publish = auto and AUDITED: CAS(pointer_version = base);
     write pointer, publication, abandon passed-over AUDITED that no live run pins, inbox event
     (protected_system: no content charge, no events_per_second spend), deliveries, audit;
     on stale: fail -> step FAILED DATA_PUBLISH_STALE | skip -> published=false;
  no new tenant_data_bytes charge: parts were charged at upload reservation (ADR-0027, D1278);
  update step, journal, next tasks/timers, outbox, completion receipt; the step's output document
     is a protected_system record (listed in data_protected_outputs, measured as zero);
COMMIT;
-- A refused completion (DATA_CONTRACT_VIOLATION, STORAGE_QUOTA_EXCEEDED, an audit-attempt failure),
-- FailStep, a lease expiry that grants a new generation and a cancellation each also claim the
-- attempt's UPLOADED DATA_SNAPSHOT_PART blobs of that fence for deletion (UPLOADED -> DELETING, retry
-- due now), after the run lock, in the same transaction (Decision 024-016).

-- API publish / rollback
BEGIN;
  receipt lookup (same key+fingerprint -> replay after current authorization);
  current membership guard (before any asset lock, D1273);
  lock asset head FOR NO KEY UPDATE; check ACTIVE, snapshot eligibility, expectedPointerVersion;
  write pointer, publication (justification class content, at most 1,024 bytes; a full content
     quota refuses QUOTA_EXCEEDED and rolls everything back), receipt, inbox event, deliveries, audit;
COMMIT;

-- Declared external write completion (database or icebergWrite step with assets.writes)
BEGIN;
  lock run; existing managed completion guards; check lease/generation/receipt;
  lock the external asset heads FOR NO KEY UPDATE ascending; recheck ACTIVE and that the node is
     the producer of the current revision (otherwise no observation, the refusal audited);
  insert data_asset_observations, update data_assets.last_observed_at, journal
     data.observation_recorded;
  update step, journal, next tasks, outbox, completion receipt;
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
| Read a `dataQuery` output | 016 payload disclosure at the highest pinned input level, a step source with no declared level counting as `pii` (`payload:read-internal`; the namespace `payload:reveal-pii` grant); a `dataQuery` cannot read a 017 database result, which stays behind `database:result:read` | per 016 |
| Read assets, revisions, snapshots, audits, publications, freshness | `namespace:read` | admin, developer, operator, viewer |
| Read lineage | `namespace:read` + `definition:read` | admin, developer, operator, viewer |
| Declare, revise, pause or delete an asset | `data:asset:manage` | admin |
| Publish a workflow version that reads or writes assets | `definition:publish` + `data:asset:use`; a write (managed transform output or external `assets.writes`) also needs the producer binding, and an external write the asset's connection | admin, developer |
| Publish or roll back through the API | `data:asset:publish` | admin, operator |
| Claim a remote database task, `ResolveDatabase` | `worker:execute` + `database:execute`, current binding of the driver's profile (Principle V's second set) | `database-runner` (profile `database`: mysql, mariadb, sqlserver, postgresql; profile `warehouse`: bigquery, snowflake) |
| Claim a data task, `ResolveData`, `ReadBlob` of pinned parts (`step_lease_blobs`), `WriteBlob` of parts | `worker:execute` + `data:execute`, current binding with profile `data` | `data-runner` (new machine role; no secret is ever disclosed to it) |

Generic `ReadBlob` of a `DATA_SNAPSHOT_PART` blob is refused unless the part belongs to a snapshot
pinned for the calling attempt. Foreign resources are opaque `NOT_FOUND`; denials are audited with
the existing bounded deduplication. Four permissions are added to the 55 of `develop` on 2026-11-26
(the total depends on the parallel drafts that also add permissions) and one machine role,
`data-runner`; tests assert the agreed sets of permissions and roles (021 adds `integration-runner`,
024 `data-runner`, 025 `agent-runner`), not a count.

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
| Container memory (includes the `/out` and `/spill` tmpfs) / CPUs | 3 GiB / 1 (class `small`) | 16 GiB / 4 (class `large`) | the data mode's class budgets (Architecture decision 0079 rule 5), memory cgroup |
| `/spill` tmpfs | 1 GiB | 4 GiB | `size=`, DuckDB `max_temp_directory_size` |
| `dataQuery` result (inline output) / rows / wall time | 256 KiB / 1,000 / 2 min | 256 KiB / 10,000 / 10 min | engine, container |
| Data step timeout (`timeoutMs`: staging, transform, upload, read-back, audit, report) | 15 min | 60 min | lease deadline |
| Transform wall time / audit wall time | 10 min / 2 min | the step timeout minus the audit time and the staging allowance / 10 min | container deadlines inside the lease |
| Transform attempts | 2 | 5 | retry policy |
| Attempts ended `DATA_RUNTIME_UNAVAILABLE` before any container started (not counted in the attempts above) | 10 per step | 10 per step | retry policy |
| Output document of a `dataTransform` or `dataPublish` step (protected) | — | 4 KiB | `data_protected_outputs` check |
| Concurrent data attempts namespace / tenant | 4 / 16 | 32 / 128 | `data_concurrency`, `tenant_data_concurrency` |
| Pending data part uploads per tenant | 128 | 512 | blob catalog |
| Retained published snapshots per asset | 5 | 50 | retention |
| Unpublished / quarantined retention | 7 days | 30 days | retention |
| Terminal-run pin grace | 24 h | 24 h | retention |
| `tenant_data_bytes` | 10 GiB | 2^50 bytes | ADR-0027 accounting, class `data` |
| Lineage response | 200 nodes, depth 4 | 500 nodes, 2,000 edges, depth 8 | API |
| Freshness `maxLagSeconds` | — | 60 s .. 31,536,000 s | domain |
| Publication justification (charged `content`, plain text without control characters) | — | 1,024 bytes | API and a column CHECK; content quota |

Effective = min(request, namespace, deployment). Zero refuses new admission and is never
unlimited. Lowering a ceiling never rewrites a retained revision, kills dispatched work or deletes
retained snapshots; it refuses new growth.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
| --- | --- | --- |
| Runner crash after uploading parts, before completion | Parts `UPLOADED`, no snapshot | Lease expiry → retry with new generation and same pins; the expiry transaction claims the old fence's parts for deletion, and the charge is released after confirmed deletion (fault; Task 024-078) |
| Retried failing transform | Each failed attempt's parts claimed for deletion in its failure transaction | Tenant data bytes and pending uploads back to baseline once deletions are confirmed, without waiting for the 24-hour time to live (integration; Task 024-078) |
| Crash after completion commit, before acknowledgement | Snapshot and pointer committed | Receipt replay returns the same outcome; no second snapshot (fault) |
| Duplicate `CompleteStep` / stale generation | — | Receipt replay / `STALE_ATTEMPT`; stale parts never committed (integration) |
| Lost broker hint for a data task | Task `READY` | Reconciler re-emits; runner polls (fault) |
| Run cancellation races an auto-publishing completion | First commit wins | Loser is an audited no-op; never half-published (race test with barrier) |
| Run deadline races an approval decision before `dataPublish` | First commit wins (014) | Audited no-op for the loser (race test) |
| API publication races a run publication | Asset row lock serializes | CAS loser `DATA_PUBLISH_STALE` / `CONFLICT` (race test) |
| Pointer moves after a step pinned its input | Step reads its pinned snapshot | Retention keeps it (integration) |
| Retention races a new pin | Pin takes asset head `FOR SHARE`; retention `FOR NO KEY UPDATE` and recheck | Pinned snapshot never deleted (race test) |
| Retention races an 018 reset that references the snapshot | Reset takes asset heads `FOR SHARE` in slot 4b and reads the snapshot state under that lock; retention `FOR NO KEY UPDATE` and rechecks `run_data_snapshot_refs` | Reset refused `SNAPSHOT_RETIRED`, or the snapshot kept (barrier race test, Task 024-068) |
| Approval longer than `retainUnpublishedDays` before `dataPublish`; a rollback during the approval | The run's staged snapshot stays `AUDITED` (live-run pin) | `dataPublish` publishes, or is stale and follows `onStale` after the rollback (race tests, Task 024-049) |
| Object store unavailable during staging | Attempt fails retryably | Nothing published (fault) |
| Object store unavailable while the engine reads footers | No state change | `CompleteStep` answers `UNAVAILABLE`; the same completion is sent again with the same key; never `DATA_CONTRACT_VIOLATION` (fault) |
| Sandbox reports success for a bad file | Engine refuses | `DATA_CONTRACT_VIOLATION` (security) |
| Audit attempt refused, unavailable, killed for memory, timed out or crashed, or a platform-generated check reports `ERROR` | No snapshot; the attempt's parts claimed for deletion | Step fails `DATA_RUNTIME_UNAVAILABLE`, retryable; never a quarantine (fault; Task 024-035) |
| A check the asset manager authored reports `ERROR` | Snapshot `QUARANTINED` | Step fails `DATA_AUDIT_FAILED`, not retryable (integration; Task 024-035) |
| A tenant file written to `/out` or `/spill` is executed | `EACCES` (`noexec`) | Step fails `DATA_TRANSFORM_FAILED`; mount-profile unit test and container canary (security) |
| `/out` filled past `size=` | `ENOSPC` in the container | `DATA_OUTPUT_LIMIT`; nothing committed (security) |
| Publication while content quota and `events_per_second` are exhausted | A run publication commits: the event and the step output documents are `protected_system` and not rate-charged. An API publication is refused: its justification is charged `content` | An auto-publishing completion and a `dataPublish` succeed; an API publication answers `QUOTA_EXCEEDED` with nothing written (integration at full quota) |
| An 018 reset point after a data node | Per the registry rules: pins and snapshot references rematerialized (a step-source pin names the source run's output, admitted through the reset link), publications and observations copied, never re-executed | Reset refused (`SNAPSHOT_RETIRED`) when a referenced snapshot is retired; a reset after a step-source pin and between a manual transform and `dataPublish` resolve the source run's records (integration with 018) |
| A declaration revision changes the producer of an external asset while its producer's write completes | Asset head lock serializes | First commit wins: the observation is recorded before the revision, or not at all after it (race test, Task 024-054) |
| Binary rollback to 0.2.0 after a new-driver connection exists | 0.2.0 store raises an internal error on the whole connection list | Prevented operationally: new drivers stay disabled until rollback is no longer planned; schema downgrade refuses (integration on a 0.2.0 binary) |
| `tenant_data_bytes` exhausted during part upload | Reservation refused; attempt fails `STORAGE_QUOTA_EXCEEDED` (ADR-0027 path) | The failure transaction claims the earlier parts for deletion; pointer unchanged (integration) |
| Quarantined snapshot routed by `onError` to a `from` input | Input refused | `DATA_INPUT_QUARANTINED`; nothing derived from it published (integration) |
| The data runner stages a blob other than the committed part for the audit (fault injection) | Engine refuses: the staged digests differ from the parts | `DATA_CONTRACT_VIOLATION`; nothing committed (fault) |
| Warehouse write, ambiguous response, lease held | Lookup decides | Known outcome reported; else `OUTCOME_UNKNOWN` (fake-server fault) |
| MySQL/MariaDB/SQL Server write, ambiguous response | `OUTCOME_UNKNOWN`, manual review | No automatic retry (real-container SIGKILL fault) |
| Failed compensation: the explicit compensating write on the error path fails | Run records both outcomes; review if unknown | Never assumed undone (fault) |
| Engine or runner restart mid-transform | Lease expiry, retry | Pins reused; old parts claimed for deletion by the expiry transaction (restart test) |
| Data-mode sandbox runner restarts mid-transform | Containers keep running to their label deadline; the new process rebuilds its slots and reservations from the labels | The data runner's `wait` answers `unavailable` and is asked again; nothing is committed twice (restart test against the engine double, Architecture decision 0079 rules 7 and 13) |
| Data mode refuses the attempt (ceiling, slot, class, deadline, directory) or is unavailable | No container | `DATA_RUNTIME_UNAVAILABLE`, retryable, not counted against `maxAttempts`, at most 10 such attempts per step (integration with the engine double) |
| Data runner dies with a container running | The restarted data runner ends, through `DELETE`, every attempt whose directory it finds before it offers activities; a container whose directory is gone ends at its label deadline | Lease expiry, retry with a new attempt id and the same pins; the slot is free again at the restart (fault) |
| Trigger dispatcher crashes after creating the run and before marking the publication event processed; an outbound webhook row is redelivered | Inbox key and the trigger's run key dedupe | One run per trigger per publication; the webhook receiver sees the delivery id again (fault) |
| OpenBao unavailable | Fail closed | `DATABASE_CREDENTIAL_UNAVAILABLE`, no fallback (integration) |
| Monitor lease lost mid-look | Alerts unchanged | Next look recomputes (integration) |

## Observability

**Spans** (attributes `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.node`,
`sagat.attempt`, `sagat.database.driver` or `sagat.data.engine`, `sagat.outcome`; never SQL, values,
locators or credentials): `database.execute` (existing, driver attribute added),
`database.warehouse.lookup`, `data.capability.resolve`, `data.stage`, `data.transform`, `data.audit`,
`data.footer.verify`, `data.snapshot.commit`, `data.publish`, `data.retention`, `data.freshness.look`.

**Metrics** (closed labels only): the existing runner metrics `database_runner_execution_total{outcome}`,
`database_runner_attempts_total{outcome}` and `database_runner_attempt_seconds` gain a closed `driver`
label (dashboards aggregate; no new name for the same count), and the data mode's sandbox metrics
mirror the lake mode's (`data_sandbox_starts_total`, `data_sandbox_kills_total`,
`data_sandbox_exits_total`, `data_sandbox_swept_total`, refusals by closed reason; Architecture
decision 0079 rule 12), then `database_connection_seconds{driver,outcome}`, `database_warehouse_lookups_total{driver,outcome}`
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
  three node kinds (`dataTransform`, `dataQuery`, `dataPublish`) and one optional `assets` property on
  `database` and `icebergWrite`, gated by the registry capability `data-pipelines-v1`; the 016 carrier
  set gains the `dataQuery` output. AsyncAPI grows (new message `DataAssetPublished`, alert kind
  values); baseline regenerated after reading the diff. Worker protocol: the next free minor of the
  register in `sdks/SPEC.md` section 9 when the contract merges (1.7 is feature 041's; Decision
  024-014), new RPC and optional fields only, `ResolvedDatabase` fields from 16 on,
  `WriteBlobHeader.purpose = 11` and `CompleteStepRequest.data_result = 15` (13 is feature 041's
  `step_logs`, 14 design 044's `accept_workflow_task`, Decision 044-256), each checked against the canonical file by a contract test (T024-008). The connection
  `config` union and `DatabaseDriverList` already exist (feature 035); they grow.
- **Database**: additive migrations, written once in the foundation slice after the latest migration
  of `develop` (`20270210000100` on 2026-11-26); downgrade refuses while assets, snapshots or
  new-driver connections exist (017 precedent) rather than deleting authority or evidence. A binary
  without these drivers (0.2.0, or a build before this feature) cannot decode a new-driver connection
  row (`StoreError::Internal`, the whole list fails), so the drivers' registry capabilities stay
  unrecorded until binary rollback to such a release is no longer planned, and the release notes say
  so; the instance registry refuses a process that lacks a recorded capability.
- **Rollout**: one registry capability at a time (Architecture decision 0082): `database-engines-v1`
  → `data-pipelines-v1` (data runtime, assets, transforms, lineage, freshness, external assets;
  Polars once design 046 is built) → `database-warehouses-v1` after the real-service suite. Removing a
  requirement stops new publication and admission; in-flight runs drain with compatible readers.
- **Rollback**: deactivating a driver's capability hides it and refuses new admission; existing
  revisions and pins stay readable. Deactivating data pipelines stops new transforms; pointers,
  snapshots and retention keep working so readers are not stranded.

## Delivery sequence

The order of work is the build slices of [tasks.md](tasks.md#build-slices): a foundation slice (types,
contracts, migrations, permissions, capabilities) by one builder, then two lanes in parallel:

1. Foundation: driver variants and managed kinds, data asset types, permissions and the role,
   registry capabilities, contracts and the protocol minor, the one migration set, error catalog,
   architecture rules, test targets (no behaviour change for PostgreSQL; compatibility proof).
2. Lane A (drivers): MySQL/MariaDB, then SQL Server; public-endpoint profile, BigQuery and Snowflake
   against fake servers; real-service gate (Owner decision 2026-11-26-161); driver Studio forms.
3. Lane B (data): data runtime image, data mode of the sandbox runner with its mount-profile test,
   the data runner; asset catalog, contracts, snapshots, transforms, engine verification, audits,
   auto publication; `dataQuery`; `dataPublish`, API publish/rollback, publication event and
   triggers, lineage, freshness, alerts; external assets; Polars once design 046 is built.
4. Six SDKs, conformance, runbooks, demo and release gates after both lanes.

## Verification plan

- **Unit**: domain validation of every driver config, contract compatibility pairs, freshness
  evaluation over DST and boundaries, snapshot transitions, retention selection, CAS rule.
- **Integration (real PostgreSQL, OpenBao, engines)**: catalog/admission/pins; each driver against
  its container; footer verification; publication transactions; retention; runtime-role sweep.
- **Contract**: OpenAPI/IR/proto/AsyncAPI compatibility baselines; error catalogue; fake-server
  protocol fixtures; six-SDK canonical documents; the statement-budget shapes
  (`integration_claim_transition_efficiency`).
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
| A data runner (`apps/data-runner`, role `data-runner`) and a data mode of the sandbox runner for DuckDB/Polars | Tenant computation must not share a process with other attempts' credentials, and the process that parses tenant output must not hold the host-root engine socket (Architecture decision 0079) | Running DuckDB inside the database runner exposes concurrent attempts' credentials to one engine bug or hostile query; letting the sandbox runner claim and start containers itself puts the engine socket in the process that reads tenant frames; a profile of the database runner would carry `database:execute` |
| Two attempts per transform (transform, audit) | Asset-owner audits must not be influenced by producer code | One container lets the transform tamper with its own audit |
| Engine-side Parquet footer verification | The contract must not rest on the sandbox's word | Trusting the report makes the contract advisory |
| New quota class `data` | Pipelines must not exhaust run payload capacity | Sharing `tenant_content_bytes` lets one large snapshot block run admission |
| Three new node kinds | Transform, query and publish are distinct durable steps (publication can wait on approval; a query writes nothing) | One node with an implicit publish cannot express audit-then-approve-then-publish; DuckDB as a 017 driver would change ADR-0036 decision 1 and the 017 result view |
| Data-profile mounts beyond ADR-0013 (the read-only attempt directory, sized `noexec` tmpfs `/out`, `/spill`, and for Polars the signed read-only layer) | Inputs and outputs of up to GiBs cannot travel on standard input and one stdout line; Polars needs its locked packages | ADR-0013's stdin/stdout-only profile cannot carry datasets; an unbounded writable disk is what ADR-0013 rejected, so the mounts are sized tmpfs charged to memory and `noexec`; the lake profile (Architecture decision 0071) is the precedent for a mounted attempt directory |
| A deployment of the database runner bound to a third profile, `warehouse`: same app, separate process, binding and NetworkPolicy | Principle V (isolation of tenant-authored effects): warehouse attempts send tenant SQL to public endpoints and must not share a pod, credentials in memory or network routes with attempts that reach private customer databases; feature 035's profile binding is the precedent | One deployment with both would let a compromised warehouse attempt reach private networks (ADR-0055 option 4); a new app would duplicate the managed protocol with no extra isolation |

## Risks

- Mediated bytes may make large snapshots slow (G1247): ceilings start low and are measured.
- Driver crates may log SQL or allocate unbounded frames (017 found both in `tokio-postgres`):
  inspected before adoption; findings can block a driver (D1252).
- Warehouse behaviour without emulators (G1241): the real-service gate is mandatory, and the
  accounts are the owner's to provide (Owner decision 2026-11-26-161).
- Polars depends on design 046 being built and on Gap 024-002 (the layer under the attempt split);
  User story 024-07 waits for both, and DuckDB does not.
- Architecture decision 0079 is Proposed and its Kubernetes half open (Gap 024-001); a change there
  changes the data mode with it.
