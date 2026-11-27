# Feature Specification: Data engines and pipelines

**Feature Branch**: `feature/024-data-engines-pipelines`

**Created**: 2026-10-26

**Status**: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision 2026-11-24-1); Architecture decision 0058 accepted (delegate, Owner decision 2026-11-26-164); not started

**Revised**: 2026-11-26 against `develop` at `e5bda041` after an independent audit: the data containers are requested through the sandbox runner's attempt API of Architecture decision 0079 by a data runner of their own; feature 035's lake path, managed kinds, binding profiles, egress ceilings and blob allow-list are reused; six SDKs; the next free worker-protocol minor; instance-registry capabilities; Owner questions 1240 to 1243 decided (Owner decisions 2026-11-26-160 to 2026-11-26-163). What changed is listed in [research](research.md) (Decisions 024-001 to 024-015). Revised again on
2026-11-27 after the correctness review (`reviews/2026-11-27-correctness.md`): every finding and what
changed is Decision 024-016.

**Input**: Add more database engines to the feature-017 database model (driver boundary in `crates/infra-database`, executor `apps/database-runner`): MySQL/MariaDB, Microsoft SQL Server, Google BigQuery, Snowflake and DuckDB (executor-local over object storage/Parquet), each with typed result-cell mapping, TLS, auth modes (BigQuery service account, Snowflake key pair) held in OpenBao, limits, probes and read-only versus execute. Add data pipelines in the sense of Windmill 2026: DuckDB/Polars transformation steps over namespace object storage, declared data assets (identity, producer, schema contract), a lineage view in the Studio, freshness status/SLAs, schema contracts and write-audit-publish (stage → validate → publish an atomic pointer).

## Context and delivery boundary

Feature 017 delivered reusable, revisioned PostgreSQL connections, OpenBao-held credentials, a
managed executor that takes leased tasks and receives only that attempt's capability, a shared
database operation contract in the SDKs and the Studio, ad hoc execution and an audited result
view ([ADR-0035](../../docs/adr/ADR-0035-database-connections-and-managed-execution.md)). Its
driver catalog advertises PostgreSQL only and its out-of-scope list names "more database engines".
This feature fills that slot and adds a data-pipeline layer on top of it.

The two halves have different execution boundaries, and the spec keeps them apart:

- **Remote engines** (MySQL, MariaDB, SQL Server, BigQuery, Snowflake) follow the 017 model
  unchanged in kind: revision pinned at admission, credential resolved per attempt through OpenBao,
  execution only in the isolated database runner, typed bounded results, at-least-once writes.
- **Engine-local data execution** (DuckDB SQL and Polars) has no remote server: the computation
  itself is tenant-authored code. It therefore runs only in **data nodes** (`dataTransform`,
  `dataQuery`; `dataPublish` is an engine node), in a **per-attempt sandbox container** that a
  data-mode sandbox runner starts when the **data runner** asks it for an attempt (Architecture
  decision 0079, the path feature 035's lake runner already uses), never in the control plane, the
  scheduler, the database runner or the data runner itself, over **immutable snapshots** of namespace
  data assets staged by the platform. The data runner and its containers receive no credential. DuckDB is deliberately **not** a 017 database
  driver: that keeps ADR-0036 decision 1 (only a database runner claims managed database tasks) and
  the 017 result view untouched. Publication of a new
  snapshot is a compare-and-set of a pointer row in PostgreSQL. That is the write-audit-publish
  guarantee, and it is only as strong as that pointer: it applies to platform-catalogued assets,
  not to tables inside a customer warehouse or a customer Iceberg table ([ADR-0058](../../docs/adr/ADR-0058-sandboxed-data-transforms-and-pointer-publication.md), accepted by Owner decision 2026-11-26-164).

Under constitution Principle V the remote engines are feature 017 managed database capabilities:
their credential is disclosed only to the lease-owning `database-runner` with `database:execute`,
the second of the two declared sets; the data runner and its containers receive none. No constitution
amendment is needed. Under Principle IV data nodes are a graph-model feature: a procedural workflow
reaches them only through a child graph workflow its envelope declares (design 044 is not yet
written, Gap 024-003). The SDK contract is the six official SDKs' (TypeScript, Go, Python, C#, Java,
Rust).

Nothing in this document is implemented. Every success criterion is required future evidence.
Throughput and latency figures are benchmark criteria to verify on a stated environment.

## Dependencies and ownership

Reuses 001 identity/contracts, 002 durable execution and leases, 003 unknown outcomes and manual
review, 004 inbox and event triggers, 005 sandbox runner and script artifacts, 006 SDK conformance,
007 Studio, 008 telemetry and the alert monitor, 009 authorization/secrets/egress/quotas, 011
immutable publication, 014 root approval and additive IR, 016 classification vocabulary and 017
connections, OpenBao facade (its KV custody for database credentials, which design 015 keeps),
managed executor and ad hoc admission. The blob claim-check store of
[ADR-0023](../../docs/adr/ADR-0023-blob-store-claim-check.md) holds snapshot bytes. ADR-0058
(accepted) refines ADR-0013 (the data-profile mounts, as Architecture decision 0071 did for the lake
profile), supersedes in part ADR-0023 and ADR-0027 (deletion of snapshot bytes under declared
retention, Owner decision 2026-11-26-163) and lists the dependents to update.

Merged work this feature reuses rather than re-invents: **035** (Architecture decisions 0071 and 0079)
for the attempt API of a mode of the sandbox runner, the attempt-directory staging, the peer-checked
socket, the table of endings, the lease allow-list of blobs (`step_lease_blobs`), `ManagedKind` and
`BindingProfile`, the per-connection `EgressCeiling` and the recovery-rule vocabulary; **018**'s
instance registry (Architecture decision 0082) for activation and its reset-rule registry
(`crates/domain/src/execution/reset.rs`) for the journal events this feature adds; **038**'s statement
budget of a lease and a completion; **041**'s rule that platform-run steps are refused step logs;
**043**'s run-input gate (Architecture decisions 0090 and 0092), which every run this feature starts
already passes; and design **046**'s package layer for Polars (Owner decision 2026-11-26-162).

