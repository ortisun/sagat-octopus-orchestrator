# Feature Specification: Apache Iceberg writes and step-output destinations

**Feature Branch**: `feature/035-iceberg-write-steps`

**Created**: 2026-10-27

**Status**: Draft — specified, clarified and planned ([plan](plan.md)); not implemented

**Input**: User description: "Quero especificar em speckit uma nova feature. Quero ter steps que permitam
salvar os dados em apache iceberg." (Steps that let a workflow save its data into Apache Iceberg tables.)

## Confirmed scope (owner, 2026-10-27)

The owner answered three scope questions before the specification was written
([research](research.md) D035-001..003, [owner decisions](../../docs/owner-decisions.md)):

1. **Write modes:** append, overwrite and upsert (merge by key) are all part of this feature.
2. **Catalogs and storage:** the broad set — Iceberg REST catalog, AWS Glue Data Catalog, Hive
   Metastore and JDBC catalog; S3-compatible object storage, Google Cloud Storage and Azure Data Lake
   Storage Gen2.
3. **Where the destination lives:** a new connection kind in the feature-017 connection catalog,
   reusing its revisions, OpenBao-backed secrets, probe, managed executor, network policy and ad hoc
   execution. No dependency on the drafts of PR #53.

After the clarification session the owner extended the scope ("Quero que o output dos steps também
possam ser salvas no iceberg, postgres, etc... Exemplo. Acabei finalizar um pagamento, esse pagamento
poderia descer para o lake"; [research](research.md) D035-018..021):

4. **Step outputs go to destinations.** Any step's output can be saved to destinations, chosen per
   destination as **synchronous** (written inside the run; the run's next steps wait for it) or
   **asynchronous** (exported after the step commits, in batches, never delaying the run).
5. **Destination kinds, all five in both modes:** Iceberg, customer PostgreSQL, object storage files,
   ClickHouse and broker streams.
6. **Placement:** asynchronous export is the declared-record export of draft feature 020, which gains
   an Iceberg sink that reuses this feature's Iceberg connection and recovery rules. This feature
   specifies the Iceberg write step and the synchronous destinations.

## Clarifications

### Session 2026-10-27

- Q: How much data should one Iceberg write step be able to take in a single atomic commit? → A: Up
  to 2 GiB per step, in at most 32 record blobs of up to 64 MiB each (the ADR-0023 blob ceiling),
  streamed into files rather than loaded whole into memory.
- Q: Should an upsert step also be able to delete rows by key? → A: Yes, through an optional delete
  marker: a boolean record field declared by the step that marks a record as "delete this key";
  inserts, replacements and deletions land in the same single commit.
- Q: What should happen when the records carry fields that the target table does not have? → A:
  Refused by default; a step may explicitly declare that unknown fields are ignored, in which case they
  are dropped, counted in the output and never written. The table schema is never changed.
- Q: Besides the effect key, which platform identifiers should each commit record in the table's
  snapshot history? → A: The run id, the step id, the workflow name and the published workflow
  version; never values, actor identity, tenant identity or credentials. Workflow names thereby
  become visible to every reader of the table's metadata, outside the platform's access control.
- Q: Should an Iceberg write step be able to create its target table when it does not exist? → A: No.
  Tables exist beforehand, created by the table owner; a missing table fails the step with a typed
  error.

## Context and delivery boundary

Today an author who wants workflow data in an Iceberg table writes their own worker code: the catalog
and storage credentials are repeated in every worker, a retried activity can append the same rows
twice, and nothing tells an operator whether a commit that timed out happened. This feature adds:

- an **Iceberg connection** kind in the 017 connection catalog (catalog, storage, credentials,
  allowed write modes, classification ceiling, warehouse locations);
- a managed **Iceberg write step** that takes records produced by earlier steps and writes them into
  an existing table as one atomic Iceberg commit, in `append`, `overwrite` or `upsert` mode;
- **recovery by lookup**: each commit carries the step's effect key, so after a crash, a lost response
  or a timeout the platform looks the key up in the table's history instead of writing again;
- an optional **revert** compensation that undoes one step's commit with a new commit, for sagas;
- authoring parity between the Studio and every official SDK, with catalog and schema browsing;
- **synchronous step-output destinations**: any step (for example *finish payment*) may declare that
  its output is also written to an Iceberg table, a PostgreSQL table, an object storage file, a
  ClickHouse table or a broker stream; the platform schedules one derived write per destination when
  the step commits, and the step's dependents wait until those writes succeed;
- two new 017 connection kinds for those destinations, `object_storage` and `clickhouse` (PostgreSQL
  already exists; broker streams reuse the outbound broker destinations of ADR-0025 and 033).

The asynchronous counterpart — the payment record flowing down to the lake in batches without ever
delaying the payment run — is 020's declared-record export; 020 adds an `ICEBERG` sink on top of this
feature's connection. An author sees both modes side by side on the step.

It does not provision catalogs, warehouses, buckets, namespaces or tables; it never changes a table's
schema, partition spec, sort order or properties; it does not run table maintenance (snapshot expiry,
compaction, orphan-file removal); it does not read Iceberg tables back into workflows; it offers no
atomic commit across several tables; and it never makes an Iceberg table the authority for platform
state. PostgreSQL stays the authority (constitution I); ADR-0058's rejection of a lake catalog as the
publication authority (alternative 5) is unchanged, and this feature is the kind of export that ADR
leaves room for.

## Dependencies and ownership

- **Built and reused:** 001 identity and canonical errors; 002 leases, fencing, journal, outbox;
  003 unknown outcomes, manual review and compensation segments (ADR-0004, ADR-0005, ADR-0010);
  006 SDK conformance; 007 Studio; 008 telemetry; 009 authorization, secrets, egress and quotas;
  011 immutable publication; 016 per-field classification (ADR-0033); 017 connection catalog,
  OpenBao secret facade, managed executor, probe and ad hoc admission (ADR-0035, ADR-0036);
  ADR-0023 blob claim check; ADR-0027 content and rate quotas.
- **Also built and reused for synchronous destinations:** 004 and ADR-0025 durable outbound delivery
  (Kafka, NATS) and 033 Redis Streams callback destinations, for broker streams.
- **020, declared-record export (draft, PR #53):** the owner chose it on 2026-10-27 as the
  asynchronous mechanism (D035-020). Dependency direction: 020's `ICEBERG` sink depends on this
  feature's connection kind and recovery rules; this feature does not depend on 020, so the write
  step and the synchronous destinations can be built first.
- **Drafts this feature coordinates with but does not depend on** (PR #53, pending owner review):
  021 (if its resource catalog is accepted, the mapping of 017 connection kinds into resources
  applies to the kinds added here too, and its `s3_bucket` operations must not fork the
  `object_storage` semantics); 024 (published data snapshots become an additional record source only
  after 024 is accepted); 034 Rust SDK (joins the parity set if merged first).
- **Decision record:** the plan proposes **ADR-0071** (next free number: ADR-0069 is taken by 033,
  ADR-0070 by 034), "Lake writes and step-output destinations are managed effects recovered at the
  destination" (Proposed).

## Domain vocabulary

- **Iceberg connection**: a 017 connection of kind `iceberg` — a catalog, a storage, credential
  references, the warehouse locations the platform may write to, the write modes it allows and the
  highest payload classification it may receive. Immutable revisions, pinned at run admission.
- **Catalog type**: `rest`, `glue`, `hive`, `jdbc`. **Storage type**: `s3`, `gcs`, `adls`.
- **Table identifier**: Iceberg namespace (one or more levels) plus table name.
- **Target ref**: the existing table branch a commit goes to; `main` unless declared.
- **Record source**: where the rows come from — an inline list of records, a record blob produced by
  an earlier step, or a 017 database result of an earlier step.
- **Write mode**: `append` adds rows; `overwrite` replaces the rows in a declared scope; `upsert`
  inserts rows, replaces existing rows that have the same key, and removes the rows whose key a
  record marks for deletion.
- **Delete marker**: an optional boolean record field, declared by an upsert step, that turns a record
  into "delete this key"; it is not a table column and is never written.
- **Effect key**: the stable step idempotency key (existing, per step and run), written into the
  commit's snapshot summary so the commit can be found again.
- **Base snapshot**: the table snapshot an attempt read before writing; conflict checks are made
  against the commits since it.
- **Commit receipt**: the outcome the platform records — snapshot id, sequence number, counts — or an
  explicit unknown outcome.
- **Revert**: a compensation that undoes exactly one earlier write commit with a new commit; it
  never moves the table back to an older snapshot.
- **Output destination**: a place a step's output is also saved. **Synchronous**: declared on the
  step, written inside the run by a destination write; the step's dependents wait for it.
  **Asynchronous**: a 020 sink that selects the step's declared record and exports it in batches
  after the step commits; the run never waits for it.
- **Destination write**: the derived step the platform schedules for one synchronous destination of
  one completed step; it has its own effect key, attempts, lease and outcome, and is shown in the run
  graph and journal as derived from the source step.
- **Destination kinds**: `iceberg`, `postgresql`, `object_storage`, `clickhouse`, `broker_stream`.
- **Failure policy**: what the run does when a destination write fails for good: `hold` (wait for a
  person), `fail` or `continue`.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Register an Iceberg connection and test it (Priority: P1)

A namespace administrator registers the company's lakehouse as an Iceberg connection: the catalog
(for example a REST catalog URL and warehouse, or a Glue region and account), the storage, the
credentials (created in OpenBao from the form or selected from existing secrets), the warehouse
locations writes may go to, the write modes the connection allows and its classification ceiling.
They test it and see whether the catalog and storage were reachable and the credentials accepted.

**Why this priority**: no write can run without a connection; it is also the security boundary
(credentials, destinations, allowed modes, classification).

**Independent Test**: create a connection for each catalog and storage type against the repository's
local fixtures, run the probe, reload the form, and check that no credential value ever comes back.

**Acceptance Scenarios**:

1. **Given** an administrator in namespace A, **When** they create an Iceberg connection with a REST
   catalog, S3-compatible storage and a new OAuth2 client secret, **Then** the secret value is stored
   in OpenBao, the connection keeps only references and non-secret settings, and reloading or editing
   returns metadata only.
2. **Given** a saved connection, **When** the administrator chooses *Test connection*, **Then** a
   fixed probe runs in the managed executor and reports, per check (catalog authentication, warehouse
   resolution, storage access), a safe result with the tested revision and time; saving alone is never
   shown as proof of connectivity.
3. **Given** a connection whose allowed modes are `append` only, **When** an author publishes a
   workflow with an `upsert` step on it, **Then** publication is refused with a typed error that names
   the disallowed mode.
4. **Given** a REST catalog that vends storage credentials, **When** the connection selects vended
   credentials, **Then** no static storage secret is required and vended credentials are never stored,
   logged or returned.

**Failure scenario**: wrong credentials, an untrusted TLS certificate, a destination outside the
deployment's allowed networks, an unavailable OpenBao and an unreachable storage produce distinct
typed categories without raw provider messages. Another tenant cannot discover the connection or its
secrets; a paused connection admits no new run.

---

### User Story 2 — Append workflow records to a table, never twice (Priority: P1)

An author adds an *Iceberg write* step after the steps that produce order records. Each run appends
its records to `sales.orders` as one commit. If the executor crashes after the commit, or the
response is lost, the retry finds the earlier commit by its effect key and reports it instead of
appending the rows again.

**Why this priority**: append is the most common use and the minimum viable feature; doing it without
duplicates under retries is what distinguishes a managed step from hand-written worker code.

**Independent Test**: run a workflow that appends a known record set, kill the executor at each
critical point, and check with an independent Iceberg reader that the table holds the rows exactly
once and the table history holds exactly one commit with the step's effect key.

**Acceptance Scenarios**:

1. **Given** a published workflow whose step appends 10,000 records from a blob produced by the
   previous step, **When** the run executes, **Then** one new snapshot appears on the target ref,
   carrying the effect key, an independent reader sees exactly the 10,000 rows, and the step output
   reports snapshot id, rows and files added.
2. **Given** the commit succeeded but the executor crashed before reporting, **When** the attempt is
   retried, **Then** the platform finds the commit by its effect key in the table history, records it
   as the step's outcome, and writes nothing.
3. **Given** another writer committed to the same table between the attempt's read and its commit,
   **When** the commit conflicts, **Then** the append is retried against the new table state within
   the attempt, within a bounded number of retries, and the result contains both writers' rows once.
4. **Given** a record whose `amount` has more decimal places than the table column allows, **When**
   the step runs, **Then** it fails with a typed schema error naming the column and the reason (never
   the value), and nothing is committed.
5. **Given** a record field classified `pii` and a connection whose ceiling is `internal`, **When**
   the step runs, **Then** it fails before any byte leaves the platform.

**Failure scenario**: the catalog accepted the commit request but the response timed out, and the
table's history has since been shortened by snapshot expiry so the lookup cannot prove absence. The
step does not write again: it ends in an explicit unknown outcome awaiting a person's audited
decision (constitution III).

---

### User Story 3 — Author the same write in the Studio and in every SDK (Priority: P1)

An author builds the step visually: chooses the connection, browses the catalog's namespaces and
tables, sees the table's schema, maps workflow fields to columns, chooses the mode and its options.
Another author writes the same step with an official SDK. Both publish the same canonical definition
and get the same behaviour, limits and errors.

**Why this priority**: the platform's contract is that visual and code authoring do not fork
semantics (constitution IV); without it the step is only half available.

**Independent Test**: publish one step from the Studio and from each official SDK, compare the
canonical definitions, run both, and compare the outputs and errors in a shared conformance scenario.

**Acceptance Scenarios**:

1. **Given** a connection the author may use, **When** they open the table picker, **Then** a bounded,
   audited listing of namespaces and tables comes from the catalog through the managed executor, and
   selecting a table shows its current schema, partition spec, identifier fields and format version.
2. **Given** the same step written in the Studio and in each official SDK, **When** both are
   published, **Then** the canonical definitions are identical and both runs produce equal outputs.
3. **Given** a step that maps a field to a column missing from the schema the picker last read,
   **When** the author edits it, **Then** the Studio warns at authoring time; publication itself does
   not contact the catalog (the API never does), and the run fails with the typed schema error if the
   column is still missing at execution.

**Failure scenario**: the author lacks use rights on the connection: the picker and publication are
refused exactly as for a missing connection, and the refusal is audited.

---

### User Story 4 — Replace a slice of a table (overwrite) (Priority: P2)

A daily workflow recomputes yesterday's aggregates and replaces them: the step overwrites the rows of
partition `day = 2026-10-26` with the new batch in one commit. Readers see either the old slice or the
new one, never a mix or an empty slice.

**Why this priority**: idempotent reprocessing of a slice is the second most common lakehouse write;
it depends on the connection and the append path.

**Independent Test**: write a slice twice with different contents and check that only the second
remains, that rows outside the scope are untouched, and that a reader concurrent with the commit sees
one whole version.

**Acceptance Scenarios**:

1. **Given** a step in `overwrite` mode with scope *partitions in the batch*, **When** the batch holds
   rows for two partitions, **Then** the commit replaces exactly those two partitions and leaves every
   other partition unchanged.
2. **Given** a step in `overwrite` mode with a declared filter `region = :region` bound to a workflow
   value, **When** a batch row does not satisfy the filter, **Then** the step fails before writing,
   because a row outside the scope would survive the next overwrite of the same scope.
3. **Given** an empty batch, **When** the step does not declare that an empty batch may delete the
   scope, **Then** it fails rather than deleting the scope's rows.
4. **Given** an unpartitioned table and scope *partitions in the batch*, **When** the step is
   published, **Then** publication is refused unless the step explicitly declares scope *whole table*.
5. **Given** another writer added rows inside the scope after the attempt's base snapshot, **When**
   the step commits, **Then** the conflict is detected, the attempt re-reads and re-validates within
   its retry bound, and otherwise fails with a retryable conflict error; it never deletes rows it did
   not see.

**Failure scenario**: the overwrite committed, the report was lost and the run was retried; the
lookup finds the commit by effect key and the second overwrite does not happen (a second overwrite
with the same rows would be harmless for the data but would add a snapshot and hide the real history).

---

### User Story 5 — Upsert records by key (Priority: P2)

A customer-sync workflow keeps `crm.customers` current: each run upserts the changed customers by
`customer_id`. Existing customers are replaced, new ones inserted, all others left as they are, in
one commit.

**Why this priority**: change propagation into a lakehouse table is a core use; it is riskier than
append and overwrite (row-level deletes, concurrency) and builds on both.

**Independent Test**: upsert batches with new and existing keys, then check with an independent reader
that each key has exactly one live row holding the latest values, and that untouched keys are
unchanged.

**Acceptance Scenarios**:

1. **Given** a table whose schema declares `customer_id` as its identifier field, **When** the step
   upserts 3 existing and 2 new customers, **Then** one commit leaves exactly one live row per key with
   the batch's values, and every other row is unchanged.
2. **Given** a table without identifier fields, **When** the step declares its key columns, **Then**
   they must be required (non-null) columns of the table, otherwise publication or execution is
   refused with a typed error.
3. **Given** a batch with the same key twice, **When** the step runs, **Then** it fails with a typed
   duplicate-key error and commits nothing.
4. **Given** a table whose format version the platform does not write (v3 in this delivery,
   D035-032), **When** the step runs, **Then** it fails with a typed unsupported-table error before
   writing any file.
5. **Given** two runs that upsert the same key concurrently, **When** both commit, **Then** the final
   table equals the result of applying the two commits in some serial order, and no key has two live
   rows.
6. **Given** a step that declares `deleted` as its delete marker and a batch with 2 changed customers
   and 1 record `{customer_id: 42, deleted: true}`, **When** the step runs, **Then** one commit
   replaces the 2 customers and removes customer 42, the output counts 2 replaced and 1 deleted key,
   and a marked key that does not exist in the table is counted as not found, not as an error.

**Failure scenario**: the executor dies after writing data and delete files but before the commit;
nothing is visible to readers, the retry starts again from a fresh base snapshot, and the files of
the abandoned attempt are removed by the platform when it can prove no snapshot references them, or
reported for the table owner's maintenance otherwise.

---

### User Story 6 — Undo a write when a saga compensates (Priority: P3)

An order saga appends shipment rows, then a later step fails. The compensation reverts the append: a
new commit removes exactly the rows that step added, leaving other writers' commits in place.

**Why this priority**: valuable for sagas (constitution III, ADR-0010) but not needed for the common
pipelines; it depends on the write modes.

**Independent Test**: run a saga that appends, overwrites or upserts and then fails, and check that the
reverted table equals the table before the step plus every commit made by others in between.

**Acceptance Scenarios**:

1. **Given** a write step that declares `revert` as its compensation, **When** the saga compensates
   and no later commit touched the files the step added or removed, **Then** a new commit undoes the
   step's changes and carries its own effect key.
2. **Given** that a later commit (for example a compaction or another writer's overwrite) rewrote or
   removed files the step added, **When** the revert runs, **Then** it does not guess: it fails and the
   compensation waits for a person's audited decision.
3. **Given** a write step without a declared compensation, **When** the saga compensates, **Then** the
   step is treated as irreversible and the run ends `PARTIAL`, as for any irreversible effect.

**Failure scenario**: the revert commit's response is lost; the retry finds it by its own effect key
and does not revert twice.

---

### User Story 7 — A finished payment is saved to the lake and to PostgreSQL inside the run (Priority: P2)

An author's checkout workflow has a *finish payment* step. They declare two synchronous destinations
on it: the Iceberg table `finance.payments` (append) and the PostgreSQL table `payments` of the
finance database (upsert by `payment_id`). When the payment step commits, the platform writes its
output to both, and the *send receipt* step that follows runs only after both writes succeeded. The
same payment can also flow to the lake asynchronously through a 020 sink; the author picks the mode
per destination.

**Why this priority**: it is the owner's motivating example; it builds on the write semantics of
US02–US05 and on 017's PostgreSQL connections, so it comes after them.

**Independent Test**: publish the checkout workflow with both destinations, run it, and check that
each destination holds the payment exactly once, that *send receipt* started only after both
destination writes completed, and that the run graph shows the two destination writes derived from
*finish payment*.

**Acceptance Scenarios**:

1. **Given** *finish payment* with an Iceberg and a PostgreSQL destination, **When** the step
   commits, **Then** the same transaction schedules one destination write per destination, both run
   in the managed executor in parallel, and the step's dependents become ready only when both
   succeeded.
2. **Given** a destination that maps output fields to columns, **When** the output has an `amount`
   classified `internal` and a `cardHolder` classified `pii` and the PostgreSQL connection's ceiling
   is `internal`, **Then** publication is refused because the mapping sends `cardHolder` above the
   ceiling; mapping only `amount` publishes.
3. **Given** the PostgreSQL destination in `upsert` mode and a crash after the database committed but
   before the platform recorded it, **When** the destination write is retried, **Then** the same row
   is upserted again with the same values and the table holds one row for the payment.
4. **Given** the lake is unreachable and the destination's failure policy is `hold` (the default),
   **When** the destination write exhausts its retries, **Then** the run waits for a person, who can
   retry or skip the destination with an audited decision; the payment is never charged again and
   never compensated because of the lake.
5. **Given** the failure policy `continue`, **When** the destination write fails for good, **Then**
   the dependents run, the failure stays visible on the run and can be redriven later.
6. **Given** an output with an array `items` of 500 line items, **When** the destination declares
   `items` as its record set, **Then** the 500 records are written in that destination's single write
   (one Iceberg commit, one database transaction).

**Failure scenario**: the payment step is retried after a lost lease and completes on its second
attempt; destination writes are scheduled only by the completion that is accepted, so the payment is
written once per destination, never once per attempt.

---

### User Story 8 — Step outputs go to object storage, ClickHouse and broker streams inside the run (Priority: P3)

The same author also wants each finished payment as a Parquet file in the finance bucket, as a row in
the ClickHouse analytics table, and as a message on the `payments.finished` Kafka topic, each written
before the receipt is sent.

**Why this priority**: the owner asked for all five kinds in both modes; these three need new
connection kinds or reuse of the broker pipeline and have weaker recovery guarantees than Iceberg and
PostgreSQL, so they come last.

**Independent Test**: run the checkout workflow with the three destinations against the repository's
fixtures, inject crashes and lost responses, and check each destination's content and the platform's
recorded outcomes.

**Acceptance Scenarios**:

1. **Given** an `object_storage` destination, **When** the destination write runs, **Then** one object
   is written under the connection's prefix with a name derived from the effect key, never
   overwriting an existing object; a retry that finds the object with the same checksum records
   success, and one that finds a different checksum fails for review.
2. **Given** a `clickhouse` destination, **When** the destination write is retried after a lost
   response, **Then** the insert carries the same deduplication token, so a server that deduplicates
   inserts keeps one copy; when the retry may fall outside the server's deduplication guarantee, the
   outcome is unknown and goes to review instead of inserting again.
3. **Given** a `broker_stream` destination on a Kafka topic, **When** *finish payment* commits,
   **Then** the delivery rows for its records commit in that same transaction, each message carries a
   stable message id, the dependents wait until every delivery is acknowledged, and a delivery that
   ends `DEAD` applies the failure policy.
4. **Given** a record larger than the broker delivery bound, **When** the workflow is published or the
   record is produced, **Then** it is refused with a typed limit error naming the bound.

**Failure scenario**: a broker accepted a message but its acknowledgement was lost; the delivery is
retried with the same message id and the consumer may see it twice (ADR-0025); the platform says so
and never claims exactly-once delivery.

---

### Edge Cases

- **History too short to decide**: the table's retained history no longer covers the time since the
  first attempt began (snapshots expired) — the lookup is inconclusive, so the outcome is unknown and
  goes to manual review; the step never assumes absence.
- **Effect key on another ref**: a commit with the step's effect key exists on a different branch than
  the target ref — treated as found only on the target ref; on another ref it is an inconsistency that
  fails the step for review.
- **Target ref deleted or replaced** between attempts — the step fails with a typed error; it never
  creates, fast-forwards or deletes refs.
- **Schema changed** by another writer between publication and execution (column added, type widened,
  column dropped) — records are validated against the schema current at execution; a dropped mapped
  column fails the step; the schema id used is recorded in the output.
- **Partition spec evolved** — the write uses the table's current default spec; old partitions keep
  theirs. An overwrite scope by partition applies to the partitions as the current spec computes them,
  and the scope semantics are stated in the contract.
- **Table relocated** or a catalog response that points data or metadata to a location outside the
  connection's warehouse locations — refused before any write (prevents a hostile or misconfigured
  catalog from redirecting writes).
- **Catalog configuration overrides** a destination (endpoint, region) — accepted only if the
  resulting destination passes the same network policy as the connection's own settings.
- **Vended credentials expire mid-write** — renewed within the attempt if the catalog allows;
  otherwise the attempt fails retryable with nothing committed.
- **Empty batch** in append mode — succeeds without a commit and says so in the output; in overwrite
  mode see US04 scenario 3.
- **Very large batch** above the step, namespace or platform ceiling — refused before any file is
  written, with the ceiling named.
- **Values Iceberg cannot store losslessly** (decimal scale overflow, integer overflow, timestamp
  without zone into a zoned column, NaN in a non-floating column, string into binary) — refused, never
  rounded, truncated or coerced silently.
- **Nested types** (struct, list, map) map recursively; a nested field missing from the table fails.
- **Cancellation during a write** — before the commit request: nothing is committed and the attempt's
  files are cleaned up; after the commit request was sent: cancellation cannot undo it, and the
  outcome is looked up and recorded (a committed write is never reported as cancelled).
- **Lease lost** (stale attempt) — a stale attempt's report is rejected (constitution I); its commit,
  if it happened, is found by the next attempt's lookup. If the stale commit and the successor's
  commit race, the conditional commit of FR-035-018 makes the later one fail and search again, so only
  one of them lands.
- **Table format versions, data file formats or features the platform does not write** (for example
  a table whose default file format is not supported, or row-level semantics the platform cannot
  honour) — refused with a typed unsupported-table error before writing.
- **Connection revision changed** after admission — the run keeps its pinned revision; revoked
  access or a paused connection still blocks credential resolution for new attempts (017 semantics).
- **JDBC catalog pointing at the platform's own database** or any platform state store — refused by
  the network policy.
- **Some destinations succeed and one fails** — the successful writes stay (they are not undone);
  only the failed destination is retried, held, skipped or redriven; the dependents follow the
  failure policy of the failed destination.
- **The source step fails, is cancelled or is retried** — no destination write is scheduled until a
  completion is accepted; a stale attempt's completion schedules nothing.
- **A run is rerun** (a new run) — its steps have new effect keys, so destinations receive the data
  again; this is expected and documented.
- **A saga compensates the source step** — synchronous destination writes are not reverted
  automatically; an Iceberg destination may declare `revert` (FR-035-021), every other kind is
  irreversible and the compensating saga ends `PARTIAL` (constitution III).
- **The output is a blob descriptor** — a synchronous destination may read the records inside the
  referenced record blobs (FR-035-009 limits apply); 020's asynchronous export keeps exporting only
  the descriptor.

## Requirements *(mandatory)*

### Functional Requirements

**Connection**

- **FR-035-001 — Iceberg connection kind**: The 017 connection catalog MUST offer a connection kind
  `iceberg` with the same lifecycle as existing kinds: namespace scope, unique logical name, immutable
  revisions with optimistic concurrency, pause, guarded deletion, pinning at run admission, promotion
  by logical name and audit. Existing connection kinds and their APIs are unchanged.
- **FR-035-002 — Catalog types**: A connection MUST declare exactly one catalog type with its
  non-secret settings and authentication:
  - `rest`: catalog URI, warehouse, optional prefix; authentication by OAuth2 client credentials,
    bearer token or request signing for cloud-hosted REST catalogs; TLS verification mandatory outside
    an explicit development mode.
  - `glue`: region, catalog (account) id; cloud credentials or an assumed role with external id.
  - `hive`: metastore URI(s); TLS where the metastore offers it; an unauthenticated metastore only on a
    deployment-approved private network with an explicit operator setting.
  - `jdbc`: a catalog database under 017's database TLS and credential rules.
  Unknown settings, credentials embedded in URIs and unsupported catalog types MUST be refused.
- **FR-035-003 — Storage types and credentials**: A connection MUST declare its storage type (`s3`
  including S3-compatible endpoints with path-style access, `gcs`, `adls`) and how storage credentials
  are obtained: static credentials held in OpenBao (access keys, assumed role, service-account key,
  account key, SAS token or service principal), or credentials vended by a REST catalog. Vended
  credentials MUST be treated as secrets: never persisted, logged, journalled, audited or returned.
- **FR-035-004 — Destination confinement**: A connection MUST declare the warehouse locations
  (storage prefixes) writes may target. Every data, delete and metadata location the platform writes,
  and every location a catalog response points it to, MUST lie within those prefixes, and every host
  the executor contacts (catalog, token endpoint, storage, metastore, catalog database, including
  hosts from catalog configuration overrides or vended endpoints) MUST pass the managed executor's
  network policy (FR-017-013): no loopback, link-local, metadata or platform state endpoints, checked
  on the resolved address and the actual connection, no redirects outside policy.
- **FR-035-005 — Allowed modes and classification ceiling**: A connection revision MUST declare which
  write modes it allows (default: `append` only) and the highest payload classification it may receive
  (`public` or `internal` by default; `pii` only when set by a holder of the 016 reveal grant for that
  namespace; `secret` never). Changing either is a new audited revision.
- **FR-035-006 — Probe**: *Test connection* MUST run a fixed platform-defined probe in the managed
  executor through the same credentials, network policy and quotas as a write: authenticate to the
  catalog, resolve the warehouse, and check storage access within the declared locations without
  writing business data. It reports a safe result per check, the tested revision and the time; it is
  evidence at that instant, not a health guarantee.
- **FR-035-007 — Catalog browsing**: Authors with use rights MUST be able to list namespaces and tables
  (bounded, paginated) and read one table's current schema, partition spec, identifier fields, refs and
  format version through the managed executor. Browsing is audited, rate-limited and returns no row
  data and no file locations.

**Step contract and data**

- **FR-035-008 — Canonical step contract**: An `icebergWrite` step MUST be defined once in the
  canonical workflow contract (additive to the IR, OpenAPI and protocol contracts): connection
  reference, table identifier, target ref, mode and mode options, record source, field-to-column
  mapping, limits and compensation. The Studio and every official SDK MUST emit exactly that contract;
  neither invents retry, authorization or serialization semantics; a field outside the contract is
  refused at publication.
- **FR-035-009 — Record sources**: A step MUST accept records from (a) an inline list of records
  evaluated from workflow values, within the payload limit; (b) one or more record blobs produced by
  earlier steps of the same run — at most 32 blobs, each within the 64 MiB blob ceiling, at most 2 GiB
  in total per step — in newline-delimited JSON or Parquet, read through the blob claim check
  (ADR-0023) with only the blobs the server-evaluated step names, and streamed into files, never held
  whole in memory; (c) a 017 database result of an earlier step. All sources of one step end in the
  same single commit. Data snapshots of draft feature 024 are not a source until 024 is accepted.
- **FR-035-010 — Schema validation and type mapping**: Records MUST be validated against the table
  schema current at execution and converted by one published mapping table covering every Iceberg
  primitive type the platform supports and nested struct, list and map types. The mapping MUST be
  lossless or refuse: no rounding, truncation, overflow, zone guessing or silent coercion. Large
  integers and decimals use the existing five-language representation (FR-017-016). Missing optional
  columns become null; missing required columns and unmapped columns are refused with a typed error
  that names the column and the reason, never the value. A record field with no table column (other
  than a declared delete marker) is refused the same way, unless the step declares that unknown fields
  are ignored: then such fields are dropped before any check or write, never leave the platform, and
  are counted in the output. The step never adds, drops or alters a column.
- **FR-035-011 — Table compatibility**: The platform MUST check, before writing any file, that it can
  write the table correctly: format version, default data file format, partition transforms of the
  current spec, identifier fields and the row-level semantics the mode requires. A table it cannot
  write correctly is refused with a typed unsupported-table error naming the reason. A table that does
  not exist fails the step with a typed table-not-found error. The step never creates a table and
  never changes a table's schema, partition spec, sort order or properties.
- **FR-035-012 — Target ref**: A step MAY declare an existing branch as its target ref (default
  `main`). The step MUST NOT create, fast-forward, replace or delete refs, and a missing ref fails the
  step.

**Write semantics**

- **FR-035-013 — One atomic commit per step**: A successful attempt MUST produce at most one commit
  on the target ref, so readers see all of the step's changes or none. The commit's snapshot summary
  MUST carry the step's effect key, the run id, the step id, the workflow name and the published
  workflow version, and nothing else of the platform's (no values, no actor or tenant identity, no
  credentials). Because these identifiers are readable by anyone who can read the table's metadata,
  the Studio and the documentation MUST say so where an author picks the table.
- **FR-035-014 — Append**: `append` MUST add the batch's rows and nothing else. A conflict with a
  concurrent commit MUST be retried against the new table state, after the lookup of FR-035-018 over
  the new commits, within a bounded number of commit retries per attempt.
- **FR-035-015 — Overwrite**: `overwrite` MUST replace the rows of exactly one declared scope with the
  batch, in one commit: *partitions present in the batch*, a *filter* over columns with typed
  parameters bound to workflow values, or the *whole table* (declared explicitly). Every batch row
  MUST satisfy a filter scope. An empty batch deletes the scope only if the step declares it. Scope
  *partitions in the batch* on an unpartitioned table requires scope *whole table*.
- **FR-035-016 — Upsert**: `upsert` MUST, in one commit, insert the batch's rows and remove every
  existing row with the same key, leaving one live row per key. The key is the table's identifier
  fields or, when the table has none, the step's declared key columns, which MUST be required columns.
  The step MAY declare a delete marker: a boolean record field that is not a table column; a record
  whose marker is true removes the live row with its key and inserts nothing, and only its key fields
  are validated and written. A batch with duplicate keys, counting marked and unmarked records
  together, MUST be refused. Row-level changes are written copy-on-write, which every
  spec-conforming reader reads correctly; a table that prefers merge-on-read is written copy-on-write
  too and the receipt says so (amended 2026-10-27, D035-032).
- **FR-035-017 — Isolation against concurrent writers**: `overwrite` and `upsert` MUST validate at
  commit that no commit since the attempt's base snapshot added or deleted rows inside the overwritten
  scope or with the upserted keys; on such a conflict the attempt re-reads from the new state and
  re-validates within its retry bound, otherwise it fails with a retryable conflict error. The final
  table MUST equal the result of some serial order of the commits; the step never removes a row it
  did not see.
- **FR-035-018 — Recovery by lookup**: Before an attempt commits, and whenever an earlier attempt of
  the same step may have committed (retry, stale lease, lost response, timeout), the executor MUST
  search the target ref's retained history for a commit carrying the step's effect key. Found: that
  commit is the step's outcome and nothing is written. Not found and the retained history provably
  covers the period since the first attempt began: the attempt may write. Otherwise the outcome is
  unknown (FR-035-019). Every commit request MUST be conditional on the exact table state the attempt
  last searched; if the table moved, the attempt searches the new commits again before any retry. A
  stale attempt and its successor therefore cannot both commit, in either order and whatever the
  timing, and this MUST NOT rely on lease timing alone. An attempt whose lease is lost MUST stop
  sending commit requests.
- **FR-035-019 — Unknown outcomes**: An attempt whose commit outcome cannot be established (commit
  sent without a response and an inconclusive lookup, history shortened by expiry, effect key found
  on another ref) MUST NOT be repeated: it enters the existing unknown-outcome and manual-review path
  (003, FR-017-018), where an authorized person records an audited decision.
- **FR-035-020 — Files of attempts that did not commit**: Data and delete files written by an attempt
  that did not commit MUST NOT become visible. The platform MUST delete them when it can prove that no
  snapshot of the table references them and that it wrote them, and otherwise record them in the
  step's diagnostics (count and total size only) for the table owner's orphan-file maintenance. After
  an executor crash the platform cannot know which files were written: they are left to that
  maintenance and the next attempt's receipt says an earlier attempt may have left files (amended
  2026-10-27, D035-031). It MUST never delete a file it did not write.
- **FR-035-021 — Revert compensation**: A write step MAY declare `revert` as its compensation. A
  revert MUST be a new commit that removes the files the step's commit added and restores the files it
  removed, only if no later commit on the target ref rewrote or removed any of them and the restored
  files still exist; otherwise it fails and waits for a person's audited decision. A revert has its
  own effect key and lookup, and MUST NOT move the table to an older snapshot (that would discard other
  writers' commits). A step without a declared compensation is irreversible: a compensating saga ends
  `PARTIAL` (constitution III).
- **FR-035-022 — Cancellation**: Cancelling a run MUST stop an attempt before its commit request when
  possible; once a commit request was sent the outcome MUST be looked up and recorded, and a committed
  write MUST never be reported as cancelled or rolled back.

**Execution, security and classification**

- **FR-035-023 — Managed execution**: Catalog, metastore, catalog-database and storage I/O MUST run
  only in the managed executor under the standard lease, fencing, heartbeat, cancellation and result
  protocol (FR-017-012). The API, scheduler and engine processes never contact a catalog or storage.
  The executor receives only the credentials of the pinned connection revision, holds no orchestration
  database credentials, and runs under finite CPU, memory, disk, network and concurrency budgets. Code
  that parses catalog metadata and data files from customer storage MUST run per attempt in an
  isolated sandbox (namespaces, seccomp, cgroups, no network route but the executor's policy-enforcing
  relay), never sharing a process, identity, network or files with the executor or another attempt
  (amended 2026-10-27, D035-048).
  Arbitrary SDK workers cannot claim these tasks or resolve these credentials.
- **FR-035-024 — Authorization**: Managing connections, using a connection in a published step,
  browsing a catalog, running ad hoc writes, and deciding unknown outcomes MUST be separate rights
  reusing 017's role matrix and current-role checks, revalidated at publication, admission and each
  capability use. Foreign resources MUST be indistinguishable from missing ones, and denials audited.
- **FR-035-025 — Classification at egress**: Before any byte leaves the platform, every written field
  MUST be checked against its 016 classification: `secret` is always refused; a field above the
  connection's ceiling is refused; an undeclared field counts as `pii` (016 restrictive reading). A
  step or destination MAY declare a record schema with per-field levels; it annotates egress only and
  is not a disclosure carrier. A written field's level is the stricter of the level inherited from its
  source (the source step's output schema) and the declared one, so a declaration can never lower an
  inherited level; values of a 017 database result count as `pii` (amended 2026-10-27, D035-039,
  revised by D035-051). The refusal names
  the field path and class, never the value.
- **FR-035-026 — Secrets**: Catalog, metastore, catalog-database and storage credentials MUST be
  OpenBao-backed references resolved per attempt through the 017 secret facade; values never enter
  workflow definitions, settings, journal, logs, spans, audit, outputs or API responses.

**Output, limits, errors, observability, delivery**

- **FR-035-027 — Output**: A step's output MUST be a bounded receipt: mode, target ref, snapshot id,
  sequence number, schema id, rows added and removed, keys marked for deletion that were not found,
  data and delete files added and removed, bytes written, the number of ignored field names and of
  values dropped (no names of fields, as they can describe data), whether the outcome was found by
  lookup, and the effect key. It contains no row values and
  no file locations (file paths can embed partition values, which are data).
- **FR-035-028 — Limits**: Records, bytes, columns, nesting depth and files per step; synchronous
  destinations per step and records per destination write; commit retries;
  catalog, storage and commit deadlines; concurrent writes per connection, namespace and tenant; and
  browse page sizes MUST have finite defaults and ceilings, the effective limit being the minimum of
  platform, namespace and step values. The platform ceiling for record input is 2 GiB in at most 32
  blobs per step (FR-035-009). A batch above a limit is refused before any file is written.
- **FR-035-029 — Typed errors**: Failures MUST use the canonical error envelope with a closed set of
  codes, additive to the existing set, distinguishing at least: refused configuration, authentication,
  TLS, destination denied, location outside warehouse, table or ref not found, unsupported table,
  schema mismatch, duplicate key, scope violation, classification refused, limit exceeded, commit
  conflict (retryable), catalog or storage unavailable (retryable) and unknown outcome, and for
  destinations also missing key constraint, deduplication not guaranteed, object conflict, delivery
  bound exceeded and destination failed. No provider message, value or file location is returned.
- **FR-035-030 — Observability and audit**: The platform MUST emit traces and metrics for probe,
  browse, validation, file writing, lookup, commit, cleanup and every destination write, with counters
  for commits and destination writes by kind, mode and outcome, rows, bytes, conflicts, lookups found,
  held destinations and unknown outcomes. Metric labels are closed sets (destination kind, mode,
  catalog type, storage type, outcome), never connection, table, run or tenant identifiers.
  Logs and spans may carry connection id, table identifier and snapshot id, never values, partition
  values, file locations or credentials. Connection changes, probes, browsing, writes, reverts and
  unknown-outcome decisions are audited.
- **FR-035-031 — Ad hoc execution**: An authorized user MUST be able to run an Iceberg write as an ad
  hoc step through 017's ad hoc admission (FR-017-015), with the same rights, limits, lookup and
  unknown-outcome rules.
- **FR-035-032 — Self-contained fixtures and operations**: The repository MUST declare, with pinned
  images, loopback-only ports and healthchecks, the local catalog and storage services the tests need
  for every catalog and storage type (AGENTS.md rule 16, owner decision of 2026-10-17). Where only an
  emulator exists,
  the type MUST be labelled in the documentation as verified against that emulator, not the real
  service. A runbook covers connection setup per catalog, credential rotation, unknown-outcome review,
  revert failures, orphan files and table maintenance guidance; a demo shows the happy path and a
  recovered crash.
- **FR-035-033 — Compatibility**: All contract changes MUST be additive; runs and workflows that do
  not use the step are unaffected; a published step keeps its semantics across releases; an
  unsupported capability fails explicitly in every SDK.

**Synchronous step-output destinations**

- **FR-035-034 — Declaration**: Any root-level step that produces an output MAY declare synchronous
  destinations in the canonical contract (additive, FR-035-008 rules). Like 017's managed database
  nodes (ADR-0036 decision 3), destinations and `icebergWrite` steps are refused inside structured
  bodies and continuation combinations until per-instance credential pins are proven (amended
  2026-10-27, D035-038). A declaration names: a name unique in the step, the
  destination kind, the connection or broker-destination reference, the target (table, object
  prefix or topic), the mode, the record set (the output itself as one record, an array path in the
  output, or the record blobs the output references), the field-to-column mapping, the failure policy
  and limits. The Studio and every official SDK emit the same contract. The number of destinations per
  step is bounded (FR-035-028).
- **FR-035-035 — Scheduling and ordering**: The transaction that accepts a step's completion MUST also
  schedule one destination write per synchronous destination (constitution I); a completion that is
  not accepted (stale, failed, cancelled attempt) schedules nothing. The source step's dependents MUST
  become ready only when every destination write has succeeded or been resolved by its failure
  policy. Destination writes of one step run independently of one another.
- **FR-035-036 — Rules common to every destination write**: Each destination write MUST be a managed
  effect with its own stable effect key (derived from the source step's effect key and the
  destination's name), lease, fencing, heartbeat, retry policy, deadline, cancellation, audit and
  telemetry, run in the managed executor (FR-035-023). Records are derived from the accepted,
  immutable output under the pinned definition. Classification at egress (FR-035-025) applies against
  the destination's ceiling, checked at publication from the step's output schema and again at
  execution. The mapping is lossless or refuses (FR-035-010); limits (FR-035-028), unknown outcomes
  (FR-035-019), secrets (FR-035-026) and destination confinement and network policy (FR-035-004) apply
  to every kind. A refusal (classification, schema, limit, missing table or constraint) is permanent
  and is never retried.
- **FR-035-037 — Failure policy**: Each destination MUST declare, or default to, one policy for a
  write that failed for good: `hold` (default) — the run waits until an authorized person retries or
  skips the destination, audited; `fail` — the run treats the destination write as a failed step
  (error routing and saga rules apply); `continue` — the dependents run and the failure stays visible
  on the run; the destination step stays terminal, and a later redrive is an ad hoc execution of the
  same destination with the same effect key, so it cannot duplicate (amended 2026-10-27, D035-055).
  No policy re-executes the source step.
- **FR-035-038 — Iceberg destination**: FR-035-011..022 MUST apply with the selected record set as the
  record source and the modes the Iceberg connection allows.
- **FR-035-039 — PostgreSQL destination**: Through a 017 PostgreSQL connection into an existing table,
  in mode `insert` or `upsert`, with platform-generated statements only (no author SQL) and no DDL;
  all records of one destination write commit in one database transaction. `upsert` requires a key
  covered by a primary key or unique constraint. `insert` requires a declared key column — a record
  field or a column that receives the effect key — covered by such a constraint, and inserts only
  absent keys, so a retry adds nothing. The constraint MUST be verified before the first write;
  otherwise the write fails with a typed error. Values map through 017's typed cells (FR-017-016).
- **FR-035-040 — Object storage destination**: A new 017 connection kind `object_storage` with the
  storage types, credentials and confinement of the Iceberg connection's storage (FR-035-003,
  FR-035-004). A destination write MUST create one Parquet or NDJSON object under the connection's
  prefix, named from the workflow name, the destination name, the run's date and the effect key (no
  record value in the name). Creation is conditional (create-only) where the provider supports it,
  otherwise preceded by an existence check; an existing object with the same checksum is the write's
  success, one with a different checksum fails for review. It MUST never overwrite or delete an object.
- **FR-035-041 — ClickHouse destination**: A new 017 connection kind `clickhouse` (HTTPS interface, TLS
  verification mandatory outside an explicit development mode, OpenBao credentials, fixed probe).
  Mode `insert` into an existing table, with platform-generated statements only and no DDL; every
  attempt carries the effect key as the insert's deduplication token. A retry is safe only within the
  deduplication the server guarantees for that table; when the destination does not declare that the
  table deduplicates, or a retry may fall outside the guaranteed window, an uncertain outcome is
  unknown and goes to review instead of inserting again. No upsert is offered (the documentation shows
  the replacing-table pattern).
- **FR-035-042 — Broker stream destination**: A destination references an outbound broker destination
  of ADR-0025 (NATS, Kafka, SQS; Redis Streams once feature 033 merges — amended 2026-10-27,
  D035-037) dedicated to step outputs (it receives no run
  events) and that carries its own egress ceiling. Its records MUST come from the step's output or an
  inline expression, never from blobs, so no object storage is read inside a run transaction. The
  record data (stored as run content) and the delivery rows MUST commit in the transaction that accepts
  the source step's completion, which also re-checks the subscription: one message per record, within
  the delivery bound and a per-completion byte cap, with a stable message id derived from the effect
  key and the record's position (amended 2026-10-27, D035-052). The destination write succeeds when every
  delivery is acknowledged; a delivery that ends `DEAD` applies the failure policy; delivery, retry and
  redrive follow ADR-0025. Consumers may receive duplicates and the platform MUST NOT claim
  exactly-once delivery.
- **FR-035-043 — Destination receipts**: Each destination write MUST produce a bounded receipt (kind,
  outcome, counts, whether found by lookup or deduplication, and for Iceberg FR-035-027), with no
  record values and no data file locations, readable by later steps as the source step's destination
  results. The source step's own output is unchanged.
- **FR-035-044 — Asynchronous export through 020**: Asynchronous destinations MUST be 020 sinks
  selecting the step's declared record (FR-020-019); this feature adds no asynchronous pipeline. 020's
  `ICEBERG` sink MUST reuse this feature's Iceberg connection kind, its classification ceiling and the
  lookup of FR-035-018 with the batch identity as effect key. The Studio MUST show a step's synchronous
  destinations and the 020 sinks that select its declared record together, each labelled with its
  mode.
- **FR-035-045 — Destination rights**: Declaring a destination requires use rights on its connection
  (and write rights for a database-backed write) or, for a broker stream, the right to manage event
  subscriptions, revalidated at publication, admission and execution. Retrying a failed destination
  write requires the run-operation right; retrying or resolving one whose outcome is unknown, and
  skipping one, require the saga-override right; redriving one requires the ad hoc and write rights;
  every such decision is audited (amended 2026-10-27, D035-066, D035-069).

### Non-Functional Requirements

- **NFR-035-SEC**: zero cross-tenant or cross-namespace access, zero contact with a non-approved host
  (including catalog-supplied and vended endpoints), zero credential, vended-credential, value or
  partition-value canary leaks across logs, spans, audit, journal and API in the security battery.
  Any violation blocks release (constitution V).
- **NFR-035-DUR**: engine state, task, outbox and receipt changes keep the existing transactional
  boundaries; no exactly-once claim for the external commit; every attempt ends in one recorded commit,
  a recorded refusal, or an explicit unknown outcome.
- **NFR-035-INTEROP**: tables written by the platform MUST read back correctly with an independent,
  spec-conforming Iceberg reader for every supported mode, type, partition transform and format
  version.
- **NFR-035-OBS**: every write has persisted correlation, audit and bounded telemetry; telemetry export
  failures never decide a write's success.
- **NFR-035-PERF**: limits and throughput targets are chosen in the plan and measured (p50/p95/p99 of
  probe, lookup, file writing and commit) on a declared environment; they are benchmark criteria, not
  promises.

### Key Entities *(include if feature involves data)*

- **Iceberg connection revision**: catalog type and settings, storage type and settings, credential
  references (catalog, storage), vended-credential choice, warehouse locations, allowed write modes,
  classification ceiling, limits; immutable, pinned by runs; extends the 017 connection.
- **Iceberg write step definition**: connection reference, table identifier, target ref, mode and
  scope or key, record source, column mapping, limits, compensation; part of an immutable published
  version.
- **Write attempt record**: run, step, attempt and fence; effect key; base snapshot; the files the
  attempt wrote (for cleanup); commit state (`NOT_SENT`, `SENT`, `COMMITTED`, `NOT_COMMITTED`,
  `UNKNOWN`); stored with the attempt under the existing transactional rules.
- **Commit receipt**: the step output of FR-035-027, retained as workflow content under existing quotas.
- **Unknown-outcome review item**: the existing 003/017 manual-review item, with the lookup evidence
  (target ref, history window searched, whether the key was found elsewhere) and the audited decision.
- **Revert record**: the compensated step's receipt, the revert's own effect key and its outcome.
- **Synchronous destination declaration**: part of a step's immutable definition (FR-035-034).
- **Destination write**: derived from one accepted completion and one destination; effect key,
  attempts, outcome, failure-policy state (`HELD`, `SKIPPED`, `REDRIVEN`) and receipt (FR-035-043);
  for broker streams, the delivery rows it waits for.
- **Object storage connection revision** and **ClickHouse connection revision**: new 017 connection
  kinds with settings, credential references, ceilings and limits, pinned like other revisions.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-035-001**: For every supported catalog and storage combination, an administrator registers a
  connection and gets a probe result in under 30 seconds on the reference environment, and in 100% of
  reloads, edits and API responses no credential value is returned.
- **SC-035-002**: For each of append, overwrite and upsert, across a generated corpus covering every
  supported column type, nested type and partition transform, an independent Iceberg reader returns
  exactly the expected rows in 100% of cases, and no reader ever observes part of a step's write.
- **SC-035-003**: With a crash injected at each critical point (before the commit request, after it
  and before the response, after the response and before the platform records it), plus lost
  responses, duplicate deliveries, stale leases and restarts, 100% of runs end with exactly one commit
  carrying the step's effect key or an explicit unknown outcome; zero duplicate commits and zero
  duplicate rows.
- **SC-035-004**: With two to eight concurrent writers on one table (platform runs and an external
  writer), the final table equals some serial order of the commits in 100% of runs; after upserts no
  key has more than one live row; no overwrite removes a row outside its scope.
- **SC-035-005**: The same step authored in the Studio and in every official SDK publishes an identical
  canonical definition and yields identical outputs and errors in the shared conformance scenario.
- **SC-035-006**: The security battery finds zero cross-tenant access, zero egress to a non-approved
  host (including catalog-redirected locations and vended endpoints) and zero leaks of credentials,
  values, partition values or file locations.
- **SC-035-007**: 100% of `secret` fields, fields above the connection's ceiling and undeclared fields
  on a ceiling below `pii` are refused before any byte reaches catalog or storage.
- **SC-035-008**: In saga tests, 100% of reverts without interfering commits leave the table equal to
  its prior content plus the other writers' commits, and 100% of reverts with interfering commits wait
  for a person; none is silently skipped or partially applied.
- **SC-035-009**: On the declared reference environment, appending 1,000,000 narrow rows (about
  100 MiB) completes within 2 minutes at p95, and the effect-key lookup on a table with 10,000 retained
  snapshots adds under 5 seconds at p95, and a 2 GiB batch read from 32 blobs commits without the
  executor exceeding its declared memory budget (benchmark criteria, not promises).
- **SC-035-010**: Every catalog and storage type runs its acceptance tests against a fixture declared
  in the repository with no hand-started service, and 100% of types verified only against an emulator
  are labelled as such in the documentation.
- **SC-035-011**: 100% of batches above any limit are refused before a file is written; every error
  category of FR-035-029 is produced by at least one test, and none of them contains a provider
  message, value or file location; exported telemetry carries only the closed label sets.
- **SC-035-012**: Workflows that do not use the step pass the existing contract, integration and SDK
  suites unchanged, and an ad hoc write follows the same lookup, refusal and unknown-outcome behaviour
  as a published one.
- **SC-035-013**: For each of the five destination kinds, with crashes injected before and after the
  external write, lost responses, stale leases, restarts and retried source steps, 100% of runs end
  with each destination holding the output exactly once or an explicit unknown outcome (for broker
  streams: every message delivered at least once with a stable id); and in 100% of runs no dependent
  of the source step starts before every synchronous destination succeeded or was resolved by its
  failure policy.
- **SC-035-014**: The checkout demo shows a finished payment reaching an Iceberg table and a PostgreSQL
  table synchronously and the lake through a 020 Iceberg sink asynchronously; with the lake down, the
  synchronous destination holds the run for a person while the asynchronous sink lets the run finish
  and catches up after recovery, with no payment charged twice.

### Traceability of criteria to requirements

| Criterion | Requirements |
|---|---|
| SC-035-001 | FR-035-001..006, FR-035-026 |
| SC-035-002 | FR-035-009..011, FR-035-013..016, NFR-035-INTEROP |
| SC-035-003 | FR-035-013, FR-035-018..020, FR-035-022, FR-035-023, NFR-035-DUR |
| SC-035-004 | FR-035-014, FR-035-017 |
| SC-035-005 | FR-035-007, FR-035-008 |
| SC-035-006 | FR-035-004, FR-035-023, FR-035-024, FR-035-026, FR-035-030, NFR-035-SEC |
| SC-035-007 | FR-035-005, FR-035-025 |
| SC-035-008 | FR-035-021 |
| SC-035-009 | FR-035-028, NFR-035-PERF |
| SC-035-010 | FR-035-002, FR-035-003, FR-035-032 |
| SC-035-011 | FR-035-027..030, NFR-035-OBS |
| SC-035-012 | FR-035-012, FR-035-031, FR-035-033 |
| SC-035-013 | FR-035-034..043, FR-035-045 |
| SC-035-014 | FR-035-035, FR-035-037..039, FR-035-044 |

## Assumptions

- Tables, namespaces, warehouses, buckets and catalog accounts exist and are owned by the customer;
  the platform needs only the privileges to load tables, write files in the declared locations and
  commit.
- Table maintenance (snapshot expiry, compaction, orphan-file removal, sort optimisation) stays with
  the table owner; the runbook explains how expiry shortens the window the lookup can search and that
  frequent small commits produce small files.
- Parquet is the data file format written in this delivery; a table whose default file format is
  another one is refused (FR-035-011) and recorded as a gap.
- Kerberos-secured Hive Metastores are not in this delivery (recorded as a gap); TLS and
  private-network metastores are.
- The table's declared sort order is not enforced on write in this delivery; Iceberg permits unsorted
  writes.
- The effect key is the existing stable step idempotency key; no new identity is introduced.
- Payload classification (016) is enforced on the values the step reads; values in a 017 database
  result follow that feature's result classification.
- Official SDKs are the five of constitution IV plus any SDK merged before this feature's delivery.
- Destination tables, buckets and topics exist and are the customer's, as for Iceberg tables.
- The failure policy default `hold` keeps a source step's committed effect (such as a charged payment)
  from being compensated because a destination is down; `fail` exists for destinations whose data the
  rest of the run cannot do without.

## Out of scope

Creating or altering tables, namespaces, refs, buckets, topics or properties on any destination;
reading Iceberg tables into workflows; multi-table atomic commits, or atomicity across destinations;
table maintenance; deleting rows other than through an overwrite scope or an upsert delete marker;
ORC and Avro data files; Kerberos for Hive Metastore; Iceberg as authority for platform state; 024
data snapshots as a source before 024 is accepted; the asynchronous export pipeline (020 owns it,
including its `ICEBERG` sink); a resource type (021) built on these connection kinds; upsert into
ClickHouse; automatic revert of destinations other than Iceberg.

## Open items

Owner questions, gaps and decisions are registered in [research.md](research.md) (D035-001..,
G035-001.., O035-001..).
