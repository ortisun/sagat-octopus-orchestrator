# Feature Specification: Data engines and pipelines

**Feature Branch**: `feature/024-data-engines-pipelines`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Add more database engines to the feature-017 database model (driver boundary in `crates/infra-database`, executor `apps/database-runner`): MySQL/MariaDB, Microsoft SQL Server, Google BigQuery, Snowflake and DuckDB (executor-local over object storage/Parquet), each with typed result-cell mapping, TLS, auth modes (BigQuery service account, Snowflake key pair) held in OpenBao, limits, probes and read-only versus execute. Add data pipelines in the sense of Windmill 2026: DuckDB/Polars transformation steps over namespace object storage, declared data assets (identity, producer, schema contract), a lineage view in the Studio, freshness status/SLAs, schema contracts and write-audit-publish (stage → validate → publish an atomic pointer).

## Context and delivery boundary

Feature 017 delivered reusable, revisioned PostgreSQL connections, OpenBao-held credentials, a
managed executor that takes leased tasks and receives only that attempt's capability, a shared
database operation contract in the five SDKs and the Studio, ad hoc execution and an audited result
view ([ADR-0035](../../docs/adr/ADR-0035-database-connections-and-managed-execution.md)). Its
driver catalog advertises PostgreSQL only and its out-of-scope list names "more database engines".
This feature fills that slot and adds a data-pipeline layer on top of it.

The two halves have different execution boundaries, and the spec keeps them apart:

- **Remote engines** (MySQL, MariaDB, SQL Server, BigQuery, Snowflake) follow the 017 model
  unchanged in kind: revision pinned at admission, credential resolved per attempt through OpenBao,
  execution only in the isolated database runner, typed bounded results, at-least-once writes.
- **Engine-local data execution** (DuckDB SQL and Polars) has no remote server: the computation
  itself is tenant-authored code. It therefore runs only in **data nodes** (`dataTransform`,
  `dataQuery`, `dataPublish`), in a **per-attempt sandbox container** of the feature-005 model,
  never in the control plane, the scheduler or the database runner, over **immutable snapshots** of
  namespace data assets staged by the platform. DuckDB is deliberately **not** a 017 database
  driver: that keeps ADR-0036 decision 1 (only a database runner claims managed database tasks) and
  the 017 result view untouched. Publication of a new
  snapshot is a compare-and-set of a pointer row in PostgreSQL. That is the write-audit-publish
  guarantee, and it is only as strong as that pointer: it applies to platform-catalogued assets,
  not to tables inside a customer warehouse ([ADR-0058](../../docs/adr/ADR-0058-sandboxed-data-transforms-and-pointer-publication.md), proposed).

Nothing in this document is implemented. Every success criterion is required future evidence.
Throughput and latency figures are benchmark criteria to verify on a stated environment.

## Dependencies and ownership

Reuses 001 identity/contracts, 002 durable execution and leases, 003 unknown outcomes and manual
review, 004 inbox and event triggers, 005 sandbox runner and script artifacts, 006 SDK conformance,
007 Studio, 008 telemetry and the alert monitor, 009 authorization/secrets/egress/quotas, 011
immutable publication, 014 root approval and additive IR, 016 classification vocabulary and 017
connections, OpenBao facade, managed executor and ad hoc admission. The blob claim-check store of
[ADR-0023](../../docs/adr/ADR-0023-blob-store-claim-check.md) holds snapshot bytes. ADR-0058
(proposed) supersedes in part ADR-0013 (data-profile mounts, `runner` permissions), ADR-0023 and
ADR-0027 (deletion of snapshot bytes, owner question O1243) and lists the dependents to update.

Parallel features, referenced not specified: **021** (ADR-0055) keeps database drivers in the 017
connection catalog, so these drivers extend that catalog; it resolves customer S3 credentials only
in `apps/integration-runner`; **018** (ADR-0050) owns the merged admission lock order, in whose
catalog slot (4) this feature's asset heads sit, and the reset-rule registry for the journal events
this feature adds; **031** owns the encoded-field registry that lists data-node parameters; **023** (ADR-0057) defines the reserved `python-data` runtime whose package catalog this
feature specifies; **020** (ADR-0054) owns run-purpose blob retention and excludes
`DATA_SNAPSHOT_PART` blobs from run purge; **018** schedules and backfill; **022** caching, test
mocks and standalone steps; **029** custom roles and folders; **030** cross-namespace access; **031**
payload codecs; **032** multi-region.

## User Scenarios & Testing *(mandatory)*

### US01 — Use MySQL, MariaDB and SQL Server like PostgreSQL (Priority: P1)

An administrator registers a MySQL, MariaDB or SQL Server connection with its credential in
OpenBao; an author uses it in a database step from the Studio or any SDK; an operator probes it and
runs ad hoc queries. Everything 017 promises for PostgreSQL holds for these engines.

**Why this priority**: These are the most common operational databases after PostgreSQL; they reuse
the whole 017 path and add no new trust boundary beyond a driver.

**Independent Test**: Against real disposable MySQL 8.4, MariaDB 11 and SQL Server 2022 containers,
the shared typed fixture runs a query and a write from all five SDKs and the Studio and produces the
same typed cells, errors and audit as PostgreSQL.

**Acceptance Scenarios**:

1. **Given** a namespace with OpenBao configured, **When** an administrator saves a `mysql`
   connection with host, port, database, verified TLS and a password secret, **Then** the revision
   stores only non-secret settings and the logical secret reference, and the probe reports a safe
   result for that revision.
2. **Given** a published workflow with a `query` step on a SQL Server connection, **When** it runs,
   **Then** the statement executes only in the database runner, inside a transaction the adapter
   always rolls back, and the result arrives as the 017 typed cells (`DATETIMEOFFSET` → timestamp,
   `DECIMAL` → decimal string).
3. **Given** a column of a type the mapping does not cover (MySQL `DATETIME`, SQL Server
   `UNIQUEIDENTIFIER`), **When** it is selected, **Then** the step fails with
   `DATABASE_TYPE_UNSUPPORTED` and no partial result.