Parallel features, referenced not specified: **021** (ADR-0055) keeps database drivers in the 017
connection catalog, so these drivers extend that catalog; it resolves customer S3 credentials only
in `apps/integration-runner`; **018** (ADR-0050) owns the merged admission lock order, in whose
slot 4b this feature's asset heads sit (and in whose slot 4a its data-catalog fence sits), and the
reset-rule registry for the journal events this feature adds; **031** owns the encoded-field registry
that lists data-node parameters; **023** (ADR-0057) reserves the language id `python-data`, which this
feature enables on design 046's package layer, so the image-catalog parts of Requirement 023-027 are
not needed; **020** (ADR-0054) owns run-purpose blob retention, decided the uniform `pii`-egress rule
(Owner question 1081, Decision 1617) and excludes
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
the shared typed fixture runs a query and a write from all six SDKs and the Studio and produces the
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
owner-provided test accounts, must pass before either driver is advertised (Owner decision
2026-11-26-161; agents create no account and spend no money).

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
`DATA_CONTRACT_VIOLATION` and commits no snapshot; the transaction that records the failure claims
the attempt's uploaded parts for deletion, and their charge is released once the deletion is
confirmed.

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
2. **Given** a MySQL query step `extract` and a `dataQuery` with the input
   `{alias: "x", step: "extract"}`, **When** the workflow is published, **Then** publication is
   refused (`VALIDATION_FAILED`, reason `DATA_NODE_SCOPE`): a 017 database result stays readable only
   through the 017 result view with `database:result:read`. **Given** instead an activity step
   `fetch` whose Parquet output declares no level, **When** a `dataQuery` reads it as a step source,
   **Then** its output counts as `pii` and only a caller holding the namespace reveal grant sees
   its values.
3. **Given** a query whose result exceeds 256 KiB, **When** it completes, **Then** it fails
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
fires and resolves; a trigger dispatcher that crashes after creating the run and before marking the
publication event processed, and a redelivered webhook row, still start exactly one run.

**Acceptance Scenarios**:

1. **Given** a policy `maxLagSeconds: 3600`, **When** the current snapshot was created more than an
   hour ago, **Then** the asset is `STALE`, `DATA_ASSET_STALE` fires for it and resolves after a
   snapshot created within the hour is published. A rollback to an older snapshot, or a late
   publication of an old one, does not make the asset fresh.
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

**Failure scenario**: A rollback names a snapshot that retention already retired; its metadata row
survives, so it fails with `CONFLICT` reason `SNAPSHOT_NOT_PUBLISHABLE` and the pointer stays.

---

### US07 — Transform with Polars (Priority: P3)

A data engineer writes a transform in Python with Polars, published as a `python-data` script version
whose own lock names Polars and PyArrow (design 046's package layer; the repository's starter lock is
the template), and uses it in a `dataTransform` step with the same snapshot, audit and publication
semantics as DuckDB.

**Why this priority**: Polars broadens the transform language; its packages come from the version's
own lock under the namespace's supply-chain policy, so the platform keeps no package catalog (Owner
decision 2026-11-26-162), but it needs design 046 built first.

**Independent Test**: From the same inputs, a Polars script produces a snapshot equal to the DuckDB
fixture of US03: the same multiset of rows and the same contract fingerprint (the bytes may differ,
because the two writers encode the same contract type differently, which the encoding table of the
data runtime contract accepts); a lock that the namespace's supply-chain policy refuses does not publish, and a
package blocked after publication stops the lease with `PACKAGE_BLOCKED`.

**Acceptance Scenarios**:

1. **Given** a script version built for the `python-data` runtime, **When** a transform step uses
   it, **Then** it runs in a container with no network, reads only the staged inputs and the
   version's signed read-only package layer, and writes only its output directory.

**Failure scenario**: The script tries to install a package at run time; the container has no
network and no package index, and the step fails with `DATA_TRANSFORM_FAILED`.

---

### US08 — Track warehouse tables as external assets (Priority: P3)

An administrator declares a Snowflake table, or an Iceberg table that a feature 035 `icebergWrite`
step writes, as an external asset; the step that writes it declares so; lineage and freshness then
cover that table.

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
- The object store is unavailable while parts are uploaded: the attempt fails retryably before the
  completion transaction; nothing is published. It is unavailable while the engine reads footers:
  `CompleteStep` answers a transient status and changes no state, and the data runner sends the same
  completion again with the same idempotency key; it is never `DATA_CONTRACT_VIOLATION`.
- A snapshot is pinned by a live run when retention looks at it: it is kept until 24 hours after
  that run ends, and it is never moved to `ABANDONED` while the run is live (an own staged snapshot
  awaiting `dataPublish` behind a long approval, a `from` input or an output base).
- A tenant's data quota is exhausted while parts upload: the part reservation is refused (ADR-0027
  charges at reservation, not again at commit), the step ends with `STORAGE_QUOTA_EXCEEDED`, the
  failure transaction claims the attempt's earlier parts for deletion and the pointer stays.
- An attempt fails, is retried under a new generation after its lease lapsed, or is cancelled after
  it uploaded parts: the transaction that ends it claims its uncommitted parts for deletion, so a
  retried transform does not hold `tenant_data_bytes` or pending-upload slots for a day.
- An asset is tombstoned while a producer run is in flight: the completion refuses publication
  (`DATA_ASSET_NOT_ACTIVE`) and the snapshot is abandoned. The tombstone transaction clears the
  pointer with a publication record, so no retired snapshot is ever named by a pointer.
- One node names the same output asset twice, or the same input alias twice: publication refuses it
  (`VALIDATION_FAILED`, reason `DATA_NODE_SCOPE`), never a store error at lease.
- An encrypted field (031) is read by a database or data-node parameter expression: publication
  refuses it with 031's `VALIDATION_FAILED` reason `ENCODING`, issue `ENCODING_READ_BY_EXPRESSION`.
- Cancellation of a run whose transform already uploaded parts: the parts are never committed and the
  cancellation transaction claims them for deletion; a cancellation of a warehouse write is never
  treated as proof that nothing was written.

## Requirements *(mandatory)*

### Functional Requirements — database engines

The per-driver details these requirements refer to are normative in
[contracts/engines.md](contracts/engines.md).

- **FR-024-001 — Driver catalog and capability gate:** The SQL drivers of the catalog MUST grow from
  `postgresql` to the closed set `postgresql, mysql, mariadb, sqlserver, bigquery, snowflake`, beside
  feature 035's lake drivers (`iceberg`, `object_storage`, `clickhouse`), which are unchanged (DuckDB
  is a data node, not a driver). A driver is advertised by `listDatabaseDrivers` only when its
  instance-registry capability is recorded (`database-engines-v1` for MySQL, MariaDB and SQL Server,
  `database-warehouses-v1` for BigQuery and Snowflake, Architecture decision 0082), the deployment
  serves it and its release gate passed. A catalog write, publication or admission naming a driver
  not enabled MUST fail with `VALIDATION_FAILED` reason `DRIVER_NOT_ENABLED` before any work, and a
  task of a driver MUST be offered only to an executor of that driver's binding profile that offers
  its reserved activity (`ManagedKind`, Decision 024-002).
- **FR-024-002 — Closed settings per driver:** Each driver MUST have a closed object of non-secret
  settings (host/port/database; BigQuery project and location; Snowflake account, warehouse, role,
  database and schema) and feature 035's `egressCeiling` (default `internal`). URLs, DSNs, embedded
  credentials and unknown fields MUST be refused. 017 revisions, pinning, pause and tombstones apply
  unchanged; PostgreSQL keeps its shape.
- **FR-024-003 — Credentials and auth modes:** Values MUST be stored through the 017 OpenBao facade
  and resolved per attempt under the managed capability (Principle V's second declared set, pinned as
  the `primary` secret role of the step credential pin), in closed shapes: `{username,password}`
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
  use the existing `database-runner` role, each bound to its `BindingProfile` (`Database` or the new
  `Warehouse`, Architecture decision 0071 decision 1), so a token of one profile is never handed the
  other's tasks or credentials. No API, scheduler, ordinary worker or data runner executes them.
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
  Each driver MUST declare its recovery rule in Architecture decision 0071 decision 7's vocabulary
  (lookup for BigQuery and Snowflake; none for MySQL, MariaDB and SQL Server, which wait for a person).
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
  platform-derived schema carries the highest level among the pinned inputs, because no
  column-level taint is tracked (ADR-0036 decision 6). An asset input counts at the highest level of
  its pinned columns. A step source has no declared columns and, as an undeclared column does, counts
  as `pii`, unless the source step's output is a 016 carrier whose pinned output schema declares a
  level for every field (it then counts at the highest of them). A `dataQuery` MUST NOT read a 017
  database result at all (FR-024-029), so its values never become readable under the 016 reveal
  grant instead of `database:result:read`; a `dataTransform` may read one, and the asset manager's
  column levels then classify what it writes. The level of every input is recorded with its first-lease pin,
  and the highest of an empty set is never taken: a `dataQuery` has at least one input. The existing
  016 disclosure then applies its
  audience rules (`payload:read-internal`, the namespace `payload:reveal-pii` grant). Any later
  surface MUST apply the level likewise.
- **FR-024-017 — Producer binding:** A managed asset MUST name exactly one producer (workflow name
  and node id). Publication of a workflow version MUST refuse an output on an asset whose producer is
  another workflow or node (`ASSET_PRODUCER_MISMATCH`) and re-check the binding at admission and at
  publication of each snapshot. Changing the producer is a new revision, audited. For a `dataPublish`
  node (origin `RUN_NODE`) the binding is checked against the `dataTransform` node named by its `from`,
  which is the producer, never against the `dataPublish` node itself. A node MUST name each output
  asset at most once and each input alias at most once; publication refuses a repeat
  (`VALIDATION_FAILED`, reason `DATA_NODE_SCOPE`).
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
  a snapshot part. A `from` input and a `dataPublish` resolve the snapshot that the named node
  committed in its successful completion, never one an earlier attempt committed as `QUARANTINED`. In
  a run created by an 018 reset, a restored `from` or `dataPublish` whose node's
  `data.snapshot_committed` is in the restored prefix resolves the snapshot that restored event names,
  which the reset run references (`run_data_snapshot_refs`), and a `dataPublish` after the reset
  point may publish it under the reset run's `RUN_NODE` origin; a restored step-source pin names the
  source run's committed step output, which the lease admits through the run's reset link and the
  pinned blob id and digest, never a caller-chosen blob. A `from` input MUST resolve only an
  `AUDITED` or `PUBLISHED` snapshot; a quarantined one fails `DATA_INPUT_QUARANTINED`, so an
  `onError` route cannot launder it; an `ABANDONED` or `RETIRED` one, which the live-run pin of
  FR-024-034 makes unreachable, fails `DATA_CONFIGURATION_INVALID`. A reader MUST never observe a
  partially written or unaudited snapshot.

### Functional Requirements — transforms and write-audit-publish