**Failure scenario**: The runner loses its connection after sending an `UPDATE` to MariaDB. The step
ends `OUTCOME_UNKNOWN` and the run waits in manual review; nothing repeats the write. A server
certificate whose name does not match fails with `DATABASE_TLS_FAILED`; a login holding `SUPER` or
`sysadmin` is refused with `DATABASE_PRIVILEGED_ROLE` before any statement runs.

---

### US02 — Query and write BigQuery and Snowflake with key-based credentials (Priority: P1)

An administrator registers a BigQuery connection (project, location, service-account key) or a
Snowflake connection (account, warehouse, role, database, schema, key pair). Authors run bounded
queries and DML; the platform bounds cost and time and can look a write up after a lost response.

**Why this priority**: Warehouses are the main targets of data workflows; they require credential
shapes and an egress profile that 017 does not have, and their job/request identities make a write
reconcilable, which the other engines cannot offer.

**Independent Test**: A protocol-faithful fake server in CI exercises authentication, parameter
binding, pagination, cost refusal, cancellation and lookup; a real-service contract suite, run on
owner-provided test accounts (O1241), must pass before either driver is advertised.

**Acceptance Scenarios**:

1. **Given** a BigQuery connection whose `maximumBytesBilled` is 1 GiB, **When** a query would scan
   more, **Then** BigQuery refuses it without charge and the step fails with `DATABASE_COST_LIMIT`.
2. **Given** a Snowflake connection with a key-pair secret, **When** a step runs, **Then** the runner
   signs a JWT of at most one hour from the private key it received for that attempt, never
   persists it, and the statement carries the attempt's deterministic `requestId`.
3. **Given** a BigQuery `execute` step whose HTTP response is lost, **When** the runner still holds
   the lease, **Then** it looks the job up by its deterministic job ID and reports the real outcome;
   only an outcome the lookup cannot establish becomes `OUTCOME_UNKNOWN`.

**Failure scenario**: The service-account key in OpenBao names a `token_uri` other than the fixed
Google token endpoint; the codec rejects the credential (`DATABASE_CREDENTIAL_UNAVAILABLE`) and no
request leaves the runner. A Snowflake role in the refused administrative set fails the probe.

---

### US03 — Declare a data asset and produce it with a DuckDB transform under write-audit-publish (Priority: P1)

A data administrator declares the asset `orders-daily` with a schema contract, audits, a freshness
policy and the workflow node allowed to produce it. The producing workflow's `dataTransform` step
reads the current snapshots of its input assets, writes a new Parquet snapshot, the platform audits
it, and only an audited snapshot becomes current.

**Why this priority**: It is the core of the pipeline feature: identity, contract and a publication
that readers can never observe half-done.

**Independent Test**: Two concurrent readers and one producer run over a seeded asset; every reader
sees either the old or the new snapshot in full, a failing audit leaves the old snapshot current,
and a crash at each commit point leaves no half-published state.

**Acceptance Scenarios**:

1. **Given** `raw-orders` has a current snapshot, **When** the producer's transform succeeds and its
   audits pass, **Then** the completion transaction commits the snapshot, its provenance and the
   pointer move together, and a `data.asset.published` event is written in the same transaction.
2. **Given** an audit `unique(order_id)` that fails, **When** the transform completes, **Then** the
   snapshot is `QUARANTINED`, the pointer does not move, and the step fails with `DATA_AUDIT_FAILED`
   (routable by `onError`).
3. **Given** another workflow declares an output on `orders-daily`, **When** it is published,
   **Then** publication is refused (`ASSET_PRODUCER_MISMATCH`).
4. **Given** a MySQL query step `extract` earlier in the run, **When** a transform declares the input
   `{alias: "orders", step: "extract"}`, **Then** the transform reads the retained typed result as a
   table and the snapshot's provenance names that step as its source.

**Failure scenario**: The transform writes a Parquet file whose schema differs from the contract and
reports success. The engine reads the footer itself, refuses the output with
`DATA_CONTRACT_VIOLATION` and commits nothing; the uploaded parts are swept as unused uploads.

---

### US04 — Explore assets with DuckDB queries (Priority: P2)

An analyst queries current snapshots of declared assets with SQL through a `dataQuery` step, in a
workflow or as a standalone step (022), and reads the bounded result through the existing
classified payload disclosure (016).

**Why this priority**: Exploration and checks need SQL over assets; it reuses the data sandbox of
US03 and the 016 disclosure, and touches neither the 017 driver catalog nor its result view.

**Independent Test**: A `dataQuery` over two assets answers a join; a query that tries to read
`/etc/passwd`, install an extension or open a socket fails inside the sandbox.

**Acceptance Scenarios**:

1. **Given** `orders-daily` (a `pii` column) and `customers`, **When** a `dataQuery` joins them,
   **Then** the snapshots current at the first lease are pinned, the bounded typed result is the
   step's output, and its values are disclosed only to a caller holding the namespace
   `payload:reveal-pii` grant, the highest level among the pinned input columns.
2. **Given** a query whose result exceeds 256 KiB, **When** it completes, **Then** it fails
   `DATA_OUTPUT_LIMIT`; large results are written to an asset with a `dataTransform`.

**Failure scenario**: The query exhausts its memory limit; the container is killed and the step
fails with `DATA_RESOURCE_EXHAUSTED`. No other attempt or tenant shares that process.

---

### US05 — See lineage and freshness, and react to new snapshots (Priority: P2)

An operator opens the lineage view of an asset and sees its producer, its inputs, its consumers and
whether each asset is fresh. A stale asset raises an alert; a downstream workflow starts once for
each new snapshot of its upstream asset.

**Why this priority**: Observability of a pipeline is what distinguishes it from a set of jobs; it
builds on US03 data but is not required to produce assets.

**Independent Test**: A fixture of three assets and two producers yields the documented lineage
graph and table; a policy of 10 minutes turns an asset `STALE` on the database clock and the alert
fires and resolves; a duplicate delivery of the publication event starts exactly one run.

**Acceptance Scenarios**:

1. **Given** a policy `maxLagSeconds: 3600`, **When** no snapshot is published for an hour, **Then**
   the asset is `STALE`, `DATA_ASSET_STALE` fires for it and resolves after the next publication.
2. **Given** a workflow with an event trigger on `data.asset.published` from source `sagat.data`
   filtered on `event.subject == "orders-daily"`, **When** a snapshot is published, **Then** one run
   starts with the payload-free publication document as its event.

**Failure scenario**: The monitor loses its lease in the middle of a look; no alert is resolved by
that look, and the next monitor recomputes from the database clock.

---

### US06 — Publish by hand, gate publication with an approval and roll back (Priority: P2)

An author sets `publish: manual`, puts a 014 approval between the transform and a `dataPublish`
node, and an operator can also publish an audited snapshot or roll back to a retained earlier one
through the API, with a justification.

**Why this priority**: Write-audit-publish is only useful in practice if a person can hold or undo
a publication; it reuses US03's pointer.

**Independent Test**: An approval decision and the run deadline race on the same run; a manual
publish and a run publish race on the same asset; in each case exactly one pointer change commits
and the other is a typed, audited no-op.

**Acceptance Scenarios**:

1. **Given** an audited, unpublished snapshot, **When** an operator with `data:asset:publish`
   publishes it with `expectedPointerVersion`, **Then** the pointer moves, a publication record and
   an audit entry commit together, and a later `dataPublish` of the same snapshot is an idempotent
   success.
2. **Given** a quarantined snapshot, **When** anyone tries to publish it, **Then** the request fails
   with `CONFLICT` reason `SNAPSHOT_QUARANTINED`; quarantine is never released by hand.

**Failure scenario**: A rollback names a snapshot that retention already deleted; it fails with
`NOT_FOUND` and the pointer stays.

---

### US07 — Transform with Polars (Priority: P3)

A data engineer writes a transform in Python with Polars, published as a feature-005 script against
the curated data catalog, and uses it in a `dataTransform` step with the same snapshot, audit and
publication semantics as DuckDB.

**Why this priority**: Polars broadens the transform language but needs a curated, pinned Python
package catalog that the platform has to maintain (O1242).

**Independent Test**: A Polars script produces the same snapshot as the DuckDB fixture of US03 from
the same inputs; a script whose lock names a package outside the catalog does not build.

**Acceptance Scenarios**:

1. **Given** a script version built for the `python-data` runtime, **When** a transform step uses
   it, **Then** it runs in a container with no network, reads only the staged inputs and writes only
   its output directory.

**Failure scenario**: The script tries to install a package at run time; the container has no
network and no package index, and the step fails with `DATA_TRANSFORM_FAILED`.

---

### US08 — Track warehouse tables as external assets (Priority: P3)

An administrator declares a Snowflake table as an external asset; a database step that writes it
declares so; lineage and freshness then cover that table.

**Why this priority**: It completes the lineage picture for teams whose outputs live in a warehouse,
without promising write-audit-publish there.

**Independent Test**: A successful declared write records exactly one observation in its completion
transaction; an unknown-outcome write records none; freshness reflects only observations.

**Acceptance Scenarios**:

1. **Given** an external asset `wh-orders` on the `analytics-wh` connection, **When** a step with
   `assets.writes: [wh-orders]` succeeds, **Then** an observation (run, node, time) commits with the
   step, and freshness uses it.

**Failure scenario**: The write ends `OUTCOME_UNKNOWN`; no observation is recorded, and freshness is
not refreshed by a write nobody can confirm.

### Edge Cases

- A connection edit after run admission cannot redirect a retry; an asset declaration edit after
  admission cannot change the contract a running producer is checked against.
- An asset's pointer moves while a downstream step is waiting to start: the step pins whatever is
  current at its first lease; a retry of that step reads the same snapshot even if the pointer has
  moved again, and retention keeps it.
- Two runs of the same producer overlap (two schedule ticks): the second to publish finds the
  pointer changed since its first lease and fails `DATA_PUBLISH_STALE` or, with `onStale: skip`,
  completes without publishing. Nothing overwrites a newer snapshot with an older one.
- An empty result (zero rows) is a valid snapshot unless an audit sets a minimum row count.
- A transform that writes more than the snapshot byte ceiling is stopped by the output quota in the
  container and fails `DATA_OUTPUT_LIMIT`; the platform never truncates a snapshot silently.
- The object store is unavailable while parts are uploaded or footers are read: the attempt fails
  retryably before the completion transaction; nothing is published.
- A snapshot is pinned by a live run when retention looks at it: it is kept until 24 hours after
  that run ends.
- A tenant's data quota is exhausted while parts upload: the part reservation is refused (ADR-0027
  charges at reservation, not again at commit), the step ends with `STORAGE_QUOTA_EXCEEDED`, earlier
  parts are swept and the pointer stays.
- An asset is tombstoned while a producer run is in flight: the completion refuses publication
  (`DATA_ASSET_NOT_ACTIVE`) and the snapshot is abandoned.
- An encrypted field (031) is read by a database or data-node parameter expression: publication
  refuses it with 031's `VALIDATION_FAILED` reason `ENCODING`, issue `ENCODING_READ_BY_EXPRESSION`.
- Cancellation of a run whose transform already uploaded parts: the parts are never committed; a
  cancellation of a warehouse write is never treated as proof that nothing was written.

## Requirements *(mandatory)*

### Functional Requirements — database engines

The per-driver details these requirements refer to are normative in
[contracts/engines.md](contracts/engines.md).

- **FR-024-001 — Driver catalog and capability gate:** The driver catalog MUST grow from
  `postgresql` to the closed set `postgresql, mysql, mariadb, sqlserver, bigquery, snowflake`
  (DuckDB is a data node, not a driver), each advertised by `listDatabaseDrivers` only when the server enables it and its release
  gate passed. A catalog write, publication or admission naming a driver not enabled MUST fail with
  `VALIDATION_FAILED` reason `DRIVER_NOT_ENABLED` before any work, and a task of a driver MUST be
  offered only to an executor that advertises it.
- **FR-024-002 — Closed settings per driver:** Each driver MUST have a closed object of non-secret
  settings (host/port/database; BigQuery project and location; Snowflake account, warehouse, role,
  database and schema). URLs, DSNs, embedded credentials and unknown fields MUST be
  refused. 017 revisions, pinning, pause and tombstones apply unchanged; PostgreSQL keeps its shape.