- **FR-024-020 — `dataTransform` node:** The IR MUST gain an additive node kind `dataTransform` with
  `engine: duckdb|polars`, inputs (alias, asset reference, selector), outputs (asset reference,
  statement or script, parameters, publish mode, stale policy), limits, `next`, `onError`, timeout and
  a bounded retry policy. The timeout (`timeoutMs`, default 15 minutes) bounds the whole attempt:
  staging, the transform, the upload, the read-back, the audit and the report; the transform's wall
  time (default 10 minutes) is bounded by what is left after the audit time
  (`limits.auditTimeoutMs`, default 2 minutes) and a fixed staging allowance, and publication
  refuses a timeout that leaves the transform less than one minute. An attempt that ends `DATA_RUNTIME_UNAVAILABLE` before any container of it started does
  not count against `maxAttempts`; such attempts are bounded separately (10 per step), after which
  they count. Like 017 database nodes (ADR-0036 decision 3), data nodes MUST be accepted
  only in the root graph of a definition without collection, loop or continuation nodes until scoped
  pins are proven; the same holds for `dataQuery` and `dataPublish`. Data nodes exist only in
  graph-model versions (Decision 024-010) and declare no `destinations`. Publication and admission
  MUST be refused until the instance-registry capability `data-pipelines-v1` is recorded
  (`VALIDATION_FAILED`, reason `DATA_PIPELINES_NOT_ENABLED`). The activity prefix `sagat.data.` MUST
  be reserved like `sagat.database.`: no ordinary activity, compensation or reconciliation reference
  may use it.