- **FR-024-003 — Credentials and auth modes:** Values MUST be stored through the 017 OpenBao facade
  and resolved per attempt under the managed capability, in closed shapes: `{username,password}`
  (MySQL, MariaDB, SQL Server login), a Google service-account key (BigQuery), `{user,
  privateKeyPem, passphrase?}` (Snowflake key pair). Codecs MUST reject unknown
  fields and MUST NOT take endpoints, token URIs or audiences from secret content. Forms keep values
  write-only; rotating a stored value is not presented as rotating the account key.
- **FR-024-004 — Remote execution:** MySQL, MariaDB, SQL Server, BigQuery and Snowflake operations
  MUST execute only in `apps/database-runner` under the 017 lease, fencing, heartbeat, two-phase
  capability and receipt rules, as `sagat.database.<driver>`; the runner registers only enabled
  drivers. BigQuery and Snowflake MUST be served by a separate deployment of the runner (warehouse
  profile, own NetworkPolicy with public HTTPS egress and no route to private customer networks);
  MySQL, MariaDB, SQL Server and PostgreSQL by the private profile, which has no public egress. Both
  use the existing `database-runner` role. No API, scheduler or ordinary worker executes them.
- **FR-024-005 — Network scope:** MySQL, MariaDB and SQL Server MUST use the 017 operator CIDR/port
  policy narrowed by namespace, with address pinning and TLS name verification. BigQuery and
  Snowflake MUST use a public-endpoint profile: driver-fixed host form, deployment host allowlist
  that namespace policy can only narrow, every resolved address public unless inside an
  operator-listed private-endpoint network, reserved, metadata and platform addresses refused, no
  redirects or proxies.
- **FR-024-006 — Statement scope and query mode:** One statement in the driver's dialect and
  placeholder syntax; multi-statement text, DDL, transaction/session control and administrative
  commands MUST be refused. `query` mode MUST be enforced by an engine mechanism where one exists
  (read-only transaction for MySQL/MariaDB, an always-rolled-back transaction for SQL Server, a
  dry-run statement-type check for BigQuery once G1241 verifies it). Where none exists (Snowflake
  custom roles; BigQuery until then) the contract MUST say that the statement allowlist plus the
  customer's grants are the only control; the platform never presents that as enforcement.
- **FR-024-007 — Privileged identity refusal:** The probe and every attempt MUST refuse identities
  with the administrative privileges the contract lists per engine; where an engine cannot be
  introspected (BigQuery IAM) the requirement is documented as the customer's control, not claimed.
- **FR-024-008 — Typed parameters and cells:** Every engine MUST bind the 017 parameter types and
  return the 017 result representation; the per-engine mapping table is normative; other types,
  zone-less timestamps and non-finite floats MUST fail `DATABASE_TYPE_UNSUPPORTED`; sessions run in
  UTF-8 and UTC.
- **FR-024-009 — Warehouse cost and time:** BigQuery connections MUST carry a finite
  `maximumBytesBilled` (refusal → `DATABASE_COST_LIMIT`); warehouse drivers (BigQuery, Snowflake)
  MUST use their own deadline ceiling, enforced engine-side as well; labels and query tags
  MUST NOT carry tenant, run or user identifiers.
- **FR-024-010 — Warehouse identity and lookup:** Each BigQuery job and Snowflake statement MUST
  carry an identity derived from the attempt (job ID; `requestId`). After an ambiguous response the
  runner MUST look the work up within the same lease and report a known outcome; otherwise
  `OUTCOME_UNKNOWN` and manual review with the opaque identity. No later attempt resubmits a write.
- **FR-024-011 — Probes:** Each driver MUST have a fixed probe through the same capability, network
  and credential path, accepting no SQL and writing no business data; the Studio states when a probe
  consumes warehouse compute.
- **FR-024-012 — Cancellation:** Cancellation MUST use the engine's mechanism within a bounded
  cleanup time and MUST never be recorded as rollback of a dispatched write.
- **FR-024-013 — Output credential scanning:** Each codec MUST list its secret-bearing fields
  (passwords, private key material) and the 017 output guard MUST scan inline and blob results for
  them before retention, with the 017 query/execute outcomes on a hit.

### Functional Requirements — data assets

- **FR-024-014 — Asset catalog:** The platform MUST provide namespace-scoped data assets with stable
  identity, a slug name unique in the namespace, kind `managed` or `external`, status
  `ACTIVE|PAUSED|DELETED`, immutable declaration revisions with optimistic concurrency and request
  receipts, and tombstones that keep history. Names resolve only in the authenticated
  tenant/namespace; a foreign asset is an opaque `NOT_FOUND`. Pausing an asset MUST block new
  publications and new runs that write it, not reads of retained snapshots.
- **FR-024-015 — Schema contracts:** A managed asset MUST declare an ordered column list (closed
  logical types: boolean, int32, int64, decimal(p,s), float64, string, binary, date, timestamp in UTC
  microseconds, json-as-string), nullability, an optional key, and a compatibility mode `additive`
  (default: only new nullable columns) or `strict` (no change). A revision that breaks its mode MUST
  be refused (`ASSET_CONTRACT_INCOMPATIBLE`); a breaking change requires a new asset. Each run MUST
  pin the declaration revision it was admitted with.
- **FR-024-016 — Column classification:** Each column MUST carry an ADR-0033 level `public`, `internal` or
  `pii`; an undeclared level MUST be `pii`; `secret` MUST be refused because assets never hold
  secrets. The level is pinned with the revision. This feature adds no new value endpoint: asset
  values leave only as `dataQuery` outputs (FR-024-029), which are 016 classification carriers whose
  platform-derived schema carries the highest level among the pinned input columns, because no
  column-level taint is tracked (ADR-0036 decision 6); the existing 016 disclosure then applies its
  audience rules (`payload:read-internal`, the namespace `payload:reveal-pii` grant). Any later
  surface MUST apply the level likewise.
- **FR-024-017 — Producer binding:** A managed asset MUST name exactly one producer (workflow name
  and node id). Publication of a workflow version MUST refuse an output on an asset whose producer is
  another workflow or node (`ASSET_PRODUCER_MISMATCH`) and re-check the binding at admission and at
  publication of each snapshot. Changing the producer is a new revision, audited.
- **FR-024-018 — Snapshots:** A snapshot MUST be an immutable set of Parquet parts stored in the
  ADR-0023 blob store under the purpose `DATA_SNAPSHOT_PART`, with recorded size, SHA-256, row count
  and schema fingerprint per part, and states `AUDITED`, `QUARANTINED`, `PUBLISHED`, `ABANDONED`,
  `RETIRED`. A snapshot never changes after commit; the current snapshot is a separate pointer.
- **FR-024-019 — Input pinning:** The input snapshots of a data step MUST be resolved in the
  transaction that grants its first managed lease, recorded in a `data.inputs_pinned` journal event
  and reused by every retry. Every journal event type this feature adds declares its 018 reset rule
  (data model). An input may select the published snapshot of an asset (`current`), the snapshot staged by
  an earlier node of the same run (`from`), or, for ingestion, the committed output of an earlier
  step of the same run (`step`): a 017 database result, staged as a table with its mapped column
  types, or a step-output blob of type `application/vnd.apache.parquet`, verified by the engine like
  a snapshot part. A `from` input MUST resolve only an `AUDITED` or `PUBLISHED` snapshot; a
  quarantined one fails `DATA_INPUT_QUARANTINED`, so an `onError` route cannot launder it. A reader
  MUST never observe a partially written or unaudited snapshot.

### Functional Requirements — transforms and write-audit-publish

- **FR-024-020 — `dataTransform` node:** The IR MUST gain an additive node kind `dataTransform` with
  `engine: duckdb|polars`, inputs (alias, asset reference, selector), outputs (asset reference,
  statement or script, parameters, publish mode, stale policy), limits, `next`, `onError`, timeout and
  a bounded retry policy. Like 017 database nodes (ADR-0036 decision 3), data nodes MUST be accepted
  only in the root graph of a definition without collection, loop or continuation nodes until scoped
  pins are proven; the same holds for `dataQuery` and `dataPublish`. Publication MUST check the
  server's enabled capability. The activity prefix `sagat.data.` MUST be reserved like
  `sagat.database.`: no ordinary activity, compensation or reconciliation reference may use it.
- **FR-024-021 — Isolated engine-local execution:** DuckDB and Polars MUST run only in a container
  per attempt started by the sandbox runner, with the ADR-0013 profile (no network interface,
  read-only root, dropped capabilities, seccomp, unprivileged user, CPU/memory/PID/time limits, no
  credentials) changed only as ADR-0058 declares: inputs as a read-only `ro,noexec,nosuid,nodev` bind
  of a runner-staged directory whose bytes the runner bounds, and `/out` and `/spill` as tmpfs
  mounted `noexec,nosuid,nodev` with a `size=` limit charged to the container's memory cgroup. Since
  `/out` is discarded with the container, parts MUST leave through a bounded, framed output channel
  on standard output before it exits. A unit test over the container specification MUST assert that
  every writable mount is `noexec` and sized and every bind is read-only. DuckDB MUST additionally run with external access disabled except the input and
  output directories, extension install/autoload disabled, only extensions baked into the pinned
  image, and configuration locked; these settings are defence in depth, not the boundary. Inputs
  MUST be staged by the runner through lease-authorized reads of exactly the pinned parts.
- **FR-024-022 — DuckDB SQL transforms:** Each output MUST be produced by one `SELECT` statement over
  the input aliases, with typed parameters bound by position; the platform wrapper writes the result
  as Parquet parts within the part and snapshot ceilings. Statements that are not a single query
  MUST be refused at publication and again in the container.
- **FR-024-023 — Output verification by the engine:** Before committing a snapshot the engine MUST
  verify every part itself: exact upload fence, measured size and SHA-256, Parquet magic and footer
  within a bounded size, unencrypted footer, schema equal to the pinned contract and row counts. A
  mismatch MUST fail the attempt with `DATA_CONTRACT_VIOLATION` and commit nothing, whatever the
  sandbox reported.
- **FR-024-024 — Audits and quarantine:** Declared audits (row-count bounds, not-null from the
  contract, key uniqueness, accepted values, custom check statements authored on the asset) MUST run
  in a fresh container after the transform container has exited, over exactly the uploaded parts,
  read back from the platform because the transform's `/out` tmpfs is gone (the runner reports the SHA-256 of every part it audited and the engine checks them against the
  committed parts) and the pinned declaration. Results MUST be payload-free (check name, outcome,
  failing count).
  A failed or errored audit MUST commit the snapshot as `QUARANTINED`, keep the pointer, and fail the
  step with `DATA_AUDIT_FAILED`.
- **FR-024-025 — Publication:** With `publish: auto`, the pointer compare-and-set MUST commit in the
  completion transaction of an audited snapshot. With `publish: manual`, the snapshot stays `AUDITED`
  until a `dataPublish` node (an engine node that needs no worker) or the API publishes it. The
  compare-and-set MUST be against the pointer version captured at the producer's first lease;
  `onStale: fail` (default) fails with `DATA_PUBLISH_STALE`, `onStale: skip` completes without
  publishing. A quarantined snapshot MUST never be published; publishing the snapshot that is already
  current MUST be an idempotent success.
- **FR-024-026 — Manual publication and rollback API:** Holders of `data:asset:publish` MUST be able
  to publish an audited snapshot or roll back to a retained previously published snapshot, with an
  `Idempotency-Key`, `expectedPointerVersion` and a justification, in one transaction with the
  publication record and audit. Replay MUST recheck current authorization. Rollback creates a new
  publication record; it deletes nothing.
- **FR-024-027 — Publication event and triggers:** Every publication MUST write, in its transaction,
  a payload-free inbox event (source `sagat.data`, type `data.asset.published`, subject the asset
  name, event id `<asset id>:<pointer version>`, so a rollback or republication of an earlier
  snapshot is a new event) and the outbound delivery rows for webhook subscriptions that ask for
  `data.asset.published`. The event document MUST be a closed, bounded (≤ 1 KiB) `protected_system`
  record under ADR-0027: it is not charged to content quota and does not spend the
  `events_per_second` bucket, so a full quota can never roll back a completion or a publication.
  Existing 004 event triggers MUST be able to start runs from it (their run admission keeps its own
  quotas); the inbox key deduplicates redelivery of the same publication.