- **FR-024-021 — Isolated engine-local execution:** DuckDB and Polars MUST run only in a container
  per attempt that a sandbox runner in the exclusive **data mode** starts when the **data runner**
  (`apps/data-runner`, role `data-runner`) asks it for an attempt through Architecture decision
  0079's API (attempt id, slot, budget class, deadline; never an image, a mount, a command or a
  limit). The data runner holds no container-engine access and no credential. The container has the
  ADR-0013 profile (no network interface but loopback, read-only root, dropped capabilities, seccomp,
  the slot's unprivileged user, CPU/memory/PID/time limits from the class budget, no credentials)
  changed only as ADR-0058 decision 1 declares: the attempt directory read-only
  (`ro,noexec,nosuid,nodev`) with the staged inputs under `in/`; `/out` and `/spill` as tmpfs mounted
  `noexec,nosuid,nodev` with a `size=` limit charged to the container's memory cgroup; and, for
  Polars only, design 046's signed layer read-only at `/opt/sagat/deps`. Since `/out` is discarded
  with the container, parts MUST leave through a bounded, framed channel over the attempt's Unix
  socket, which the data runner serves only to the slot's peer user, before the host exits. A unit
  test over the container specification MUST assert that every writable mount is `noexec` and sized,
  every bind is read-only, the only executable mount is the signed layer, and nothing else is
  mounted. DuckDB MUST additionally run with external access disabled except the input and output
  directories, extension install/autoload disabled, only extensions baked into the pinned image, and
  configuration locked; these settings are defence in depth, not the boundary. Inputs MUST be staged
  by the data runner through lease-authorized reads of exactly the pinned parts (feature 035's
  `step_lease_blobs`).
- **FR-024-022 — DuckDB SQL transforms:** Each output MUST be produced by one `SELECT` statement over
  the input aliases, with typed parameters bound by position; the platform wrapper writes the result
  as Parquet parts within the part and snapshot ceilings. Statements that are not a single query
  MUST be refused at publication and again in the container.
- **FR-024-023 — Output verification by the engine:** Before committing a snapshot the engine MUST
  verify every part itself: exact upload fence, measured size and SHA-256, Parquet magic and footer
  within a bounded size, unencrypted footer, a schema that the pinned contract accepts under the
  normative encoding table of the [data runtime contract](contracts/data-runtime.md#contract-types-and-parquet-encodings)
  (column names and order equal; each column's physical and logical type one the table lists for its
  contract type; repetition `REQUIRED` or `OPTIONAL`, never `REPEATED`; a `nullable: false` column
  written `OPTIONAL` is accepted and its nulls are caught by the generated not-null audit), and row
  counts. A mismatch MUST fail the attempt with `DATA_CONTRACT_VIOLATION` and commit nothing,
  whatever the sandbox reported. An object-store error while reading a footer MUST NOT be a mismatch:
  `CompleteStep` answers a transient status with no state change and the same completion is sent again.
- **FR-024-024 — Audits and quarantine:** Declared audits (row-count bounds, not-null from the
  contract, key uniqueness, accepted values, custom check statements authored on the asset) MUST run
  in a fresh attempt of the data mode after the transform container has ended, in the transform's
  budget class (the class is chosen from the step's limits, which cover the snapshot size, never a
  fixed `small`), over exactly the uploaded parts and the pinned declaration. The parts are read back
  from the platform because the transform's `/out` tmpfs is gone. What binds the audit to the
  uploaded bytes is the data runner's read-back: it reads exactly this attempt's uploads with
  `ReadBlob` and checks each against the SHA-256 the server measured at upload. The data runner
  reports the SHA-256 of what it staged and the engine refuses a completion whose digests differ from
  the committed parts, which catches a data runner that staged other bytes; it proves nothing about
  what the untrusted audit container read, and the audit outcomes rest on the sandbox (Gap 1249).
  Results MUST be payload-free (check name, outcome, failing count).
  A `FAILED` outcome, or an `ERROR` outcome of a check the asset manager authored, MUST commit the
  snapshot as `QUARANTINED`, keep the pointer, and fail the step with `DATA_AUDIT_FAILED` (not
  retryable). A failure of the audit attempt itself (the data mode refused it or was unavailable, the
  container was killed for memory, timed out or crashed, or a platform-generated check reported
  `ERROR`) MUST commit no snapshot and fail the step retryably with `DATA_RUNTIME_UNAVAILABLE`; it is
  never a quarantine.
- **FR-024-025 — Publication:** With `publish: auto`, the pointer compare-and-set MUST commit in the
  completion transaction of an audited snapshot. With `publish: manual`, the snapshot stays `AUDITED`
  until a `dataPublish` node (an engine node that needs no worker) or the API publishes it. The
  compare-and-set MUST be against the pointer version captured at the producer's first lease;
  `onStale: fail` (default) fails with `DATA_PUBLISH_STALE`, `onStale: skip` completes without
  publishing. A quarantined snapshot MUST never be published; publishing the snapshot that is already
  current MUST be an idempotent success. A `dataPublish` that meets an `ABANDONED` snapshot (which the
  live-run pin of FR-024-034 makes unreachable) treats it as stale and follows its `onStale`. An
  auto-publishing output whose compare-and-set is stale commits its snapshot directly as `ABANDONED`
  when no `from` input or `dataPublish` of the definition names its node; otherwise it stays
  `AUDITED`, pinned by the live run.
- **FR-024-026 — Manual publication and rollback API:** Holders of `data:asset:publish` MUST be able
  to publish an audited snapshot or roll back to a retained previously published snapshot, with an
  `Idempotency-Key`, `expectedPointerVersion` and a justification (plain text without control
  characters, at most 1,024 bytes), in one transaction with the publication record and audit. The
  justification is tenant-authored text and stays in the charged `content` class, as ADR-0027
  requires of free text: at a full content quota an API publication or rollback is refused
  `QUOTA_EXCEEDED` with nothing written, like any synchronous API mutation. Replay MUST recheck current authorization. Rollback creates a new
  publication record; it deletes nothing.
- **FR-024-027 — Publication event and triggers:** Every publication MUST write, in its transaction,
  a payload-free inbox event (source `sagat.data`, type `data.asset.published`, subject the asset
  name, event id `<asset id>:<pointer version>`, so a rollback or republication of an earlier
  snapshot is a new event) and the outbound delivery rows for webhook subscriptions that ask for
  `data.asset.published`. The event document MUST be a closed, bounded (≤ 1 KiB) `protected_system`
  record under ADR-0027: it is not charged to content quota and does not spend the
  `events_per_second` bucket. The other records these transactions write are protected too: the
  output document of a `dataTransform` step and of a `dataPublish` node is a closed record of asset
  names, snapshot ids, states, pointer versions and counts, at most 4 KiB, that the content
  accounting measures as zero (data model). A run publication carries no free text, so a full content
  quota can never roll back an auto-publishing completion or a `dataPublish`. An API publication
  carries the charged justification of FR-024-026 and obeys the content quota: it is refused
  `QUOTA_EXCEEDED` before any write when the quota is full. A `dataQuery` output carries values and stays charged: at full content quota it fails
  `STORAGE_QUOTA_EXCEEDED` (ADR-0027), which publishes nothing. Existing 004 event triggers MUST be able to start runs from it (their run admission keeps its own
  quotas); the inbox key deduplicates redelivery of the same publication.
- **FR-024-028 — Polars transforms:** A Polars transform MUST reference a published script version
  of the language `python-data` (reserved by feature 023) whose artifact is design 046's bundle
  version 5 with the version's own lock (ecosystem `pypi`), resolved through the package gateway,
  checked by the namespace's supply-chain policy at publication and by the block check at lease
  (`PACKAGE_BLOCKED`), exactly as for a Python script with packages (design 046, Architecture decision
  0120; Owner decision 2026-11-26-162). There is no platform-curated package catalog: the `python-data`
  runtime carries the interpreter and the data contract host only, and Polars, PyArrow and any library
  come from the version's signed read-only layer. The repository ships a reviewed starter lock for
  Polars and PyArrow as a template. A `python-data` version runs only as a Polars transform. It MUST
  use the same container, staging, verification, audit and publication contract as DuckDB and MUST
  NOT install packages at run time.

### Functional Requirements — DuckDB queries, lineage, freshness, external assets

- **FR-024-029 — DuckDB queries:** The IR MUST gain an additive data node `dataQuery` (root-only,
  like `dataTransform`) with inputs (alias, asset or step source; a step source MUST NOT be a 017
  database step, which publication refuses with `VALIDATION_FAILED` reason `DATA_NODE_SCOPE`, so a
  database result keeps the 017 audience of ADR-0035 decision 10), one `SELECT` statement, typed
  parameters and limits. It runs in the data sandbox as the reserved activity `sagat.data.query`,
  pins its inputs at first lease, writes nothing, and returns the 017 typed-cell result shape as its
  inline step output (≤ 256 KiB; above it `DATA_OUTPUT_LIMIT`). That output MUST be a 016
  classification carrier whose platform-derived schema is annotated with the highest level among
  its pinned inputs as FR-024-016 counts them (a step source with no declared level counts as
  `pii`), disclosed only through the existing 016 payload disclosure. DuckDB MUST
  NOT be a 017 database driver, and the 017 result view MUST NOT serve these outputs. Ad hoc
  queries use 022 standalone steps when they exist.
- **FR-024-030 — Lineage:** The platform MUST serve a bounded lineage graph per namespace combining
  declared edges of active workflow versions (node reads/writes asset) and recorded snapshot
  provenance (snapshot produced by run/node from input snapshots). It MUST carry no values, MUST be
  bounded in nodes, edges and depth, and MUST show a producer the caller may not read as opaque.
- **FR-024-031 — Freshness:** An asset MAY declare `maxLagSeconds` and an optional cron deadline
  (five fields, IANA zone, grace). Status `FRESH|STALE|NEVER_PUBLISHED|NOT_TRACKED` MUST be derived
  from the database clock and the age of the data: for a managed asset the creation time of the
  current snapshot (its producer's completion), never the time it was published, so a rollback to an
  older snapshot or a late publication of an old one does not make the asset fresh; for an external
  asset the last observation. It is served by the API and shown in the Studio. The 008 monitor MUST
  raise `DATA_ASSET_STALE` (fires while the asset is `STALE`, resolves when it is not) and
  `DATA_SNAPSHOT_QUARANTINED` (fires while a `QUARANTINED`, not yet `RETIRED` snapshot of the asset
  exists that was created after the current snapshot, or at all when the asset was never published;
  resolves when none does, because a newer snapshot was published or the quarantined one was retired)
  alerts (subject: asset name) with the existing fire/resolve semantics. No automatic re-run is
  triggered.
- **FR-024-032 — External assets:** An external asset MUST name a connection (a SQL driver's, or a
  feature 035 Iceberg connection's), an opaque locator and one producer (workflow and node). A
  database step or a feature 035 `icebergWrite` step MAY declare `assets.reads` and `assets.writes`;
  a declared write MUST be refused at publication unless the step is the asset's producer and uses
  the asset's connection. A successful declared write MUST record an observation in its completion
  transaction, after locking the external asset heads and rechecking under that lock that each asset
  is `ACTIVE` and that the node is still its producer on the current revision; when the recheck fails
  the step still completes (the write happened) but no observation is recorded and the refusal is
  audited. An unknown or failed outcome MUST record none. The completion of a database or
  `icebergWrite` step without `assets.writes` keeps its statement budget unchanged. External assets have no snapshots, pointer, audits or write-audit-publish; lineage
  is declared, never inferred from SQL text.

### Functional Requirements — cross-cutting

- **FR-024-033 — Authorization:** New permissions `data:asset:manage` (admin), `data:asset:use`
  (admin, developer), `data:asset:publish` (admin, operator) and the machine permission
  `data:execute` MUST be added; `data:execute` is held only by a new machine role `data-runner`
  (`worker:execute`, `data:execute`), bound with `BindingProfile::Data`; the `runner` role is
  unchanged (Decision 024-001). Tests MUST assert the set of built-in roles agreed across features
  (021 adds `integration-runner`, 024 `data-runner`, 025 `agent-runner`) and the set of permissions,
  never a count. Asset metadata,
  snapshots and freshness need `namespace:read`; lineage also needs `definition:read`. Authority MUST
  be rechecked at workflow publication, run admission, lease, capability resolution, completion,
  snapshot publication and receipt replay. The exact matrix is in the plan.
- **FR-024-034 — Retention:** Per asset, the current snapshot plus the declared number of previous
  published snapshots, every snapshot pinned by a live run, and pins of runs that ended less than 24
  hours ago MUST be kept. A snapshot pinned by a live run (its own staged snapshot, a `from` input, an
  output base, or a reference of a reset run) MUST NOT move to `ABANDONED`, whether by its time to
  live or by being passed over by another publication. Unpublished, quarantined and abandoned
  snapshots MUST be kept for their declared period, counted from the snapshot's creation whatever its
  state, so no snapshot is kept twice that period. A snapshot that an asset's pointer names MUST NOT
  become `RETIRED`; tombstoning an asset clears its pointer with a publication record first.
  Deletion MUST mark the snapshot `RETIRED` and its parts `DELETING` in one audited transaction
  before provider deletion, with ADR-0023's durable retry; bytes are released only after confirmed
  deletion. Destroying customer data this way supersedes in part ADR-0023 and ADR-0027 and is
  decided (Owner decision 2026-11-26-163): retention deletes the bytes under the retention the asset
  manager declared, audited in the deciding transaction, after the parts' lease allow-list rows are
  released.
  Run purge (020, ADR-0054) MUST NOT delete `DATA_SNAPSHOT_PART` blobs or snapshot metadata
  (snapshots, parts, provenance, audits, publications); this feature owns their retention. Data that left through queries, events or exports is outside platform deletion.
- **FR-024-035 — Quotas and finite ceilings:** Every new resource MUST have a default and a hard
  ceiling (plan table); the effective value is the minimum of request, namespace and deployment;
  zero refuses new admission and never means unlimited; admission fails before work. Snapshot bytes
  MUST count against a new tenant aggregate `tenant_data_bytes`, separate from
  `tenant_content_bytes`. The transaction that fails a data attempt, retries it under a new
  generation after its lease lapsed, cancels it or refuses its completion MUST claim that attempt's
  uncommitted `DATA_SNAPSHOT_PART` uploads for deletion (ADR-0023 tombstone with an immediate retry
  time), and an upload of that fence that finishes later is claimed when it finishes; the bytes and
  the pending-upload slot are released after confirmed deletion (ADR-0027), not after the 24-hour
  upload time to live, which stays only as a backstop.
- **FR-024-036 — Observability and audit:** The feature MUST emit OTel spans and metrics with closed
  labels only (`driver`, `engine`, `outcome`, `origin`) and redacted logs. SQL text, parameters,
  result cells, asset content, credentials, provider bodies and locators MUST NOT appear in logs,
  spans, metrics or audit. Asset changes, publications, rollbacks, quarantines, snapshot reads by an
  attempt, retirements and denials MUST be audited in the deciding transaction.
- **FR-024-037 — Typed errors:** REST errors MUST reuse the closed `Error.code` set with new
  `details.reason` values; step failures MUST use closed codes (new database codes and a closed data
  failure set) with no driver or provider message. The lists are in [contracts/README.md](contracts/README.md).
  Every step failure code MUST also be a `worker`-origin entry of `sdks/error-catalog.json`, of
  `sdks/SPEC.md` section 2 and of each SDK's embedded copy (the precedent of feature 035's codes).
- **FR-024-038 — Contracts, SDKs and compatibility:** OpenAPI, IR schema, worker protocol and
  AsyncAPI changes MUST be additive; new worker messages MUST take the next free minor of the
  register in `sdks/SPEC.md` section 9 when their contract merges (1.7 is feature 041's, Owner
  decision 2026-11-15-8) and MUST never be offered to an older runner; new field numbers follow
  feature 035's. The connection `config` union already exists (feature 035); its growth MUST be
  tested with an older generated client reading a list that contains a MySQL connection. A binary
  without these drivers (0.2.0, or a build before this feature) cannot decode a new-driver row (it
  raises an internal store error and fails the whole connection list), so the drivers' registry
  capabilities MUST stay unrecorded until binary rollback to such a release is no longer planned, and
  the release notes MUST say so. The six SDK builders (TypeScript, Go, Python, C#, Java, Rust) MUST
  emit the new node kinds and the `assets` property identically, and shared conformance scenarios
  MUST cover them.
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
- **FR-024-042 — Reuse of merged features:** Every run this feature starts MUST pass design 043's
  run-input gate at an existing row (an event trigger on `data.asset.published` at row 9; a probe or
  an ad hoc execution of a new driver at row 13); this feature adds no run creator. The first lease
  of a data node and the data completion MUST have statement-budget shapes of their own in
  `integration_claim_transition_efficiency`, and the budget of ordinary steps MUST NOT grow (design
  038). Data steps are platform-run managed steps and MUST be refused step logs (Requirement
  041-016); the host's bounded, redacted log is kept as a script step's log is. A step that writes a
  `pii` value to a connection of a new driver MUST meet the uniform `pii`-egress rule (Owner question
  1081, Decision 1617): a ceiling of `pii` is set only by an administrator holding the namespace
  reveal grant, rechecked at each resolution, revoked by pausing the connection, audited per attempt;
  `secret` never leaves.

### Amendment to feature 016 (coordinator decision C6)

This feature adds exactly one carrier to FR-016-002: the output of a `dataQuery` node, whose schema
is platform-derived (the 017 typed-cell result shape) and annotated at the highest
`x-sagat-classification` level among the pinned inputs (asset columns at their declared level, a
step source with no declared level at `pii`, FR-024-016). `dataTransform` and `dataPublish`
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
- **NFR-024-003 (compatibility):** Existing PostgreSQL and lake connections, workflows, runners at an
  older protocol minor and SDK builds keep working unchanged; new contract fields are optional or new.
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
| SC-024-001 | Against real MySQL 8.4, MariaDB 11 and SQL Server 2022 containers, all six SDKs and a Studio-authored workflow run the shared query/write fixture with byte-identical typed results and errors; wrong host name, untrusted CA, privileged login, denied destination and ambiguous write each produce their typed outcome | FR-024-001..008, 011..013 |
| SC-024-002 | The fake BigQuery/Snowflake servers prove authentication, binding, pagination, cost refusal, cancellation and lookup in CI; a network-policy test shows the warehouse runner cannot open a socket to a private customer network and the private runner cannot reach a public endpoint; the real-service suite passes once on owner-provided accounts before either driver is advertised, and its date and scope are recorded | FR-024-003..005, 009..011 |
| SC-024-003 | Every row of the per-engine type table either maps to its cell type or fails `DATABASE_TYPE_UNSUPPORTED`, tested on each engine; zero partial results | FR-024-008 |
| SC-024-004 | DuckDB and Polars canaries (read outside mounts, extension install, socket, executing a file written to `/out` or `/spill`, filling them past `size=`, fork bomb, memory exhaustion) fail inside the container in 100% of runs; the mount-profile unit test asserts every writable mount `noexec` and sized and the signed layer as the only executable mount; no DuckDB or Polars code is linked into or executed by the control plane, engine, scheduler, database runner or data runner, and the data runner links no container engine (architecture check) | FR-024-021, 022, 028, 029 |
| SC-024-005 | Under barrier-controlled concurrency, readers observe only whole published snapshots in 100% of rounds; stale compare-and-set loses with a typed error; a quarantined snapshot is never current | FR-024-018, 019, 023, 025, 026 |
| SC-024-006 | Crash injection at every commit point of staging, completion, publication, rollback and retention leaves either the old or the new state, no orphan pointer, no double publication, and recovers after restart; an 018 reset after each data journal event type follows its declared registry rule and never re-publishes, a reset after a step-source pin and a reset between a manual transform and its `dataPublish` resolve the source run's step output and snapshot, and a reset racing retention on the same snapshot (barrier) either is refused `SNAPSHOT_RETIRED` or keeps the snapshot | FR-024-019, 023..027, 034 |
| SC-024-007 | A sandbox that reports success for a contract-violating file is refused by the engine's own footer check; every row of the encoding table is accepted as written by the pinned DuckDB image and by the Polars starter lock, and every other encoding is refused; a data runner made to stage a blob other than the committed part (fault injection) is refused with `DATA_CONTRACT_VIOLATION`; contract compatibility modes accept and refuse the documented revision pairs | FR-024-015..017, 023, 024 |
| SC-024-008 | The lineage fixture matches its expected graph and table; freshness flips on the database clock, a rollback to an old snapshot leaves the asset `STALE`, and both alerts fire and resolve on their stated conditions; a trigger dispatcher crash after creating the run and before marking the event processed, and a redelivered webhook row, still give exactly one run per trigger per publication, and a rollback is a new event; external observations follow outcomes, only the producer can record them, and a producer change that commits first leaves no observation | FR-024-027, 030..032 |
| SC-024-009 | The tenant battery covers every new REST operation and worker RPC: another tenant's or namespace's resource is indistinguishable from a missing one and the probe is audited; a `dataQuery` output over `pii` columns, and a `dataQuery` over a Parquet step output with no declared level, is disclosed only with the reveal grant; a `dataQuery` whose step source is a 017 database step (a MySQL query step) is refused at publication; and the 017 result view never serves a `dataQuery` output | FR-024-014, 016, 029, 033 |
| SC-024-010 | Exhausting each ceiling of the plan table fails with its typed error before any work, including BigQuery bytes billed and `tenant_data_bytes` at part upload; an auto-publishing completion and a `dataPublish` node each commit with their event while content quota and `events_per_second` are exhausted, and an API publication at that full content quota is refused `QUOTA_EXCEEDED` with the pointer, receipt and audit unchanged; after a retried failing transform the tenant's data bytes and pending uploads return to their baseline once the deletions are confirmed, without waiting a day | FR-024-009, 026, 027, 035 |
| SC-024-011 | Real OTLP export, logs and audit of every scenario contain no SQL, parameter, cell, asset-content, credential or locator canary; every metric label is in its closed set; every error is a catalogued code or reason | FR-024-013, 036, 037 |
| SC-024-012 | The new shared conformance scenarios pass in TypeScript, Go, Python, C#, Java and Rust with identical canonical documents and outcomes | FR-024-020, 028, 038 |
| SC-024-017 | Ordinary lease and completion budgets are unchanged and the data shapes hold their recorded budget (`integration_claim_transition_efficiency`); a `pii` write to a new-driver connection without a `pii` ceiling is refused, and revoking the saver's reveal grant pauses the connection; a run started from a publication event is checked by the run-input gate | FR-024-042 |
| SC-024-013 | Real-browser tests cover each driver form, the asset catalog, lineage graph and table, freshness and publish/rollback, with no accessibility violation of WCAG 2.2 AA in automated checks | FR-024-039, NFR-024-006 |
| SC-024-014 | Retention keeps every pinned and in-window snapshot and deletes the rest only after commit; a crash during deletion retries without losing a retained snapshot; a quarantined snapshot cannot reach a reader through `from`; a snapshot staged by a live run stays `AUDITED` through an approval longer than its window and through a rollback that passes it over; no pointer ever names a `RETIRED` snapshot; a feature 020 run purge leaves `DATA_SNAPSHOT_PART` blobs and snapshot metadata untouched (once that purge exists) | FR-024-014, 019, 025, 034 |
| SC-024-015 | Existing 017 suites pass unchanged; an older generated client reads a connection list containing a MySQL connection; a runner registered below the new minor never receives a new task kind; encrypted-field, cache and mock combinations are refused at publication | FR-024-038, 041, NFR-024-003 |
| SC-024-016 | The quickstart demo runs from a clean checkout with the documented commands, showing the happy path and the audit-failure path, and its runbooks are linked from the operator index | FR-024-040 |

## Assumptions

- PostgreSQL remains the authority; snapshot bytes live in the platform's ADR-0023 blob backend
  under the namespace (Owner decision 2026-11-26-160).
- Test engines run as disposable containers (rule 16). BigQuery and Snowflake have no free local
  emulator the project can depend on; CI uses fake protocol servers and the real-service suite needs
  accounts the owner provides (Owner decision 2026-11-26-161).
- Feature 005's container driver and runtime (G98 accepted trade-off), in the data mode of the
  sandbox runner that Architecture decision 0079 defines for the lake mode, is the isolation used for
  DuckDB and Polars, with the data-profile mounts ADR-0058 declares; no stronger runtime (gVisor,
  Kata) is assumed. Architecture decision 0079 is Proposed and its Kubernetes half open (Gap 024-001).
- Snapshot bytes travel through the engine process as ADR-0023 requires; that bounds the ceilings
  and the throughput (G1247).
- Polars runs on the `python-data` language that 023 reserves, with design 046's package layer and no
  platform catalog (Owner decision 2026-11-26-162); design 046 is built before User story 024-07.
- Partitioned assets, incremental append and range backfill are not in the first delivery (G1245).

## Out of scope

DuckDB as a 017 database driver (a data node instead); automatic materialization or re-run on
staleness; partitioned assets and backfill; customer-owned buckets as snapshot storage and DuckDB
reads of external object storage (customer S3 credentials resolve only in 021's integration runner,
G1246); warehouse
native write-audit-publish (table swap or clone) and warehouse schema verification (G1244); SQL
Server Entra ID and Windows authentication (G1243); nested Parquet types; Iceberg, Delta and DuckLake
as snapshot formats (Iceberg tables are external assets written by feature 035; exporting a snapshot
to one, and the new drivers as feature 035 destination kinds, are Gap 024-004); value previews of assets other than `dataQuery` outputs; cross-namespace asset reads (030);
asset declarations in 011 GitOps manifests (G1248); automatic lookup for warehouse writes whose
attempt was lost (G1240); pruning of retired snapshot metadata (G1254; snapshot metadata is excluded
from 020's run purge and owned by ADR-0058);
data steps inside collections, loops, continuation or saga bodies.

## Owner questions

All four are decided under the owner's delegation (Owner decision 2026-11-24-4); the text of each
question is kept, with its answer first.

- **O1240 — Where do managed snapshots live?** **Decided (Owner decision 2026-11-26-160): the
  platform blob backend under the namespace, counted in `tenant_data_bytes`; a customer bucket stays
  Gap 1246.** Recommended default was: in the platform's existing blob
  backend under the namespace, counted in the new `tenant_data_bytes` quota. A customer-owned bucket
  is a later extension: under ADR-0055 its credentials resolve only in `apps/integration-runner`, so
  it would need that runner to stage objects for the data sandbox (G1246).
- **O1241 — How are BigQuery and Snowflake released?** **Decided (Owner decision 2026-11-26-161):
  built and tested against the fake protocol servers and not offered until the real-service suite
  passes on accounts the owner provides; agents create no account and spend no money.** Recommended
  default was: the drivers stay hidden
  (`DRIVER_NOT_ENABLED`) until the real-service contract suite passes on test accounts the owner
  provides; CI keeps the fake servers.
- **O1242 — Does the project maintain a curated Python data catalog for Polars?** **Decided (Owner
  decision 2026-11-26-162): no platform-curated catalog; Polars and PyArrow come from each script
  version's own lock and design 046's signed layer; a reviewed starter lock ships as a template; User
  story 024-07 stays.** Recommended default was: yes, one hash-pinned catalog (Polars, PyArrow) re-pinned on a stated cadence with
  vulnerability evidence, through 023's `python-data` mechanism; if not accepted, US07 is dropped
  and DuckDB remains the only transform engine.
- **O1243 — May the platform destroy superseded snapshot data?** **Decided (Owner decision
  2026-11-26-163): yes, under the retention the asset manager declares, audited; ADR-0058 supersedes
  ADR-0023 and ADR-0027 in part for that purpose only.** ADR-0027 does not authorize deleting
  customer data and ADR-0023 never deletes committed blobs. Recommended default: yes, but only under
  the retention an asset manager declares on the asset (current plus `retainPublished`, live pins,
  the unpublished window), audited, with the runbook stating that copies outside the platform are
  not reached; until the owner decides, retention marks snapshots `RETIRED` and keeps their bytes.
  (This is a fourth owner question; the coordinator asked for it explicitly.)

See [research](research.md) for decisions D1240–D1279 and D024-001–D024-016 and gaps G1240–G1256
and G024-001–G024-005, the
[plan](plan.md), the [data model](data-model.md), the [contracts](contracts/README.md), the
[quickstart](quickstart.md), the [tasks](tasks.md) and the [requirements checklist](checklists/requirements.md).