- **FR-024-028 — Polars transforms:** A Polars transform MUST reference a published script version
  of the reserved image-catalog runtime `python-data` (FR-023-027, 023 D1229): a bundle v4 whose
  `runtimeCatalog` equals the configured runtime image's catalog digest, with a hash-pinned
  requirements lock matching catalog entries exactly. 023 owns that mechanism; this feature owns the
  runtime image, its catalog (023's `catalog.schema.json`, upstream `pypi`: Polars and PyArrow with
  their reviewed transitive closure), the execution environment, their evidence and the re-pin
  procedure. It MUST use the same container, staging, verification, audit and publication contract
  as DuckDB and MUST NOT install packages.

### Functional Requirements — DuckDB queries, lineage, freshness, external assets

- **FR-024-029 — DuckDB queries:** The IR MUST gain an additive data node `dataQuery` (root-only,
  like `dataTransform`) with inputs (alias, asset or step source), one `SELECT` statement, typed
  parameters and limits. It runs in the data sandbox as the reserved activity `sagat.data.query`,
  pins its inputs at first lease, writes nothing, and returns the 017 typed-cell result shape as its
  inline step output (≤ 256 KiB; above it `DATA_OUTPUT_LIMIT`). That output MUST be a 016
  classification carrier whose platform-derived schema is annotated with the highest level among
  the pinned input columns, disclosed only through the existing 016 payload disclosure. DuckDB MUST
  NOT be a 017 database driver, and the 017 result view MUST NOT serve these outputs. Ad hoc
  queries use 022 standalone steps when they exist.
- **FR-024-030 — Lineage:** The platform MUST serve a bounded lineage graph per namespace combining
  declared edges of active workflow versions (node reads/writes asset) and recorded snapshot
  provenance (snapshot produced by run/node from input snapshots). It MUST carry no values, MUST be
  bounded in nodes, edges and depth, and MUST show a producer the caller may not read as opaque.
- **FR-024-031 — Freshness:** An asset MAY declare `maxLagSeconds` and an optional cron deadline
  (five fields, IANA zone, grace). Status `FRESH|STALE|NEVER_PUBLISHED|NOT_TRACKED` MUST be derived
  from the database clock and the last publication or observation, served by the API and shown in
  the Studio. The 008 monitor MUST raise `DATA_ASSET_STALE` and `DATA_SNAPSHOT_QUARANTINED` alerts
  (subject: asset name) with the existing fire/resolve semantics. No automatic re-run is triggered.
- **FR-024-032 — External assets:** An external asset MUST name a connection, an opaque locator and
  one producer (workflow and node). A database step MAY declare `assets.reads` and `assets.writes`;
  a declared write MUST be refused at publication unless the step is the asset's producer and uses
  the asset's connection. A successful declared write MUST record an observation in its completion
  transaction; an unknown or failed outcome MUST record none. External assets have no snapshots, pointer, audits or write-audit-publish; lineage
  is declared, never inferred from SQL text.

### Functional Requirements — cross-cutting

- **FR-024-033 — Authorization:** New permissions `data:asset:manage` (admin), `data:asset:use`
  (admin, developer), `data:asset:publish` (admin, operator) and the machine permission
  `data:execute` (granted to the existing `runner` role, superseding in part ADR-0013's exact
  permission set for that role) MUST be added. This feature adds no built-in role; tests MUST assert
  the set of built-in roles agreed across features (021 adds `integration-runner`, 025
  `agent-runner`), never a count. Asset metadata,
  snapshots and freshness need `namespace:read`; lineage also needs `definition:read`. Authority MUST
  be rechecked at workflow publication, run admission, lease, capability resolution, completion,
  snapshot publication and receipt replay. The exact matrix is in the plan.
- **FR-024-034 — Retention:** Per asset, the current snapshot plus the declared number of previous
  published snapshots, every snapshot pinned by a live run, and pins of runs that ended less than 24
  hours ago MUST be kept. Quarantined and abandoned snapshots MUST be kept for their declared period.
  Deletion MUST mark the snapshot `RETIRED` and its parts `DELETING` in one audited transaction
  before provider deletion, with ADR-0023's durable retry; bytes are released only after confirmed
  deletion. Destroying customer data this way supersedes in part ADR-0023 and ADR-0027 and needs the
  owner's decision (O1243); until then retention marks snapshots `RETIRED` and keeps their bytes.
  Run purge (020, ADR-0054) MUST NOT delete `DATA_SNAPSHOT_PART` blobs or snapshot metadata
  (snapshots, parts, provenance, audits, publications); this feature owns their retention. Data that left through queries, events or exports is outside platform deletion.
- **FR-024-035 — Quotas and finite ceilings:** Every new resource MUST have a default and a hard
  ceiling (plan table); the effective value is the minimum of request, namespace and deployment;
  zero refuses new admission and never means unlimited; admission fails before work. Snapshot bytes
  MUST count against a new tenant aggregate `tenant_data_bytes`, separate from
  `tenant_content_bytes`.
- **FR-024-036 — Observability and audit:** The feature MUST emit OTel spans and metrics with closed
  labels only (`driver`, `engine`, `outcome`, `origin`) and redacted logs. SQL text, parameters,
  result cells, asset content, credentials, provider bodies and locators MUST NOT appear in logs,
  spans, metrics or audit. Asset changes, publications, rollbacks, quarantines, snapshot reads by an
  attempt, retirements and denials MUST be audited in the deciding transaction.
- **FR-024-037 — Typed errors:** REST errors MUST reuse the closed `Error.code` set with new
  `details.reason` values; step failures MUST use closed codes (new database codes and a closed data
  failure set) with no driver or provider message. The lists are in [contracts/README.md](contracts/README.md).
- **FR-024-038 — Contracts, SDKs and compatibility:** OpenAPI, IR schema, worker protocol and
  AsyncAPI changes MUST be additive; new worker messages MUST be negotiated by protocol minor and
  never offered to an older runner. The connection `config` becoming a `oneOf` MUST be tested with
  an older generated client reading a list that contains a MySQL connection. A 0.2.0 server cannot
  decode a new-driver row (it raises an internal store error and fails the whole connection list),
  so new drivers MUST stay disabled until binary rollback to 0.2.0 is no longer planned, and the
  release notes MUST say so. The five SDK builders MUST emit the new node kinds and driver
  references identically, and shared conformance scenarios MUST cover them.
- **FR-024-039 — Studio:** The Studio MUST offer per-driver connection forms with write-only
  credential fields, an asset catalog (contract, revisions, snapshots, audits, freshness,
  publications), a lineage graph with an equivalent table, publish/rollback actions gated by
  permission, editors for the new nodes and their runtime graph, using design tokens, WCAG AA and
  never drawing sample values (FR-007-010).
- **FR-024-040 — Delivery:** Ship pinned images for the test engines and the data runtime, the
  fake warehouse protocol servers, development and production configuration, runbooks (engines,
  data runner, assets and retention) and a reproducible happy/failure demo.
- **FR-024-041 — Interplay with other features:** Data-node parameter expressions
  (`dataTransform.outputs[].parameters`, `dataTransform.scriptInput`, `dataQuery.parameters`) are
  registered positions in 031's encoded-field registry; reading an encoded field is refused with
  031's codes (`ENCODING_READ_BY_EXPRESSION`), never a code of this feature. In 022 draft test runs a
  data node MUST be pinned or mocked or the test run is refused (`TEST_RUN_UNMOCKED_EFFECT`, 022); a
  mocked data node stages no snapshot and publishes nothing; data steps are not cached by 022; asset content is not an analytics sink of 020; the 016 carrier set
  gains the `dataQuery` output (FR-024-029), and the `dataTransform` and `dataPublish` outputs are
  platform metadata disclosed as structure only (ADR-0028).

### Amendment to feature 016 (coordinator decision C6)

This feature adds exactly one carrier to FR-016-002: the output of a `dataQuery` node, whose schema
is platform-derived (the 017 typed-cell result shape) and annotated at the highest
`x-sagat-classification` level among the pinned input columns. `dataTransform` and `dataPublish`
outputs are not carriers (structure only). The single consolidated amendment of FR-016-002 is opened
by the first of the parity features to be implemented (T024-069).

### Non-functional requirements

- **NFR-024-001 (security):** Zero tested cross-tenant/namespace access to connections, assets,
  snapshots, lineage or publications; zero credential, private-key or asset-content canary leakage;
  sandbox-escape canaries (file read outside the mounts, extension install, network, process limits)
  refused. Any failure blocks release.
- **NFR-024-002 (durability):** A committed publication is never lost or half-applied; readers see
  whole snapshots; external writes stay at-least-once with lookup or manual review; no exactly-once
  external effect is claimed.
- **NFR-024-003 (compatibility):** Existing PostgreSQL connections, workflows, protocol 1.6 runners
  and SDK builds keep working unchanged; new contract fields are optional or new.
- **NFR-024-004 (observability):** Every accepted operation has correlated spans and audit; metric
  label sets are closed; export failure never decides a commit.
- **NFR-024-005 (performance):** Measure, on a declared environment, admission, connection,
  execution and lookup latency per driver, staging and footer-verification throughput, transform
  container start-up and publication commit latency. No throughput or SLO is promised.
- **NFR-024-006 (accessibility):** The new Studio views meet WCAG 2.2 AA and offer a table
  alternative to every graph.

### Key Entities

- **Driver**: closed engine identity with its operations, auth modes, network profile and limits.
- **Connection revision** (017): non-secret settings per driver plus a logical secret reference.
- **Data asset**: namespace identity, kind, status, current revision, pointer and pointer version.
- **Asset declaration revision**: immutable contract, classification, producer, audits, freshness,
  retention (managed) or connection and locator (external).
- **Snapshot / part**: immutable Parquet parts with sizes, digests, row counts, schema fingerprint,
  provenance and audit outcomes.
- **Publication**: an immutable record of a pointer change (origin run, API or rollback).
- **Input pin / output base**: per step, the snapshots read and the pointer version written against.
- **Observation**: a confirmed write of an external asset by a run.

## Success Criteria *(mandatory)*

All evidence below is required future evidence, not completed tests.

| Criterion | Measurable outcome / required automated proof | Requirements |
|---|---|---|
| SC-024-001 | Against real MySQL 8.4, MariaDB 11 and SQL Server 2022 containers, all five SDKs and a Studio-authored workflow run the shared query/write fixture with byte-identical typed results and errors; wrong host name, untrusted CA, privileged login, denied destination and ambiguous write each produce their typed outcome | FR-024-001..008, 011..013 |
| SC-024-002 | The fake BigQuery/Snowflake servers prove authentication, binding, pagination, cost refusal, cancellation and lookup in CI; a network-policy test shows the warehouse runner cannot open a socket to a private customer network and the private runner cannot reach a public endpoint; the real-service suite passes once on owner-provided accounts before either driver is advertised, and its date and scope are recorded | FR-024-003..005, 009..011 |
| SC-024-003 | Every row of the per-engine type table either maps to its cell type or fails `DATABASE_TYPE_UNSUPPORTED`, tested on each engine; zero partial results | FR-024-008 |
| SC-024-004 | DuckDB and Polars canaries (read outside mounts, extension install, socket, executing a file written to `/out` or `/spill`, filling them past `size=`, fork bomb, memory exhaustion) fail inside the container in 100% of runs; the mount-profile unit test asserts every writable mount `noexec` and sized; no DuckDB or Polars code is linked into or executed by the control plane, engine, scheduler or database runner (architecture check) | FR-024-021, 022, 028, 029 |
| SC-024-005 | Under barrier-controlled concurrency, readers observe only whole published snapshots in 100% of rounds; stale compare-and-set loses with a typed error; a quarantined snapshot is never current | FR-024-018, 019, 023, 025, 026 |
| SC-024-006 | Crash injection at every commit point of staging, completion, publication, rollback and retention leaves either the old or the new state, no orphan pointer, no double publication, and recovers after restart; an 018 reset after each data journal event type follows its declared registry rule and never re-publishes | FR-024-019, 023..027, 034 |
| SC-024-007 | A sandbox that reports success for a contract-violating file is refused by the engine's own footer check; contract compatibility modes accept and refuse the documented revision pairs | FR-024-015..017, 023, 024 |
| SC-024-008 | The lineage fixture matches its expected graph and table; freshness flips on the database clock and the alert fires and resolves; duplicate deliveries of a publication event start exactly one run per trigger per publication, and a rollback is a new event; external observations follow outcomes and only the producer can record them | FR-024-027, 030..032 |
| SC-024-009 | The tenant battery covers every new REST operation and worker RPC: another tenant's or namespace's resource is indistinguishable from a missing one and the probe is audited; a `dataQuery` output over `pii` columns is disclosed only with the reveal grant, and the 017 result view never serves it | FR-024-014, 016, 029, 033 |
| SC-024-010 | Exhausting each ceiling of the plan table fails with its typed error before any work, including BigQuery bytes billed and `tenant_data_bytes` at part upload; a publication and its event commit while content quota and `events_per_second` are exhausted | FR-024-009, 027, 035 |
| SC-024-011 | Real OTLP export, logs and audit of every scenario contain no SQL, parameter, cell, asset-content, credential or locator canary; every metric label is in its closed set; every error is a catalogued code or reason | FR-024-013, 036, 037 |
| SC-024-012 | The new shared conformance scenarios pass in TypeScript, Go, Python, C# and Java with identical canonical documents and outcomes | FR-024-020, 028, 038 |
| SC-024-013 | Real-browser tests cover each driver form, the asset catalog, lineage graph and table, freshness and publish/rollback, with no accessibility violation of WCAG 2.2 AA in automated checks | FR-024-039, NFR-024-006 |
| SC-024-014 | Retention keeps every pinned and in-window snapshot and deletes the rest only after commit; a crash during deletion retries without losing a retained snapshot; a quarantined snapshot cannot reach a reader through `from` | FR-024-014, 019, 034 |
| SC-024-015 | Existing 017 suites pass unchanged; an older generated client reads a connection list containing a MySQL connection; a protocol 1.6 runner never receives a new task kind; encrypted-field, cache and mock combinations are refused at publication | FR-024-038, 041, NFR-024-003 |
| SC-024-016 | The quickstart demo runs from a clean checkout with the documented commands, showing the happy path and the audit-failure path, and its runbooks are linked from the operator index | FR-024-040 |

## Assumptions

- PostgreSQL remains the authority; snapshot bytes live in the ADR-0023 blob backend (O1240).
- Test engines run as disposable containers (rule 16). BigQuery and Snowflake have no free local
  emulator the project can depend on; CI uses fake protocol servers and the real-service suite needs
  accounts the owner provides (O1241).
- Feature 005's sandbox runner and its runtime (G98 accepted trade-off) is the isolation used for
  DuckDB and Polars, with the data-profile mounts ADR-0058 declares; no stronger runtime (gVisor,
  Kata) is assumed.
- Snapshot bytes travel through the engine process as ADR-0023 requires; that bounds the ceilings
  and the throughput (G1247).
- Polars runs on the image-catalog runtime `python-data` that 023 reserves (FR-023-027); this feature
  owns its image and package catalog (O1242).
- Partitioned assets, incremental append and range backfill are not in the first delivery (G1245).

## Out of scope

DuckDB as a 017 database driver (a data node instead); automatic materialization or re-run on
staleness; partitioned assets and backfill; customer-owned buckets as snapshot storage and DuckDB
reads of external object storage (customer S3 credentials resolve only in 021's integration runner,
G1246); warehouse
native write-audit-publish (table swap or clone) and warehouse schema verification (G1244); SQL
Server Entra ID and Windows authentication (G1243); nested Parquet types; Iceberg, Delta and DuckLake
formats; value previews of assets other than `dataQuery` outputs; cross-namespace asset reads (030);
asset declarations in 011 GitOps manifests (G1248); automatic lookup for warehouse writes whose
attempt was lost (G1240); pruning of retired snapshot metadata (G1254; snapshot metadata is excluded
from 020's run purge and owned by ADR-0058);
data steps inside collections, loops, continuation or saga bodies.

## Owner questions

- **O1240 — Where do managed snapshots live?** Recommended default: in the platform's existing blob
  backend under the namespace, counted in the new `tenant_data_bytes` quota. A customer-owned bucket
  is a later extension: under ADR-0055 its credentials resolve only in `apps/integration-runner`, so
  it would need that runner to stage objects for the data sandbox (G1246).
- **O1241 — How are BigQuery and Snowflake released?** Recommended default: the drivers stay hidden
  (`DRIVER_NOT_ENABLED`) until the real-service contract suite passes on test accounts the owner
  provides; CI keeps the fake servers.
- **O1242 — Does the project maintain a curated Python data catalog for Polars?** Recommended
  default: yes, one hash-pinned catalog (Polars, PyArrow) re-pinned on a stated cadence with
  vulnerability evidence, through 023's `python-data` mechanism; if not accepted, US07 is dropped
  and DuckDB remains the only transform engine.
- **O1243 — May the platform destroy superseded snapshot data?** ADR-0027 does not authorize deleting
  customer data and ADR-0023 never deletes committed blobs. Recommended default: yes, but only under
  the retention an asset manager declares on the asset (current plus `retainPublished`, live pins,
  the unpublished window), audited, with the runbook stating that copies outside the platform are
  not reached; until the owner decides, retention marks snapshots `RETIRED` and keeps their bytes.
  (This is a fourth owner question; the coordinator asked for it explicitly.)

See [research](research.md) for decisions D1240–D1279 and gaps G1240–G1256, the
[plan](plan.md), the [data model](data-model.md), the [contracts](contracts/README.md), the
[quickstart](quickstart.md), the [tasks](tasks.md) and the [requirements checklist](checklists/requirements.md).
