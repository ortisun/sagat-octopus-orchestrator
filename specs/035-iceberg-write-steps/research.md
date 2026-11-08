# Research and registers — feature 035 Apache Iceberg write steps

Append-only registers (decisions `D035-NNN`, gaps `G035-NNN`, owner questions `O035-NNN`). Numbers are
feature-scoped, as in feature 033, so they cannot clash with the global ranges of other branches.
`/speckit-plan` adds the technical research below these registers; it does not renumber them.

## Owner decisions (2026-10-27)

Asked before the specification was written; recorded in `docs/owner-decisions.md`.

- **D035-001 — Write modes.** The owner chose append, overwrite and upsert (merge by key) for the
  first delivery, against the recommendation of append + overwrite with upsert later. Consequence:
  row-level deletes, key validation and concurrent-upsert isolation are in scope (FR-035-016,
  FR-035-017, US05).
- **D035-002 — Catalogs and storage.** The owner chose the broad set: REST, AWS Glue, Hive Metastore
  and JDBC catalogs; S3-compatible storage, Google Cloud Storage and ADLS Gen2 — against the
  recommendation of REST + S3-compatible first. Consequence: one fixture per type, emulator fidelity
  gaps (G035-003) and a larger network-policy surface (FR-035-004).
- **D035-003 — Destination model.** The Iceberg destination is a new connection kind of the feature-017
  catalog (recommended option). It reuses revisions, OpenBao secrets, probe, managed executor, network
  policy and ad hoc admission, and takes no dependency on the PR #53 drafts (020, 021, 024).

## Specification defaults (agent, reviewable at `/speckit-clarify`)

- **D035-004 — Tables are the customer's.** The step writes into existing tables and never changes a
  schema, partition spec, sort order, property or ref (FR-035-011, FR-035-012), mirroring 017's "no
  provisioning".
- **D035-005 — Effect key in the snapshot summary, lookup before writing.** An Iceberg commit is an
  atomic metadata swap, and the snapshot summary accepts writer properties; the stable step idempotency
  key goes there, and the executor searches the target ref's retained history for it before
  committing (FR-035-013, FR-035-018). Precedent to verify during planning: Flink's Iceberg sink
  records its last committed checkpoint id in the snapshot summary to avoid duplicate commits on
  recovery. The lookup is conclusive only within retained history; otherwise the outcome is unknown
  (FR-035-019). This is a reconciliation lookup in the sense of constitution III, not an exactly-once
  claim.
- **D035-006 — Revert is a new inverse commit, never a rollback of the table pointer.** Rolling the
  table back to an older snapshot would discard concurrent writers' commits (FR-035-021).
- **D035-007 — Destructive modes are opt-in per connection revision.** Default allowed modes are
  `append` only; overwrite and upsert need an administrator's explicit revision (FR-035-005).
- **D035-008 — Classification ceiling.** Default ceiling `internal`; `pii` only by a holder of the 016
  reveal grant; `secret` never; undeclared fields count as `pii` (016 restrictive reading, owner
  decision 2026-10-17 item 3) (FR-035-005, FR-035-025).
- **D035-009 — File locations are data.** Iceberg data file paths can embed partition values, so file
  locations never appear in outputs, logs, spans, audit or errors (FR-035-027, FR-035-030).
- **D035-010 — Parquet only in this delivery.** Tables whose default data file format is ORC or Avro
  are refused (FR-035-011, G035-002).
- **D035-011 — Catalog responses are untrusted.** Table locations, configuration overrides and vended
  endpoints returned by a catalog are confined to the connection's warehouse locations and pass the
  network policy (FR-035-004), so a hostile or misconfigured catalog cannot redirect writes.
- **D035-012 — Lookup and commit are tied by a conditional commit.** Every Iceberg catalog commits by
  compare-and-swap of the table's metadata pointer. Each commit request is conditional on the exact
  state the attempt last searched for its effect key, and a moved table means a new search before any
  retry (FR-035-018). This closes the race between a stale attempt and its successor without relying
  on lease timing. The plan must show that the client's own internal commit retries do not bypass the
  search (a client that silently re-applies an append to a newer state would reopen the race).

## Owner answers at `/speckit-clarify` (2026-10-27)

Recorded in the spec's Clarifications section and in `docs/owner-decisions.md`.

- **D035-013 — Batch size.** At most 2 GiB per step, in at most 32 record blobs of up to 64 MiB each
  (ADR-0023 blob ceiling), streamed into files (FR-035-009, FR-035-028, SC-035-009). Recommended
  option. This resolves the earlier inconsistency between a single-blob source (64 MiB) and the
  100 MiB benchmark of SC-035-009. The ceiling equals feature 024's snapshot ceiling.
- **D035-014 — Deletes in upsert.** An upsert step may declare a delete marker, a boolean record field
  that is not a table column; marked keys are removed in the same commit (FR-035-016, US05 scenario 6).
  Recommended option.
- **D035-015 — Unknown record fields.** Refused by default; a step may declare that they are ignored
  (dropped before any check, counted, never written). No schema change ever (FR-035-010,
  FR-035-027). Recommended option.
- **D035-016 — Snapshot summary identifiers.** Effect key, run id, step id, workflow name and published
  version; no values, actor or tenant identity, or credentials (FR-035-013). The owner chose the
  richer lineage option over the recommendation (key + run and step ids), accepting that workflow
  names become visible to readers of the table's metadata. Answers O035-001.
- **D035-017 — No table creation.** Confirms D035-004: a missing table fails with a typed error; the
  step never creates tables (FR-035-011). Recommended option.

## Scope extension: step-output destinations (owner, 2026-10-27)

Owner's words: "Quero que o output dos steps também possam ser salvas no iceberg, postgres, etc...
Exemplo. Acabei finalizar um pagamento, esse pagamento poderia descer para o lake". Asked as four
questions after the clarification session; recorded in `docs/owner-decisions.md`.

- **D035-018 — Both modes, chosen per destination.** A step's output may go to synchronous
  destinations (inside the run; dependents wait) and asynchronous ones (batched export after commit).
  The recommendation was asynchronous only, for the payment-to-lake example.
- **D035-019 — Five destination kinds, all in both modes.** Iceberg, customer PostgreSQL, object
  storage files, ClickHouse and broker streams. The recommendation for the synchronous mode was
  Iceberg and PostgreSQL only; the owner chose all five (FR-035-038..042, US08).
- **D035-020 — Asynchronous export is 020's.** Draft 020 already specifies declared records
  (FR-020-019) exported by sinks (`OBJECT_STORAGE`, `CLICKHOUSE`, `POSTGRESQL`, `STREAM`, `PULL`) from
  the journal with per-sink cursors; its US04 is the payment example. 020 gains an `ICEBERG` sink that
  reuses this feature's connection kind and lookup (recommended option). 020's pending constitution
  amendment concerns retention, not the export.
- **D035-021 — Synchronous destinations do not depend on 020.** They reuse this feature's record
  sources and mapping (FR-035-009, FR-035-010) over the step's output, not 020's declared-record
  contract, so they can be built before 020; the Studio shows both modes together (FR-035-044).
  Agent decision; reviewable.
- **D035-022 — Destination writes are derived steps scheduled by the accepted completion.** The
  completion transaction schedules them (constitution I); dependents wait for them (FR-035-035).
  Precedent: derived compensation steps (ADR-0010).
- **D035-023 — Default failure policy `hold`.** A destination outage must not compensate a source
  step's committed effect (a charged payment); `fail` and `continue` are explicit choices
  (FR-035-037). Agent decision; owner question O035-004.
- **D035-024 — Recovery per kind** (FR-035-039..042): PostgreSQL by a constraint-backed key in one
  transaction; object storage by an effect-key object name, create-only or checked, compared by
  checksum; ClickHouse by the effect key as the insert deduplication token, bounded by the server's
  guarantee; broker streams by delivery rows committed with the completion (ADR-0025) and a stable
  message id, at-least-once.
- **D035-025 — Two new 017 connection kinds**, `object_storage` and `clickhouse`; broker streams reuse
  ADR-0025/033 outbound destinations; PostgreSQL reuses the 017 kind.

## Gaps

- **G035-001** — Kerberos-secured Hive Metastores are not supported in this delivery.
- **G035-002** — ORC and Avro data files are not written; such tables are refused.
- **G035-003** — Emulator fidelity: Glue, GCS and ADLS Gen2 (and possibly the REST catalog flavours
  of cloud vendors) will be tested against local emulators or reference implementations; real-service
  verification needs cloud accounts the repository does not hold. Each such type is labelled
  "verified against <emulator>" until real-service evidence exists (FR-035-032, SC-035-010).
- **G035-004** — No atomic commit across several tables.
- **G035-005** — A revert cannot proceed after the table owner compacts, rewrites or expires the files
  involved; it waits for a person (FR-035-021). Expiry also shortens the lookup window (FR-035-018).
- **G035-006** — Table maintenance stays with the customer; frequent small commits produce small
  files, documented in the runbook but not mitigated by the platform.
- **G035-007** — The table's sort order is not enforced on write.
- **G035-008** — ClickHouse deduplication holds only for table engines that deduplicate inserts and
  within the server's deduplication window; outside it an uncertain insert goes to review
  (FR-035-041).
- **G035-009** — Broker streams are at-least-once: a consumer may see a message twice and must
  deduplicate by message id (FR-035-042, ADR-0025).
- **G035-010** — No atomicity across destinations: one destination can hold the data while another
  failed (edge cases); only Iceberg destinations can be reverted.
- **G035-011** — With synchronous destinations every run makes its own commit or insert; at high
  volume Iceberg gets many small commits and files. The asynchronous 020 sink batches; the runbook
  says which mode suits which volume.

## Owner questions

- **O035-001** — Should the snapshot summary carry the run and step identifiers beside the effect key
  (useful lineage, but platform identifiers become visible to everyone who can read the customer's
  table metadata) or the effect key only? Recommendation: effect key plus run and step ids, no actor
  or tenant identity (as written in FR-035-013); confirm at `/speckit-clarify`. **Answered
  2026-10-27 (D035-016):** the owner added the workflow name and version.
- **O035-002** — Priority of ORC/Avro (G035-002) and Kerberos Hive (G035-001) for a later delivery.
- **O035-003** — Default and maximum limits per step (records, bytes, files) and the reference
  environment of SC-035-009; the plan proposes values, the owner approves them. **Answered
  2026-11-02 (D035-070):** the plan's table is approved as written.
- **O035-004** — Confirm `hold` as the default failure policy of synchronous destinations (D035-023).
  **Answered 2026-11-02 (D035-070):** confirmed.
- **O035-005** — Should 020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference the new 017 connection
  kinds (`object_storage`, `clickhouse`) instead of carrying their own settings and secret, so a
  destination is configured once for both modes? Recommendation: yes (020's `POSTGRESQL` sink already
  uses a 017 connection, and its `ICEBERG` sink will). **Answered 2026-11-02 (D035-070):** yes.

## Sources to consult during planning

Not yet consulted for this specification; the plan records what was read and when: the Apache
Iceberg table specification (format versions, snapshots, summary, refs, row-level deletes, partition
transforms), the REST catalog OpenAPI (configuration, credential vending, commit requirements), the
AWS Glue, Hive Metastore and JDBC catalog commit protocols, and the Flink Iceberg sink's
duplicate-commit protection.

## Phase 0 — technical research (`/speckit-plan`, 2026-10-27)

Two read-only investigations ran on 2026-10-27: a map of the code on `origin/develop` (a164482f) that
this feature extends, and a web survey of Iceberg clients, ClickHouse deduplication, conditional object
writes and test fixtures. Every external fact below was checked against the source named; nothing here
is measured on this platform yet.

### Sources consulted (2026-10-27)

- iceberg-rust 0.10.1 (2026-08-01, MSRV 1.94): https://github.com/apache/iceberg-rust/releases/tag/v0.10.1;
  `crates/iceberg/src/transaction/{mod,append,snapshot}.rs`, `crates/catalog/{glue,sql,hms,rest}` at that tag.
- PyIceberg 0.12.0 (2026-09-01): https://pypi.org/project/pyiceberg/0.12.0/,
  https://github.com/apache/iceberg-python/releases/tag/pyiceberg-0.12.0, `pyiceberg/table/__init__.py`,
  `table/metadata.py`, `io/__init__.py` at that tag.
- Iceberg Java 1.12.0 (2026-09-30): `docs/docs/configuration.md`, `SnapshotProducer.java`; Flink sink
  `IcebergFilesCommitter.java` and `SinkUtil.java` at apache-iceberg-1.12.0 (snapshot-summary
  idempotency precedent, D035-005 confirmed).
- ClickHouse 26.8 LTS (v26.8.34.13-lts, 2026-10-22): `src/Core/Settings.cpp`, `MergeTreeSettings.cpp`,
  `SettingsChangesHistory.cpp`; https://clickhouse.com/blog/clickhouse-release-26-01.
- Conditional creates: https://aws.amazon.com/about-aws/whats-new/2024/08/amazon-s3-conditional-writes,
  https://docs.cloud.google.com/storage/docs/request-preconditions,
  https://learn.microsoft.com/en-us/rest/api/storageservices/specifying-conditional-headers-for-blob-service-operations,
  https://developers.cloudflare.com/r2/api/s3/api/, https://github.com/minio/minio (archived).
- Fixture images on Docker Hub, Quay and MCR (tags and push dates in D035-041).

### Facts that drive the design

- **iceberg-rust 0.10.1 only appends to `main`.** Fast append only; no released overwrite,
  replace-partitions, row delta or deletion vectors (open PRs #3300, #2203); `MAIN_BRANCH` is
  hard-coded; the Hive Metastore catalog cannot commit (`update_table` returns `FeatureUnsupported`);
  the REST `storage-credentials` list is parsed but unused; commits retry internally (default 4) and
  only table properties disable that.
- **PyIceberg 0.12.0 covers the owner's scope** except v3: `append`, `overwrite(overwrite_filter)`,
  `dynamic_partition_overwrite`, `delete`, `upsert`, each with `snapshot_properties=` and `branch=`;
  REST, Glue (`VersionId`), Hive (metastore locks), SQL catalogs; s3, gs, abfs/abfss via PyArrow or
  fsspec; REST `storage-credentials` used. Upsert and delete are copy-on-write (a table asking for
  merge-on-read gets copy-on-write with a warning). "Writing V3 is not yet supported". Since 0.12.0
  commits retry internally (refresh and rebuild, same snapshot id, stop if that snapshot landed),
  controlled only by `commit.retry.*` table properties. A `RecordBatchReader` input is consumed by the
  first attempt.
- **Java 1.12.0** is the most complete (merge-on-read, row delta, v3, every catalog) but brings a JVM.
- **ClickHouse**: `insert_deduplication_token` works for MergeTree and ReplicatedMergeTree, per
  partition; replicated windows default to 10,000 blocks and 3,600 s; plain MergeTree defaults to no
  deduplication (`non_replicated_deduplication_window = 0`); since 26.2 `async_insert` defaults to true
  and `deduplicate_insert` covers both paths.
- **Create-only writes**: S3 `If-None-Match` (2024-08-20), GCS `ifGenerationMatch=0`, Azure
  `If-None-Match: *`; R2 not on multipart completion; MinIO community is archived; RustFS support is
  unverified.
- **Fixtures**: moto's Glue ignores `VersionId` (conflicts untestable); LocalStack Glue is paid;
  Azurite has no ADLS Gen2/DFS endpoint.
- **Code on develop** (paths in plan.md): `ConnectionConfig` has one `Postgres` variant and the driver
  lives in `config` JSON; the database node is an `Activity` with reserved activity
  `sagat.database.postgresql` (prefix `sagat.database.`) run by `apps/database-runner` under
  `Role::DatabaseRunner`; derived steps exist for compensation/reconciliation (`<id>~c`, `<id>~r`,
  `NodeOrigin`, `steps.kind`); unknown outcomes are `OUTCOME_UNKNOWN` with the step `WAITING`
  (`RECONCILIATION`/`OPERATOR`) and run operations `RETRY_STEP`/`RESOLVE_STEP(SUCCEEDED|FAILED)`;
  outbound deliveries are `webhook_deliveries` rows (64 KiB `CHECK`, unique per subscription and event)
  with destination kinds WEBHOOK, NATS, KAFKA, SQS; Redis Streams (033) is not merged; no
  classification-ceiling check exists; no parquet, arrow, iceberg or ClickHouse crate is in the
  workspace; `object_store` 0.14.2 is; worker protocol minor is 6 and drafts 020, 021, 025 and 026 each
  plan 1.7; managed database execution is root-only (ADR-0036 decision 3).

### Decisions

- **D035-026 — Iceberg client: PyIceberg 0.12.0 in a pinned Python runtime.**
  *Rationale:* the only client that commits append, overwrite (filter and dynamic partitions) and
  upsert/delete to named branches on all four catalogs and three storages, with snapshot properties
  and REST credential vending. *Alternatives:* iceberg-rust 0.10.1 (append to `main` only, no Hive
  commits — fails D035-001/002; revisit when overwrite #3300 and RowDelta #2203 ship); Iceberg Java
  1.12.0 (complete, but a JVM in the executor, larger image and attack surface; kept as the fallback
  for v3 and merge-on-read, G035-015/016).
- **D035-027 — One runner binary, two deployments.** `apps/database-runner` keeps the `database`
  deployment (PostgreSQL operations, PostgreSQL and ClickHouse destinations) and gains a `lake`
  deployment built as a new image target `lake-runner`: the same binary plus the Python lake runtime
  (`runners/lake/`). Both run as `Role::DatabaseRunner` with activities under the reserved
  `sagat.database.` prefix, so ordinary workers can never claim them and no role is added (ADR-0036
  decision 5 keeps seven roles). *Alternatives:* a new `apps/lake-runner` binary and machine role
  (duplicates lease/credential code and changes the role catalog); per-attempt containers through
  `apps/sandbox-runner` (strongest isolation, but needs a networked container profile and 1–3 s
  start-up per step; recorded as the escalation path if a lake-runtime CVE class appears).
- **D035-028 — One Python subprocess per attempt.** *(Superseded by D035-048 after the security review.)* The runner spawns `python3 -I` on the pinned host
  program for each attempt; it is never shared between attempts, tenants or connections, receives the
  attempt's resolved credentials only on standard input, inherits no platform credential or token in
  its environment, works in a per-attempt directory on a size-limited volume, and is killed on lease
  loss, deadline or cancellation. Concurrency per pod is bounded by a memory admission budget.
  *Rationale:* the host parses untrusted catalog metadata, Avro manifests and Parquet footers from
  customer storage; a process boundary per attempt confines a parser compromise to that attempt's own
  credentials. *Alternative:* a long-lived Python worker shared by attempts (cheaper, but one hostile
  table could read another tenant's credentials from memory).
- **D035-029 — Egress of the lake runtime goes through a per-attempt relay.** *(Revised by D035-049.)* The host program
  configures every client (REST HTTP, Glue and S3 SDK endpoints, GCS, Azure, Hive Thrift, the JDBC
  catalog database) to connect through local ports owned by the Rust runner: an HTTPS `CONNECT` proxy
  and TCP relays. The runner applies 017's destination policy (`crates/infra-database/src/network.rs`)
  on every connection: resolve once, check every address, connect to the checked address, deny
  loopback, link-local, metadata and platform state endpoints. A deployment `NetworkPolicy` limits the
  pod to the operator-approved ranges as a second layer. Canary tests run each client with direct
  egress blocked to prove it only works through the relay (G035-018). *Alternative:* trust the
  libraries' own resolution (no enforcement of FR-035-004 against a catalog that redirects).
- **D035-030 — Commit protocol of the host program** (FR-035-013, FR-035-018, D035-012):
  1. Load the table; check compatibility (FR-035-011); read the target ref's head `H`.
  2. Search for the effect key by walking parent snapshots from `H` until a snapshot older than the
     first attempt's start minus 5 minutes (catalog clock margin). The search is *conclusive* when the
     walk reaches such a snapshot, or the table's first snapshot, with an unbroken parent chain;
     otherwise it is *inconclusive* (expiry broke the chain). The first attempt's start comes from the
     platform's attempt record (database clock), passed in the attempt input.
  3. Found → report the receipt of that commit; write nothing. Inconclusive → `OUTCOME_UNKNOWN`.
  4. Write data and delete files in the attempt's directory under the table location.
  5. Commit once with requirements `assert-table-uuid` and `assert-ref-snapshot-id(ref, H)`, and
     `snapshot_properties` `sagat.effect-key`, `sagat.run-id`, `sagat.step-id`, `sagat.workflow`,
     `sagat.workflow-version` on every snapshot of the commit (D035-016). PyIceberg's internal retry is
     neutralised by overriding `commit.retry.num-retries = 0` on the loaded table object in memory
     (never committed to the table); test T-commit-once proves exactly one commit request per search.
     If that override proves ineffective, the host builds the updates and calls the catalog's commit
     directly (G035-020).
  6. Conflict (the ref moved): back to step 2 over the new snapshots, re-validate overwrite and upsert
     scopes against them (FR-035-017), within the commit-retry bound. Commit state unknown (timeout or
     5xx): back to step 2; because both a delayed request and a new one assert the same `H`, at most one
     can land, and either carries the key.
  *Rationale:* conditional commits plus search-before-every-commit make duplicates impossible between
  any two attempts that follow the protocol, independent of lease timing; Flink's committer uses the
  same summary-marker idea.
- **D035-031 — Files of a crashed attempt are the table owner's.** The host deletes the files it
  wrote when its own commit is definitively refused; after a process or pod crash the platform cannot
  prove which files were written, so the next attempt's receipt flags `priorAttemptMayHaveLeftFiles`
  and the runbook points to orphan-file removal. Amends FR-035-020.
- **D035-032 — Copy-on-write for row-level changes; v3 refused.** Upsert, delete markers and filter
  overwrites are copy-on-write. A table that prefers merge-on-read is still written correctly for
  every spec-conforming reader; the receipt records `rowLevelMode: COPY_ON_WRITE` and the runbook
  states the cost. Format v3 tables are refused (`ICEBERG_UNSUPPORTED_TABLE`); v1 and v2 are written.
  Amends FR-035-016 and US05 scenario 4.
- **D035-033 — Records reach the host as files.** The runner reads the step's record blobs over the
  existing lease-checked `ReadBlob` and writes them, or the inline records, to the attempt directory;
  the host reads NDJSON and Parquet into Arrow batches and casts them to the table's Arrow schema
  with checked casts only. Append streams batches (`RecordBatchReader`); overwrite and upsert
  materialise the batch, so they have a lower default ceiling (D035-046).
- **D035-034 — PostgreSQL destination in the database deployment.** Fixed, platform-generated
  statements in `crates/infra-database`: column and constraint discovery by fixed catalog queries,
  quoted identifiers checked against the discovered columns, `INSERT … ON CONFLICT (<key>) DO NOTHING`
  for `insert` and `… DO UPDATE SET` for `upsert`, all records in one transaction with 017 deadlines.
  The key-backed statement is the declared, proven effect contract ADR-0035 decision 11 requires, so
  ambiguous commits are retried automatically.
- **D035-035 — ClickHouse destination in the database deployment.** HTTPS over the workspace
  `reqwest`; `INSERT INTO <db>.<table> (<columns>) FORMAT JSONEachRow` with
  `insert_deduplication_token = <effect key>`, `deduplicate_insert = 'enable'` and `async_insert = 0`
  (synchronous acknowledgement). The probe reads the table engine and deduplication settings from
  `system.tables` and `system.merge_tree_settings`; a retry is automatic only while the time since the
  first attempt is below the smaller of the declared and discovered window seconds minus a margin, and
  only for a table whose deduplication the probe confirmed; otherwise `OUTCOME_UNKNOWN`. Tested LTS:
  26.8.
- **D035-036 — Object storage destination in the lake deployment.** NDJSON is encoded by the runner,
  Parquet by the host (PyArrow); the upload uses `object_store` 0.14 (features `aws`, `gcp`, `azure`)
  with `PutMode::Create`. The probe writes a probe object twice and expects the second create to be
  refused; a provider that does not refuse is marked `conditionalCreate: false`, and writes then use
  an existence check plus the `sagat-sha256` metadata comparison. Object name:
  `<prefix>/<workflow>/<destination>/dt=<run admission date, UTC>/<hex sha-256 of the effect key>.<ext>`.
- **D035-037 — Broker stream destination through the engine's delivery pipeline.** *(Revised by D035-052 and D035-053.)* A new
  `OutboundKind::StepOutput`; `webhook_deliveries` gain nullable `run_id` and `step_key` linking a
  delivery to its derived destination step; the source step's completion transaction inserts one
  delivery per record (`event_id` = UUID v5 of the effect key and record index) on a subscription
  whose purpose is `STEP_OUTPUT` (it receives no run events, as 020's `STREAM` sink requires); the
  dispatcher transaction that settles a step's last delivery completes or fails the derived step.
  Kinds: NATS, Kafka, SQS (ADR-0025); Redis Streams once 033 merges (G035-017). Amends FR-035-042.
- **D035-038 — Destination writes are derived nodes, root-only.** *(Policies revised by D035-055; derivation point by D035-054.)* Publication derives one node
  `<source>~d.<name>` (`NodeOrigin::Destination`, `steps.kind = 'DESTINATION'`) per synchronous
  destination, between the source and its successors, as `derive_companions` does for compensation.
  The effect key is the source key plus `~d.<name>`. Following ADR-0036 decision 3, `icebergWrite`
  nodes and destinations are refused inside structured bodies (forEach, while, repeatUntil, scopes) and
  in continuation combinations until per-instance credential pins are proven (G035-019); amends
  FR-035-034. Failure policies map to existing states: `hold` → step `WAITING`/`OPERATOR` with
  `RETRY_STEP` or `RESOLVE_STEP` (`SUCCEEDED`, or the new `SKIPPED`, valid only for destination
  steps); `fail` → step `FAILED` and the normal failure routing; `continue` → step `SKIPPED` with the
  failure receipt, and `RETRY_STEP` stays allowed on it while the run is not terminal (the effect key
  is unchanged, so a redrive cannot duplicate).
- **D035-039 — Classification by declared record schema.** *(Superseded by D035-051.)* `icebergWrite` and each destination MAY
  declare a `recordSchema` with `x-sagat-classification`, a new member of 016's closed carrier set;
  undeclared fields are `pii`; a mapping from a source output field inherits that field's level from
  the source node's `outputSchema`. The ceiling check is static at publication and repeated at lease
  time against the pinned connection revision. A new domain function
  `classification::within(level, ceiling)` is added. Database results (017) are outside 016's
  carriers (ADR-0036 decision 6), so their fields count as `pii`.
- **D035-040 — Connection kinds and credentials.** `ConnectionConfig` gains `Iceberg`,
  `ObjectStorage` and `ClickHouse` variants (driver tags `iceberg`, `object_storage`, `clickhouse` in
  the existing `config` JSON; no new column). Every variant, PostgreSQL included, gains
  `egressCeiling` (default `internal`; existing revisions read as `internal`). `ResolvedDatabase`
  gains additive fields for the three new kinds; credentials keep the 017 two-phase OpenBao path.
- **D035-041 — Fixtures** (pinned tags at least two weeks old): `rustfs/rustfs:1.0.1` (S3),
  `apache/iceberg-rest-fixture:1.10.1`, `quay.io/lakekeeper/catalog:v0.14.0` (REST with vended
  credentials), `apache/hive:standalone-metastore-4.2.1`, the repository's PostgreSQL image (JDBC/SQL
  catalog), `motoserver/moto:5.2.4` (Glue, conflicts untestable — G035-012),
  `fsouza/fake-gcs-server:1.56.1`, `mcr.microsoft.com/azure-storage/azurite:3.37.0` (blob endpoint
  only — G035-013), `clickhouse/clickhouse-server:26.8.25.14`; images pinned by digest when added.
- **D035-042 — Typed failures.** New codes join the closed `DatabaseFailure` set (both `code()` and
  `parse()`), the REST `ErrorCode` set for publication refusals, and `sdks/error-catalog.json`; the list
  is in contracts/errors.md, with a new test that pins the full set.
- **D035-043 — Protocol minor.** The additive worker-protocol fields take the next free minor when this
  feature merges; 1.7 is claimed by drafts 020, 021, 025 and 026, so the number is assigned at merge
  and recorded in `SPECIFICATION-INDEX.md` (shared-concern registry).
- **D035-044 — One migration**, timestamp `20270115000000_lake_destinations` (after every branch's
  latest, `20261225000000`): `steps.kind` accepts `DESTINATION`; `webhook_deliveries` gains `run_id`,
  `step_key` and the `STEP_OUTPUT` kind; subscriptions gain `purpose`; quota resources and permissions;
  `sagat_app` grants; storage classification of every new variable-size column.
- **D035-045 — Ceilings** proposed in plan.md "Finite ceilings" (answers O035-003 as a proposal; the
  owner approves the values).
- **D035-046 — Lower default for materialising modes.** Overwrite and upsert default to 256 MiB of
  record input per step (ceiling 2 GiB, D035-013) because they hold the batch in memory; append
  streams and defaults to 1 GiB.
- **D035-047 — ADR-0071** records D035-026..030 and D035-038 as one decision (proposed).

### New gaps

- **G035-012** — Glue commit conflicts cannot be tested locally: moto ignores `VersionId`; SC-035-004
  for Glue needs a real AWS account.
- **G035-013** — Azurite has no ADLS Gen2 (DFS) endpoint; ADLS is tested through the blob endpoint
  that PyIceberg's `adlfs` uses, and DFS-only accounts need a real storage account.
- **G035-014** — RustFS support for `If-None-Match` is unverified; the probe decides per connection
  (D035-036).
- **G035-015** — Format v3 tables are refused until PyIceberg writes v3 (or the Java fallback lands).
- **G035-016** — Merge-on-read preferences are not honoured; row-level changes are copy-on-write.
- **G035-017** — Redis Streams broker destinations wait for feature 033 to merge.
- **G035-018** — A client library that ignores the relay configuration would bypass D035-029 inside
  the operator-approved ranges; per-client canaries and the NetworkPolicy bound the risk.
- **G035-019** — Destinations and `icebergWrite` are root-only (ADR-0036 decision 3).
- **G035-020** — Neutralising PyIceberg's internal commit retry must be proven by T-commit-once
  before the protocol of D035-030 is claimed.

## Review round 1 (2026-10-27)

Independent read-only reviews of the plan, ADR-0071, the data model and the contracts: an
`architecture-reviewer` (3 blocking, 9 should-fix, 7 minor) and a `security-reviewer` (3 high, 9 medium,
6 low, 2 suspicions). Every finding was checked against the code it cites; all are accepted. The
decisions below change the design; the contracts, data model, plan and ADR-0071 were updated in the
same change.

- **D035-048 — Each lake attempt runs in its own sandbox container** (supersedes D035-028; security
  H1). A subprocess with the runner's UID, pod network and `/work` volume could read the runner's
  identity token, other attempts' records and relays. The host now runs in a per-attempt container
  with a `lake` profile of the feature-005 container driver (ADR-0013): own user, pid, mount and
  network namespaces; no network interface but loopback; read-only root; no capabilities;
  `no-new-privileges`; seccomp; cgroup `memory.max`, `pids.max` and CPU; a distinct UID; core dumps
  off; only the image's `/opt/sagat/lake`, the attempt directory and one Unix socket mounted. The
  container is started by `apps/sandbox-runner`; the credential-holding lake runner never receives the
  container engine's socket. Credentials reach the container on its standard input only. The
  start-up cost (1–3 s per attempt) is accepted. The exact co-location of the lake runner and the
  sandbox runner (one pod sharing an `emptyDir`, or a node-local socket) is fixed by spike T-isolation
  in I0, whose acceptance tests are the H1 scenarios; I1 cannot close before it passes.
- **D035-049 — The relay is the only route, authenticated and over a Unix socket** (revises D035-029;
  security H1, H2, L). The container has no route but a Unix socket in its attempt directory, served
  by the lake runner's relay and authenticated by a per-attempt token. An in-container forwarder, part
  of the pinned host program, exposes it as loopback ports to the client libraries. The relay allows
  only the endpoints *declared on the pinned connection* or derived from its storage type and region
  (official regional endpoints); vended endpoints and catalog-supplied hosts are untrusted and
  allowed only if they match. TLS through the TCP relays is verified against the declared host name
  (psycopg `host=<name> hostaddr=127.0.0.1 sslmode=verify-full`; Thrift TLS `server_hostname`).
  Denied connections are counted and returned in the result for auditing.
- **D035-050 — Catalog-supplied configuration is filtered** (security H2). The host accepts from the
  REST `/v1/config` response and `LoadTable.config` only a closed allowlist of keys (warehouse,
  prefix, and the vended credential keys of the declared storage type); it pins `py-io-impl` and the
  FileIO implementation, refuses remote signing unless the connection declares it, sets
  `AWS_EC2_METADATA_DISABLED=true`, never uses ambient credential chains (GCS and Azure credentials
  are passed explicitly), and fails closed when credentials are missing. The allowlist is tested by a
  hostile-catalog fixture returning `py-io-impl`, `s3.endpoint`, `s3.proxy-uri` and signer settings.
- **D035-051 — Record schemas are egress annotations, not disclosure carriers** (supersedes D035-039;
  architecture B1). Making `recordSchema` a member of 016's carrier set would let a `public`
  annotation downgrade a `pii` source field under ADR-0036 D283, and the reserved `sagat.database.*`
  nodes are outside that set (ADR-0036 decision 6). The effective level of a written field is the
  **stricter** of the level inherited from the source (`outputSchema`, 016) and any level the
  `recordSchema` declares; fields with neither are `pii`; database results are `pii`. A test proves
  that a `public` annotation cannot lower an inherited `pii` field.
- **D035-052 — Broker destinations: inline sources only, a ceiling, and a payload by reference**
  (architecture B2, security H3 and M3, M8). Broker-stream destinations accept only `output`,
  `outputPath` or `inline` records (no blob or database-result source, so no object I/O inside the
  completion transaction). `STEP_OUTPUT` subscriptions gain `egressCeiling` (setting `pii` needs the
  016 reveal grant), are referenced with `event:manage` at publication, and are re-checked under lock
  in the completion transaction; a disabled subscription or a lowered ceiling fails the destination
  step in that transaction. The record data is stored in the content-addressed `payloads` table (run
  content: quotas, retention, erasure and inspection apply), and the delivery row keeps only the
  envelope and a payload digest, so `webhook_deliveries.payload` stays `protected_system`. The
  delivery worker composes the body when it sends. Total delivery bytes per completion are capped (4
  MiB default, 16 MiB ceiling). The body uses the existing versioned event envelope with type
  `step.output.v1` rather than a new CloudEvents body.
- **D035-053 — The engine, not the dispatcher, settles destination steps** (architecture B1). Under
  ADR-0025 the delivery worker commits only delivery and subscription state, and steps belong to the
  scheduler shard owner (ADR-0020). The dispatcher's settling transaction adds an outbox hint
  `destination.delivery.settled`; the shard owner completes or fails the waiting destination step by
  reading the authoritative delivery rows, and a reconciliation scan finds `WAITING(DELIVERY)` steps
  whose deliveries are all terminal.
- **D035-054 — Destination nodes are derived at parse, before region analysis, and never stored**
  (architecture S1). They are part of the forward graph, so parallel joins and saga regions must see
  them; they belong to the source's saga region, and the `fail` policy is an ordinary forward-step
  failure in that region.
- **D035-055 — Failure policies without reopening terminal steps** (architecture S2). `hold` →
  `WAITING(OPERATOR)`; `RETRY_STEP` (needs `run:operate`) or the existing `ReviewAction::Skip`, widened
  from compensation steps to destination steps (needs `saga:override`); no new `RESOLVE_STEP SKIPPED`.
  `continue` → terminal `SKIPPED` with the failure receipt; it is never reopened. A later redrive is
  an ad hoc destination execution (017 ad hoc admission) that reuses the destination's effect key, so
  it cannot duplicate (amends FR-035-037). `OUTCOME_UNKNOWN` → `WAITING(OPERATOR)`, since a reserved
  activity cannot declare `reconcileWith`.
- **D035-056 — Revert is a reserved compensation** (architecture S3). The workflow validation that
  rejects reserved activities in `compensate_with` admits `sagat.database.iceberg.revert` for
  `icebergWrite` nodes only; the derived `~c` node uses the source node's connection pin (root path).
- **D035-057 — Deployment profiles bind identities and activities** (architecture S4, security M2).
  Each `DatabaseRunner` binding names a profile (`database` or `lake`). The server allows only that
  profile's activities, only once the negotiated minor includes them, and `ResolveDatabase` refuses a
  connection kind outside the profile. A runner offers the new activities only when the agreed minor
  includes them, so mixed versions keep working.
- **D035-058 — Build separation** (architecture S5). `apps/database-runner` gains a `lake` cargo
  feature, built in a separate `cargo build` invocation for the `lake-runner` image, so feature
  unification cannot enable `object_store`'s `gcp` and `azure` in other binaries. The fitness check
  inspects `cargo tree -e features` for `control-plane` and `execution-engine`, not just crate edges.
- **D035-059 — Rollout and rollback** (architecture S6, S7). `lake.enabled` gates admission as well as
  publication. An older engine refuses runs whose version uses the feature with a typed
  `UNSUPPORTED_NODE` (tested). The down migration also refuses while any subscription has
  `purpose = 'STEP_OUTPUT'`.
- **D035-060 — ClickHouse retries need both windows proven** (architecture S8, security M6). The
  writer reads the engine and deduplication settings at the start of every attempt, and records the
  table's per-partition maximum block number before the first insert. A retry is automatic only when
  both the elapsed time and the block-number delta since then are inside the windows; otherwise the
  outcome is unknown. Identifiers are matched against `system.tables` and `system.columns`; table and
  column names travel as `{name:Identifier}` query parameters with percent-encoding. The target
  database must be the connection's database (the IR field is removed).
- **D035-061 — The runner's own HTTP clients follow the policy** (security M1, L). The `object_store`
  and `reqwest` clients use a resolver backed by `NetworkPolicy::resolve`, `redirect::Policy::none()`,
  `no_proxy()` and explicit credentials only. IPv6 metadata addresses `fd00:ec2::254` and `fd20:ce::254`
  are added to `network.rs`'s prohibited set.
- **D035-062 — Receipts and diagnostics are rebuilt, not passed through** (security M4, M5). The
  receipt schema forbids unevaluated properties; the runner rebuilds each receipt from typed fields.
  Host standard error is never stored or logged: only a closed exception class and a line count.
- **D035-063 — Bounded decoding** (security M7). Limits on decoded bytes (8× the input ceiling), row
  group size, metadata file size and manifest count, enforced by the host inside the container's
  `memory.max`.
- **D035-064 — Object checksums are over the records, not the file** (architecture minor). Parquet
  bytes are not guaranteed to be identical across attempts, so the object metadata
  `sagat-records-sha256` holds the SHA-256 of the canonical NDJSON form of the mapped records, and
  `ALREADY_PRESENT` compares that value.
- **D035-065 — Browse results are not run content** (security suspicion). Browse executions are ad
  hoc runs with origin `CATALOG_BROWSE`. Run lists and reads exclude that origin for everyone. The
  browse endpoint returns the result only to the actor who started it, and keys a same-key replay by
  tenant, namespace and actor (another actor gets the same 404 as a missing browse). Tests check both.
- **D035-066 — Authorization follows the code's role grants** (architecture B3). `database:write` is
  admin-only, so publishing and ad hoc execution of Iceberg writes and of database-backed destinations
  are admin-only, as 017 writes are. Developers browse (`database:browse`, new) and probe
  (`database:use`). Broker destinations need `event:manage`. Retrying a destination needs
  `run:operate`; skipping it needs `saga:override`. ADR-0071 records the permission-catalog change
  (ADR-0036 decision 5: 48 → 49 as written; 52 → 53 on develop, amended by D035-071).

### New gaps (review round 1)

- **G035-021** — Bucket confinement to `warehouseLocations` cannot be enforced at the relay
  (path-style S3 hides the bucket, and shared front ends allow domain fronting); it rests on the
  sandboxed host and on credentials scoped to the declared prefixes. The probe tries one write outside
  the prefix and warns if it is allowed, and the runbook requires prefix-scoped credentials.
- **G035-022** — The co-location mechanism of D035-048 is unproven until spike T-isolation passes.
- **G035-023** — Python supply chain: pip and setuptools are removed from the runtime image,
  `pip-audit` runs on a schedule as well as on bumps, and an SBOM is published with the image.

## Review round 2 — verification (2026-10-27)

A read-only verification pass checked the six blocking and high findings of round 1: three resolved,
three partly. It found ten new contradictions introduced by the revision. All were fixed in the same
change:

- **D035-067 — Credentials reach the container over the relay**, not through the sandbox runner's
  start request. The lake runner writes a per-attempt token file into the attempt directory, and the
  host authenticates on the relay socket and reads its request line there. The start request carries
  no credential.
- **D035-068 — Broker destination retry and redrive.** A broker step has no activity, so `RETRY_STEP`
  under `hold` returns it to `WAITING(DELIVERY)`. The shard owner re-inserts any missing delivery
  idempotently (unique on subscription and `event_id`, from the retained payload digest), the operator
  redrives dead deliveries with the ADR-0025 action, and T11 settles it again. Under `continue` the
  step stays `SKIPPED` and the redrive is the delivery redrive alone.
- **D035-069 — Retrying an unknown outcome needs `saga:override`.** That matches
  `crates/application/src/runs.rs`, which already requires it for waiting steps whose outcome may have
  happened. `run:operate` covers only retrying a failed destination. The authorization matrix,
  FR-035-045, the data model and the contracts now agree.

The other fixes were wording:

- ADR-0071 names its departures from ADR-0036 decision 6, ADR-0033, ADR-0010 and ADR-0013.
- The record-schema description and the "undeclared" wording are corrected.
- The error description now covers every broker cap.
- The quickstart's list of admin-published destinations is complete.
- The new columns are classified `protected_system`.
- `docs/architecture/security-and-sandbox.md` joins the files the feature updates.

## Implementation start (2026-11-02)

- **D035-070 — Owner gates of T035-001.** On 2026-11-02 the owner accepted ADR-0071, approved the
  "Finite ceilings" table of plan.md as written (answers O035-003; the values are now D035-045 as
  approved, not proposed), confirmed `hold` as the default failure policy of synchronous destinations
  (answers O035-004, confirms D035-023) and decided that 020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks
  reference the new 017 connection kinds (answers O035-005). The record is
  `docs/owner-decisions.md`, 2026-11-02, items 1–4. O035-002 stays open and does not block I0–I1.
  Developers may not publish writes: D035-066 and ADR-0036 decision 5 stand.
- **D035-071 — Reserved numbers re-checked against `origin/develop` `df59eb0e` (T035-002).**
  - Migration `20270115000000_lake_destinations` still sorts after every branch's latest migration;
    the latest is now `20261231000000_capacity_worker_membership_indexes` (D035-044 named
    `20261225000000`).
  - ADR-0071 is still the only ADR with this number. ADR-0072..0076 are taken by features 036, 033,
    034, the capacity work and 037.
  - The permission catalog has **52** entries on develop (`crates/domain/tests/identity.rs`,
    `Permission::ALL.len()`), not 48. `database:browse` makes it **53**. Every "48 → 49" in this
    feature (D035-066, plan.md, data-model.md, T035-011) now reads "52 → 53"; the grants do not change.
  - `PROTOCOL_MINOR` is 6 in `crates/application/src/worker.rs`, as the plan assumed. The 035 fields
    still take the next free minor when the feature merges (D035-043).
- **D035-072 — The independent reader needs two crates, and three advisories are ignored with a written
  reason.** `iceberg` 0.10.1 has no storage features (`default = []`): S3 access lives in the sibling
  crate `iceberg-storage-opendal` 0.10.1 (feature `opendal-s3`). T035-005 named only `iceberg`; the
  reader therefore declares both, as dev-dependencies of `sagat-tests` only, and
  `scripts/check-architecture.py` refuses any `iceberg*` dependency of another package, of any kind
  (negative tests in `scripts/tests/test_check_architecture.py`). The two crates add 94 crates to
  `Cargo.lock`; no existing crate changes version (18 crates gain a second, older version beside the
  current one). They make `cargo audit --deny warnings` fail on three advisories, which the owner
  decided on 2026-11-02 ("Ignorar os 3, documentados") to ignore until 2026-12-02: G035-024.
- **G035-024 — Advisories reached only through the test-only Iceberg reader.**
  RUSTSEC-2026-0194 and RUSTSEC-2026-0195 (`quick-xml` 0.39.4, severity 7.5: quadratic run time on
  duplicate-attribute checks, and unbounded namespace allocation in `NsReader`), pulled in by
  `opendal-core` 0.57.0 and `opendal-service-s3` 0.57.0 under `iceberg-storage-opendal`; both fixes
  need `quick-xml` >= 0.41, a different 0.x series that `[patch]` cannot substitute. RUSTSEC-2024-0436
  (`paste` 1.0.15, unmaintained), a compile-time macro pulled in by `parquet` 58.4.1 under `iceberg`.
  The path is `sagat-tests` (dev) → `iceberg-storage-opendal` → `opendal`; the reader parses only the
  responses of the disposable fixtures of `infra/docker/compose.lake.yaml`. Evidence: `cargo audit
  --deny warnings` exits 0 with the three ignores in `.cargo/audit.toml` and failed with exit 1 and
  "2 vulnerabilities found, 1 denied warning" without them; the lockfile before the change passed.
  *Exit condition:* remove the ignores when opendal moves to `quick-xml` >= 0.41 and parquet drops
  `paste`; re-evaluate on 2026-12-02.
- **G035-025 — RustFS reports healthy before its storage layer is ready.** `GET /health` answers 200 while a bucket
  request still gets `503 Service not ready: waiting for storage_quorum`. A first `tests/support/lake.rs` waited for
  `/health` and failed in 1 of 4 runs of the container case. `LakeAdmin::create_bucket` now waits for that 503 and for
  no other refusal, under `bounded::within` (a store that never becomes ready ends the stage in a named panic).
  Evidence: `create_bucket_waits_while_the_store_says_it_is_not_ready` (three 503s, then the bucket) failed first with
  the production message; 12 consecutive runs of `integration_iceberg_writes` passed afterwards. *Exit condition:*
  none while RustFS is the S3 fixture; the same wait applies to any fixture that reports health early.
- **G035-026 — One unexplained failure of a lake fixture, and the diagnosis added to catch the next.** In 1 of 12
  consecutive runs of `integration_iceberg_writes` (with two container cases running at once) the case
  `the_admin_creates_a_table_the_catalog_can_load` panicked at the lookup of the REST fixture's mapped port, right
  after `start()` returned; the panic text was not kept, so the cause is not known. 24 and then 30 consecutive runs
  passed, and a probe that starts 12 RustFS containers at once and reads the port at once got all 12 ports, so a
  plain race on the port lookup is unlikely. The leading suspicion, not shown, is a fixture that died after it
  started (a JVM under memory pressure). `ready_container` (in `tests/support/lake.rs`) now ends the wait at once when
  the container is not running and panics with its exit code and the last 30 lines of its stdout and stderr; before,
  a dead container was a bare "port not exposed" or a five-minute wait. Evidence:
  `a_container_that_exited_is_reported_with_its_exit_code_and_its_log` starts RustFS with a flag it refuses and
  checks the three facts; two mutations of the diagnosis (log dropped, state always "failed") each failed it.
  *Exit condition:* if the failure returns, its panic now names the cause; record it here and fix that cause. Close
  this gap after the lake fixtures have run in CI without a failure for a release cycle.
- **D035-073 — What the contract tests of T035-007 pin, where the task text was imprecise.** Amended here, not
  reinterpreted in silence (specs/AGENTS.md). Decided by: delegate.
  - *Samples.* The fourteen IR and five receipt samples of planning were checked in a scratch directory and never
    saved, so they are written again: 8 whole workflow documents under `tests/contract/iceberg-writes/fixtures/ir/valid`
    and 6 receipts under `fixtures/receipts`. A refusal case is an accepted sample plus one edit by JSON pointer
    (35 IR cases, 17 receipt cases), so the edit alone explains the refusal.
  - *`GET /database-drivers`* exists since feature 017 (`listDatabaseDrivers`, `x-implemented: '017'`). 035 widens the
    `DatabaseDriverList` schema to the four drivers and the operation keeps its marker; the operations that carry
    `x-implemented: '035'` are the two catalog-browse operations.
  - *Markers (amended again at T035-008).* The two catalog-browse operations are merged before any route serves them.
    `tests/AGENTS.md` fixes the convention: a planned operation carries `x-planned`, a built one `x-implemented`, and
    the cross-tenant battery attacks every `x-implemented` operation live and lists the others by name. They therefore
    carry `x-planned: '035'` and are listed in the battery's operations not built yet; the task that adds a route flips
    its marker to `x-implemented: '035'` and removes it from the list. The contract test accepts either marker, so it
    does not need a change when the route lands.
  - *"Every existing operation is byte-identical"* is a digest of each operation and of the parameters of its path,
    taken from the canonical file before the merge (`fixtures/openapi-operations.json`, 170 operations). A deliberate
    change re-blesses with `SAGAT_CONTRACT_BASELINE_BLESS=1` after the diff is read (rule 18). *"No number is reused"*
    already has an owner: `crates/api-grpc/tests/compatibility.rs` with `worker.baseline.json` (a number that exists
    keeps its name and type, a new one may be added). A first version of the contract tests carried a second baseline
    for it; it was removed as a duplicate, and the new messages joined `worker.baseline.json` (344 lines added, none
    removed).
  - *Errors.* Each code of the table in errors.md is an entry of `sdks/error-catalog.json` with the table's retry flag
    (D035-042); that needs a typed constant in each of the six SDKs, whose own tests require one per code, so it is
    T035-024's work and not the contract merge's (done on 2026-11-05: the 25 entries are in the catalog and every SDK
    has its constants). Each refusal reason (the `VALIDATION_FAILED` reasons, `UNSUPPORTED_NODE`, `CREDENTIAL_IN_URI`) is written
    in the OpenAPI `Error` schema, where the reasons of the existing codes are; a reason refines a code and is not a
    new `Error.code`. The existing step failure codes of 017 (`DATABASE_*`) are not in the catalog today, so the table's
    `OUTCOME_UNKNOWN` joins it with the new ones; T035-024 chooses the entry shape.
  - *Placement.* The loop and `chainPolicy` refusals of `database` are made by the publication codec, not by the JSON
    Schema, so the schema cases cover the root-only rule (a body refuses `icebergWrite` and `destinations`) and the
    codec rules stay with the domain tests (T035-009..).
- **D035-074 — Who may set the `pii` egress ceiling, and where that is checked.** FR-035-005 says only a holder of
  the reveal grant of the namespace. Decided by: delegate (found by an independent review of PR #95: the domain rule
  existed with no caller, so nothing enforced it).
  - *Where.* In `write_connection`, in the transaction that writes the revision, by
    `has_effective_payload_reveal_grant` (the function disclosure uses: the current membership role still reads
    internal values, and the grant set is held shared until commit), so a revocation cannot commit between the check
    and the write. The rule itself is `ConnectionConfig::authorize_egress_ceiling`; the store calls it.
  - *Which writes.* Only a write that leaves the connection `ACTIVE` at `pii`. A write that pauses or deletes carries
    the ceiling the connection already had and needs nothing: otherwise revoking a grant would leave a `pii`
    connection that nobody can pause or remove. Un-pausing is an active write, so it is checked. A retry of a write
    that was accepted is answered from its receipt, before the check, like every other check of this write.
  - *Refusal.* `StoreError::RevealGrantRequired`, which the application layer turns into `Forbidden` naming
    `payload:reveal-pii` (403), as disclosure refuses. It is not 404 (the actor does manage connections, so nothing
    is hidden) and not 409 (the state of the resource is not the cause).
  - *Reach.* Only the store and the service can set a ceiling today; the REST body still refuses the field
    (G035-027 item 7), so the grant check is reached through the application service until the connections work of
    US01 opens the field.
  - *Evidence.* `egress_ceiling_needs_the_reveal_grant` (integration_database_connections, real PostgreSQL): seen
    failing first, with the store accepting a `pii` write by an admin who held no grant (`main.rs:703`, revision 1
    stored at `pii`). After the fix `integration_database_connections` passes 7 of 7, and 7 of 7 with
    `SAGAT_TEST_AS_RUNTIME_ROLE=1`. One mutation, the check applied to a write that pauses or deletes too, fails the
    scenario at the pause that follows the revocation (`main.rs:797`); it was restored. The REST body still refusing the
    field and the response not carrying it are pinned by two unit tests in `dto_database.rs` (each failed under its
    own mutation), and the stored shape of a revision written before the feature by a golden in `infra-postgres`.
- **D035-075 — Every catalog browse is one audit row.** `contracts/connections.md` says every browse is audited. The
  first version of `AuditAction::DatabaseCatalogBrowsed` collapsed repeats like the denial actions do, which would have
  hidden how much of a catalog someone walked. None of the three actions of the lake (`database.catalog_browsed`,
  `destination.decided`, `iceberg.revert_requested`) collapses; the volume of browse rows is bounded by the browse limit
  of the API (30 a minute for an actor, D035-066), not by a dedupe. Decided by: delegate (found by an independent
  review of PR #95). Evidence: `the_audit_actions_of_the_lake_have_stable_names_and_none_collapses_repeats`
  (domain, `tests/audit.rs`).
- **D035-076 — Configuration hygiene: where the hostile catalog lives, and what the filter accepts.** T035-018 and
  T035-033, amended here and not reinterpreted in silence (specs/AGENTS.md). Decided by: delegate.
  - *Placement.* The hostile catalog is `runners/lake/tests/hostile_catalog.py`, not `tests/fixtures/lake/hostile-catalog/`
    as the task says. `scripts/select-test-targets.py` treats a file under `tests/` that no target owns as shared by
    the suites, so a fixture there would turn a change to the lake runtime into a run of every target (Constitution
    4.1.0, CI gates). Next to the runtime it stays inside the lake gate, and the Rust canaries of T035-031 start it from
    there. It serves over real HTTP on the loopback, so a test parses the JSON a client receives.
  - *Allowlist.* From `/v1/config`: `warehouse` and `prefix` only, the overrides over the defaults. From
    `LoadTable.config`: the vended credential keys of the declared storage type, and only when the connection declares
    vended credentials (S3: access key, secret key, session token; GCS: token and its expiry; ADLS: `adls.sas-token`,
    the other forms arrive with their tests in the phase of every storage type), plus the three signer keys when remote
    signing is declared on S3. A region, an endpoint, a role, a proxy and `py-io-impl` are never accepted. A value that
    is not a string is dropped under any key.
  - *Pin and environment.* `py-io-impl` is the first FileIO PyIceberg itself prefers for the scheme (PyArrow for S3 and
    GCS, fsspec for ADLS), set by the host over whatever the catalog or the connection says. The environment is an
    allowlist (`PATH`, locale, `TZ`, `TMPDIR`, three Python switches) plus `AWS_EC2_METADATA_DISABLED=true` and
    `/dev/null` for the AWS config files, not a list of variables to remove. A missing credential is
    `DATABASE_CREDENTIAL_UNAVAILABLE`, the code `errors.md` reuses from 017.
  - *Locations.* The table location, the metadata file, the table properties that move the data
    (`write.data.path`, `write.metadata.path`, `write.folder-storage.path`, `write.object-storage.path`), the manifest
    lists, the metadata log and the statistics files must all lie under a declared prefix, compared at a path boundary
    (`s3://lake/warehouse-evil/` is not under `s3://lake/warehouse/`). A path with a `.` or `..` segment, an encoded dot,
    slash or NUL, a backslash, a control character, a query or a fragment is refused, and so is a response that does not
    show where its files are. The error carries `ICEBERG_LOCATION_NOT_ALLOWED` and no location.
  - *Evidence.* `pytest` in `runners/lake`: 93 cases in `test_config_hygiene.py`, 151 in the whole suite, with ruff,
    `ruff format --check` and `mypy` (strict) clean. Written first and seen failing (no module). 21 mutations of the
    module were applied one at a time: an extra accepted key, every `LoadTable` key accepted, a vended region, the
    catalog replacing the pin, undeclared signing, an unclosed prefix, dot segments, encoded separators, an unclosed
    location, the data-path properties, the manifest lists, a whole environment, missing credentials not failing, no
    scheme check, an uncounted denial, an unchecked catalog prefix, an unchecked ADLS authority, an unbounded warehouse
    value, undeclared vended credentials, user info in an S3 prefix, an unconfined warehouse. Two survived the first
    tests (the ADLS authority and the length of a warehouse value); each has a test now and none survives.
- **D035-077 — What the `sagat.lake.v1` codec accepts, and what it leaves to later tasks.** T035-017 and T035-032 (the
  Rust half). Decided by: delegate.
  - *Heartbeats.* The contract says at most one a second. The runner measures when a line arrived, and a socket
    delivers late lines in a burst, so a window of one second holds two lines (the one on time and one of slack) and a
    third is a flood (`HeartbeatFlood`). The time comes in as an argument, so no test sleeps.
  - *One result.* A second result, or any line after the result, is `LineAfterResult`; a host that closes without one is
    `NoResult`; a line over 1 MiB is `LineTooLong` (**16 KiB since D035-110**). The attempt driver reads the socket a line at a time
    under that bound.
  - *The result line.* Exactly `{"receipt": …, "relayDenials": n}` or `{"failure": …, "relayDenials": n}`; any other set of
    members is `UnknownMember`, and a count that is not an integer from 0 to 2^32-1 is `Malformed`. Every
    `ProtocolFailure` is `LAKE_HOST_PROTOCOL` and carries a closed kind, never content of the line.
  - *Receipts.* Only the two kinds a host produces are read: `iceberg` and `object_storage`. The PostgreSQL, ClickHouse,
    broker and `failed` receipts are written by the runner, so a host that sends one is refused. Every field is a type
    that accepts only what `receipts.schema.json` names (a count is a canonical decimal string, a hash is 64 lowercase hex
    digits), a field outside the schema is refused, and the receipt is built again from the typed fields. Two typed
    fields are stricter than the schema on purpose: a snapshot id is a canonical signed 64-bit integer (the schema
    allows 20 digits), and an effect key and a ref have no control character.
  - *Failures.* The code must be one of `DatabaseFailure::ALL`. `column` is 1 to 128 characters with no control
    character, and `reason` is 1 to 48 characters of `A-Z`, `0-9` and `_`: a closed charset and not a closed set, because
    the reasons of each code arrive with the check that raises it (`ICEBERG_UNSUPPORTED_TABLE` names none today).
  - *The request.* The envelope (`protocol`, `operation`, `effectKey`, `firstAttemptStartedAt`, `relay`) is typed and a body
    cannot override it. The body of each operation (the pinned connection with its credentials, the table, the mode, the
    limits) is an open map for now: its typed builder comes with the task that adds the operation (US01 and US02), and
    the Iceberg connection model does not exist yet. That is a pending decision and not a design. The token and the body
    are never shown by `Debug`.
  - *Standard error.* Reduced to a closed `ExceptionClass` and a line count; the text is not kept. The class is the last
    line's name when it is a CamelCase identifier, so a line that is only a secret classifies as none.
  - *Not done.* The probe and browse results (`receipts.schema.json` has no shape for them; they arrive with their
    tasks), the Python host, and the reader that applies the line bound to the socket.
  - *Evidence.* `cargo test -p sagat-infra-lake --locked`: 31 passed; clippy `-D warnings` and `cargo fmt --check` clean;
    the architecture check sees the new crate in the adapter layer. Written first and seen failing (unresolved
    imports). 25 mutations of the codec were applied one at a time and all are caught; the first run let one survive
    (an object-storage receipt that accepts unknown fields: the test only tried the Iceberg receipt) and a test now
    covers both kinds.
- **D035-078 — The relay's wire protocol and policy.** T035-016 and T035-030 (the Rust half), which left the wire format to
  the implementation. Decided by: delegate.
  - *Wire.* One connection per tunnel to the attempt's Unix socket. Line 1 is `SAGAT-RELAY/1 <token>`; anything else, or a
    wrong token, a preface over 256 bytes or one that is late, closes the connection with no reply, so a process that is
    not the attempt's host learns nothing. Line 2 is `CONNECT host:port HTTP/1.1` with its headers (an HTTP proxy tunnel)
    or `TCP <name>` (a mapping the connection declared, for Hive and a catalog database). `CONTROL`, the conversation of
    `sagat.lake.v1`, is served by the attempt driver (T035-035) and is refused here. Replies: `200`, `403`, `502` or `400`
    with no body for a tunnel; `OK` or `DENIED` for a mapping. A reply never repeats the target. The request is read a byte
    at a time and no more than 8 KiB, so bytes a client sends right behind the request reach the destination.
  - *Policy, in this order.* The name (a host and port the connection declared, or an S3 regional endpoint of its region
    and buckets, compared exactly: no wildcard, no suffix) before anything is resolved; then one resolution, as a fully
    qualified name; then every address of the answer against the managed executor's policy and the operator's networks;
    then a connection to the address that was checked (an IPv4 address when the answer has one, an IPv4-mapped address
    as the IPv4 it was checked as). An answer with one forbidden address is refused as a whole.
  - *Counts.* `RelayDenials`, by the five closed kinds of `LakeRelayDenial`; its total is the `relayDenials` of the result
    line. A destination that was allowed and could not be reached is a `502` and is not a denial.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 34 relay cases, written first; clippy `-D warnings` clean. 24 mutations of
    the first version, all caught (one survived the first run because a test case ended the request in a way that was
    refused for another reason; the case now isolates the rule).
- **D035-079 — What the independent security review of the relay changed.** An independent read of `relay.rs` found seven
  things; four were fixed with tests, one hardened, two are left to the attempt driver (G035-029). Decided by: delegate.
  - *The relay was open by default.* `RelayConfig::new` took no list of networks, so a driver that forgot `with_deny` let a
    tenant point a declared name at the cluster's own addresses (a service IP, the PostgreSQL pod). The operator's allowed
    networks are now a required argument, as for the managed database executor: an address outside them is refused and an
    empty list reaches nothing. `with_deny` stays for the platform's own networks inside an allowed range.
  - *Slots leaked and nothing was bounded.* A `JoinSet` kept every finished connection until shutdown, and the number of
    connections, the resolution and the connect had no limit. Now a slot is taken before a connection is accepted (64 by
    default, the rest wait in the kernel's backlog), a `TaskTracker` keeps nothing of a finished connection, the name server
    and the destination share one admission timeout (10 s), and a shutdown ends a connection at any point.
  - *Search domains.* The name went to the resolver without its trailing dot, so `ndots` could turn `catalog.example` into
    `catalog.example.<namespace>.svc.cluster.local`. It is asked for as a fully qualified name.
  - *Weak tokens.* `RelayToken::new` accepted an empty token and one with a newline, which breaks the line. It takes 32 to 128
    characters of `A-Z a-z 0-9 - _` and returns an error otherwise; `RelayToken::generate` draws 32 bytes from the operating
    system and writes 64 hex digits.
  - *Defense in depth.* `with_peer_uid` refuses a peer whose user id, read by the kernel, is not the container slot's. An
    IPv6 authority needs its brackets, and the denial counters stop at the maximum instead of wrapping.
  - *Evidence.* 34 relay cases, 33 protocol cases and a unit test; 15 more mutations (the allowed networks ignored, the name
    without its dot, no resolution timeout, no connect timeout, no shutdown, no connection limit, no user id check, a wrapping
    counter, an unbracketed authority, a zero limit, mapping names, a short token, any character in a token, a constant
    token), all caught. The gaps the reviewer listed in the tests were closed: an IPv4-mapped answer is connected to as
    IPv4, a mapped metadata address through a mapping is refused, spellings of an undeclared name are refused.
- **D035-080 — The Iceberg connection: one document from the API to the database, and a SQL step never uses it.** T035-038,
  T035-041, T035-042 and the REST half of T035-043. Decided by: delegate.
  - *The type.* `ConnectionConfig::Iceberg(Box<IcebergConfig>)` in `crates/domain/src/database/iceberg.rs`, member by
    member the `IcebergConnectionConfig` schema of the OpenAPI contract: the four catalogs, the three storage types, the
    prefixes, the modes (`append` by default), the two secret references, the ceiling and the limits. Every rule of
    data-model section 1 is a rule of `IcebergConfig::validate`, and each refusal is a closed `LakeConfigError` whose
    `reason()` the API returns (`CREDENTIAL_IN_URI`, `VENDED_NEEDS_DELEGATION`, ...).
  - *What the domain cannot know.* Plain `http`, a GCS emulator and a plaintext JDBC catalog are for a development
    namespace (`requires_development()`, enforced where PostgreSQL's plaintext already is, in the transaction that
    writes the revision). A Hive metastore with `tls: none` is for a deployment that allows it
    (`uses_unauthenticated_metastore()`): `ConnectionService` refuses it with `UNAUTHENTICATED_METASTORE_NOT_ALLOWED`
    unless `with_unauthenticated_metastore_allowed(true)`, and nothing sets that yet (G035-030).
  - *A SQL step never uses a lake connection.* `secret()` and `limits()` were the accessors of a PostgreSQL connection;
    they are `sql_secret()` and `sql_limits()`, which are `None` for a lake connection, and every SQL path answers that
    the connection is unavailable: admission (a published `database` step that names it is `NotFound`, as an unknown
    connection is), the execution of a step, the managed executor, the SQL runner and the gRPC answer of the SQL profile.
    The lake profile answers a lake connection (T035-044).
  - *Stored and shown as the same document.* The stored form is the contract's document (the egress ceiling only when it
    is not the default, the modes in the order written); the reader ignores a member it does not know and checks every
    rule again, so a document that breaks one is corrupt and is not used. The REST body is `deny_unknown_fields` at every
    level, and `type` selects the one member that may be present.
  - *Both secrets, and the ceiling.* `write_connection` locks and checks every secret the connection names (one for
    PostgreSQL, a catalog's and a storage's for Iceberg) for OpenBao authority, and the `pii` check of D035-074 applies to
    both kinds. `egressCeiling` is part of the REST body of both kinds and the response omits `internal`, which closes
    G035-027 item 7.
  - *Refusals.* `AppError::InvalidConnection { reason }` is a `422 VALIDATION_FAILED` whose `details.reason` is the closed
    name, with one sentence of advice for each of the fourteen reasons; the message never repeats a value.
  - *Evidence.* `cargo test` of the domain, application, API, store, executor and gRPC crates: 91 suites, 0 failed. The
    domain rules were written first (compile-red), and 29 mutations of them are all caught (one survived: nothing
    tested a zero or out-of-range port in a catalog address). `integration_database_connections` passes 7 of 7, also with
    `SAGAT_TEST_AS_RUNTIME_ROLE=1`, with a new scenario (`lake.rs`) against real PostgreSQL and OpenBao: the stored
    document, the retry, the audit row, another tenant's 404, a missing secret writing nothing, a SQL step refused,
    plaintext by environment, the `pii` grant, and the history of a paused connection.
- **D035-081 — The container's side: the forwarder, the host skeleton and how it is started.** T035-030 (the forwarder)
  and T035-032 (the host skeleton). Decided by: delegate.
  - *The entry point.* `lake-runtime.md` says `python3 -I -X utf8 /opt/sagat/lake/host.py`, but the image holds the package at
    `/opt/sagat/lake/sagat_lake/` and T035-032 names `sagat_lake/host.py`. The host is started as
    `python3 -I -X utf8 /opt/sagat/lake/sagat_lake/host.py --attempt-dir /work/<attempt-id>`. `-I` ignores `PYTHONPATH` and
    does not put the script's directory on the path, so the file puts its package's parent on `sys.path` itself, once, when it is
    run as a script; a test starts the real process isolated to prove it. The text of the contract is amended with the
    contract-touching PR that follows (G035-031), because a contract file selects every suite.
  - *The control conversation.* After the relay's first line, the host sends `CONTROL`; the lake runner answers with the one
    request line; the host writes its heartbeat lines and its one result line on the same connection and closes it. The
    token is read from `relay.token` in the attempt directory and has the shape the relay checks (32 to 128 of `A-Z a-z 0-9
    - _`). The Rust relay refuses `CONTROL` today, since the attempt driver serves it (T035-035).
  - *What the host does and does not do.* `PR_SET_DUMPABLE=0` is the first action, even for a bad argument list. The request is
    read to at most 4 MiB, counting the payload and not its newline, and a line that never ends is refused as soon as it passes
    the bound; 30 s is the wait for it. A heartbeat is written every second while an operation works, never after the result.
    The host serves no operation yet, and a request for one is `LAKE_HOST_PROTOCOL`. An exception no operation expected is
    `WORKER_ERROR` and writes only the name of its class to standard error. `relayDenials` is always 0: the count is the
    runner's, which owns the relay (G035-028).
  - *Failures cannot carry text.* `Failure` refuses a code, a column or a reason the runner would refuse, so a bug cannot
    produce a result line that fails the whole attempt as a protocol break.
  - *The forwarder.* One HTTP proxy port and one TCP port per declared mapping, on `127.0.0.1` with ports the system chooses.
    Each client connection gets a relay connection of its own that presents the token first, then forwards the client's
    `CONNECT` request or asks for `TCP <name>` and hides the relay's one-line answer. It copies bytes and reads none of them.
    A relay that is not there closes the client instead of hanging it; a denied request is answered as the relay answered it.
  - *Evidence.* `pytest` in `runners/lake`: 214 passed and 1 skipped (the `prctl` test is Linux's and runs in CI), `ruff`,
    `ruff format --check` and `mypy` (strict) clean. Written first and seen failing. 22 mutations of the forwarder, the host and
    the failures are all caught; the first run let two survive (nothing pinned the request bound apart from the length check, and
    the length check counted the newline, which a test of the exact boundary now covers). The relay is a double that speaks the
    bytes of the Rust one (`tests/relay_double.py`); the real clients through the real relay are T035-031.
- **G035-027 — Places where the contract fragments do not yet fit the canonical shapes or the code.** Found while writing
  the contract tests; none is pinned by a test yet, each is settled by the task that implements it.
  1. `connections.md` names `ConnectionProbeResult.checks[]`, but the canonical OpenAPI has no such schema: the probe
     returns a `Run` and `DatabaseConnectionProbe` is the latest-probe read. The schema to extend is chosen with the
     probe task.
  2. A `STEP_OUTPUT` subscription "selects no run-event kinds", but `CreateWebhookSubscriptionRequest.events` requires
     one to eight items of `run.finished`. `events` must become optional for that purpose, or the purpose must be
     refused with events; the subscription task decides.
  3. The ClickHouse destination "declares only the table" while `tableIdentifier` requires a namespace; the samples
     use `target.relation`, which the fragment allows. The T035-008 merge states which of the two is canonical.
  4. The fragment says every root-level output-producing node gains `destinations`, without listing them. The merge
     adds the property to the two kinds the contract tests cover, `activity` and `database`. `script`, `child` and
     `icebergWrite` itself are undecided: `icebergWrite` refuses the property (its definition is closed), and the
     schema does not read it on a `script` or `child` node (open objects), so only the codec stops it there, as it
     does on every node until the domain parses destinations.
  5. connections.md widens `operateRun` and adds a redrive through `database-executions` for `DESTINATION` steps. Both
     change existing operations, so they are merged with the step kind (the destination tasks), not at T035-008, which
     keeps the 170 existing operations byte-identical.
  6. The Studio lists a lake connection by its driver and offers neither the PostgreSQL form nor a probe for it, because
     `DatabaseConnection.config` is now a union; the kind-specific forms are the Studio tasks of the feature.
  7. *Closed in D035-080.* The OpenAPI allows `egressCeiling` on a PostgreSQL connection and the REST body now takes it,
     for both kinds; the store checks the reveal grant for `pii` (D035-074).
  8. `DatabaseFailure::retryable()` (the retry flag of the table of failure codes) has no production reader yet; the
     pin tests keep it equal to `contracts/errors.md`, and the retry decision of the lake and destination steps is
     the first reader (the lake driver and destination tasks, which map a step failure to a retry).
  *Exit condition:* each is closed by the task that implements the operation, with a test.
- **G035-028 — Where the host reports its configuration denials.** `Hygiene.denied` counts what the filter refused by a
  closed kind (`key_not_allowed`, `location_not_allowed`, `credential_missing`), but the result line of `sagat.lake.v1`
  has one count, `relayDenials`, which is the relay's, and an extra field is `LAKE_HOST_PROTOCOL`. So nothing carries the
  host's count to the runner's audit and metrics yet. Pending decision, taken with the codec (T035-032): add a typed
  `hygieneDenials` object to the result line (a change to `lake-runtime.md` and to the codec tests), or fold the count
  into `relayDenials` and lose the kind. A location or credential denial already fails the attempt with its code; the
  dropped keys are the part that would be invisible. *Exit condition:* T035-032 decides and a test shows the count
  reaching the audit.
- **G035-029 — What the attempt driver must do with the relay.** The relay is a library until the driver (T035-035) builds it
  from a pinned connection; these are the review's findings that only the driver can close, each with the test that
  closes it. (1) Pass the operator's allowed networks and the platform's state networks from the deployment, and never
  an unconditional `0.0.0.0/0`. (2) Set `development(true)` only from the deployment's development profile, since it
  allows the loopback interface. (3) Create the socket in a directory only the runner and the slot's user can reach, set
  `with_peer_uid` to the slot's user id, and write `relay.token` mode 0400 from `RelayToken::generate`. (4) The container's
  forwarder (`runners/lake/sagat_lake/forwarder.py`, T035-030) is not written. (5) Reading the request a byte at a time
  costs up to 8 K system calls per connection; the connection limit and the handshake timeout bound it, and a buffered
  reader that hands its unread bytes to the tunnel would be cheaper. *Exit condition:* closed by T035-035 and T035-030
  with the integration tests of T035-031.
- **G035-030 — What the Iceberg connection leaves open.** (1) The OpenAPI pattern of `warehouseLocations`,
  `^(s3|gs|abfss)://[^?#@\s]+$`, excludes `@`, so the `abfss://container@account.dfs.core.windows.net/...` form that ADLS
  needs cannot be sent over REST, while the domain and the host's hygiene filter accept it. S3 and GCS are unaffected;
  the contract is amended with the phase of every storage type. (2) `GET /database-drivers` still lists PostgreSQL only:
  `iceberg` is listed when a probe exists (T035-045), so it does not advertise an unbuilt capability. (3)
  `allowUnauthenticatedMetastore` is a builder on `ConnectionService` whose default is off, and no control-plane setting
  turns it on, so a Hive metastore without TLS is refused everywhere until one does. (4) The Studio does not know the new
  kind (T035-046); its types already read the union. (5) The lake profile does not answer a lake connection yet
  (T035-044); until then nothing can resolve one. *Exit condition:* each is closed by the task named, with a test.
- **G035-031 — The container's side leaves open.** (1) The text of `lake-runtime.md` gives the host's path as
  `/opt/sagat/lake/host.py` (D035-081); it is amended with a PR that touches the contract. (2) The relay double and the Rust relay
  are two implementations of one wire and can drift; T035-031 runs the real forwarder against the real relay. (3) The forwarder
  accepts any number of local clients and each one costs a relay connection; the relay's own cap makes the rest wait, and the
  container's `pids.max` and `memory.max` bound the forwarder, but the forwarder has no cap of its own. (4) No operation is served:
  `probe`, `icebergWrite`, `icebergRevert`, `objectWrite` and `browse` arrive with their tasks. (5) The Rust relay answers `CONTROL`
  with `400` until the driver serves it. *Exit condition:* each is closed by the task named, with a test.
- **D035-082 — The Studio form for the Iceberg connection.** T035-046. Decided by: delegate.
  - *Offered by the deployment.* A new connection can be Iceberg only if `GET /namespaces/{id}/database-drivers` lists
    `iceberg` (the contract says the Studio and the SDKs never hard-code the list); a failed or PostgreSQL-only answer leaves
    PostgreSQL alone. An existing connection keeps its kind.
  - *Scope.* The form edits a REST catalog over S3-compatible storage, which is what the task names. A connection of another
    catalog or storage type is listed and deleted but has no edit button until T035-078 (`editableIcebergOf` decides).
  - *Credentials.* Catalog and storage credentials are an existing OpenBao secret or values entered once and written to OpenBao
    first (017's flow), as the JSON of `data-model.md` "Credential secrets" (OAuth2 `clientId`/`clientSecret`, bearer `token`,
    AWS keys). The secret is named `lake-catalog-<request id>` or `lake-storage-<request id>`; a retry reuses the id, changed
    values take a new one, and the values leave the state once the write succeeds. The connection body carries references only.
    Vended storage credentials need the catalog to delegate access, and then no storage secret is written, even if one was typed.
  - *The ceiling.* `pii` is offered only to a session that holds `payload:reveal-pii`. That is a hint: the namespace reveal grant
    is the server's decision (D035-074) and the save shows its answer through the error state.
  - *Saving is not proof.* The form says that saving does not test the catalog or the storage, and shows no connectivity result.
  - *Evidence.* `ConnectionsPage.test.tsx`: 30 tests (8 of the earlier behaviour, one of them adapted, and 22 new). The form was written
    before its tests, so rule 18 was not followed to the letter; a mutation run is what shows that the tests catch it: 30 mutations
    of the form, the editor and the page, 24 caught at once and six survivors (a storage secret written for vended credentials, the
    vended state kept after the catalog stops delegating, `limits: {}` sent, more than 16 locations, an empty catalog URL, the
    type chooser on an edit through the cached driver list). Each became a test and all six are now caught. `npm run lint`,
    `typecheck`, `check:api`, `npm test` (87 files, 1,217 tests) and `npm run build` pass.
- **G035-032 — What the Studio form leaves open.** (1) `GET .../database-drivers` returns PostgreSQL alone today, so no
  deployment offers the form yet: the lake entries arrive with `lake.enabled` (T035-036) and the rest of T035-043. (2) The
  per-check probe results of T035-046 need the probe (T035-045) and a field for them: `DatabaseConnectionProbe` in the OpenAPI
  carries `run`, `revision` and `failureCode`, while T035-040 asks for `CATALOG_AUTH`, `WAREHOUSE` and `STORAGE_ACCESS` with the
  tested revision and time. That is a contract amendment with T035-045; `ConnectionProbe` stays PostgreSQL-only until then.
  (3) Other catalogs and stores (T035-078) and the browser journey (T035-074). (4) `remoteSigning` is kept on an edit and cannot
  be set (D035-050). (5) The `@sagat/e2e` typecheck was not run here: it needs the TypeScript SDK built first
  (`sdks/typescript/conformance/prepare.sh`), as in CI. *Exit condition:* each is closed by the task named, with a test.
- **D035-083 — The `lake` container profile, its engine and what the isolation tests found.** T035-028 (the profile and
  the engine; the sandbox-runner API is not built), T035-015 (the Docker half) and the first half of T035-029. Decided by:
  delegate.
  - *Where.* `crates/infra-sandbox/src/lake.rs` holds the configuration as a pure function (`container_config`) and
    `lake/engine.rs` the engine calls (`run`, `start`, `exec`, `stop`, `sweep`, the work volume). They are not inside
    `docker.rs`, as the task text says: that file is 2,100 lines of the script sandbox, and the two share five small helpers,
    which became `pub(crate)`.
  - *The profile.* `lake-runtime.md` "Isolation", with these choices: the user is `40000 + slot` (4,096 slots, so never root and
    never a system account); the environment is exactly the four names of the contract; `/tmp` is a memory-backed `noexec`
    scratch with a size of its own; the largest file a process may write is the work budget; the one mount is the attempt's own
    sub-path of the work volume; three labels (`sagat.lake`, `.owner`, `.attempt`) serve the sweep and the per-node ceiling
    that comes with the start API; every limit of zero is refused, because zero is not unlimited.
  - *The work volume is memory-backed and mounted by sub-path.* A named volume of the local driver with `type=tmpfs,size=N`
    is the size limit of all attempt directories together, and each container mounts only `<volume>/<attempt id>` (the
    engine's volume sub-path option), so another attempt's directory is not there to be read.
  - *Found: a memory-backed volume keeps its content only while some container has it mounted.* The first run of the tests
    lost the directories when the preparer exited. In a deployment the lake runner's own mount keeps the volume for as long
    as the runner runs; a restart of the runner drops every attempt directory, which is acceptable because the next attempt's
    lookup resolves whatever the lost one had sent (D035-030). The tests use a holder container for the same reason.
  - *Found: a test sweep must not share a label with its neighbours.* A sweep by owner removed the containers of tests that
    ran at the same time; every test case has its own owner label now.
  - *Evidence.* 19 configuration tests (`crates/infra-sandbox/tests/lake_profile.rs`) and 49 mutations of the module, six of
    which survived at first (a hexadecimal range, a space alone or a control character alone in a name, the length limits of
    an image and an owner, the first character of a volume name); each became a test and all are caught. Eight tests against a
    real engine (`tests/integration/iceberg-writes/isolation.rs`, Docker 29 with runc and cgroup v2): the user and group of the
    slot, no effective or bounding capability, `no-new-privileges`, a root that a world-writable path cannot be written
    through, a `/tmp` that cannot run code and holds no more than its size, loopback as the only interface, no route to the
    metadata addresses (IPv4 and IPv6) or to a public address, no engine socket or service-account directory, only the
    attempt's own directory listed, its own token readable and another attempt's token absent, the same token unreadable
    to the user of another slot, the memory, swap and process limits as cgroup values, no core dumps, a file larger than the
    work budget and a third file past the volume both refused, an out-of-memory kill, a deadline kill that leaves no container,
    and an image the engine does not have. 13 mutations of the isolation properties against that engine: seven were caught at
    once and six survived, for weak probes (a write to `/`, which a non-root user cannot do anyway; an exec of BusyBox under an
    unknown applet name; a file-size probe that met a full volume first; no look at the swap limit) or for redundancy
    (`network_mode: none` and the disabled-network flag are each enough alone). The probes were fixed, both network settings
    are mutated together, and all are now caught. `cargo clippy -p sagat-infra-sandbox -p sagat-tests --all-targets -- -D
    warnings`, `cargo fmt --check` and `check-architecture.py` pass.
- **G035-033 — What the lake profile leaves open.** (1) **Who gives a file to the user of a slot.** A token that is mode 0400 and
  owned by the user of the slot needs `CAP_CHOWN`; the tests use a root holder. A lake runner that is a plain user
  (`USER 10001`) cannot do it, so one of three has to be chosen before T035-035: the runner runs with the capability (a file or
  ambient capability), the sandbox runner prepares the directory (it holds the engine, and the token is a per-attempt relay
  token, not an external credential), or the slot users share a group and the mount alone separates the attempts, which gives
  up the defence in depth that a user per slot is for. (2) A user namespace of the container's own is the engine's setting
  (`userns-remap`, rootless mode, gVisor); the tests do not assert it. (3) The Kubernetes half of the spike (a pod sharing an
  `emptyDir`, the Helm profile, the validation profile) is not run, so T035-029 stays open and G035-022 is not closed.
  (4) The start, stop and kill API of the sandbox runner, and its per-node ceiling, are not built (the second half of T035-028);
  the engine's `start`, `stop` and `sweep` are what it will call. (5) The runtime image does not exist yet (T035-027): the
  tests use `python:3.13-alpine` (`SAGAT_LAKE_ISOLATION_IMAGE` names another). (6) `/dev/shm` is a separate memory-backed
  scratch of the engine's default size that the profile does not set. *Exit condition:* each is closed by the task named, with a
  test; (1) needs a decision first.
- **D035-084 — The `icebergWrite` node in the domain (append only).** T035-047 (its tests) and T035-052 (the type and the
  node). Decided by: delegate.
  - *Where.* `crates/domain/src/database/iceberg_write.rs` (`IcebergWrite`, `IcebergWriteParts`, `TableIdentifier`,
    `ColumnMapping`, `RecordSource`, `WriteIntent`, the two reserved names and the ceilings) and `Node::iceberg` /
    `Node::iceberg_write` in `workflow.rs`. The node stays an `Activity`, so no `NodeKind` variant changed and no exhaustive
    match broke.
  - *What an author cannot change.* The retry is `RetryPolicy::DEFAULT` (three attempts): the contract has no retry field and the
    spec says retries are automatic because recovery looks the commit up by its effect key. The effect is high risk, and
    irreversible unless the step declares `compensation: revert`; `check_node` recomputes it and refuses any other. A revert is
    `compensate_with` on the reserved revert name, which only an Iceberg write may name, and the derived `<id>~c` runs it (derived
    companions are built after the declared nodes are checked, so they never meet the reserved-name rule).
  - *Root-only (G035-019).* `WorkflowError::IcebergNodeNotRoot` for a step in a body, beside a forEach, while or repeatUntil root
    node, or under a continuation policy; saga, parallel and the other root nodes stay legal. The codes `INVALID_ICEBERG_NODE` and
    `ICEBERG_NODE_NOT_ROOT` are mapped in `crates/application/src/definitions.rs`.
  - *Bounds.* A mapped field is a JSON pointer (`/a/b`, no empty segment, at most 512 bytes), at most 16 segments deep; a column
    is 1 to 255 bytes; at most 1,024 mapped columns (the hard ceilings of plan.md; the platform defaults of 256 and 8 are
    publication settings). A table is 1 to 8 namespace levels and a name of 1 to 255 bytes; a ref is 1 to 255 bytes; no control
    character anywhere; the limits are those of a connection (`LakeLimits`).
  - *A rule the contract does not state.* A field is mapped once and a column is written once (`DuplicateField`,
    `DuplicateColumn`): two fields into one column would make the write depend on record order. The schema has no
    `uniqueItems` for it, so the contract is amended with the codec (G035-034).
  - *Append only.* `WriteIntent` has one variant. Overwrite and upsert need options (a scope and a filter, key columns and a
    delete marker) and tests of their own, so they arrive with user stories 4 and 5 (T035-080, T035-085); `recordSchema` arrives
    with the codec (T035-053), which is where it is read.
  - *Evidence.* 22 tests in `crates/domain/tests/iceberg_write.rs`, written first and seen failing (the types did not exist);
    35 mutations of the module and of the changes in `workflow.rs`, all caught (three needed a better pattern to compile or
    match, none survived); the whole `sagat-domain` suite, `cargo clippy --workspace --all-targets -- -D warnings` and
    `cargo fmt --check` pass.
- **G035-034 — What the domain node leaves open.** (1) The contract schema should say that a field and a column are each mapped
  once (D035-084); until then the domain refuses what the schema accepts. (2) At most 32 record blobs is a constant
  (`MAX_RECORD_BLOBS`): the count is known only when the expression is evaluated, so publication and the runner enforce it
  (T035-054, T035-056). (3) The codec still refuses `icebergWrite` (`ICEBERG_WRITE_NOT_ENABLED`, T035-053), so no published
  definition can carry the node yet. (4) Places that branch on a database node's presence and need an Iceberg decision, found
  by a read-only survey on 2026-11-05: the fail-closed default of `DefinitionStore::check_database_access` (an Iceberg node
  passes silently today), `infra-postgres` `definitions.rs:215`, `execution.rs:450` and `:1086`, `database/admission.rs:57` and
  `:111`, `application/src/worker.rs:1006`, `:1078`, `:1270`, `:2156` and `managed_capabilities_allowed`, the scoped worker
  paths, `api-http/src/dto_exec.rs:576` (the step type label), and `apps/database-runner` (`runner.rs:188`, `attempt.rs:42`).
  (5) The HTTP reason, the CLI help (`apps/cli/src/diagnostics.rs`) and `docs/quickstarts/errors.md` for the two new codes come
  with the publication checks (T035-054). *Exit condition:* each is closed by the task named, with a test.
- **D035-085 — The `icebergWrite` codec, and a store that fails closed until its admission exists.** T035-053. Decided by:
  delegate.
  - *A pure parser.* `crates/infra-ir/src/lake.rs` builds `Node::iceberg` from the node the schema describes; `convert.rs` no
    longer refuses it and `schema.rs` knows the type (`ROOT_NODE_TYPES`, and a diagnostic that does not echo a value). There is
    no codec flag: `JsonIrCodec` is a unit struct used in 461 places, and whether a deployment may publish the step is the
    business of the definition service and the store, not of the parser.
  - *Why nothing can be published yet.* The refusal of `lake_capabilities.rs` existed so that a step the schema accepts but the
    engine cannot run is never published. Taking it away alone would let a deployment publish a step that no worker serves, and
    the run would hang. So the store refuses a definition that holds an Iceberg node: the default of
    `DefinitionStore::check_database_access`, `PgDefinitionStore::check_database_access` and `admit_definition`, which the
    transaction of a publication also calls. The author sees what an unusable connection shows (`NotFound`). This is a marked
    placeholder, replaced by T035-054 (access, allowed modes, ceiling, `lake.enabled`).
  - *Refused as unsupported, never dropped.* An `overwrite` or `upsert` option (also beside `append`) and a `recordSchema`: a
    step that wrote less than it said, or that lost a stricter classification of its fields, is worse than none.
  - *Evidence.* 14 tests in `crates/infra-ir/tests/iceberg_write.rs` (written first and seen failing: 10 of 12 failed because the
    converter refused the node), the application test of the default, and the lifecycle scenario of
    `integration_database_connections` against real PostgreSQL and OpenBao, which now publishes an Iceberg write and sees it
    refused (1 passed in 220 s). 15 mutations of the codec, all caught. The guard of `admit_definition` is mutation-checked
    against the real-store scenario.
- **G035-035 — What the codec leaves open.** (1) `lake.enabled` and the code `LAKE_DISABLED` are not built (T035-036): until they
  are, the refusal is the one of an unusable connection. (2) `records.databaseResult` must name an earlier root-level database
  node of the same run; neither the codec nor the graph checks it (T035-054). (3) Run admission (`execution.rs`, `worker.rs`
  and their scoped paths) still keys on `database_operation()`; a definition with an Iceberg node cannot exist until T035-054,
  so nothing reaches it, but each site needs a decision then (G035-034 item 4). (4) The author-feedback path (`validate`) calls only
  `check_database_access` and is not covered by the real-store scenario, which exercises the publication. (5) Overwrite, upsert
  and `recordSchema` come with their tasks (T035-080, T035-085, T035-054). *Exit condition:* each is closed by the task named,
  with a test.
- **D035-086 — The lake containers run as their slot's user in the runner's group.** Closes point 1 of G035-033. **Decided by:
  owner** (OD-2026-11-05-2).
  - *The decision.* The runner, a plain user, makes the attempt directory 0770, `relay.token` 0640 and `relay.sock` 0660 in its own
    group; the container runs as `<slot uid>:<runner gid>`. `LakeSpec` carries the group (never root's); `container_config` sets
    `user: "<uid>:<group>"`.
  - *What it gives up.* The user id of a slot no longer protects the files of one attempt from a container that were given another
    attempt's directory. The separation is the sub-path mount and the token on the relay. The decision is stated by a test that
    fails if the user becomes a boundary again (`the_user_of_a_slot_is_not_a_boundary_between_attempts`).
  - *The contract.* `lake-runtime.md` is amended: the modes and owners above, and the host path `python3 -I -X utf8
    /opt/sagat/lake/sagat_lake/host.py` (D035-081, point 1 of G035-031).
  - *Evidence.* 20 configuration tests; 9 tests against a real Docker engine (the group of the container, a container in another
    group reading nothing, the slot's own token readable, the cost test); 3 mutations of the group logic, all caught, one of them
    against the real engine.
- **D035-087 — The lake is one operator switch that the publication path reads; `LAKE_DISABLED` names it.** Part of T035-036.
  **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The decision.* `SAGAT_LAKE_ENABLED` is a strict boolean (`true` or `false`; anything else is a configuration error) of the
    control plane, off when unset. `ServeConfig.lake_enabled` reaches `DefinitionService` and `PublishingService` through
    `with_lake_enabled`, which `serve` calls for both. With it off, **publication** refuses a definition that holds an `icebergWrite`
    with `VALIDATION_FAILED`, reason `LAKE_DISABLED` and the node (`details: {reason, node}`, status 422), before any check of secrets
    or connections, so an operator who left the lake off is told about the switch and not about a connection; **validation**
    reports the same problem at the node. A **draft** may hold the node either way: an author can prepare a workflow before the
    operator turns the lake on, and nothing runs from a draft.
  - *What it does not do.* With the switch on, the store still fails closed until the publication checks exist (T035-054, D035-085):
    the switch is necessary, not sufficient. It is read once at start; changing it takes a restart, like `SAGAT_SCHEDULES_ENABLED`.
  - *The error mapping.* `LAKE_DISABLED`, `INVALID_ICEBERG_NODE` and `ICEBERG_NODE_NOT_ROOT` have an HTTP reason, a CLI help text and
    a section of `docs/quickstarts/errors.md` with the same sentence. The two reasons of the domain were already problem codes
    of validation (`ir_problems`) but had no HTTP `details.reason`, help or section; the note of T035-024 deferred them to the check
    that raises them, and publication now raises them.
  - *Evidence.* 4 application tests (refused at publication, named by validation, still the store's call with the lake on, no cost
    for a definition without the node); 1 configuration test (off by default, strict); 3 HTTP mapping tests; the real-store scenario of
    `integration_database_connections` (lake off → `LAKE_DISABLED` at node `save`; lake on → the store's fail-closed refusal);
    and `the_lake_switch_reaches_the_services_that_publish` in `e2e_platform_foundation`, which starts the real `serve` both ways
    and publishes over REST. 15 mutations of the gate (the default, the inverted switch, each of the two gates, the problem
    code, the setter, the configuration variable read two ways, the wiring in `serve`, the three HTTP reasons and the node, the CLI
    sentence): 14 caught, 1 survives (G035-036 point 3).
- **G035-036 — What the switch leaves open.** (1) The execution engine has no `SAGAT_LAKE_ENABLED` and run admission
  (`runs.rs`, `worker.rs`) does not check it: T035-020 asks that "both publication and admission" refuse, and only publication does,
  so T035-036 stays unticked. A definition with an Iceberg node cannot exist until T035-054, so no run can meet the gap before
  the task closes it (T035-055). (2) The deployment documents (compose, runbook `docs/runbooks/lake-destinations.md`) do not list the
  switch yet; it goes in with the runtime image (T035-027) and the runbook. (3) **One mutant survives:** `PublishingService::with_lake_enabled`
  forwards the switch to the service that approved publication and promotion go through (`publish_through`), and no test sees
  it. The plain `POST .../publish` does not use that service, so the e2e test does not reach it; a test needs a namespace
  publication policy that asks for approval and a second approver (or `tests/support` changed, which selects every suite in CI).
  Nothing can hold an Iceberg version for these paths to promote or approve until the store's guard is replaced, so the code is
  unreachable today, not untested by choice. *Exit condition:* point 1 by T035-055 with a test of a run that is refused with the
  lake off; point 2 by the runbook; point 3 by T035-054, whose publication checks make a version holding the node reachable and
  must be tested through the approval path with the lake off and on.
- **D035-088 — The level of a written field is read by the codec from a plain reference, and anything else is `pii`.** Part of
  T035-054 (the egress check of FR-035-025). **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The problem.* 016 classifies the input, the result and the output of each step, and nothing derived from them, so the
    platform cannot follow a value through an expression. FR-035-025 asks for "the level inherited from its source (the source step's
    output schema)", and does not say how a step's source is found.
  - *The decision.* Only a **plain reference** is followed: `input.a.b`, or `steps.<id>.output.a.b` for a root step. Its list of records is
    read at that place of the schema, each mapped field below the record (`/a/b`, or the whole record when the step lists no
    columns) takes the level the schema gives it, and `written_level(inherited, None)` applies (D035-051: unmarked is `pii`). Everything
    else is `pii`: any other expression (an index, an operator, a call, an unknown name), a list of blobs, and a database result (which is
    `pii` by D035-051 whatever a schema says). A carrier with no schema, or a step that does not exist, is `pii`.
  - *Strictest wins, and what a schema does not say is `pii`.* A field is as strict as the strictest level that can reach it: every
    record position the schema describes on its own (`prefixItems`, then the remaining `items`), and everything below the place that
    answers, which is the schema that governs a name or an item the pointer did not spell out (`additionalProperties`, the remaining
    items), not only the place the pointer names. A value that can hold more than one thing (a structure, a list, a whole record, any
    schema that does not say its type is a scalar) is at least `pii`: a key or an element the schema does not name is `pii` under 016,
    and nothing closes the object. A numeric segment may be a position or a property named by digits, so it reads everything under
    what it indexes. A list described by more than 64 positions is read as `pii`, so the cost of the reading is bounded. (A review
    found the first version read too low in four of these places; each has a test now.)
  - *Where it lives.* The codec computes it, once, for `parse` and `parse_stored` alike (it is the only place that holds the whole
    document), and attaches it to the step as derived data: `IcebergWrite::egress()`. It is not part of the canonical bytes, so no
    digest changes. A step that carries no list is refused whole (`EgressRefusal::NotComputed`), so a codec that forgot to classify
    fails closed. Admission restores the pinned version and meets the same list.
  - *What it gives up.* An author who builds the records with an expression gets `pii` for every field, so the connection needs a `pii`
    ceiling (which needs the reveal grant of the namespace) until `recordSchema` (T035-085) lets them declare levels, and a write from
    blobs or a database result needs the same. That is FR-035-025's rule, now visible: the common safe shape is a plain reference to a
    classified schema.
  - *Evidence.* 9 tests of the domain type and its check (`crates/domain/tests/iceberg_egress.rs`), 32 tests of the codec
    (`crates/infra-ir/tests/iceberg_egress.rs`; publication and restore agree, the canonical document is the document and nothing
    more). Two mutation rounds: 23 mutations of the first version (4 survived the first tests and each got a test) and 24 of the
    reading above (5 survived the first tests: a secret below an unnamed property, a schema with no type, a type given as a list,
    the bound on positions, and a carrier counted into another; each got a test), all caught in the end.
- **G035-037 — What the levels leave open.** (1) Nothing compares the levels with the ceiling of a connection yet: the publication
  check (access, allowed modes, `CLASSIFICATION_EGRESS_REFUSED`, connection kind and pins) is the rest of T035-054, and the store still
  fails closed until it lands. (2) `recordSchema` declares levels that can only raise these (T035-085). (3) A numeric segment of
  a mapped field (`/items/0/x`) reads everything under the list or object it indexes, and a structure or a whole record is at least
  `pii` because the schema cannot say it is closed: conservative, and a complaint to take up if authors need a lower level (the
  walk would have to expose `additionalProperties: false`, and `recordSchema` of T035-085 would let an author say it). (4) A reference through a body-local step or `result` is not followed. *Exit condition:*
  point 1 by the next slice of T035-054, with a test of a `pii` field against an `internal` ceiling and of a `secret` field.
- **D035-089 — The settings of the connection decide a lake write at publication, and a run of one is refused until its admission
  exists.** The rest of T035-054. **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *Publication and feedback.* In the store's admission, in the transaction of the publication and for the author's feedback
    (`validate`), an Iceberg write needs `database:use` and `database:write` of every actor (it is a write), an active connection of
    the namespace at the pinned revision or the current one, of the lake kind, reachable (plaintext only in a development
    namespace), and every secret it refers to (the catalog's and the storage's) must exist with the OpenBao authority and be usable
    by every actor. Whatever fails there is the one refusal of a connection nobody may use: a missing connection, a paused one, a SQL
    connection, a revision that does not exist, a developer who lacks `database:write`. It reads as not found and a denial is audited
    (`database.denied`). Only then does the connection's own word count: `IcebergWrite::refusal_by(config)` refuses first a mode the
    connection does not allow (`MODE_NOT_ALLOWED`, naming the mode), and only then fields above its ceiling or never classified
    (`CLASSIFICATION_EGRESS_REFUSED`, listing path and class, never a value; the HTTP details list at most 20 with a count of the
    rest, and validation caps its problems at 20 as it does for every check). A step that **pins a revision** is judged by that
    revision and by the current one, the stricter winning: a mode or a ceiling that was tightened since applies to it, so pinning an
    older revision cannot loosen what the connection says now (a security review found it could). A refusal by the connection's
    settings counts as `denied` in `database_admissions_total`, not as an error of the store.
    These are `VALIDATION_FAILED`, not hidden: the author may use the connection, so nothing is learned that they could not read.
    The domain decides (`refusal_by` is pure and tested without a database); the store finds the connection.
  - *Run creation.* `create` of a run, and the replay of a run that is already there, now see an Iceberg write: a definition that
    holds one is refused at run creation with the same refusal as an unusable connection, until T035-055 builds the admission
    (pins, quotas, the direct, child, event, rerun and ad hoc paths). Without this the removal of the store's fail-closed guard would
    have let a published version start a run that waits for a worker that is not there (the hazard of G035-035). A published Iceberg
    version exists after this change; it cannot run.
  - *Where the cases live.* T035-048 names `tests/integration/iceberg-writes/publication.rs`. The cases are in the lifecycle scenario of
    `integration_database_connections` (`lake.rs`) instead, because the world, the OpenBao secret, the roles and the connections are
    there, and a second copy would double a start-up of about a minute. This decision amends the task text.
  - *Evidence.* 7 and 9 domain tests (the decision, the levels), 6 application tests, 7 HTTP tests, the CLI consistency test, and the
    scenario against real PostgreSQL and OpenBao (also as `sagat_app`): within the ceiling published; a `pii` field refused by an
    `internal` ceiling and named; a `secret` field refused by every ceiling; unclassified refused by `internal` and accepted by `pii`;
    a mode refused by name; validation says the same as problems; a missing, SQL or unpinned-revision connection is not found; a
    developer is refused and audited; a paused connection is refused; a run of the published version is refused; the promotion of the
    version to another namespace is refused with the lake off, not found without the target's connection, and published with it.
    Publication never contacts the catalog: `catalog.example` does not resolve and the publication succeeds. 26 mutations of the
    decision, the conversions, the HTTP details and the store; 25 caught, 1 survives (G035-038 point 1). It also closes the mutant
    that survived in #115 (`PublishingService` forwarding the switch; G035-036 point 3), now caught by the promotion. Two more
    mutations of the pinned-revision rule, both caught by the scenario case that tightens the modes of a connection and then
    publishes a step pinned to the first revision. A correctness review and a security review read this change and the levels of
    D035-088 before it merged; what they found is fixed (D035-088, D035-090) or recorded below.
- **G035-038 — What the publication checks leave open.** (1) The replay of a run receipt (`check_database_replay`) sees an Iceberg
  write like the creation does, but no test reaches it: no run of an Iceberg write can exist yet, so there is no receipt to replay. It
  is defensive, and T035-055 gives it its test with the run admission. (2) `records.databaseResult` must name an earlier root-level
  database step of the same definition; neither the codec nor the graph checks it (G035-035 point 2). (3) The worker paths (claim, scoped
  claim, ad hoc, rerun) still branch on `database_operation()` only; none can meet an Iceberg write while a run of one is refused at
  creation (T035-055 decides each). (4) The ACL of the secrets cannot be told apart from the role at publication in the current role
  matrix (only an administrator holds `database:write`, and an administrator may use any secret), so a lake connection whose secret
  ACL excludes the publisher has no test of its own; it is the same code the SQL path runs. *Exit condition:* points 1 and 3 by T035-055;
  point 2 closed by D035-090; point 4 when a role other than an administrator can hold `database:write`. (5) The deployment's
  `allowUnauthenticatedMetastore` is checked when a connection is written (`crates/application/src/database.rs`) and not again at
  admission, so turning the flag off later leaves an old Hive `tls: none` connection publishable. The run admission of T035-055 must
  check it again. (6) The codec reads a mapped field's JSON pointer (`~1` before `~0`, a numeric segment as everything under what
  it indexes). The host that writes the record (`runners/lake`, T035-057) must read the same property for `~0`, `~1` and a number
  on an array, or the field that is written differs from the field that was classified; its tests must pin that. (7) An
  egress refusal is not audited (a `database.denied` is for an access denial) and `validate` audits nothing, as for the SQL path;
  a refusal to send `secret` data may deserve a record of its own when the audit vocabulary is next touched.
- **D035-090 — A database result is read from a database step that runs before the write, and the graph checks it.** Closes point 2 of
  G035-035 and G035-038. **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The rule.* `records.databaseResult` names a node of the same graph. It has to exist, to be a managed database step, and control has
    to be able to pass from it to the write along the edges of the graph, so its result is there when the write starts. A node that
    does not exist, one that is not a database step, the write itself, a database step after the write, and one on another branch
    (the two arms of a condition) are refused as `INVALID_ICEBERG_NODE`. It is a rule of the graph, so it is checked where the graph is
    built (`DefinitionGraph::new`, after the acyclic and reachable checks), for the codec and for any caller of
    `WorkflowDefinition::new`; the help of `INVALID_ICEBERG_NODE` says it. Two things a review found are part of the rule: a join is
    not an edge of the graph, so a database step inside a parallel branch or a saga body reaches what follows the node that owns
    the branch (and not a sibling branch); and the error edge of the source itself is not a way past it, since a write reached
    only when the source failed has no result to read (the error edge of a later step is fine).
  - *What it does not prove.* That a database step reaches the write does not mean it ran on every path (a condition can skip it), so the
    attempt that runs the write still has to cope with a result that is not there; that is not publication's to prove.
  - *Cost.* The search keeps the set of nodes it has been to; without it a chain of branches has an exponential number of paths, a risk
    at publication. A test builds forty diamonds with a limit of ten seconds.
  - *Evidence.* 15 tests in `crates/domain/tests/iceberg_source.rs` and one in `crates/infra-ir/tests/iceberg_write.rs`, written first (the
    refusals failed; so did the parallel, saga and failed-source cases once a review asked for them). Two rounds of mutation: 7 of the
    first version (one survived: the visited set, caught by the linear-time test) and 14 of the version that follows a join and drops
    the error edge of the source (one survived and showed a call that nothing needed, which is removed); all caught.
- **D035-091 — The host decides three things with pure functions: how a value maps, whether the key is already there, and whether
  the table can be written.** T035-049 and T035-057, first part. **Decided by: delegate** (the owner delegated the decisions
  of this feature).
  - *Why pure.* The correctness of the write rests on these three decisions, and each can be tested without a catalog, an object
    store or a container: `mapping.py` (a record against the schema of the table), `lookup.py` (the walk of D035-030 over a mapping
    of snapshots) and `compat.py` (FR-035-011 over facts read from the loaded table). The host's adapters (the table, its
    snapshots, its metadata size) come later and only build their inputs.
  - *The mapping table is published* in `docs/guides/iceberg-type-mapping.md` (FR-035-010 asks for one published table; none existed).
    The choices it makes where the specification was silent: a large integer may arrive as a string of ASCII digits; a number is
    read with its digits (`parse_float=Decimal`), so `12.50` stays `12.50`; a decimal never takes a float (it is already
    inexact), and a digit that would be rounded away is `SCALE_OVERFLOW` while a trailing zero is accepted; `NaN` and the
    infinities only reach `float` and `double`; a timestamp without a zone into a zoned column is `ZONE` (not guessed), and a zone
    into a column without one is `ZONE` too (it would be dropped); microseconds, and a digit past the sixth that is not zero is
    `SCALE_OVERFLOW`; **binary and fixed take only the typed cell of feature 017** (`{"type": "binary", "value": "<padded
    base64>"}`), so a bare string into binary is `TYPE`, as the specification lists; a map is an array of key and value pairs,
    or an object when the keys are strings, and the same key twice is `TYPE`; a nested key the table does not have is always
    `UNKNOWN_FIELD` (ignoring unknown fields is for the record); a column named in the step's mapping that the table lacks is
    `UNKNOWN_FIELD`, and a field the mapping does not name is neither written nor unknown.
  - *Lookup.* `OUTCOME_UNKNOWN`, with no text, for a search that cannot decide (a parent the table no longer holds inside the
    window, or more than 10,000 snapshots) and for the effect key found on another ref (FR-035-019, the edge case of the
    specification). A key on the target ref wins even when the same commit is visible from another ref. A snapshot exactly at the
    start minus five minutes is not older than the window, so the walk goes on to its parent.
  - *Compatibility.* Format versions 1 and 2 are written, 3 and anything newer are not; the default file format is Parquet,
    read without regard to case; the transforms are `identity` and `void` only, because those are the ones the runtime
    can apply (G035-040; the first version of this decision accepted year, month, day, hour, bucket and truncate and was wrong); a location is inside the warehouse by path and not by text (`warehouse-evil` is not
    below `warehouse`), a `..` segment is inside nothing, and an empty root allows nothing rather than everything. The checks
    run in a fixed order: the size of the metadata, the ref, the format version, the file format, the transforms, the location
    provider (D035-092), the locations.
  - *Evidence.* 64 tests of the mapping, 25 of the lookup and 45 of the compatibility, written first and seen failing (the modules
    did not exist). Mutations of each module: 35 of the mapping (7 survived the first tests and each got a test or, for a check
    that was redundant, was removed), 17 of the lookup (2 survived and each got a test) and 22 of the compatibility (3 survived:
    tests were added for one, and two checks that nothing needed were removed, leaving 21); all caught in the end. The whole of `runners/lake`: 348 passed, 1
    skipped; `ruff check`, `ruff format --check` and `mypy` clean.
- **G035-039 — What the host's pure checks leave open.** (1) Nothing builds their inputs yet: the facts of a loaded PyIceberg
  table, the mapping of its snapshots, the reading of NDJSON and Parquet into records, the Arrow batches, the data files in the
  attempt's directory and the conditional commit (T035-034) are still to write, and the T-commit-once spike (T035-019) that
  decides whether PyIceberg's retry can be switched off still has to run. (2) `test_append.py` of T035-049 is not written, and
  with it the bounds on decoding (D035-063). (3) The rules for overwrite and upsert (a scope, a delete marker, duplicate keys,
  identifier fields) are not in `compat.py`; they come with T035-082 and T035-088. (4) The type table names the types the
  Iceberg v1 and v2 formats have; `timestamp_ns`, `variant` and the geometry types are v3 and refused with the version.
  *Exit condition:* points 1 and 2 by the append path of T035-057; point 3 by the tasks named.
- **G035-040 — The runtime cannot write a table partitioned by anything but identity or void.** Found 2026-11-06 by writing to
  a table of each transform with the packages the runtime image holds. `identity` and `void` write. `year`, `month`, `day`,
  `hour`, `bucket[n]` and `truncate[n]` fail with `NotInstalledError: pyiceberg_core needs to be installed`: PyIceberg 0.12 applies
  them with the optional package `pyiceberg-core`, and `runners/lake/requirements.in` is
  `pyiceberg[pyarrow,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0`, with no `pyiceberg-core`. Until that changes, `compat.py`
  refuses such a table before any file (`ICEBERG_UNSUPPORTED_TABLE`, reason `TRANSFORM`), which is the contract's own rule ("partition
  transforms PyIceberg cannot apply") applied to the runtime as it is. **This is a large limit:** most tables a lake holds are
  partitioned by day or month, so until it is lifted most real tables are refused. *Recommendation (a dependency change, not made
  here):* add the `pyiceberg-core` extra to `requirements.in`, regenerate the hash lock the way its header says (`uv pip compile
  ... --generate-hashes --universal --python-version 3.12 --exclude-newer <date>`), check that it ships wheels for the platforms
  of the image (the Dockerfile installs wheels only), and run `pip-audit` and the SBOM check, since it puts a native (Rust) wheel in the
  sandbox image. *Exit condition:* the owner decides; then `_TRANSFORM` in `compat.py` accepts the transforms that a test of
  writing to a table of each shows the runtime applies.
  *2026-11-06, closed by D035-103: the owner said to follow the recommendation, `pyiceberg-core` is in the lock and `compat.py` accepts every transform.*
- **D035-092 — The host reads the facts of a table from the table PyIceberg loaded, and refuses a table that names its own location
  provider.** T035-057, second part. **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The adapters.* `runners/lake/sagat_lake/table_facts.py` builds the inputs of `compat.check` and `lookup.lookup` from a loaded
    table and reads nothing else (no listing, no scan): `facts_of` (the table location, the metadata file, `write.data.path` and
    `write.metadata.path` when not empty, the format version, `write.format.default` with Parquet as the default, the transforms of
    the spec in force after any evolution, the size of the metadata file as the file IO reports it, the refs with `main` always
    among them because the first commit creates it and the host never creates another), `refs_of` (the head of each branch and tag)
    and `snapshots_of` (id, parent, the catalog's timestamp and the summary, which is empty for a snapshot that has none).
  - *What PyIceberg 0.12 really reads.* Read in its source and tried, not assumed: `write.data.path` and `write.metadata.path` move
    where files go; the Java-only `write.folder-storage.path` and `write.object-storage.path` do not (a test shows the provider still
    places data under the table), so they are not facts about where the host writes.
  - *Found in that reading: a table property runs code.* `write.py-location-provider.impl` makes PyIceberg `importlib.import_module`
    the module the property names and call the class with the table location and its properties; creating a table that names a
    missing class already fails with `Could not initialize LocationProvider`. Whoever can set a property of a table in the catalog
    chooses which importable callable the host runs, and it also decides where every file is placed, which no check of a location
    can see. `catalog` hygiene (`config_hygiene.py`) covers the catalog's own `py-io-impl` but nothing covered a table's properties.
    I did not try to turn it into an execution and the refusal does not depend on whether it can be. `TableFacts.location_provider`
    carries the name (a blank value means none, as it does for PyIceberg) and `check` refuses with `ICEBERG_LOCATION_NOT_ALLOWED`,
    the contract's code for a location the host does not accept, so the closed set of codes and reasons does not change.
  - *Independent review (security-reviewer, read-only, given the diff and the spec and not my conclusion).* It read the PyIceberg
    source for every property that decides where the host reads, writes or deletes, and found what the first version missed. Each
    claim was checked against the source before anything changed. **Fixed here:** (1) the manifest list of every snapshot, the
    metadata log (PyIceberg deletes the old metadata files it lists after each commit, `table/__init__.py:1850`) and the statistics
    files are facts too and must lie inside the warehouse; the REST hygiene checked them, the other catalogs did not; (2) only a
    branch is a ref the host can write to (an overwrite to a tag raises after it has written its data files); the lookup still
    reads tags; (3) `inside` accepted a location the object store reads as another key: a `#` or `?` (dropped by the URL parser, so
    every file of the table would land on one key), a backslash, an encoded dot or separator, a dot segment, a control character.
    It now uses the one rule the connection's own hygiene has (`clean_path`, made public) for what is below the root; (4) a root
    that is no more than a scheme (`s3://`) allowed every location of that scheme; (5) the size of the metadata file was asked of
    the object store before its location was checked, which answers a question about someone else's object; it is now asked only
    for a file inside the warehouse; (6) a docstring said the size bound protected the load, which it cannot: the table is loaded
    and parsed before the facts are read.
  - *Evidence.* 29 tests in `test_table_facts.py` against a real SQLite catalog with a file warehouse (no network, no container),
    and `test_compat.py` extended (95 together); written first and seen failing. Mutations of `table_facts.py` and of the new
    rules in `compat.py`: 32 on the first version (2 survived and got tests; 1 equivalent) and 16 on the review fixes (2 survived and
    got tests: a metadata file that starts like the warehouse by text, and the root that is trusted while only what is below it is
    cleaned); all caught in the end. `runners/lake`: 398 passed, 1 skipped; `ruff check`, `ruff format --check` and strict `mypy` clean.
  - *Still open (G035-039):* the readers of NDJSON and Parquet into records, `modes/append.py`, the conditional commit and the
    T035-019 spike, the deletion of the attempt's own files, `priorAttemptMayHaveLeftFiles` and `test_append.py`.
- **G035-041 — A table's own properties reach PyIceberg's file IO, and the host's filter does not look at them.** Found 2026-11-06 by
  the security review of D035-092 and checked in the source. PyIceberg builds the IO of a table from the catalog's properties
  **overridden by the table's own** (`catalog/rest/__init__.py:837`: `{**metadata.properties, **table_response.config, ...}`; the SQL,
  Glue, Hive and DynamoDB catalogs pass `metadata.properties` to `_load_file_io` as well), and `load_file_io` imports and calls the class
  named by `py-io-impl` first (`io/__init__.py:371`). So `py-io-impl`, `s3.endpoint`, `s3.proxy-uri`, signer and credential keys in the
  properties of a table beat what D035-050 pins, and it happens inside `load_table`, before any fact is read. `Hygiene.accept_table`
  filters `LoadTable.config` and the four location properties, and reads nothing else of `metadata.properties`. The relay allow-list
  (D035-049) still stops a redirected endpoint from leaving the sandbox; the import of a named class is not stopped by it.
  *Measured the same day:* `tests/test_table_io_premise.py` makes the catalog pin `py-io-impl` and the table name another class;
  PyIceberg builds the table's, on `load_table`. **Closed for the REST path by D035-094** (`accept_table` returns only the table
  properties the host lets PyIceberg see, with the hostile fixture carrying `py-io-impl` and `s3.endpoint` in a table's properties).
  *Still open, exit condition:* the loader that builds the REST catalog hands PyIceberg `AcceptedTable.properties` and not the
  document's own (it does not exist yet); and for a catalog that reads the metadata file itself (Glue, Hive, SQL) the loader builds the
  table's IO from the host's own properties, or that catalog type is refused until it does.
  *2026-11-06, the REST half is closed end to end by D035-102 (`LakeRestCatalog` hands PyIceberg the accepted properties); Glue, Hive
  and JDBC remain.*
- **G035-042 — Table properties that size the write are not bounded.** Found by the same review. The retry loop reads
  `commit.retry.num-retries`, `.max-wait-ms` and `.total-timeout-ms` from the table (2026-11-06, measured: a table that says 6
  retries gets 7 commit requests; see T035-034), and the write reads `write.target-file-size-bytes` and the Parquet row-group, page,
  dictionary and bloom-filter sizes. The first is closed by the in-memory pin of T035-034. For the rest only the sandbox's memory and
  time limits stop a hostile value. *Exit condition:* the host pins or caps them in memory the way it pins the retries, when the
  append path writes.
- **D035-093 — The commit is made once: the host sets PyIceberg's commit retry to zero on the table it loaded, and the claim is
  measured.** T035-034 (the part that decides G035-020) and the Python half of T035-019. **Decided by: delegate** (the owner delegated
  the decisions of this feature).
  - *Measured* (2026-11-06, PyIceberg 0.12.0; a REST catalog over real HTTP on the loopback with a real SQL catalog behind it,
    counting the commit requests one `append` sends). Without the override: a refused commit (409) is sent **5 times** (PyIceberg
    refreshes the table, rebuilds the snapshot on the new head and commits again, `table/__init__.py:1090-1143`); 500, 502 and 504
    (`CommitStateUnknownException`), 503, 429 and a connection closed without a reply are sent once; a 401 twice (one refresh of the
    token). The number of retries is read from `commit.retry.num-retries` among the properties of **the table the catalog returned**:
    a table whose properties say 6 gets 7 requests, so the owner of a table chooses how often the host commits. A writer that loses a
    race is rebased and lands its rows on top of the new head without the host having searched the new history, which is the
    hazard D035-030 exists to prevent.
  - *With the override.* `commit.pin_commit_once(table)` sets that one property to `"0"` on the loaded table object, in memory; the
    stored table never has it and the other properties are kept. Then every answer above is one request, except the 401; a table that
    says 1000 gets none; a writer that loses a real race is refused with `CommitFailedException` by the requirement on the head that
    PyIceberg adds itself, and nothing of ours lands; an uncontested commit lands once and carries its effect key.
  - *The one repeat left* is the 401: PyIceberg's REST client asks again once after refreshing the token. The catalog applied nothing
    to a request it did not authorise, so it is safe and it is at most one more request.
  - *Consequence for G035-020.* The override works on PyIceberg 0.12.0, so the fallback of D035-030 (build the updates and call the
    catalog's commit directly) is not needed. The tests that state what PyIceberg does without it are kept as the premise: a newer
    PyIceberg that changes the mechanism fails them, and the override is looked at again then.
  - *Evidence.* `runners/lake/tests/test_commit.py` (17 tests) with the fixture `counting_catalog.py`, written first and seen failing
    (the module did not exist); 6 mutations of `commit.py`, all caught; `runners/lake`: 370 passed, 1 skipped; `ruff check`, `ruff
    format --check` and strict `mypy` clean. **Not done:** T035-019 as written (the Rust test with the counting relay in front of the
    REST fixture, end to end through the relay; it needs Docker), and the rest of T035-034 (the conditional commit around the append,
    which comes with the append mode).
- **G035-043 — PyIceberg's REST client has no request timeout.** Measured 2026-11-06 with a scratch script against a local catalog
  that accepts the commit and never answers: three seconds later `append` was still waiting, and `catalog/rest/__init__.py` passes no
  `timeout` to `requests`. FR-035-018 and D035-030 step 6 speak of a commit that "times out", but nothing in PyIceberg produces one.
  *Exit condition:* decide when the catalog loader is written between (a) mounting an adapter with a timeout on the catalog's session
  (`catalog._session`, a private attribute, pinned by the exact version 0.12.0 and a test) and (b) relying on the sandbox runner's
  deadline of the attempt (T035-035) and treating the killed attempt as an unknown outcome, which D035-030 already handles by
  searching the history again. Either way a test must show the hang ends.
- **D035-094 — A table's properties pass an allowlist before PyIceberg sees them.** T035-018 follow-up, closing the REST half of G035-041.
  **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *Why.* D035-050 filtered the catalog's own configuration (`/v1/config` and `LoadTable.config`) and left the `properties` of the table's
    metadata alone except for four locations. PyIceberg merges those over the catalog's properties when it builds the file IO, so a
    table could name the class to import and move the endpoint, the proxy, the signer and the region. The premise is measured in
    `test_table_io_premise.py`.
  - *The rule.* `Hygiene.table_properties` keeps a key only if it is text under the `write.`, `commit.`, `gc.`, `history.`, `read.` or
    `schema.` family (the whole key, in exact case, as PyIceberg reads it) and drops and counts every other: `py-io-impl`, `s3.*`,
    `gcs.*`, `adls.*`, `client.*`, `uri`, `warehouse`, `token`, `credential`, `rest.*`, and also the harmless `owner` and `comment`, so
    the count is a trend for the audit of a connection and not an alarm by itself. `write.py-location-provider.impl` is kept on purpose:
    dropping it would give the table a default placement without anyone having said no, and D035-092 refuses it. The properties that
    move the data files are still checked as locations.
  - *Evidence.* 24 new tests in `test_config_hygiene.py` (the hostile table served over real HTTP, 14 keys that must not pass, the six
    families that must, a value that is not text, the provider) and 2 premise tests; 18 mutations of the filter, all caught (one needed a
    corrected mutation to be applied). `runners/lake`: 424 passed, 1 skipped; `ruff check`, `ruff format --check` and strict `mypy` clean.
- **D035-095 — An attempt writes its data files under one directory, and the host deletes exactly that directory's files when a commit
  is refused.** T035-057, third part (D035-030 step 4, FR-035-020). **Decided by: delegate** (the owner delegated the decisions of this
  feature).
  - *Measured* (2026-11-06, PyIceberg 0.12.0, a SQL catalog and a file warehouse). Setting `write.data.path` on the loaded table object, in
    memory, to `<data location>/<attempt id>` puts every data file of the next append in that directory, and the stored table never has the
    property. A commit that is refused (409) leaves **only the data file** there: PyIceberg removes the manifests it built for the refused
    commit (the metadata directory is the same before and after) but cannot know the data files, which is the leak FR-035-020 asks the host
    to close.
  - *`pin_attempt_directory(table, attempt_id)`* (`runners/lake/sagat_lake/attempt.py`) sets it and returns the directory. The data
    location is the table's own `write.data.path` or `<location>/data`, computed from the properties and not by asking PyIceberg for
    its location provider, which imports the class a table names. The id is 1 to 64 letters, digits and hyphens and starts with a letter or a
    digit, so it cannot name another directory. A table laid out by hash (`write.object-storage.enabled`) keeps its files below the directory.
  - *`remove_attempt_files(table, directory, attempt_id, warehouse_locations=...)`* is the dangerous half and trusts nothing. It deletes
    only below a directory that is inside the warehouse by path (the rule of D035-092, so a `..`, a `#`, an encoded dot are not) and whose
    last segment is the attempt's own id: the whole data location, another attempt's directory, an id that starts or ends like the attempt's,
    a directory below the attempt's and a way out are all refused with `WORKER_ERROR` before anything is listed, because they would be a bug
    of the host and the host must not delete on the strength of one. A directory that does not exist has nothing in it; removing twice is
    removing once.
  - *Evidence.* 40 tests in `test_attempt.py` against a real SQLite catalog with a file warehouse, written first and seen failing (the module
    did not exist); 21 mutations of `attempt.py`, all caught once the harness stopped caching bytecode (see below). `runners/lake`: 481
    passed, 1 skipped; `ruff check`, `ruff format --check` and strict `mypy` clean.
  - *The mutation harness had a flaw, found here.* Two mutants of the same size written within one second share a bytecode cache entry, so
    the second was judged by the first one's code and was reported "caught" without being tried. With `PYTHONDONTWRITEBYTECODE=1` the
    mutation suites of D035-092, D035-093 and D035-094 were run again: no survivor appeared in them (the trailing-slash survivor of this
    module was the only one the flaw had hidden, and it got its test).
- **G035-044 — Removing the files of an attempt is written only for PyIceberg's own file IO.** `remove_attempt_files` lists and deletes with
  `pyarrow.fs` through `PyArrowFileIO`, which serves S3 and GCS. The file IO of ADLS is fsspec's (D035-050); for it the function refuses
  with `WORKER_ERROR` and deletes nothing, a test shows it with the fsspec IO on a local warehouse. *Exit condition:* write the fsspec
  listing and test it against an Azurite container (the compose file of the repository would need the component, pinned, with a healthcheck),
  or decide that an ADLS attempt leaves its orphans to the lifecycle rule of the storage account.
- **D035-096 — A number written to cost something is refused cheaply, and a value is never stored as zero.** Fixes three defects of the
  mapping of D035-091, found by probing it with hostile numbers before writing the readers. **Decided by: delegate** (the owner delegated
  the decisions of this feature).
  - *Measured* (2026-11-06, Python 3.12). Records are read with `parse_float=Decimal`, so `1e1000000` is a `Decimal` of one digit with
    an exponent of a million. Mapped to an `int` or `long` column, `int()` of it built a number of a million digits: **20 seconds** of CPU,
    quadratic (`1e100000` took 0.2 s), and `1e999999999` does not return. A string of more than 4,300 digits raised Python's own
    `ValueError` ("Exceeds the limit (4300 digits) for integer string conversion") out of the mapping, which would have ended the attempt with
    the wrong failure instead of `OVERFLOW`. And `1e-400` into a `double` was stored as `0.0`, a value that changes by all of itself.
  - *The rule.* An integer column judges the size of a `Decimal` before converting it (more than 20 digits cannot fit in 64 bits; a zero is
    zero whatever its exponent); a string of digits is judged by its digits without the leading zeros and converted from them; a value that is
    not zero and would be stored as zero in a `float` or a `double` column is `OVERFLOW`, because it is out of the range of the column as a
    large one is. The published type table says so.
  - *Evidence.* 8 test functions (38 cases) written first (a time guard fails the test that takes more than two seconds; the huge exponents were not run
    against the old code beyond one million, since a conversion in C cannot be interrupted), 12 mutations of the new code, all caught after
    one more test for zero in a `float` column. `runners/lake`: the mapping suite 102 passed; whole suite below.
  - *Not changed:* a number with a large negative exponent into a `decimal` or an integer column is `SCALE_OVERFLOW` or `TYPE` at once; a
    huge positive exponent into a `decimal` is `OVERFLOW` at once (the context precision bounds `quantize`); both were measured and are tested.
- **D035-097 — The NDJSON of an attempt is read a bounded line at a time, and what is wrong carries nothing.** T035-057, fourth part
  (D035-033, D035-063). **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *`records.read_ndjson(path, budget, max_line_bytes=...)`* (`runners/lake/sagat_lake/records.py`) yields the records of one file in
    order. A line is at most `max_line_bytes` (16 MiB by default), terminator included, and it is read with `readline(limit + 1)` so that a
    line with no end is never read whole: a test spies on the calls and none asks for more than a line may hold. The files of a step share a
    `Budget` of bytes (the contract's `maxDecodedBytes`), spent per line read, blank lines included; going over either bound is
    `ICEBERG_LIMIT_EXCEEDED`.
  - *What is refused.* Text that is not UTF-8 (a byte order mark and UTF-16 or UTF-32 included: the decoding is explicit, since
    `json.loads` on bytes would guess); a line that is not one JSON object; an object that repeats a key at any depth (the second would
    silently replace the first, and so would the same key written two ways); nesting deeper than the decoder goes; a blank other than the
    four of JSON (a no-break space is not an empty line). All are `ICEBERG_SCHEMA_MISMATCH`, reason `TYPE`, with **no column, no line and
    no position**, since the contract has no code for a malformed line and a value must not travel in a failure. An integer of more than
    4,300 digits, and an exponent a `Decimal` cannot hold, are reason `OVERFLOW`: outside the range of every column.
  - *What is kept.* A number keeps the digits it was written with (`parse_float=Decimal`), `NaN` and the infinities reach the mapping, which
    lets only `float` and `double` hold them, and a blank line or a Windows line end is not a record.
  - *Not here:* the Parquet reader (it needs a decision on how a Parquet column maps to the table: checked casts, and what the step's
    JSON-pointer mapping means for it), the Arrow batches, and the cap on a step's total (the runner sets it, 8 times the input ceiling).
  - *Evidence.* 50 tests in `test_records.py`, written first and seen failing (the module did not exist); 24 mutations, all caught (one by a
    hang, which is a legitimate catch of a loop that never ends); two survived the first tests and each got one (the unbounded `readline`, which
    only a spy can see, and the Unicode blanks). `runners/lake`: 531 passed, 1 skipped; `ruff check`, `ruff format --check` and strict
    `mypy` clean.
- **D035-098 — The mapped rows reach PyIceberg as Arrow batches built against a storage schema and cast to the table's own.** T035-057, fifth
  part (D035-033). **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *Measured* (2026-11-06, PyIceberg 0.12.0, pyarrow 26.0.1). `RecordBatch.from_pylist` cannot build Arrow's uuid extension type from Python
    values (`ArrowNotImplementedError`), but it builds the fixed binary of 16 that the type is stored as from a `uuid.UUID`, and a cast of
    that storage to the extension type works at the top level and inside a struct, a list and a map. So `batches.to_batches` builds the rows
    against a **storage schema** (every uuid a `binary(16)`, nested positions included) and casts each batch to `schema_to_pyarrow` of the
    **table's own schema**, which carries the uuid type and the field ids PyIceberg checks. The ids are the table's, not those of the schema a
    table was created from: `create_table` renumbers nested fields, and an Arrow schema built from the old numbers is refused by the append.
  - *Evidence of fidelity.* A row of every Iceberg type (boolean to uuid, struct, list, map, a list of structs with a uuid inside, a map with
    uuid values and one with uuid keys, the edges of every range, `NaN` and the infinities, the earliest and latest date and timestamp) is
    mapped, batched, appended and scanned back, and what comes back equals what went in.
  - *Bounded.* A batch is cut at `max_rows` (1,024) or when it has taken `max_bytes` (32 MiB) of what the reader spent on its rows, read
    from a `meter` (the `Budget` of D035-097); a row past the limit is a batch of one; rows are taken as they are needed, which a test checks.
  - *Evidence.* 28 tests in `test_batches.py`, written first and seen failing (the module did not exist); 28 mutations, all caught after the
    survivors asked for three things: a test of the byte limit at its edge and of a meter that does not start at zero, and the removal of code
    that did nothing (the explicit uuid-to-bytes conversion, which pyarrow does itself and the round trip proves, the field metadata of the
    storage schema, which the cast restores, and the copy of each row).
- **D035-099 — A map key is primitive.** Found by the batches work. A struct, a list or a map is a legal key in the format and an unhashable
  one in Python: `mapping._map` looks for the same key twice with a set and crashed with `TypeError: unhashable type: 'dict'`, which would
  have ended the attempt with the wrong failure. A key of another type than a primitive is now `ICEBERG_SCHEMA_MISMATCH`, `TYPE`, with the
  column, for a non-empty and an empty map alike (the table cannot be written either way). 3 cases written first, 4 mutations, all caught.
- **G035-045 — PyIceberg 0.12 streams a write only into an unpartitioned table.** `Transaction.append` accepts a `RecordBatchReader` and
  microbatches it by `write.target-file-size-bytes`, but passing one for a partitioned table raises `NotImplementedError` (a test shows
  both). The only partition transforms the runtime applies are `identity` and `void` (G035-040), and an identity-partitioned table is
  partitioned: for it the append builds a whole Arrow table in memory. D035-033 and D035-046 say append streams; for a partitioned table it
  does not, so its effective ceiling is the lower one that overwrite and upsert have. *Exit condition:* the append mode of T035-057
  chooses the stream or the table by the partition spec, and the effective ceiling of the step says which (T035-046).
- **G035-046 — PyIceberg 0.12 writes a null list of structs and a null map with struct values as empty.** Found by the round trip. The
  projection that PyIceberg runs on every write rebuilds a list whose elements are structs with
  `LargeListArray.from_arrays(list_array.offsets, value_array)` and a map whose values are structs with `MapArray.from_arrays(...)`
  (`io/pyarrow.py`, `ArrowProjectionVisitor.list` and `.map`, citing apache/arrow#38809), and neither keeps the validity of the container: a
  null becomes `[]` in the **data file**, not only in a scan. A null list of integers, a null struct and a null map of integers are kept;
  so is a null element of a list of lists of structs, but a null at the top, inside a struct, is lost. `batches.to_batches` refuses a null
  wherever the type is a list of structs or a map with struct values, with the column (`points`, `box.points`, `grid[]`, `index{}`), as
  `ICEBERG_SCHEMA_MISMATCH`, `TYPE`: the contract has no reason that fits better, and `TYPE` is in its closed set. The refusal is
  wider than what was measured because one too wide costs a failed step and one too narrow costs a value. **It also refuses a record that
  leaves such a column out**, since an optional column that is absent is null. A premise test fails when a newer PyIceberg keeps the null.
  *Exit condition:* either a PyIceberg that keeps the validity, or the host patches the two methods with the mask (a pinned version, a
  version guard and the same round trip), or the host writes its data files itself; until then a table with an optional list of structs
  takes only records that name the column.
- **D035-100 — The append mode: one commit per search, a directory per try, and nothing deleted that a landed commit may reference.**
  T035-057, sixth part (D035-030, D035-031, FR-035-018, FR-035-020). **Decided by: delegate** (the owner delegated the decisions of this
  feature).
  - *The flow* (`runners/lake/sagat_lake/modes/append.py`, `append(load_table, request)`). Load the table; check that it can be written
    (D035-092); search the history of the ref for the effect key and, if it is there, return that commit's receipt without writing a file
    (`foundByLookup`); otherwise write the files and commit once, asserting the head that was read (D035-093); then decide from the answer.
  - *A refused commit* (`CommitFailedException`: the catalog says it did not land) deletes the files of that try, loads the table again,
    searches again and commits again, up to `max_commit_retries`, then `ICEBERG_COMMIT_CONFLICT`. A race is tested for real (another writer
    commits between the load and the commit), with the number of commit requests counted at the catalog.
  - *An unknown outcome* (any error after the commit request was sent that is not a refusal: a 500, a lost connection) **deletes nothing**,
    because the commit may have landed and may reference the files. The host loads again and searches: the key is there, and that commit is
    the receipt; or it is not, and a new try asserts the head it reads, so a request still in flight and the new one cannot both land. Beyond
    the bound, `OUTCOME_UNKNOWN`, with every try's files left in place (D035-031). Whether a commit was sent is not guessed: the catalog's
    `commit_table` is wrapped for the duration of the try.
  - *A directory per try* (`<attempt id>-<n>`, the attempt id at most 60 characters), so the files of a refused try can be deleted without
    touching those of a try that is still unknown. When the step ends committed or found, the directories of the unknown tries are cleaned
    of every file the landed commit does **not** reference (read from its manifests) and the files it does are kept. A file that cannot be
    deleted, or a directory that cannot be listed, is counted in the receipt as `uncommittedFiles` and never raised.
  - *Found by testing a failure in the middle of a stream: PyIceberg's writer threads outlive the exception.* `ThreadPoolExecutor.map`
    consumes the whole iterable of write tasks before it returns anything, and when the reader raises (a bad record in the thousandth
    line) it raises with writes still running; a deletion made at once missed two files (the test failed on every run, and passed as soon as
    a debug print slowed the deletion down). Before deleting the files of a try that failed before its commit, the host waits for the pool
    to be idle: one task per thread meets the others at a barrier, so none returns before every thread has finished what it had been given.
    If they cannot be waited for within the time, **nothing is deleted** and the leftover is counted: a file deleted and then written again
    is worse than one that stays. `ExecutorFactory.get_or_create()` is PyIceberg's public accessor; the number of threads is read from the
    executor's `_max_workers`, and a missing attribute is treated as "cannot wait".
  - *The receipt* is the `iceberg` branch of `receipts.schema.json`: `COMMITTED` with the snapshot id, sequence number, schema id, format
    version, `rowLevelMode: NONE` and the counts read from the snapshot's own summary (`added-records`, `added-data-files`,
    `added-files-size`), each checked against the contract's pattern for a count, so a summary written by someone else cannot put anything
    else in a receipt; or `EMPTY_NO_COMMIT` with no snapshot. No records means `ICEBERG_SCOPE_VIOLATION` unless the step allows it.
    `ignoredFields`, `uncommittedFiles` and `priorAttemptMayHaveLeftFiles` appear only when they are true or non-zero.
  - *A stream or a table.* An unpartitioned table is written from a `RecordBatchReader` (bounded memory); a partitioned one from a whole Arrow
    table (G035-045).
  - *Evidence.* 53 tests in `test_append.py` through the REST catalog over real HTTP with a real SQL catalog behind it (the happy path, a
    repeat, a key deep in the history, a race, conflicts past the bound, a lost response with the commit applied, a 500 and a closed
    connection with it not applied, an outcome that stays unknown, an unknown try's files cleaned after a later commit, no records, a bad
    record with and without files already written, a storage error among files, a budget too small, a table that cannot be written, a
    missing ref, a partitioned table, a branch, hostile counts in a snapshot someone else wrote, and the leftovers in the receipt),
    written first and seen failing (the module did not exist). 52 mutations of `append.py`: 11 survived the first tests and each got a
    test (the length of an attempt id, negative retries, files deleted after an error that is not a `Failure`, a deletion made although
    the writers could not be waited for, leftovers counted in the receipt, the ref passed to the commit, the batch bounds and the meter
    reaching the batches, the check of a count); a twelfth, a distinction between a request library's error and a storage error, had
    no case at all, because PyIceberg makes no catalog call while it writes, and the clause was removed. The files of `attempt.py` got 6
    more (what is kept, what fails, what a directory is). All caught in the end. `runners/lake`: 655 passed, 1 skipped; `ruff check`,
    `ruff format --check` and strict `mypy` clean.
- **G035-047 — The host has no entry point yet.** `append(load_table, request)` is a function; nothing builds an `AppendRequest` from the
  one JSON line of `sagat.lake.v1`, loads the catalog from the pinned connection (with `Hygiene.accept_table` and a request timeout,
  G035-041 and G035-043), reads `inputs` in Parquet, or writes the result line. *Exit condition:* `host.py` gains the `icebergWrite`
  operation that does, tested through the protocol; overwrite and upsert follow.
- **D035-101 — The request line is checked field by field, against the limits of the domain.** T035-057, seventh part (G035-047, first
  half). **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *`request.parse_write(document, attempt_dir=...)`* (`runners/lake/sagat_lake/request.py`) turns the one JSON line of `sagat.lake.v1`
    into a `WriteRequest`: the table, an `AppendRequest` and the pinned connection. The runner is the platform's own, but the line carries
    what people write (a table, a mapping, a ref) and a connection with its credentials. A field the host does not know, one that is
    missing and one that does not hold what the contract says are the same break, `LAKE_HOST_PROTOCOL`, with no field name and no value
    (`Failure` has no free text). The things that are not served yet are breaks too: another mode, an `overwrite` or `upsert` section, a
    Parquet input; the docstring of `host.py` already says a request for an operation that is not there is answered so, never with a
    pretence.
  - *The limits are those of the domain* (`crates/domain/src/database/iceberg_write.rs`), so the host takes everything the platform
    publishes and nothing more: names and refs 1 to 255 **bytes** with no control character (Unicode `Cc`, not `isprintable`, which
    would refuse a no-break space the platform accepts), 1 to 8 namespace levels, a mapped column up to 255 bytes, a JSON pointer up to
    512 bytes and 16 segments with only the escapes `~0` and `~1`, at most 1,024 mappings and 32 inputs. A first version used the
    wrong limits (characters, 16 levels, a restricted charset for the ref); reading the domain showed that it would have refused
    `feature/x` and any table the platform lets a user name with an accent, and the tests now say what the platform publishes is accepted.
  - *Paths are resolved here.* An input is `in/<name>` with a name of letters, digits, `.`, `_` and `-` that does not start with a dot, and is
    resolved below `<attempt dir>/in/`: nothing downstream follows a name out of the attempt's directory. The attempt id is the name of
    the attempt directory and must be a valid one (at most 60 characters, for the numbers each try adds).
  - *Counts and times.* The two byte limits are decimal strings within int64 (the five-language representation), the retries an integer
    from 0 to 16 and not a boolean, the times `YYYY-MM-DDTHH:MM:SS[.f{1,6}]Z` read to milliseconds. The snapshot properties are a closed set
    of five keys: PyIceberg refuses a key it owns, and the host does not let a request name any other.
  - *The connection* (`catalog`, `storage`, `warehouseLocations` of 1 to 16, `credentials` as text to text) is kept whole for the catalog
    loader that does not exist yet, and is not part of `repr`, so a log of the request cannot carry a credential.
  - *Evidence.* 141 tests in `test_request.py` (every field missing, a field unknown, 71 wrong values, 23 wrong sections, the limits at
    their edge from both sides, what the platform publishes up to its limits, the repr, the failure), written first and seen failing
    (the module did not exist); 68 mutations, all caught after the corrections that the survivors asked for (a mapping that is not a list,
    the limits taken from the domain). One mutation, a time with an offset, is an equivalent mutant: the next step refuses it anyway.
- **D035-102 — The REST catalog of the host believes nothing the catalog says about itself, its tables or its credentials.** T035-018
  follow-up, closing the REST half of G035-041 and G035-043 with a real client. **Decided by: delegate** (the owner delegated the decisions
  of this feature).
  - *`rest_catalog.LakeRestCatalog`* is PyIceberg 0.12.0's `RestCatalog` with three changes, each where an answer would otherwise be used
    before anyone looked at it. (1) `/v1/config` is read through `Hygiene.catalog_config` (only `warehouse` and `prefix` are taken) and
    the result is overridden by what the connection declares, with the file IO pinned. (2) A table's response goes through
    `Hygiene.accept_table` and the new `Hygiene.storage_credentials` before `_response_to_table` builds its file IO: the locations must lie in
    the warehouse, the table's properties pass the allowlist of D035-094, the config and the vended credentials keep only the keys of the
    declared storage, and a credential for a prefix outside the warehouse is dropped and counted. (3) Every request of the session has a time
    limit unless the caller names one (`session.request` is wrapped, since PyIceberg posts a commit with `session.post(url, data=...)` and
    no timeout).
  - *Measured with PyIceberg's own client against a catalog that lies, over real HTTP.* The overrides of `/v1/config` win over the client's
    own properties: `uri` moved the client to another host and `py-io-impl` was replaced, in a plain `RestCatalog` (a test says so, and
    fails when a newer PyIceberg stops doing it). A table whose properties name `py-io-impl` gets that class built by a plain client and
    not by this one. Against a catalog that does not answer, `load_table` and the first request of the constructor end in
    `ReadTimeout` within the limit instead of waiting without end.
  - *`_fetch_config` is a copy* of PyIceberg's with the filtering (the one place where `self.uri` and the properties are set from the
    response); the version is pinned in the lock and the tests above show the behaviour, so an upgrade that changes it is noticed.
  - *Left open.* The loader that builds the properties of this catalog from the connection (`auth` oauth2, bearer or sigv4, the storage
    block, the credentials) and the `icebergWrite` operation of `host.py` are not here (G035-047). Glue, Hive and JDBC catalogs read the
    metadata file themselves and do not pass through `accept_table`: G035-041 stays open for them. A request timeout for those
    catalogs' own clients is also open.
  - *Evidence.* 16 tests in `test_rest_catalog.py` (the lying config, the lying table, the vended credentials declared and not, the storage
    credentials of the warehouse and of others, the time limits, the pin when the caller forgot it) and 13 more in `test_config_hygiene.py`
    for `storage_credentials`, written first and seen failing (the module did not exist); 26 mutations, 25 caught in the end after three
    tests (an exact message for the timeout check, which urllib3 would otherwise match, and the pin when the caller did not name the IO);
    one, a timeout of `0` from a caller replaced by the default, is an equivalent mutant because urllib3 refuses `0`. `runners/lake`: 683
    passed, 1 skipped; `ruff check`, `ruff format --check` and strict `mypy` clean.
- **D035-103 — The runtime has `pyiceberg-core`, and every partition transform is written.** Closes G035-040. **Decided by: owner**
  (2026-11-06, "Seguir as recomendações", after being asked: "Porque precisamos do pyiceberg core?").
  - *Why.* PyIceberg 0.12 computes the partition value of each row while it writes. It does `identity` and `void` itself and delegates
    `year`, `month`, `day`, `hour`, `bucket[n]` and `truncate[n]` to the native package `pyiceberg-core`, which is the iceberg-rust
    transforms compiled for Python (its home page is `rust.iceberg.apache.org`). Without it the host refused almost every real table,
    which is partitioned by day or month. Reimplementing the transforms in Python was rejected: `bucket` is a murmur3 hash and an error
    would put rows into the wrong partition without a sign.
  - *What changed.* `requirements.in` gains the extra (`pyiceberg[pyarrow,pyiceberg-core,...]==0.12.0`); both locks were regenerated with the
    command of their header and the same `--exclude-newer`, and the diff of each is exactly `pyiceberg-core==0.10.1` and its six wheel
    hashes (no other package moved; the lock is now 79 packages). `compat.py` accepts all transforms of the format: `bucket[N]` and
    `truncate[W]` with a positive 32-bit integer without a leading zero, and still refuses a name that is not one.
  - *Evidence.* (1) `tests/test_transforms_write.py` writes to a table of each transform (ten cases, on timestamp, string and long columns),
    scans it back, and asks the rule about the same table: eight of the ten failed with `NotInstalledError` before the lock changed, and
    pass now. (2) `test_package.py` imports the native module and checks the values of `bucket`, `truncate`, `day`, `month` and `year`
    against numbers worked out in the test (19,000 days after 1970-01-01 is 2022-01-08), so a wheel that imports and computes the wrong
    partition fails. (3) `uv pip install --dry-run --python-platform {x86_64,aarch64}-manylinux_2_28 --only-binary :all: --require-hashes`
    resolves `pyiceberg-core==0.10.1` as a wheel for both Linux platforms, which is what the Dockerfile installs. (4) `pip-audit -r
    requirements.lock --require-hashes --disable-pip` (the command of the CI): no known vulnerabilities. (5) `runners/lake`: 859 passed, 1
    skipped; `ruff` and strict `mypy` clean.
  - *Not run here.* The image build, `smoke-image.sh` (a line was added that loads the native transforms inside the read-only container) and
    the SBOM check need Docker and run in the release CI. The SBOM check names the packages it requires in
    `.github/workflows/ci.yml`; `pyiceberg-core` should be added to that list at the next change of the workflow, which this change
    does not make because a change of `.github/workflows` runs the whole stack.
  - *Cost.* A native (Rust) wheel in the sandbox image. It is built by the Apache Iceberg project from the same source as the crate in
    `tests/Cargo.toml`, is hash-locked, and the sandbox has no network and a read-only root.
- **D035-104 — The choice of PyIceberg over iceberg-rust is reviewed again, and stands.** Asked by the owner on 2026-11-06 ("Reavalie o
  iceberg rust"). **Decided by: owner** (told the result, "Seguir com o plano então").
  - *What was checked* (2026-11-06; releases, the pull requests D035-026 named, the epic and two files of the source through the GitHub API).
    iceberg-rust is at **v0.11.0 (2026-11-03)**. Its notes add variant, the reading of v3 deletion vectors, encryption, the endpoints of
    `/v1/config`, an `AuthManager`, and `write.metadata.path`, and no write action. FastAppend is still the only transaction action;
    `FastAppendAction` has no branch parameter (its methods are `with_check_duplicate`, `add_data_files`, `set_commit_uuid` and
    `set_snapshot_properties`, and its tests fix `MAIN_BRANCH`). The overwrite action (PR #3300) is open, updated on 2026-11-06, waiting for
    #3564 and the refactor #3266. RowDelta (#2203) is open, copy-on-write over data files only (`add_delete_files` returns
    `FeatureUnsupported`) and in conflict with `main`. The epic of the missing write actions (#2269) was closed as not planned on
    2026-10-03. The Hive Metastore catalog's `update_table` still returns `FeatureUnsupported`.
  - *Against the scope* (modes append, overwrite and upsert; named branches; REST, Glue, Hive and JDBC catalogs; the commit under our
    control; credentials vended by the REST catalog): iceberg-rust covers append to `main` with snapshot properties and nothing else of it.
    D035-026 and ADR-0071 stand.
  - *Trigger to look again:* a published release with the overwrite action, a branch on the commit actions and a Hive `update_table`. The
    gain would be an image without Python and one language in the platform. Look at each release of iceberg-rust.
  - *Not verified:* the Glue and JDBC catalogs, ADLS and GCS through OpenDAL, and the use of REST `storage-credentials` in the code (only
    its absence from the release notes).
- **D035-105 — The pinned connection is turned into the catalog client by the host, and the catalog is asked only for what the
  connection declares.** T035-018 follow-up; closes the loader half of G035-047. **Decided by: delegate** (the owner delegated the
  decisions of this feature; the shape of `credentials` was announced to the owner on 2026-11-06 and not objected to).
  - *The shape of the request's `connection`* is the one of `data-model.md` §1, as stored, with the secrets next to it:
    `catalog: {type: "rest", rest: {uri, warehouse, accessDelegation, prefix?, auth?, remoteSigning?}}`,
    `storage: {type: "s3", credentials: "static" | "vended", s3: {region, pathStyle, endpoint?}}`, `warehouseLocations`, and
    `credentials: {catalog?: {...}, storage?: {...}}`, where each is the secret document of the data model (`{clientId, clientSecret}`
    for OAuth2, `{token}` for a bearer, `{accessKeyId, secretAccessKey, sessionToken?}` for S3). `request.py` checks that `credentials`
    has those two keys at most and that each is text to text; `connection.py` checks the rest. **`contracts/lake-runtime.md` still says
    `"credentials": {...resolved values...}`; it is updated with the Rust runner that writes it (T035-035), because a change of a
    contract file runs the whole stack in CI.**
  - *`connection.open_connection(connection)`* returns the hygiene of the attempt (storage type, warehouse locations, what may be
    delegated), the client properties, and `catalog(request_timeout=...)`, which builds the `LakeRestCatalog` of D035-102. Serves a
    REST catalog with no authentication, OAuth2 or a bearer token, over S3 with static or vended credentials. A Glue, Hive or JDBC
    catalog, SigV4, remote signing, GCS, ADLS and an assumed role (`roleArn`, `externalId`) are not served yet and are
    `LAKE_HOST_PROTOCOL`, like anything unknown or of the wrong shape. A static credential that is missing is told apart, with the
    hygiene's own `DATABASE_CREDENTIAL_UNAVAILABLE`.
  - *Consistency rules, each refused as a break.* Vended storage needs `accessDelegation: vended-credentials`, static storage needs
    `none`; credentials sent for vended storage (nothing would use them in the sandbox); catalog credentials that are not those of its
    `auth`, or any for a catalog without one; an OAuth2 client id with a `:` (PyIceberg joins the two with it). An address has no user
    information, query or fragment, and is `https`, or `http` on the loopback only (the catalog's, not the token server's or the
    storage's), so a secret is never sent in the clear to another host.
  - *Found while reading PyIceberg: it asks every catalog for vended credentials by default.* `_config_headers` does
    `session.headers.setdefault("X-Iceberg-Access-Delegation", "vended-credentials")`, and a `header.` property can say anything. A
    connection that declared static credentials still invited the catalog to hand over others; `Hygiene` already dropped them on
    arrival, but the invitation was sent. `Hygiene.access_delegation` now says what the connection allows (`vended-credentials`,
    `remote-signing`, both, or nothing), and `LakeRestCatalog` sends exactly that header and none when there is nothing to ask. A
    test shows the plain client sending it, and one shows a `header.` property unable to ask for more.
  - *Evidence.* 91 tests in `test_connection.py` (what is built, each consistency rule, what is not served, every part missing or
    unknown, the limits of addresses, regions, prefixes and locations, and two with the catalog it opens against the stub that lies:
    one loads a table with the file IO pinned, the credentials in and no delegation header sent, one gets a `ReadTimeout`), 2 groups in
    `test_config_hygiene.py` and 5 cases in `test_rest_catalog.py` for the delegation, and `test_request.py` moved to the new shape.
    Written before the module; I did not run them before it existed, so they were not "seen failing" except the 8 of the delegation,
    which were. `runners/lake`: 965 passed, 1 skipped; `ruff` and strict `mypy` clean. No mutation pass (OD-2026-11-06-2).
- **G035-048 — The Glue, Hive and JDBC catalogs, SigV4, remote signing, GCS, ADLS and an assumed role have no loader.** Each is a
  `LAKE_HOST_PROTOCOL` today. They read the metadata file themselves, so G035-041 and G035-043 apply to them again. *Exit condition:*
  one loader per catalog and storage with its own hostile test, in the order the owner's connections need them.
- **D035-106 — The host serves `icebergWrite`: parse, open the catalog, append, answer with the receipt.** T035-057, last part of the host
  side; closes G035-047. **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The operation* (`runners/lake/sagat_lake/write.py`, `iceberg_write(attempt_dir)`; `host.main` builds `operations_for(attempt_dir)` and
    passes it to `run`) does what the protocol asks in order: `request.parse_write`, `connection.open_connection`, `modes.append.append`,
    and the result is the receipt. PyIceberg blocks, so the whole of it runs on a thread (`asyncio.to_thread`) and the event loop keeps writing
    the heartbeats of the protocol; a test with a catalog that takes 0.4 s sees at least three heartbeats before the result, which an operation
    on the loop could not write.
  - *What is mapped to a code, in this one place.* A catalog that refuses the client (401, 403, an OAuth2 error) is
    `DATABASE_AUTHENTICATION_FAILED` (the code of 017 for it; the contract has none of its own for a catalog). A catalog that is down, slow or
    overloaded (any `requests` error, a time limit, 5xx, 429) is `CATALOG_UNAVAILABLE`, which is retryable. A table or namespace the catalog
    does not have is `ICEBERG_TABLE_NOT_FOUND`. A location the hygiene refuses carries the hygiene's code (`ICEBERG_LOCATION_NOT_ALLOWED`),
    a missing static credential `DATABASE_CREDENTIAL_UNAVAILABLE`, an `OSError` of the storage `STORAGE_UNAVAILABLE`. `requests`' errors are
    `OSError`s too, so the catalog is asked about first. Anything else is `WORKER_ERROR`, and `host._outcome` writes only the name of the
    class to standard error.
  - *Not used: the deadline.* The request carries `limits.deadlineAt` and `parse_write` reads it, but the host does not act on it. The
    runner enforces it by killing the container (contract, "One attempt"); the host bounds each request to the catalog (30 s) so that it
    never waits without end between two of the runner's checks.
  - *The seam.* `iceberg_write(..., open_catalog=...)` takes the function that opens the catalog. The success path needs a write, and a
    real connection speaks S3 and a write would leave the machine; the tests of the success path hand it a plain REST client to the stub
    whose warehouse is a directory, and those of every failure that comes before a write use the real opener, so the request, the
    connection, the hygiene and the catalog are the ones the host runs.
  - *Evidence.* 21 tests in `test_write_operation.py` through the real protocol (a relay double, the host's `run`, the stub catalog over
    HTTP): the receipt and the rows, a repeat that finds its commit, the heartbeats during a slow catalog, the mapping and flags reaching the
    append, a record that does not map, a request the host cannot serve, a missing table, a table said to be outside the warehouse, 401, 403,
    500, 503 and 429, a catalog nobody listens to, a catalog that does not answer in time, a missing static credential, a canary credential
    that must not reach the result or either standard stream, an unexpected exception (only the class name on stderr), and a failure of the
    closed set raised by the opener. The two old host tests that used `icebergWrite` as "an operation not served" use `objectWrite`.
    `runners/lake`: 986 passed, 1 skipped; `ruff` and strict `mypy` clean. The test of the whole line through the real `host.py` as a
    subprocess and the real relay of the Rust runner (T035-031) are not here.
- **G035-049 — Nothing runs the host yet.** The Rust side is not written: the attempt driver (T035-035) that leases the task, writes
  `in/`, opens the relay and asks the sandbox runner for the container, the `lake` profile of the sandbox runner, the image target, the
  admission of a run that contains an Iceberg write (T035-055), and the update of `contracts/lake-runtime.md` to the shape of `connection`
  and `credentials` of D035-105. Also open: the Parquet input, overwrite and upsert, the catalogs and storages of G035-048, and
  `priorAttemptMayHaveLeftFiles`, which the runner has to set from its lease. *Exit condition:* the driver, tested against the real host in
  its container.
- **D035-107 — A Parquet blob is a table of records, read into the JSON form of the mapping table, so one mapping decides for both
  formats.** T035-057, the Parquet input that G035-039 left open; the question the earlier note called a decision ("checked casts or the
  JSON-pointer mapping"). **Decided by: delegate** (the owner delegated the decisions of this feature).
  - *The choice.* Each row of the file is a record, and each cell is turned into the representation `docs/guides/iceberg-type-mapping.md`
    publishes for JSON, then the record goes through `mapping.map_record` and the step's JSON-pointer `mapping` exactly as an NDJSON line
    does. Rejected: casting the Arrow table straight to the table's schema (D035-033 said "checked casts only"). It would have been a
    second mapping table with its own lossless rules to publish, test and keep equal to the first, for a path whose only consumer is the
    same step. The cost of the choice is a Python object per cell, which is why the rows are read a batch at a time.
  - *The conversion* (`runners/lake/sagat_lake/parquet_input.py`, `read_parquet`): a date as `YYYY-MM-DD`; a time and a timestamp as
    `isoformat`, which keeps the offset the mapping stores as UTC; binary and fixed binary as the typed cell with padded base64; a uuid as
    its canonical text; a decimal as `Decimal`; a list as a list; a struct as an object; a map as an array of `{"key", "value"}`; a
    dictionary-encoded column as its values; integers, floats (`NaN` and the infinities included), booleans and text as they are.
  - *Bounded* (D035-063). The length of the footer is read from the last eight bytes and bounded (16 MiB) **before** the footer is parsed;
    the uncompressed size of each row group, which the footer states, is spent from the step's `Budget` **before** that row group is decoded,
    so the first byte over the limit stops the step with no row of that group delivered; rows are decoded a batch at a time
    (`iter_batches(batch_size=1024, row_groups=[i])`). A footer can understate a row group; `memory.max` of the container is what stops
    that, as the contract already says for the whole host.
  - *Refused.* A file that is not Parquet, is cut, has the wrong magic at either end, a footer of zero or longer than the file, is
    `ICEBERG_SCHEMA_MISMATCH`/`TYPE` with no column. A column named twice at any depth (a row is a dictionary and would keep one), and a
    type the mapping has no form for (a duration, an interval, a union), are `TYPE` naming the column. A nanosecond that would be lost
    (pyarrow without pandas raises "not safely convertible") is `SCALE_OVERFLOW`; one that would not be lost is read. A missing file
    raises `FileNotFoundError`: it is the runner's bug, not the data's, and becomes `WORKER_ERROR`.
  - *Wired in.* `request.py` takes `format: "parquet"` and carries it per file (`InputFile(path, format)`); `modes/append` dispatches on
    it; the budget is the same object for both formats, so 8 times the input ceiling bounds a step whatever its mix of files.
  - *Evidence.* 22 tests in `test_parquet_input.py` (every scalar, the temporal forms, binary in four shapes, a uuid, lists/structs/maps
    down to their cells, a dictionary column, four codecs, a row of every Iceberg type mapped into the table it was written for, eleven
    kinds of file that are not Parquet, the footer bound at its edge, the row-group budget at its edge, a shared budget, a duplicate
    column, the types without a form, the lossy and lossless nanoseconds, a missing file, no value in a failure, bounded batches), a
    test and ten format cases in `test_request.py` and 4 tests in `test_append.py` for the wiring (Parquet and NDJSON in one commit, a cell that does not fit, a file that
    is not Parquet, the shared budget). Written before the module; seen failing for its absence. No mutation pass (OD-2026-11-06-2).
    `runners/lake`: 1022 passed, 1 skipped; `ruff` and strict `mypy` clean.
- **G035-050 — The Parquet reader trusts the footer's sizes.** Decompression bombs inside a page, a column chunk that decodes to far more
  than the row group declares, and a footer that understates, are stopped by `memory.max` of the container and by its deadline, not by the
  host. *Exit condition:* a test that runs the host in the container of the image with a crafted file and the real limits (T035-031/035),
  and, if it is not enough, a limit on the compressed size of a column chunk read from the footer.
- **D035-108 — The receipts the Python host writes are checked against `receipts.schema.json`, the same file the Rust runner's types are
  checked against.** Conformance of the two halves of the wire without running one against the other. **Decided by: delegate**.
  - *Why.* The runner rebuilds each receipt from typed fields with `deny_unknown_fields` (`crates/infra-lake/src/protocol/receipt.rs`) and
    refuses a line that is not exactly what the schema describes. The host and the runner were written apart; a member the host
    spells differently would show only when the driver exists. The Rust tests (`crates/infra-lake/tests/protocol.rs`) validate their samples
    against the schema; this adds the same check on the Python side, so each half conforms to the one contract and the two conform to each
    other through it. The envelope of the request already agrees by reading: the Rust `encode_request` writes `protocol`, `operation`,
    `effectKey`, `firstAttemptStartedAt` and `relay`, which `request.py` expects, and leaves the body, which the Rust builder of T035-035
    has to write in the shape of D035-101 and D035-105.
  - *The checker* (`runners/lake/tests/schema_check.py`, about 70 lines) implements the keywords that file uses (`type`, `const`, `enum`,
    `pattern`, `maxLength`, `required`, `properties`, `allOf`, `$ref`, `unevaluatedProperties: false`) and **reports a keyword it does not
    know**, so a schema that grows one cannot turn the check into a pass. No dependency is added to the image or the lock.
  - *What it found.* Nothing: the four receipts the host writes (committed with `ignoredFields` and `priorAttemptMayHaveLeftFiles`, found by
    lookup, `EMPTY_NO_COMMIT`, and a committed one after a refused try whose files could not all be deleted, `uncommittedFiles`) conform on the
    first run, and the checker rejects seven kinds of wrong receipt (an extra member, a count with a leading zero, a count that is a number,
    an outcome outside the enum, a missing `ref`, format version 3, a snapshot id with letters). The host never writes `droppedValues`.
  - *Not covered.* The failure line (`{"failure": {...}}`) has no schema file; the Rust `HostFailure::from_wire` is its definition, and
    `test_host_protocol.py` checks the Python side against the table of codes in `failure.rs`. Receipts of the other operations come with them.
- **D035-109 — The body of the `icebergWrite` request is built by a typed Rust builder, `WriteAppend`, and the line it writes is a fixture
  that both halves read.** First slice of T035-035. **Decided by: delegate**.
  - *Why.* The runner's envelope (`encode_request`) was typed but the body was an open map, so the one place where the platform's
    definition meets the host's parser was unchecked. A member spelled differently, a limit above what the host accepts, or a secret with a
    newline would show only as `LAKE_HOST_PROTOCOL` from inside a container. The builder refuses all of it before a container exists, and
    the same JSON file (`tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json`) is asserted equal to what the builder
    writes (`crates/infra-lake/tests/write_body.rs`) and is parsed by the host's own `parse_write` and `open_connection`
    (`runners/lake/tests/test_wire_request.py`), so neither half can drift without a test failing on its side.
  - *Shape.* `WriteAppend::new(table, ref, connection, inputs, limits)` with `snapshot_property`, `map_column`, `ignore_unknown_fields`,
    `allow_empty` and `prior_attempt_may_have_left_files`; `into_body()` returns the `Map` for `Request::new`. The table and the mapping are
    the domain's `TableIdentifier` and `ColumnMapping` (rule 6: one definition of what the contract refuses). `Connection` forwards the
    `catalog` and `storage` documents exactly as `crates/infra-postgres/src/database/iceberg.rs::to_json` stored them, so the storage format
    stays defined in one place, and carries the resolved secrets as two text-to-text documents that are left out when empty. Mode sections
    that are not served (`overwrite`, `upsert`) are `null`. `Debug` shows no secret and no value of the body.
  - *What the builder checks that the domain does not own.* The record file name (one path component, 1 to 128 bytes, no leading dot), 1 to
    32 files each named once, at most 16 commit retries, byte ceilings up to `i64::MAX` (sent as decimal strings), the deadline as an
    instant that exists in the calendar (not only its shape), the ref, the 5 snapshot properties the platform writes (D035-016), at most
    1024 mapped columns with no field and no column twice, 1 to 16 warehouse locations of at most 2048 bytes and secrets of at most 16384
    bytes without a byte below `0x20` or `DEL` (the host's own rule for a secret). Every ceiling is in **bytes**, as the host counts.
    `encode_request` also refuses an effect key or a first-attempt instant the host would refuse (`EncodeError::Envelope`) and a line
    longer than the host reads, 4 MiB without its newline (`EncodeError::TooLong`): the `catalog` and `storage` documents are forwarded as
    stored, so that size bound is the only one on them.
  - *What it does not check.* Which connections the host serves. A Glue, Hive or JDBC catalog, GCS or ADLS, an assumed role, remote
    signing and an `http` S3 endpoint are refused by `connection.py` as `LAKE_HOST_PROTOCOL` today (G035-048); a copy of that list in Rust
    would have to move with the host, so the host stays the one place that knows, and admission is where a connection it cannot open
    should be refused with a name. The module doc says so; an earlier draft of it claimed the opposite.
  - *What it found (G035-051, closed).* `ColumnMapping::new` accepted `/a~2`, `/a~` and `/~x`, which RFC 6901 does not allow and the host
    refuses as a protocol break, so a step that validated and published would have failed at run time with `LAKE_HOST_PROTOCOL`. The domain
    now refuses a `~` that does not start `~0` or `~1` (`IcebergWriteError::Field`), at the place the step is written. The contract
    schema's pattern (`^(/[^/]+)+$`) is looser and is left as it is: changing a contract file selects the whole stack, and the domain is where
    the rule is enforced. Test written first and seen failing (`/a~` was accepted).
  - *Independent review (security and correctness reviewers, given the files and the contract, not my conclusion).* No path was found by
    which a value that a person controls changes the structure of the line, leaves `in/` or reaches an envelope member; the file-name
    rule and the pointer rule agree with the host's patterns. Each claim below was checked against the code before I acted on it:
    - *Done.* `into_body()` returned a bare `Map` whose `Debug` prints every secret: it is now a `Body` that shows only a member count
      (also used by `Request`). The deadline was checked for shape only (`2026-02-30`, hour 25, second 60 and year 0 passed the Rust check
      and failed `fromisoformat` in the container); it is now a calendar check, and `firstAttemptStartedAt` and the effect key are
      checked the same way. Ceilings were pinned by ASCII examples only, so swapping bytes for characters would have passed; the tests now
      use `é` at the edge of the ref, the location, the secret and the property value. Two inputs of the same name would have been read
      twice and written twice (the host does not deduplicate); a field or a column mapped twice likewise: both are refused. A request line
      over 4 MiB was bounded nowhere before the host dropped it: refused in `encode_request`.
    - *Host defects found by the review, fixed here (G035-052, G035-053).* `mapping.py` read an array index with `str.isdigit()`:
      `"²"` raises `ValueError` (reported as `WORKER_ERROR`, not as a schema mismatch), `"٣"` selected element 3, and `"01"` selected
      element 1, which RFC 6901 forbids. It is now `0|[1-9][0-9]*` in ASCII. A namespace level or a table named `.` or `..` was
      accepted, and `requests` removes dot segments from the path (checked: namespace `..` and table `t` is sent to `/v1/p/tables/t`, not
      to a namespace); the host refuses them in `request.py` and the domain refuses them in `TableIdentifier::new`. Each test was seen
      failing first (`ValueError` on `²`; five dot-segment cases accepted with the check disabled).
    - *Cross-language ceilings.* `test_wire_request.py` takes the last value the builder allows and the first it refuses for each
      ceiling, the calendar lists, and the envelope, on the host's own parser. It passed on the first run: the host already agreed on all of
      them. That is the intent of the test (it fails the day one side moves a number alone), not a finding.
    - *Not done.* `encode_request` returns a plain `Vec<u8>` that holds the relay token and every secret, and its `Debug` would print
      them; as before, zeroising and not showing it belong to the driver that owns the buffer (T035-035, next slice). A definition
      stored before this change with a mapped field such as `/a~x`, or a table named `..`, now fails to load in `infra-ir`; the feature
      is unreleased and `lake.enabled` is off by default, so none is expected, and I did not search a database for one.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 90 passed (22 in `write_body.rs`); `cargo test -p sagat-domain`: 848 passed, 0 failed
    (new: `a_tilde_in_a_field_starts_only_the_two_escapes_of_a_json_pointer`, `a_dot_segment_is_not_a_namespace_level_or_a_table_name`);
    `runners/lake`: 1084 passed, 1 skipped, `ruff check`, `ruff format --check` and strict `mypy` clean; `cargo fmt --check` and
    `cargo clippy --workspace --all-targets --locked -- -D warnings` clean (the only warnings are the vendored `rskafka`'s);
    `check-architecture.py`, `check-design-docs.py`, `check-agent-config.py` pass; unit tests of `sagat-infra-ir` and
    `sagat-application` (76 and 7) pass. The Rust tests failed first (the builder or the field did not exist, a compile error), and one
    of them was wrong once (the base builder already maps `/amount`) and was corrected, not the code. No mutation pass (OD-2026-11-06-2).
    Not run: `crates/application/tests/definition_service.rs` (Docker); it holds no `~` pointer and no dot-segment name.
- **G035-051 — Closed by D035-109.** A mapped field with a `~` that is not `~0` or `~1` validated in the domain and failed in the host.
- **G035-052 — Closed by D035-109.** `mapping.py` accepted Unicode digits and leading zeros as array indexes and crashed on `"²"`.
- **G035-053 — Closed by D035-109.** A namespace level or table named `.` or `..` changed the path of the catalog request.
- **D035-110 — The control conversation is a function over a stream, the relay hands the host's `CONTROL` connection to the driver once, and
  the rest of the attempt driver is cut into the slices below.** Second slice of T035-035. **Decided by: delegate**.
  - *What I found before cutting.* My earlier note said the sandbox side had nothing for a lake container. It has more: the `lake` profile
    (`container_config`, `crates/infra-sandbox/src/lake.rs`) and the engine calls (`LakeEngine::{connect, create_work_volume, run, start,
    sweep}` and `LakeContainer::{exec, stop}`, `lake/engine.rs`) exist and pass T035-015 against a real engine. What does **not** exist is
    the way the *lake runner* asks for a container: D035-048 says it never receives the engine socket, and `apps/sandbox-runner` today only
    polls the control plane for build and test jobs (its `api.rs` is the client of that, with an admin page on `/livez`). Also, the relay
    answered `CONTROL` (the word the host sends after its token, `host.py`) as an unsupported request: nothing served the conversation.
  - *Done in this slice.* (1) `crates/infra-lake/src/control.rs`, `converse(stream, request, config, began, on_heartbeat)`: writes the one
    request line, reads the host's lines to its one result, and returns the typed `HostResult` or a `ControlFailure` (`Silent` or the
    `ProtocolFailure` of the `Decoder`; both are `LAKE_HOST_PROTOCOL`). A line is read through a limit (`MAX_LINE_BYTES`, a host with no
    newline costs one line of memory); a whole line must arrive within `silence` of the last one (15 s by default, the host beats once a
    second; a line that trickles in a byte a second does not keep it alive; a host that never reads the request is silent too); after the
    result the host has `result_grace` (2 s) to close, and a byte before it is a break while a host that keeps the connection open still
    gave its result. Time is `tokio::time`, so the tests pause the clock. (2) `Relay::take_control()` and the hand-over: after the token, a
    request that is exactly `CONTROL` goes to the driver as a `ControlStream` through a one-shot channel; a second one is refused and
    counted as `request_unsupported`, one that arrives after the driver stopped waiting is closed and not counted, and the tunnels work
    beside it. `handle` and `serve` take a `'static` stream now. The existing test that listed `CONTROL` as unsupported was the gap itself
    and now lists three near misses (`CONTROL now`, `control`, `CONTROL\r`).
  - *A defect found on the way.* `Decoder::beat` computed the window floor with `saturating_sub`, so a beat at exactly zero elapsed was
    already "out of the window" and any number of beats at that instant passed. Now `checked_sub`: before the first second nothing
    expires. Tests: `a_flood_in_the_first_second_is_a_flood_even_at_the_very_first_instant` (seen failing at `0ns`) and
    `a_beat_leaves_the_window_exactly_one_second_after_it_arrived`.
  - *The rest of T035-035, in the order that keeps each slice testable without a container.*
    (a) **Supervisor** (pure): events in (beat, result, lease lost, cancelled, deadline, memory kill, host exited, sandbox refused), a
    decision out (keep going, or kill and report this code), with the clock injected. (b) **Staging**: `/work/<attempt>/` with the
    modes of D035-086 (directory 0770, token 0640, socket 0660 in the runner's group), `in/` files from inline records, `ReadBlob` and a 017
    result, the effective limit checked while streaming (`ICEBERG_LIMIT_EXCEEDED` before any container), removal and zeroising at the end.
    (c) **A `LakeSandbox` port** (start by attempt id and limits, wait for the exit, kill) with an in-memory double for the driver's tests
    and an adapter to the API of (d). (d) **The sandbox-runner API** (below). (e) **`apps/database-runner/src/lake.rs`**: lease, two-phase
    `ResolveDatabase`, the above in order, the receipt rebuilt from the typed result, `CompleteStep` or `FailStep`; registration of the lake
    activities only when `lake.enabled`. (f) Compose and Helm co-location, the image target, the canaries of T035-031 (the real Python host
    through the real relay, which neither language's tests can show; the Python `relay_double.py` is the stand-in until then).
  - *Recommendation for (d), which is G035-022's open half; to be decided with that slice and recorded as an ADR because it adds a server
    to the one process that holds the engine socket.* The lake runner asks the sandbox runner over a node-local Unix socket (a directory
    both mount, `SO_PEERCRED` checked) for three calls: start, wait and kill, as small HTTP/JSON on the `axum` the sandbox runner already
    links, not gRPC (no streaming, no code generation for three calls). **The request carries no image, mount, user, capability or
    command.** The sandbox runner builds the `LakeSpec` from its own configuration (image digest, work volume, the lake group, ceilings),
    allocates the slot (the uid) itself, clamps the limits to its ceilings, and refuses an attempt directory that is not already in the
    work volume under a valid attempt id. It enforces the "lake containers per node" ceiling and gives every container a hard deadline at
    start, so a lake runner that dies leaves nothing running past it (the existing label sweep removes the rest). Refused alternatives:
    the lake runner linking `LakeEngine` itself (gives the credential-holding process the engine socket: D035-048; fine in the integration
    tests, which already do it) and a control-plane-brokered job claimed by the sandbox runner like a build (a table, a lease type and
    polling latency on every attempt, in the files the 018 and 014 sessions are changing; it stays the fallback if a node-local socket
    cannot be provisioned on Kubernetes, which T035-029 decides, and if none co-locates the owner chooses, as that task says).
  - *Independent review (security and correctness reviewers, given the files and the contract, not my conclusion).* Both read the hand-over
    and found no way for a process without the token to get the stream, for two token holders to both get it, for a stream to be lost
    between the sender and the receiver, for a secret to reach a `Debug` or an error, or for the `'static` bound to change what `handle`
    guarantees; `Take` over `BufReader` loses and duplicates no byte across calls, and every edge of the line allowance is right. Each claim
    below was checked against the code, and each defect was seen failing before it was fixed:
    - *Fixed.* **The line limit was a megabyte, and the runner parses a line into a tree before it checks the shape.** A line of `0,` becomes
      sixteen times its size in values: about sixteen megabytes per hostile attempt, in the process that holds the credentials, outside the
      container's cgroup. A legitimate result line has no collection in it (an effect key of at most 512 characters, a ref of at most 256,
      counts of at most 19 digits) and is under 4 KiB, so `MAX_LINE_BYTES` is now **16 KiB**, which revises the 1 MiB of D035-077. A test
      pins the longest receipt the contract allows under half of it, and one pins the amplification. **After the result the runner read
      up to the whole line allowance** (`1056768` bytes taken in the test) **and a stray byte without a newline was accepted** if the host
      then stayed silent, because the timeout dropped what `read_until` had read; both are fixed by asking `fill_buf` whether any byte is
      there instead of reading a line: a flood after the result costs one buffer (`≤ result + 2 × 8 KiB` in the test), and a byte is a
      break whether or not the host closes. Two tests could not fail for what they named (they passed with the limit removed); they now
      count the bytes taken from the stream (a `Counting` wrapper), and I loosened the allowance to three times to see the line test fail
      (`57344 bytes were taken for a limit of 16384`) before restoring it.
    - *Documented, for the supervisor and the driver.* `converse` bounds each line, not the conversation: a host that beats at least every
      15 s may go on for ever, so the attempt's deadline, a lost lease and a cancellation end it by dropping the future (G035-054's
      supervisor). `on_heartbeat` must not block: the `Decoder` judges a beat by when it is read, so a runner that stalls for a second or two
      reads the host's beats in one burst and calls it a flood (D035-077's slack of one extra beat covers about a second); that is an
      availability limit and the next attempt resolves what was sent. The sustained rate the `Decoder` accepts is two beats a second, not
      one (D035-077; unchanged).
    - *G035-056 (new).* `peer_uid` is optional on the relay, and `CONTROL` is the first request that gives the peer something: whoever
      presents the token first receives the request line with the credentials. The token file is group-readable (D035-086), so the driver
      must set `with_peer_uid` to the slot's user, and the driver slice needs a test that a peer of another user is refused.
    - *G035-057 (new, not this slice).* A failure line's `column` and a receipt's strings are host-controlled text, and the host holds the
      credentials. They are bounded (128 characters for a column, typed fields for a receipt) but a column is free text, so a compromised
      host has a few dozen bytes per attempt through it to whatever the runner shows or audits. When the driver reports a failure it must
      treat `column` as untrusted (a conservative character set, or only a column the platform can match against the table).
  - *Evidence.* `cargo test -p sagat-infra-lake`: 122 passed (24 in `control.rs`, 40 in `relay.rs`, 35 in `protocol.rs`, 22 in
    `write_body.rs`, 1 unit); `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean (only the
    vendored `rskafka` warns). Written before the code and seen failing: the control tests (the module did not exist; two of them then failed
    for the reasons above, one of them my test's, an unused `_host` binding dropped the stream before the script ran), the relay tests (no
    `take_control`), the Decoder test, and after the review the four tests above. No mutation pass (OD-2026-11-06-2). Not run: the real
    Python host against this relay (T035-031).
  - *Not done.* Nothing here starts a container or reads a record; `T035-035` stays unticked.
- **G035-054 — Which code a kill reports is not in the contract.** `errors.md` has `LAKE_HOST_PROTOCOL` (no result line, malformed or extra
  fields) and `LAKE_SANDBOX_UNAVAILABLE` (the container could not be started), and `lake-runtime.md` lists the kill conditions (lease loss,
  cancellation, deadline, missing heartbeat, `memory.max`) without codes. Proposed for the supervisor slice: a silent or protocol-breaking host
  and an exit with no result are `LAKE_HOST_PROTOCOL` (done here for the conversation); a deadline is `DATABASE_TIMEOUT`; a cancellation is
  `DATABASE_CANCELLED`; a lease loss reports nothing (fenced); a `memory.max` kill needs a decision (`ICEBERG_LIMIT_EXCEEDED` says the write was
  too large, `LAKE_HOST_PROTOCOL` says the host broke). Whichever, the next attempt's effect-key lookup settles what was already sent
  (D035-030), so none of them is `OUTCOME_UNKNOWN`. *Exit condition:* the supervisor's table, with a test for each row, and `errors.md` if a
  code is added.
- **G035-055 — The sandbox-runner API for the lake runner (the open half of G035-022).** See the recommendation in D035-110 (d). The ADR
  is written and Proposed (ADR-0079, D035-116, 2026-11-07). *Exit condition:* the ADR accepted, the API, the engine additions, the per-node
  ceiling, and T035-015 run through it on Compose, and on Kubernetes under T035-029.
- **G035-056 — The relay does not require `peer_uid`, and `CONTROL` is the first request that gives the peer something.** Whoever
  presents the token first receives the request line with the credentials; the token file is group-readable (D035-086). *Exit condition:*
  the driver sets `RelayConfig::with_peer_uid` to the slot's user, with a test that a peer of another user is refused (D035-110, review).
- **G035-057 — A failure's `column` is text the host chooses, and the host holds the credentials.** Narrowed by D035-111 (a column that is
  not a plain name is not repeated); not closed, because letters and digits still carry a few dozen bytes an attempt. *Exit condition:* the
  driver reports a column only when the platform can match it (a mapped column, or a field of the staged records), or the owner accepts
  the channel.
- **G035-054 — Closed by D035-111.**
- **D035-111 — What the runner reports when an attempt ends, and which ending wins when two are ready.** Third slice of T035-035.
  **Decided by: delegate**.
  - *Why a table in its own module.* The driver waits on the conversation, the container, the deadline, the lease and a cancellation at
    once, and the contract lists the kill conditions without saying what each reports (G035-054). `crates/infra-lake/src/supervisor.rs`
    is that table as a pure function, `dispose(Ended, ContainerFacts) -> Disposition`, with no I/O and no clock: every row is a test, and
    `Ended` is exhaustive so a new way to end is a compile error until someone decides.
  - *The rows.* The host's receipt completes the step (and stands whatever happens to the container afterwards); the host's failure is
    reported with its code, its reason and its column; no result, a break of the protocol or silence is `LAKE_HOST_PROTOCOL` (retryable);
    the same with the memory limit having killed the container is `ICEBERG_LIMIT_EXCEEDED` (not retryable: the write does not fit, so
    repeating it repeats the kill; a break that came before the kill, such as a malformed line, stays `LAKE_HOST_PROTOCOL`, and so does a
    host that never connected: it died of how the deployment started it, not of what the write needs); a container that could not be
    started or was lost is `LAKE_SANDBOX_UNAVAILABLE` (retryable); **the deadline or a cancellation before the request was sent** is
    `DATABASE_TIMEOUT` / `DATABASE_CANCELLED` (`errors.md` says the 017 codes are reused); **after the request was sent** both are
    `OUTCOME_UNKNOWN`; a lost lease reports nothing. `contracts/errors.md` is not edited (a contract file selects the whole stack in CI,
    and its test pins codes and retry flags, not descriptions); the memory kill is a wider reading of "bytes … above the effective
    limit", noted here. The flag reported with a failure is the contract's flag for its code (`Failure::retryable`).
  - *Correction after the review: an unknown outcome after the request.* My first version said that **no** ending is `OUTCOME_UNKNOWN`
    because the next attempt's lookup settles it. That holds only for the endings the engine retries (`LAKE_HOST_PROTOCOL`,
    `LAKE_SANDBOX_UNAVAILABLE`); a deadline or a cancellation is not retried, so a commit that landed just before it would have ended
    as a final failure with a snapshot in the table and no receipt, and nothing would ever look the effect key up. Two precedents say
    otherwise: the 017 executor turns a timeout or cancel after dispatch into `OutcomeUnknown` (`executor.rs`), and G035-043 says to treat
    a killed attempt as an unknown outcome. So the facts now carry whether the request was sent (`AttemptFacts::request_sent`), and
    after it a deadline or a cancel is `OUTCOME_UNKNOWN`; a person retries it with `saga:override` (D035-069) and the lookup finds the
    commit (`foundByLookup`) instead of writing again. The memory kill stays `ICEBERG_LIMIT_EXCEEDED`: a kill while data files are built
    is the usual case, and a manual retry finds a commit made just before it, but this is a residual risk, not a proof.
  - *Differs from the 017 runner on purpose.* `apps/database-runner/src/attempt.rs` folds three causes into one cancellation token (the
    lease ran out, a heartbeat failed or timed out, the server asked for a cancel) and reports `Cancelled` for all of them, so a network
    blip during a heartbeat can fail a step for good if the lease is still valid. The lake driver keeps them apart: only a cancel the server
    asked for reports `DATABASE_CANCELLED`; a heartbeat that is refused or fails, or a lease that runs out, is `LeaseLost` and reports
    nothing. The container is killed either way; the step is leased again when the lease expires, and the next attempt's lookup of the
    effect key settles what was committed. The cost is the lease's remaining time before the retry; the gain is that an uncertain
    situation never changes the step's state.
  - *Which ending wins.* Two can be ready in the same poll. `Ended::strongest` decides, with a test for every pair: a lost lease, then a
    result the host gave, then a cancellation, then the deadline, then a broken conversation, then a host that never connected, then a
    sandbox that did not start. A result that arrives as the deadline passes is a completed step; a lost lease beats even a result.
  - *G035-057 narrowed.* A failure's `column` is kept only if it is letters and digits of any script, `_`, `.`, `-` and a space; anything
    with an encoding character (`+ / =`), markup, a separator or an invisible one is dropped and the code is reported without it. A
    control character never gets here: the decoder refuses a failure that has one.
  - *Independent review (correctness reviewer, given the files and the contract, not my conclusion).* `strongest` is right (exhaustive
    `rank`, the first of equals wins, every pair tested); the codes exist and their flags match `failure.rs`. Findings, each checked
    against the code:
    - *Fixed.* The unknown outcome after a deadline or a cancel (above). **A host could report any code**, including `DATABASE_CANCELLED`,
      `DATABASE_TIMEOUT` and `LAKE_SANDBOX_UNAVAILABLE`, because the decoder accepted every code of the table and the supervisor passed it
      through: a hostile or buggy host could make a run look cancelled or timed out. The decoder now accepts the 18 codes a host may
      report (`HOST_MAY_REPORT`, `OUTCOME_UNKNOWN` among them because the host reports it when its lookup is inconclusive, D035-030 step
      3), and anything else is `InvalidFailure`, which is `LAKE_HOST_PROTOCOL`. A list of that kind can drift, so a test scans the Python
      host's sources for the code literals it raises and fails if one is not on the list (it fails closed and visibly when a Hive loader
      or a new operation adds a code). A memory kill no longer blames the write for a host that **never connected** (a container that dies
      at start died of how the deployment started it). The `Disposition` does not carry a retry flag of its own; `Failure::retryable` is the
      contract's flag for the code, with a test that the reported flag is that flag.
    - *Documented, not changed.* A heartbeat that merely failed is `LeaseLost` too, so a finished commit may be discarded while the lease is
      still valid at the server; the step is leased again and the lookup finds the commit. The engine's `oom_killed` is Docker's
      `State.OOMKilled`, which an OOM event on any process of the cgroup sets: a host that lost a child to it and then went quiet for
      another reason reads as a memory kill (the host is the only process and its threads are not processes, so this is the rare case).
      `LakeContainer` has no inspect, so the attempt path has no source for `memory_killed` until G035-055. `SandboxUnavailable` now
      says it also covers a sandbox lost mid-attempt, which may leave a host running that holds credentials; the next attempt's
      conditional commit does not mind it (D035-030), and the driver slice must still try to kill it.
    - *Tests.* The memory guard lists now include `InvalidReceipt` and `InvalidFailure`; a host failure with a memory-killed container
      is tested; the retry flags are asserted; rows have their `request_sent` variants.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 148 passed (22 in `supervisor.rs`, 24 in `control.rs`, 40 in `relay.rs`, 39 in
    `protocol.rs`, 22 in `write_body.rs`, 1 unit); `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`
    clean. The decoder tests (a host cannot report a code of the runner's) were written first and seen failing (`DATABASE_CANCELLED` was
    accepted). For the supervisor I changed tests and module together before running, so those were not seen failing; they are
    equality assertions on the code of each row. No mutation pass (OD-2026-11-06-2).
  - *Not done.* Nothing calls `dispose` yet: the driver (`lake.rs`) gathers the facts and acts on the answer, and the container's
    `memory_killed` comes from the sandbox API (G035-055).
- **D035-112 — The attempt directory and the record files are made by `AttemptDir`, with the modes of D035-086 and the limit counted while
  the bytes stream.** Fourth slice of T035-035. **Decided by: delegate**.
  - *What it is.* `crates/infra-lake/src/staging.rs`. `AttemptDir::create(work_root, attempt_id, token, limits)` makes
    `<work>/<attempt id>/` (0770, which must not exist), `in/` (0770) and `relay.token` (0640, the token and a newline, which is what the
    host's `read_token` reads). The modes are set explicitly after each creation, because a umask only takes bits away. The attempt id is
    the intersection of the host's pattern and the lake profile's (1 to 60 lower-case hexadecimal digits and hyphens, starting with a
    digit), so it cannot name another path. `stage_inline(records)` writes `in/inline.ndjson`, one compact JSON value a line;
    `stage_blob(format, stream)` writes `in/blob-<n>.<ndjson|parquet>` byte for byte; `inputs()` is the list the request builder takes
    (`WriteAppend`, D035-109). `remove()` deletes the directory with the token in it, and `Drop` does it if the driver forgot.
  - *The limit.* Bytes of every file together, and records (the inline records and the lines of NDJSON blobs, counted with or without a last
    newline; a blank line counts). Going over is `StagingError::LimitExceeded`, which the driver reports as `ICEBERG_LIMIT_EXCEEDED` before
    any container starts; the partial file is removed. A blob is read in 64 KiB chunks and stopped at the limit, so a stream that never
    ends costs one chunk past it (tested with `tokio::io::repeat`). A limit of zero is refused (zero is not unlimited). At most 32 files
    (`MAX_RECORD_BLOBS`), inline included.
  - *No new dependency.* A `Cargo.toml` change selects every package in CI, so the file work is `std::fs` and the streams go through a
    bounded channel to a blocking task: no `tokio` feature is added.
  - *Errors name no path and no token*: `StagingError` is a closed kind (`AttemptId`, `Exists`, `Io`, `LimitExceeded`, `TooManyFiles`,
    `ZeroLimit`), with a test that no error shows either.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 26 new tests in `staging.rs` (modes, one directory per attempt, thirteen refused ids,
    exact byte and record limits for inline and blob sources, an endless blob, a failing stream, a refused blob leaves no file, 32 files,
    the inputs feed `WriteAppend`, removal and drop, a removal that fails and is retried, a call dropped midway); `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D
    warnings` clean. The module and the tests were written together before the first run, so they were not seen failing for the absence of
    the module; the first run failed to compile on two type slips of mine in the tests. No mutation pass (OD-2026-11-06-2).
  - *Independent review (security reviewer, given the files and the contract, not my conclusion).* No tenant string reaches a path (the id
    is `[0-9a-f-]`, names come from a counter and an enum); `create` returns `Exists` before the value exists, so `Drop` never removes a
    directory the runner did not make; a planted symlink cannot make `remove_dir_all` leave the directory (the pinned toolchain unlinks
    symlinks); the token is written only to `relay.token`; memory is one 64 KiB buffer and four queued chunks; the counting is right at
    chunk boundaries. Findings, each checked by reading the code:
    - *Fixed.* **`remove` consumed the handle and `Drop` swallowed its error**, so a removal that failed left the tree, the token possibly
      among it, with nobody to say so. `remove` now takes `&mut self` and keeps the attempt on failure, so the driver can retry, report
      and count the leftover, and `Drop` tries once more (test: a directory the runner cannot enter, skipped when run as root). The
      directories are now made with their final mode (`DirBuilder`), so there is no instant at which they are wider than 0770; the file's
      mode is set on the open file (`fchmod`), not on a path. The additions that bound the counts are saturating, because a limit can be
      any non-zero `u64` (no test can feed 2^64 bytes, so this one is by reading). A call dropped midway (the caller's deadline on a
      stalled source) removes the half-made blob and leaves its name usable (`Partial`); with the guard's removal disabled three tests
      fail, which I did to see that they are real. The id's doc said "starts with a digit"; it starts with a hexadecimal digit.
    - *Not fixed, because it changes an owner decision: O035-006.* The container has the directories read-write in the runner's group
      (D035-086, OD-2026-11-05-2), so a compromised host can create in them a directory the runner cannot enter (owned by the slot's user,
      mode 0700, or 000) and `remove_dir_all` stops at it. Repeated, that fills the size-limited work volume and the effect crosses
      tenants (G035-060). The host writes nothing in the directory (I searched `runners/lake`: it reads `relay.token` and `in/*` and
      connects to `relay.sock`, which needs search on the directory and write on the socket, not write on the directory), so 0750 on
      `<attempt>/` and `in/` would take the ability away and nothing else. That tightens a mode the owner chose, and `lake-runtime.md` and
      the isolation tests state it, so it waits for an answer; 0770 stays until then.
    - *For the driver.* Wrap `stage_*` in a timeout (a stalled source holds a blocking thread); the attempt directory takes the group of the
      work root if that is setgid, so the volume's manifest says it must not be; zeroising is the driver's (above). The wording of the
      slice list in D035-110 (b) said "removal and zeroising at the end": zeroising is not here.
  - *Not done.* The zeroising of the token and of the encoded request line is the driver's, in `apps/database-runner` where `zeroize` is
    already a dependency (adding it here would change a manifest). Nothing calls staging yet.
- **O035-006** — Should the attempt directories be 0750 instead of 0770? (D035-086 chose 0770 in the runner's group.) *Recommendation: yes.*
  The container reads `relay.token` and `in/*` and connects to `relay.sock`; it never writes in `<attempt>/` or `in/`. With 0770 a
  compromised host can create a directory the runner cannot enter, which cannot be removed (G035-060). With 0750 it cannot create
  anything. The token (0640), the record files (0640) and the socket (0660) keep their modes. It changes the amended text of
  `lake-runtime.md`, the isolation tests of T035-015 and the `container_config` test that states the group, so it needs the owner's word.
  *Until answered:* 0770, as decided.
- **G035-060 — A compromised host can leave a tree on the work volume that the runner cannot remove.** See O035-006 and the review in
  D035-112. `AttemptDir::remove` keeps the handle on failure so the driver can retry and report; what the runner cannot enter, it cannot
  delete. *Exit condition:* the owner answers O035-006 (0750 closes it), or a cleanup through the engine (a container as the slot's user
  that deletes the directory) is built, and the work volume's free space is a metric.
- **G035-058 — A Parquet blob counts toward the bytes and not toward the records.** The runner does not parse a footer, so
  `max_records` is not enforced on Parquet input by the runner; the host's decoded-bytes budget (8 times the byte ceiling, D035-107) is the
  bound. *Exit condition:* read the row count from the footer in the runner (a Thrift read of a bounded tail), or accept the budget as the
  bound and say so in `lake-runtime.md` with the next contract change.
- **G035-059 — The contract stages a 017 database result as `result.json`, and the host reads only NDJSON and Parquet.** `lake-runtime.md`
  step 2 says "a 017 database result as `result.json`"; `request.py` accepts the formats `ndjson` and `parquet` only. A step has exactly one
  record source (`RecordSource`), so the driver turns the result's rows into JSON values and stages them with `stage_inline`; the file the
  contract names is not one the host reads. *Exit condition:* the driver does so, with a test, and `lake-runtime.md` says it with the next
  contract change.
- **D035-113 — `run_attempt` joins the relay, the sandbox, the conversation and the supervisor behind a `LakeSandbox` port, and is tested
  with a scripted host on the real socket.** Fifth slice of T035-035. **Decided by: delegate**.
  - *What it is.* `crates/infra-lake/src/attempt.rs`. The port has three calls (`start(StartRequest)`, `exited(id)`, `kill(id) ->
    ContainerExit`), and a start request carries an attempt id and a life time and **no credential, image, mount, user or command**
    (D035-110, G035-055). `run_attempt(sandbox, attempt_dir, relay, plan, signals, on_heartbeat) -> AttemptOutcome` runs the attempt: it
    starts nothing for an attempt that is over (cancelled, lease lost, deadline past); binds `relay.sock` with mode 0660; takes the
    hand-over of `CONTROL`; asks for the container; waits for the host up to `host_start`, or until the container is gone, the step is
    cancelled, the lease is lost or the deadline passes; runs the conversation, with the same four able to end it; joins the ending with
    what is ready in the same instant (`Ended::strongest`); kills the container (once, after every start that was attempted, also after a
    start that failed); stops the relay; and returns what `dispose` says plus `request_sent`, `memory_killed` and the relay's denials.
  - *The request goes out only when the host has connected.* `request_sent` is false for every ending before that, so the credentials
    never left the runner for a host that did not start, a deadline that passed, a cancel or a lost lease; those stay a plain timeout or
    cancellation, and only the endings after the request are `OUTCOME_UNKNOWN` (D035-111).
  - *A race this found.* The first version computed the container's life time from the deadline after a check that began the attempt;
    under load the deadline passed in between, and the sandbox would have been asked for a container with no time left. Found by
    running 24 copies of the test binary at once (8 failed, on the double's own assertion that the life time is positive). The time left is
    now read once, and a container with none is not started: `Ended::DeadlinePassed`, no start, no kill. A past deadline has a
    deterministic test; the race itself has none, only the reading. After the change 48 copies at once pass.
  - *The tests.* 20, with a scripted sandbox whose host does what the Python host does at the wire (reads `relay.token`, connects to the
    real `relay.sock`, presents the token, asks for `CONTROL`, reads the request, plays one ending): a receipt, a failure, beats counted,
    a silent host, a host that dies, a memory kill, a host that never connects (waits its time, no request), a container gone at once
    (not waited for), a start refused, a deadline and a cancellation before and after the request, a lease lost, an attempt over before
    it starts, a lost lease beating a cancellation, the relay's denials counted and returned, the socket's mode, the relay stopped with the
    attempt, no secret in the outcome. Times are real and short (a paused clock does not advance while real socket I/O is pending); the
    only margin that matters is the deadline after the request, 2 s.
  - *Independent review (security and correctness reviewers, given the files and the contract, not my conclusion).* No path was found by
    which the request reaches a recipient other than the authenticated control stream of this attempt's host, a late `CONTROL` after the
    receiver is gone, or the relay's `serving.await` hanging on a hostile host; the token reaches no `Debug` or error; `request_sent` is
    conservative. Findings, each checked by reading the code, and what I did:
    - *Fixed. The request could be written after the attempt was over.* The stream and a signal (a cancellation, a lost lease, the
      deadline) can both be ready when the host connects, `select!` polls its arms in no fixed order, and the first poll of `converse`
      writes the credentials. The conversation stage (`conversation`, which takes any stream) now checks the signals first and its
      `select!` is `biased` with the signals first; `request_sent` is no longer "the branch was reached" but set by the conversation
      itself when it begins to write (an `Observer`, `control.rs`), so a cancel that wins is `DATABASE_CANCELLED` and not
      `OUTCOME_UNKNOWN`. **My first test of this did not test it**: with the protections disabled it still passed, because on this
      runtime the host's signal wakes the attempt before the relay hands over the stream, so the signal wins the earlier wait. The
      race (a live stream and a signal ready at the same poll) is entered directly by two unit tests in `attempt.rs`, run 60 rounds each
      way; with the protections removed both fail, which I did to see it.
    - *Fixed. A receipt that was already decoded could be dropped.* `converse` returns only after the host's grace to close (up to 2
      s), and a cancellation or the deadline in that window dropped the future and the receipt with it, although `supervisor.rs` says a
      receipt stands. The conversation now tells its caller the moment a result is decoded (`Observer::result`), and the join keeps it
      against everything but a lost lease (unit tests; a byte after the result is still a break and the result does not replace it).
    - *Fixed. `start` and `kill` were unbounded.* `start` is raced against the signals and the deadline (a hung sandbox runner no longer
      holds the worker); `kill` has a limit (`Plan::kill_limit`) and the outcome says whether the container is gone
      (`container_ended`, from `ContainerExit::ended`): a kill that cannot be confirmed is said, not hidden, because a host with the
      credentials may still be up until its life time runs out. The relay now stops **before** the container is ended (its tunnels are
      the container's only route out), and a lease lost while it is being ended still reports nothing.
    - *Fixed. `kill` was called when nothing had been started* (the path with no time left), and no signal was checked between the first
      check and the start. A start is now preceded by the same check, and `kill` follows only a start that was attempted.
    - *Fixed. Dropping the future left the relay serving.* The relay's task is held by a guard that stops it and aborts the task. The
      container is not killed on a drop (that needs an `await`); the backstop is the life time in `StartRequest::deadline`, which the
      sandbox enforces even if the runner is gone, and this is now in the module doc.
    - *Fixed. `peer_uid` was not required* (G035-056, the part that belongs to `run_attempt`): a relay that does not check its peer is
      refused (`Relay::requires_peer`, `SandboxUnavailable`, nothing started), and a process of another user that has the token is
      refused by the relay and counted. The slot's user is still the caller's to pass to the relay.
    - *Fixed. `Plan` and `AttemptOutcome` derived `Debug`* over the request bytes and over `Ended`, which holds the raw column of a
      failure that `dispose` filters. Both are manual now: the plan shows the size of the request, the outcome omits `ended`.
    - *Not changed, recorded.* The `chmod` of the socket is by path after `bind`; a member of the runner's group could race it with a
      symlink before any container exists (nothing else mounts the work root), a low residual. A failure of `bind` or `take_control`
      is reported as the retryable `LAKE_SANDBOX_UNAVAILABLE`, which hides a runner-local fault; a panic in the relay's task is swallowed
      by `let _ =`; a sandbox whose `exited` resolves spuriously fails every attempt as `LAKE_HOST_PROTOCOL` (visibly). The container's
      life time is read before `start` returns, so it can outlive the plan's deadline by the start latency.
    - *Tests.* Two of mine depended on the machine: the deadline after the request (a 2 s window) now pauses the clock once the host has
      the request, and the three tests of a hung start run with a paused clock from the start (a hung start leaves no real I/O to wait
      for); with 3 of 32 copies at once failing before that, 0 of 32 and 0 of 48 (twice) fail after. The test that "the outcome shows no
      secret" could not fail (`AttemptOutcome` held none) and was replaced by two that can.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 217 passed (32 in `attempt.rs`, 8 unit tests in `attempt.rs`, 28 in `control.rs`); 32
    copies at once and 48 copies at once, twice, pass; `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D
    warnings` clean. The first 19 tests were written with the module and passed at once; the observer tests of `control.rs` were
    written first and failed to compile; the rest of the review's tests were written before the fixes and failed to compile for the new
    API, and their runtime failures were not all seen before the fix, except as the mutation above. No mutation pass (OD-2026-11-06-2).
  - *Not done.* The production `LakeSandbox` (a client of the sandbox runner: G035-055 and its ADR), the slot's user for the relay's
    `peer_uid` (the caller), zeroising the request buffer, the lease heartbeat that feeds `on_heartbeat`, `ResolveDatabase`, `ReadBlob`,
    and the report to the control plane (`apps/database-runner/src/lake.rs`).
- **D035-114 — What the sandbox may reach is derived from the pinned connection by a closed function, `endpoint_policy`.** Sixth slice of
  T035-035. **Decided by: delegate**.
  - *Why.* D035-049 says the relay allows only the endpoints declared on the pinned connection or derived from its storage type and
    region. The relay had the pieces (`Endpoint`, `EndpointPolicy`, `with_s3_regional`) and nothing built an allowance from the documents
    the platform stores. This is where the text of a URL becomes a list of hosts a sandbox that holds credentials may connect to.
  - *What it is.* `crates/infra-lake/src/policy.rs`, `endpoint_policy(catalog, storage, locations)` and `Connection::endpoint_policy()`.
    A REST catalog declares its address (`https`, or `http` for a loopback host only, the way `connection.py` reads it) and, for OAuth2,
    its token endpoint (`https` only). S3 without an `endpoint` gets the official regional hosts of its region and of each bucket the
    warehouse locations name (path-style and dual-stack, and virtual-hosted for a bucket without a dot). S3 with an `endpoint` (an
    S3-compatible store) declares that address, `https` only, **and AWS's regional hosts are not added beside it**: they reach any bucket
    of anyone's (G035-021), and a store that is not AWS has no use for them.
  - *Closed.* A URL that could name another host than it seems to is refused: user information, a query, a fragment, a backslash,
    whitespace, a control character, a non-ASCII byte, a port that is not 1 to 65535, an address with a colon and no brackets, an
    unclosed bracket, more than 2 KiB. Every connection the host cannot open yet is refused with a name before a container is started
    (a Glue, Hive or JDBC catalog, GCS or ADLS, an assumed role, remote signing, SigV4: G035-048), because an allowance for a connection
    that cannot run is an allowance for nothing. The region and the buckets are checked whether or not S3's hosts are used. Errors are a
    closed kind and repeat nothing of the document (a URL may be a secret pasted where an address was meant).
  - *Independent review (security reviewer, given the files and the contract, not my conclusion).* No security finding: every URL form it
    traced (percent-encoding, zone ids, bare IPv6, `host:443:80`, control characters, non-ASCII, a leading or doubled dot) is refused or
    read as the same host on both sides; the loopback rule is the host's own and the relay refuses loopback addresses outside
    `development` anyway; a crafted region or bucket cannot leave `amazonaws.com[.cn]` (a region is one label between fixed text); the
    errors repeat nothing of the document. What it found, checked:
    - *Fixed (availability).* **A custom S3 endpoint with `pathStyle: false` could not run.** I measured it with PyArrow against a local
      socket instead of relying on a recollection: with virtual addressing the client sends `Host: mybucket.localhost:PORT`, with path
      style `Host: localhost:PORT`. The host sets virtual addressing when `pathStyle` is false, so the policy, which declared only the
      endpoint's own host, would have blocked every request. A store addressed by bucket is now allowed `<bucket>.<endpoint host>` for each
      bucket of the warehouse without a dot, and nothing else (not another bucket, not another port, not a store at an address).
    - *Fixed (defence in depth).* A host whose last label is a number (`2130706433`, `127.1`, `0x7f.1`) is refused, as the URL standard
      reads such a host as an IPv4 address in a form this parser does not. A canonical dotted quad is an address and is fine. (The reviewer
      said these pass as plain names and that this widens nothing, because the policy holds the literal string and the relay validates
      every address it resolves; I copied them into my test as refused before checking, the test failed, and I made the code match the
      standard instead of relaxing the test.)
    - *Test gap, closed.* The reviewer listed the inputs that nothing pinned; they are in `addresses_that_a_client_might_read_differently_
      are_refused_here`, and **one corpus of catalog addresses (`tests/contract/iceberg-writes/fixtures/urls/catalog-uris.json`) is read by
      the Rust test and by `runners/lake/tests/test_wire_urls.py`**, so the two parsers cannot drift apart without a test failing on one side
      (9 addresses both accept, 21 both refuse; the host agreed with all 30 on the first run).
    - *Recorded, not changed (all fail closed).* G035-061: a development namespace cannot use a plain-`http` S3 or token endpoint, which the
      domain accepts (`infra-postgres` has a fixture with `http://rustfs.local:9000`): the host and this policy both refuse it
      (D035-105), so the Compose fixtures of T035-004 need a development-only decision. G035-062: regions outside `amazonaws.com[.cn]`
      (`amazonaws.eu`), buckets the domain accepts and S3 no longer creates (uppercase, underscores, under three characters), and whether
      PyArrow with `us-east-1` asks for `s3.amazonaws.com` (the legacy global host) instead of the regional one, which I did not measure.
  - *Evidence.* `cargo test -p sagat-infra-lake`: 243 passed (26 in `policy.rs`, which also pins that the connection of the wire fixture
    allows exactly its four hosts and nothing else); `runners/lake`: 1116 passed, 1 skipped, `ruff` and strict `mypy` clean; `cargo fmt
    --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean. The first 20 tests were written with the module
    and passed at once; the virtual-addressing test was written before the fix and failed for the right reason. Two mutations by hand:
    letting AWS's hosts in beside a custom endpoint fails its test; accepting user information in the authority does **not** fail any,
    because `Endpoint::new` already refuses a host with an `@` (an equivalent mutant: the explicit check is defence in depth). No mutation
    pass (OD-2026-11-06-2).
  - *Not done.* The operator's list of networks the relay may reach (`RelayConfig::new(allow)`) and the mappings for non-HTTP routes
    (Hive, a JDBC catalog) come with the loaders (G035-048); nothing calls `endpoint_policy` yet: the driver builds the `Relay` with it.
- **G035-061 — A development namespace cannot use a plain-`http` S3 or token endpoint.** The domain accepts one
  (`crates/domain/src/database/iceberg.rs`; the `infra-postgres` round-trip fixture uses `http://rustfs.local:9000`); the host
  (`connection.py`, D035-105) and `endpoint_policy` refuse it, on purpose. The Compose fixtures of T035-004 (RustFS, a catalog) need a
  decision: a development-only allowance in the host and in the policy, tied to the relay's `development(true)`, or `https` fixtures.
  *Exit condition:* that decision, with a test on both sides.
- **G035-062 — The S3 allowance is narrower than S3.** `endpoint_policy` fails closed for regions outside `amazonaws.com[.cn]` (the
  `amazonaws.eu` partition), for buckets the domain accepts and S3 no longer creates (uppercase, underscores, fewer than three
  characters), and I did not measure whether PyArrow with `us-east-1` asks for the legacy global host `s3.amazonaws.com` instead of
  `s3.us-east-1.amazonaws.com` (a test asserts the legacy host is not allowed). *Exit condition:* measure it against the pinned
  PyArrow, and either allow the host or write down why not; decide the other partitions with the loaders.
- **D035-115 — The runner reads the Iceberg connection of `ResolveDatabase` into the documents the host reads, and zeroises the
  secret bytes as it parses them.** Seventh slice of T035-035, and the runner's half of T035-044. **Decided by: delegate**.
  - *Why.* The server answers `ResolveDatabase` with the typed message `IcebergConnection` (`worker.proto`, already in the contract):
    the settings of one revision and `credentials`, a map with one entry per secret (`catalog`, `storage`), each the JSON document of the
    data model. The host reads the documents as the platform *stores* them (`to_json`). Something has to turn the one into the other,
    and it is the place where the secret bytes first sit in the runner.
  - *What it is.* `apps/database-runner/src/lake/connection.rs`, `convert(&mut pb::IcebergConnection) -> Converted`. A REST catalog over
    S3 becomes the stored documents (`{"type":"rest","rest":{...}}`, `{"type":"s3","s3":{...},"credentials":"static"|"vended"}`); an
    empty string is "not set" (protobuf has no other); the allowed modes and the limits are carried (`0` is not set). Glue, Hive, JDBC,
    GCS and ADLS are `Unsupported`, with a name, before a container is started (G035-048); an incomplete connection is `Shape`. **A test
    holds the result to the wire fixture** that the Rust request builder and the Python host both read, so the stored JSON of the domain,
    the protobuf and the request line agree on this connection.
  - *Secrets.* Each entry of `credentials` must be a JSON object of text to text, at most 16 KiB, with no name twice (refused at parse
    time, not by keeping the last one); the bytes are zeroised in place as soon as they are parsed, **also when the connection is refused
    for another reason and also when a secret is refused** (the secrets are read first for that), and the parsed values are `Zeroizing`
    strings held in a `Secret` newtype whose `Debug` shows a count and nothing else (the `Debug` of `Zeroizing` prints the value). At most
    two entries are read; more are refused and scrubbed before any is parsed. A key other than `catalog` and `storage` is refused, and
    so is a `storage` secret for storage whose credentials the catalog vends: either would sit in the runner (and the sandbox) for nothing.
    `Converted` shows no secret and no address; an error repeats nothing. `capability::scrub` also zeroises the `credentials` maps of the
    three lake connections of a `ResolvedDatabase` that the PostgreSQL path receives and never reads.
  - *What the converter refuses before a container is started.* Everything the host cannot open (`connection.py`): SigV4, remote signing,
    an assumed role (`role_arn`, `external_id`), a catalog and a storage that disagree about vended credentials. It does not decide whether
    a *mode* is allowed; it carries the allowed modes, the limits and the **egress ceiling**. A ceiling that is not set (or a number the
    runner does not know) is `Shape`, not the domain's default `internal`: the server always sends one, so its absence is an anomaly and
    must not widen what a revision may receive. Warehouse locations are bounded (16 of at most 2048 bytes) before they are cloned.
  - *Independent review (security-reviewer, on the first version of this PR).* Findings taken: the egress ceiling was dropped (now carried,
    absent refused); a `serde_json::Value` copy of every secret value was left unscrubbed (values are now parsed straight into `Zeroizing`
    strings); duplicate names were silently collapsed; the number of entries and the location bounds were not checked before cloning;
    vended-credentials disagreement and a storage secret for vended storage were left to fail in the host; the lake maps of a
    PostgreSQL response were not scrubbed; `Secret` was an alias of the map and its `Debug` printed the values.
  - *What the zeroisation does not reach (stated, not hidden).* `serde_json` copies a string that contains an escape (`"`, `\`, a control
    character) into a scratch buffer of its own, and the message of a type error repeats the value it met; both are dropped unscrubbed. A
    secret with an escape, and a document of the wrong shape, can therefore leave a short-lived copy in freed heap memory that this module
    cannot zero. A hand-written parser or a scrubbing allocator could; neither is in this slice (G035-063 covers the same family in
    `infra-lake`).
  - *Evidence.* `cargo test -p sagat-database-runner`: 25 passed (21 in `lake::connection`, 1 new in `capability`); `cargo fmt --check`,
    `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three repository checkers clean. The module and the tests
    were written together and passed at once, so they were **not seen failing**; instead 14 single mutations were run by hand against
    the protections (scrub on too many entries, duplicate names, vended storage secret, vended mismatch, unset ceiling, location count,
    location bytes, secret bound, remote signing, role ARN, unknown key, byte zeroisation, `Debug` redaction, lake-map scrub), and each
    is killed by a test. One survived the first time (the unknown-key test added a third entry, so the entry bound refused it before the
    name was looked at); the test now has two entries and kills it. `Guard::drop` calling `scrub` is not observable after the drop and is
    covered by reading, not by a test; `scrub` itself is tested directly. No mutation pass (OD-2026-11-06-2).
  - *Not done, and why this slice has no new dependency.* The server does not fill `iceberg` yet (T035-044: nothing in `service.rs` or
    `worker/database.rs` mentions Iceberg). `Converted` is not yet turned into `infra_lake::Connection`: that needs
    `sagat-infra-lake` as a dependency of `sagat-database-runner`, and **a new dependency edge changes `Cargo.lock`, which the CI
    selector treats as an input of every package (every unit test and every integration target)**. The same goes for `zeroize` in
    `infra-lake` (G035-063). Both are one deliberate change, taken together with the driver, rather than one expensive CI run each.
- **G035-063 — `infra-lake` holds the secrets as plain strings.** `Connection` keeps them in `BTreeMap<String, String>`, the body in
  `serde_json::Value` strings, and `encode_request` returns a `Vec<u8>` of the whole line; none is zeroised when it is dropped. The runner
  can zeroise what it owns (D035-115) but not what `infra-lake` copies. *Exit condition:* `zeroize` in `infra-lake`'s manifest (a
  `Cargo.lock` change, so the whole CI: do it once, with the `database-runner` → `infra-lake` edge), secrets as `Zeroizing`, the body
  scrubbed on drop, and `encode_request` returning a `Zeroizing<Vec<u8>>`, each with a test.
  *Closed by D035-122* (the scrub on drop and the type of the secrets are not observable by a test without `unsafe`, which the workspace forbids;
  the escape-scratch residual stays).
- **D035-116 — The lake runner asks the sandbox runner for an attempt, never for a container (ADR-0079, Proposed).** Closes the API half of
  G035-022 / G035-055 as a decision; **Decided by: delegate** (the owner's delegation for feature 035), to be accepted by a person because it
  adds a server to the process that holds the engine socket. *What it is:* a node-local Unix-socket API of three calls on a **lake-mode**
  sandbox runner (`POST /v1/attempts/{id}`, `GET .../wait`, `DELETE`), the request an attempt id, a **slot** in a range, a closed budget
  **class** and a deadline, nothing else; the lake runner links neither `bollard` nor `infra-sandbox`. *What the reviews changed* (a security
  and an architecture review of the first draft, each claim checked against the code): the relay is built, and needs the container's user,
  **before** `start`, so the slot is the lake runner's choice in a range the sandbox validates (a sandbox-allocated uid could not satisfy
  `with_peer_uid`); all lifecycle state is rebuilt from engine labels (the deadline is a label, swept every 30 s, because the existing
  sweeper runs every 300 s by age since creation); no tombstones; the deadline is refused, not clamped; one budget became two classes (the
  plan has 2 GiB and 6 GiB); lake mode is exclusive (ADR-0071 and the plan say a lake sandbox runner serves only lake containers);
  the `lstat` check is advisory and the engine's sub-path confinement is a tested precondition; drain, wire-type ownership and version
  skew are specified. *Not built:* everything; the ADR lists the checks it needs and the `LakeEngine` additions (list, inspect, wait, kill
  by label reading the out-of-memory flag first). Kubernetes (T035-029) stays open and rules 2 and 3 are conditional on it.
- **D035-117 — The engine lists, finds, waits for and ends a lake container by its labels (ADR-0079, rule 7).** Eighth slice of T035-035 and
  the engine half of what G035-055 depends on. **Decided by: delegate**.
  - *What.* `container_config` writes `sagat.lake.slot`, `.class` and `.deadline` beside the owner and attempt labels (`LakeSpec` gains
    `class` and `deadline_epoch_secs`); `LakeEngine` gains `launch`, `list`, `find`, `wait`, `end`, `sweep_expired` and `connect_unix`,
    with `LakeInfo`, `Wait`, `EndedContainer` and `Sweep`, and `EngineError::{Exists, Invalid}`. Nothing calls it yet: the API is the next
    slice. `crates/infra-sandbox/src/lake.rs`, `lake/engine.rs`; tests in `tests/integration/iceberg-writes/lifecycle.rs`.
  - *Semantics, stated.* `end` kills, **reads the out-of-memory flag before the forced removal**, and `confirmed` is true only when the
    removal succeeded, the engine said there was none (404), or the listing showed none; an engine that cannot be asked, that refuses a
    removal, or that does not answer in time is `confirmed: false`, so the lake runner's "the credentials may still be up" report is never
    dropped by an answer the engine did not give. An engine that cannot **inspect** still removes (the credentials come first) and does
    not invent a memory kill: the supervisor then sees a host that broke (retryable) where a limit was exceeded (not), which is the price
    of a flaky engine. `end` and the sweep are tasks of their own: a caller that gives up (the lake runner bounds its kill) does not stop
    a removal halfway. A state the engine did not name counts as not ended. The sweep ends a running container at its label deadline
    (`now >= deadline`) and keeps an exited one for `retain` after it (so its facts can be read), up to four at a time, and its result
    separates removed, failed, unlabelled and "could not list".
  - *Found while designing.* A container named after its attempt makes a second launch of the attempt refuse itself at the engine
    (`Exists`), and also when two arrive at once, but then **the removal guard must not be armed before the create**: a guard armed first
    removes the container of the first launch when the second is refused. It is armed after, for names made of an attempt; the existing
    fresh-id names keep it armed first. A test fails if it is armed first (`a_second_launch…first_is_untouched`).
  - *Tightened by the review.* `attempt_id_ok` is the ADR's rule 10 (1 to 60 characters, a hexadecimal digit first; it was 64 and any
    first character); `owner_ok` is `[A-Za-z0-9._-]{1,128}`; a deadline label is 1 to the year 2100 (`u64::MAX` is refused, a label that
    is not one reads as none); `launch` refuses a body without the profile's labels (`Invalid`); the owner and attempt are validated
    before they reach a label filter, and an invalid name finds, lists, ends and sweeps nothing without asking the engine.
  - *Independent review (correctness-reviewer and security-reviewer, given the files and the ADR, not my conclusion).* No Blocker from
    either. **Major (correctness), checked and confirmed:** `a_start_that_fails_leaves_nothing_behind` could not fail: a missing image
    fails at `create`, so the cleanup it is named for was never reached; it now starts a real container whose command is missing with the
    engine's init off, so that the **start** fails (with init, the container starts and exits 127). **Taken:** a failed start whose
    removal the engine refused let go of its guard (now retried in the background; also in `run` and `stop`); `start_container` had no
    timeout; `end` was cancellable halfway; a missing, malformed or never-reached deadline label was invisible and `u64::MAX` accepted;
    `sweep_expired` hid a failure behind `0`; the memory flag could be overwritten by a later answer; a `hold` of `Duration::MAX`
    panicked; the poll interval 250 ms to 500 ms. **Tests:** the flaky wait (`sleep 3` against a 500 ms wait) became two cases, a vacuous
    user-id assertion was dropped, and the fixture now ends what a failed case leaves. **Not taken:** one poller for all waiters (their
    number is bounded by the ceiling, ADR-0079 rule 4; G035-064); a lifetime parameter on `sweep_expired` (the age sweep `sweep` runs
    beside it and takes the unlabelled, see the doc); a paused container on a real engine (`LakeEngine` cannot pause one; a paused
    state counts as not ended, which is unit-tested).
  - *Obligations on the API slice, from the reviews.* The handler MUST refuse a deadline above `now + ceiling` using its own clock and
    pass `now_epoch_secs` from its own clock, never from a request; the label deadline is the attempt's deadline **rounded up** plus the
    time the lake runner has to end the container, never earlier (a container ended by the label under a host still writing is reported
    as a host that went silent, not as `OUTCOME_UNKNOWN`); `start` and `end` of one attempt are serialised; `LakeInfo.name` is not echoed;
    `class` is looked up in a closed set the node configures; both `sweep_expired` and `sweep` (by age, with the longest lifetime) run.
  - *Evidence.* `cargo test -p sagat-infra-sandbox`: 28 unit tests (6 new, in `label_tests`; 5 more need an engine and are ignored there) and 21 profile tests
    (1 new, 4 changed); the integration cases against the project's Docker 29.5.2 (arm64), `isolation::` and `lifecycle::`: 34 passed (25
    new: 13 on the real engine, one of them against a socket nobody listens on, and 12 on a scripted one that speaks the engine's HTTP API
    over a Unix socket and ignores the label filters, so that an empty answer proves the engine was not asked). `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three
    repository checkers clean. **Not seen failing first:** the profile-label and validation tests were (red before the labels and checks
    existed); the lifecycle cases were written before the API and failed to compile against it, and the fixes of the review were written
    with their tests together. Instead 21 single mutations were run by hand, 20 killed by a test: the one that survived (`end` skipping
    its own attempt check) is equivalent, because `find`, which it calls, validates the same name and the answer is the same. No mutation
    pass (OD-2026-11-06-2).
  - *Not done.* The sandbox-runner API itself; the production `LakeSandbox` client; the lake-mode exclusivity; the work-volume mount read
    only; the Compose service. The cases leave a few seconds of real time (the grace of the engine's kill, 5 s, twice).
- **G035-064 — The engine calls poll.** `wait` is one scan of the engine's containers per waiter every 500 ms, and `end` and the sweep end
  containers one engine call at a time (four at once in the sweep). That is nothing at the plan's 2 to 8 containers per node and a cost at
  hundreds. *Exit condition:* measure it with the API under its ceiling (the canaries of T035-031), and put one poller per owner behind
  the waiters if the scans show.
- **D035-118 — The wire of the lake sandbox API has one home and one corpus (ADR-0079, rules 3, 5 and 14).** Ninth slice of T035-035, and the
  dependency-free half of the API: the types, the strict readers and the corpus, so that the whole-CI change that follows (the sandbox
  runner's server, the lake runner's client, the `database-runner` → `infra-lake` edge, `zeroize`) is only plumbing. **Decided by: delegate**.
  - *What.* `crates/infra-lake/src/sandbox_api.rs`: `StartBody` (slot, class, deadline; nothing else), `Class` (a closed set of two),
    `Reason` (five), `StartAnswer`/`WaitState`/`KillOutcome`/`Plain`, the encoders the server writes with (`start_body`, `wait_body`,
    `kill_body`, `plain_body`: a status and a body each), the readers the client reads with (`read_start`, `read_wait`, `read_kill`),
    `uid_for_slot` (the relay needs the container's user before `start`, rule 5), `attempt_id_ok` (the staging rule, shared) and the paths.
    One corpus, `tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json` (25 requests, 22 start answers, 13 wait answers, 16 kill
    answers), read by `crates/infra-lake/tests/sandbox_api.rs` now and by the sandbox runner's server later.
  - *The rule the readers keep.* An answer that is not recognised **exactly** is never read as started or as ended: a status that does
    not belong to the answer, a member added or missing, a name written twice, a variant nobody knows, a body that is not text, or one
    over 1 KiB is `Unavailable` (for a kill, `ended: false`, "the container may still be up"). A name written twice is a case of its own
    because `serde_json::Value` keeps the last and drops the first without a trace, which would turn
    `{"ended":false,…,"ended":true}` into a confirmed end; the readers go through a member reader that refuses a repeat.
  - *Found by the corpus.* The first request reader was the derived one, and serde's derive also reads an **array** in the order of the
    members: `[3,"small",60000]` was a valid request. The request is now an object with exactly the three names, read by the same
    strict member reader, and the corpus pins the array.
  - *Evidence.* `cargo test -p sagat-infra-lake --test sandbox_api`: 13 passed; the array case failed first, for the reason above. 16
    single mutations by hand (a status unchecked, a member added or missing, a name twice, the body bound, the slot range, the deadline
    of zero, the uid range, a class in capitals, a reason dropped, for the three readers and the request) were each killed by a test.
    No mutation pass (OD-2026-11-06-2). `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the
    three repository checkers clean.
  - *Not done.* Everything with a socket: the server (lake mode, the listener and its limits, admission over `LakeEngine`), the client
    behind `LakeSandbox`, the equality of `uid_for_slot` here and in `crates/infra-sandbox` (a test in the sandbox runner, which will
    depend on both: the `tests` crate depends on only one of them, and an edge there would select the whole CI for one assertion).
- **D035-119 — Lake mode of the sandbox runner serves the API of ADR-0079.** Tenth slice of T035-035 and the server half of the API:
  `sandbox-runner lake`. **Decided by: delegate**. It changes `Cargo.lock` (the sandbox runner depends on `sagat-infra-lake` and on
  `rustix`; `tempfile` is a dev-dependency of two crates), which selects the whole CI: one deliberate run. The client (and so the HTTP
  client's dependencies), the `database-runner` → `infra-lake` edge and `zeroize` are the next lock change.
  - *What.* `apps/sandbox-runner/src/lake/`: `config` (the settings, and what is refused at start), `engine` (the port, the Docker adapter
    and an in-memory fake), `admission` (start, wait, kill, the sweeps, the drain), `api` (the router: a closed body for every answer and
    every refusal), `serve` (the socket, the lock, the sweeper, the drain); the `lake` subcommand and its help; and
    `sagat_api_http::serve_unix`: the HTTP loop of the platform generalised over a private `Acceptor` (TCP and TLS unchanged), with the
    peer's user checked at accept, before a byte is read.
  - *Decisions the ADR left open, taken here.* Occupancy is read from the engine's labels on each start, never kept; the check and the
    reservation are made under one lock held across the list and not the launch; the reservation is a **guard** on a plain mutex, and
    `start` runs in a task of its own; an idempotent start is keyed on the attempt label and compares slot and class; a start and a kill of
    one attempt never run at once; waits are as many as the ceiling; ended containers that are kept are capped at twice the ceiling; the
    memory a container is charged is the **label it was created with** (`sagat.lake.memory`), the largest class when there is none; the
    deadline label is the attempt's rounded up plus a grace, refused above the node's ceiling and never cut; the sweep (30 s, at most 60)
    is the deadline's only clock, with the age sweep beside it as the backstop for a label that is wrong or missing; the drain refuses
    starts, answers waits and kills for 30 s, and leaves the containers to their labels.
  - *Refused at start-up.* Any `SAGAT_RUNNER_*` variable (by prefix, over the whole environment); a peer that is root, the overflow id, a user
    of the lake containers' range, or (outside development) the user of the process; a group of root or of the overflow id; no owner; an image
    by tag outside development; a limit of zero or past its cap (the plan's table: 2 and at most 8 containers, 10 and at most 60 minutes,
    at most 16 GiB per container, and a cap on each other limit that catches a number written in bytes); a node with less memory than its
    largest class; a work root that is not a directory; an engine whose API is older than 1.45 (the sub-path mount that confines a
    container to its attempt directory); a socket directory that is a link, is not the process's, holds anything but the socket and the lock,
    or whose lock is held (one process to a socket).
  - *Independent review (security-reviewer and correctness-reviewer, given the files and the ADR, not my conclusion; every claim checked
    before it was taken).* **Blocker, from both:** a start whose client gave up leaked its reservation for good (hyper's `half_close` is
    off by default, so a closed connection drops the handler), and the ADR's "a start runs in a task of its own" was not implemented; after
    `ceiling` abandoned starts every start was refused until a restart. Fixed by the spawn and the guard, with four cases: the start dropped
    inside the engine, the caller going away, a launch that panics, and a real client that closes its connection mid-start (two cases over
    the socket). **Majors taken:** a second process took a live socket and its exit deleted the next one's (an exclusive lock, and the
    directory is checked, not trusted); the socket directory followed links and its owner was not checked; exited containers were not bounded
    and a peer chooses their deadline; the config defaults and caps contradicted the plan's table; the ADR's "in-process timer" did not exist
    (the ADR now says the sweep is the clock); memory was charged by the settings of today. **Minors taken:** the admission lock is waited
    for a bounded time; a reservation and its container are counted once; `Exists` is `started` only for the same slot and class; the admin
    listener is bound before the socket; the body is read within five seconds; the gauge falls when containers end;
    the large class's file limit is not the small one's; the work root is checked; the engine's API version is checked.
    **Not taken:** a configurable slot range (G035-065); ending the drain early when idle, the comparison of the node's memory with the
    machine's, and a bound on hyper's header buffer, which holds the 4 KiB limit as policy after the head is parsed (G035-066); an overlap
    check of the socket directory with the work root; a suspicion about the per-attempt lock map that is a few bytes at worst.
  - *Found by the cases themselves.* A mutation of the ceiling check made a case **hang** (the second start waited on a gate nobody opened):
    the gates of the fake now give way after five seconds and the waits of the cases are bounded, so that a missing protection fails a suite
    instead of stalling it; `/v1/attempts//wait` is a malformed attempt id (400), not an unknown route; the fake kept a memory flag after a
    removal that the engine does not.
  - *Evidence.* `cargo test` of `sagat-api-http`, `sagat-infra-lake`, `sagat-infra-sandbox` and `sagat-sandbox-runner`: 487 passed, 0 failed
    (71 cases in `lake`, 3 of the command line, 6 of `serve_unix` over a real socket); the whole workspace's unit tests: 526 passed, 0 failed;
    `integration_iceberg_writes` against the project's Docker (29.5.2): 48 passed (27 of the lifecycle, two of them new: the engine's API
    version, real and scripted); `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and the three repository
    checkers clean. **Mutations by hand: 32 on the server and 3 on `serve_unix`, all killed by a test.** The reviews' findings were fixed
    together with their cases, so those were not seen failing first; the mutations stand in for that. No mutation pass (OD-2026-11-06-2).
  - *Not done.* The client behind `LakeSandbox` and the case that runs it against this server and a real engine over the real socket
    (ADR-0079 rule 14); the Compose service and the Kubernetes shape (T035-029); the isolation cases through this API.
- **D035-120 — The lake runner reaches the sandbox API through a client that believes only an exact answer.** Eleventh slice of T035-035 and
  the client half of the API of ADR-0079: `SocketSandbox` in `crates/infra-lake/src/sandbox_api/client.rs`, behind the `LakeSandbox` port.
  **Decided by: delegate**. It changes `Cargo.lock` by the dependencies of `sagat-infra-lake` (`hyper` with its client feature,
  `hyper-util`, `http-body-util`, `bytes`; four lines, no new package), which selects the whole CI.
  - *What.* One connection to the node-local socket per call (connect within 2 s; optionally the kernel's word that the process at the other
    end runs as the expected user, `expecting_server`; `Connection: close`; the answer read through a bound of `MAX_BODY_BYTES`; the whole call
    under a limit, a start and a kill shorter than a wait, which the server holds for `WAIT_HOLD`). The three calls: `start`, `exited` (the
    wait) and `kill`. The port changed with it: `StartRequest` carries the slot and the budget class the lake runner chose (the relay needs
    the peer's user `uid_for_slot(slot)` before the container starts), `SandboxError::Refused(Reason)` keeps the reason of a refusal, and
    `AttemptOutcome.sandbox_refusal` carries it to the report, so that a refusal is not shown as an outage.
  - *Rules of the client (ADR-0079 rule 6, held by cases).* An answer that is not exactly one of those the wire defines is never read as
    started or as ended (the strict readers of D035-118). A wait never resolves on a transport error, a timeout or a malformed answer: it
    pauses (at least 250 ms after "running", so a server that answers at once is not asked in a loop; backing off to 2 s after an error)
    and asks again, and an attempt id the API would refuse is pending for ever instead of being sent. A kill is "not confirmed" unless the
    server says `ended: true`. The deadline goes in whole milliseconds, rounded up and never zero. An id that fails `attempt_id_ok` is
    never sent: it is a defect of the caller, and the API would only refuse it.
  - *Independent review (security-reviewer, given the client, the port and the ADR, not my conclusion).* No Blocker and no Major. Its findings
    were fixed together with their cases (the round-trip file), so those cases were not seen failing first; the hand mutations below stand
    in for that. **Obligations it left, not code of this slice:**
    the relay of an attempt is built with `with_peer_uid(uid_for_slot(slot))` (G035-056); the trace context and the failure-kind metric are
    the driver's (`apps/database-runner/src/lake.rs`). **Residual, accepted and written down:** a `kill` and the `start` before it are on
    separate connections, so the kill may reach the server first; the relay is already stopped by then and the container ends by its
    deadline label (the sweep is the clock), and the driver may kill again.
  - *Evidence.* 19 round-trip cases in `apps/sandbox-runner/src/lake/roundtrip.rs` (the real router on a real socket with the fake engine,
    and a scripted server that answers bytes, closes, holds, or never stops): the request as the API reads it, the rounding of the deadline,
    ids never sent, each refusal and the outage with its reason, a wait that ends only when the container has, a cancelled start ended by the
    kill that follows, a wrong server not believed (another user, a wrong framing, a chunk, `100-Continue`), an answer cut at its bound, a
    server that never answers given up on. `cargo test -p sagat-infra-lake -p sagat-sandbox-runner`: 110 passed in the sandbox runner's
    library, 34 in `attempt`, 0 failed; `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and the three repository
    checkers clean. **Mutations by hand on the client: all killed by a test but one**, "the abort of the connection's driver removed", which
    survives because hyper itself ends a connection whose request was dropped: an equivalent mutant today, kept as a guarantee that does
    not depend on the library (its comment says so). Several of my own cases were wrong the first time (an id that is not hexadecimal, the
    size of a chunk, a counter read before the call it counted) and were corrected, not retried. No mutation pass (OD-2026-11-06-2).
  - *Against a real engine (ADR-0079 rule 14).* `tests/integration/iceberg-writes/sandbox_api.rs`: the client, the node (`serve::run` with the
    Docker adapter) and the project's Docker (29.5.2, API 1.54) on a real socket, with a real memory-backed work volume and a real sub-path
    mount. Two cases: an attempt is started, the engine holds one container of it in the slot, the class and the deadline label that were
    asked for, the wait ends when the container has, and the kill removes it and says so; an attempt without its directory is refused as
    `directory` and makes no container. `integration_iceberg_writes`: 50 passed, 0 failed (the two new; `isolation.rs` now takes its holder
    container from a shared function, and its cases pass). **Four hand mutations of the node, all killed by these cases:** the slot of the
    container, the class label, a directory that is never missing, a deadline label that ignores the attempt's deadline. (The first run of the
    mutations said all four survived: my reading of the output looked for `FAILED` at the end of a line, which `--nocapture` moves; read again
    from the failures list, all four were killed.) `cargo clippy -p sagat-tests --all-targets -- -D warnings` clean.
  - *Not done.* A container that is held up, through the API and the real engine (the ceiling, the deadline sweep, a kill of one that runs):
    the stand-in image has no host program, so the container the API starts ends by itself; those are `lifecycle.rs` (the engine) and
    `roundtrip.rs` (the router). That the work root the node checks and the volume the container mounts are one volume (they are two in
    these cases so that they run against an engine in another machine; the deployment is T035-029). The `database-runner` → `infra-lake`
    edge, `zeroize` (G035-063) and the driver; the Compose service and the Kubernetes shape (T035-029).
- **D035-121 — The attempt input of a lake step has one wire, a strict reader and one corpus.** Twelfth slice of T035-035 and the shape that
  T035-056 will build. **Decided by: delegate**. No `Cargo.lock` change: the reader is in `sagat-infra-lake`, which already links what it uses.
  - *What.* `crates/infra-lake/src/protocol/attempt_input.rs`: `AttemptInput::read(input_json)`, a document of schema
    `sagat.lake.attempt-input.v1` with the effect key, the instant of the first attempt, the five snapshot properties the platform writes, the
    table, the ref, the mode, the records, the mapping, three flags (`ignoreUnknownFields`, `allowEmpty`, `priorAttemptMayHaveLeftFiles`) and the
    effective limits (`maxRecordBytes`, `maxRecords`, `timeoutMs`). **No credential, no relay, no connection and no deadline**: the credentials
    come from `ResolveDatabase`, the relay is the runner's, and the deadline is the lease's. `AttemptInput::write_append` joins it with what the
    runner owns (the resolved connection, the staged files, the host limits) into the `WriteAppend` of D035-109.
  - *Decisions.* The reader is **strict in the way the host's request is**: one object of at most 4 MiB in which **no name is written twice at
    any depth** (`serde_json::Value` keeps the last of a name written twice and drops the first, which is how a limit or a mapping could be read
    as one the author did not write), every member present and of its type, no member the contract does not name. The records are exactly one
    of `inline` (objects; the size is the document's) and `blobs` (1 to 32, a UUID and `ndjson` or `parquet` each, each once). What the contract
    names and this runner does not serve is refused **as unsupported by name**, not read as something else: the operations other than
    `icebergWrite`, the modes other than `append`, and a 017 database result as the source (G035-067). The limits are the intersection that the
    server computes, as it does for a SQL step (`database_snapshot`); the runner may still lower them with its own deployment ceilings. An
    error names the member and never a value, and a `Debug` shows how many records and not the records.
  - *A cost I accepted.* The ranges of the three limits (1 KiB to 2 GiB, 1 to 100 million records, 1 to 3600 s) are copied from the domain's
    `LakeLimits`, whose own check is private to the domain's module; the corpus holds each edge, so a change on one side fails a test here, but it
    is two places to change.
  - *Evidence.* `crates/infra-lake/tests/attempt_input.rs`: 9 tests, 101 cases of
    `tests/contract/iceberg-writes/fixtures/attempt-input/cases.json` (12 accepted, 11 not a document, the rest by member), the edges (32 blobs and
    the 33rd, an effect key of 512 bytes and 513, a ref of 255 and 256, a document of exactly 4 MiB and one byte more), no value in an error or in a
    `Debug`, and the request built from the base document is, as JSON, **the fixture the host's own tests parse**
    (`requests/iceberg-write-append.json`), so the lease input and the host's request cannot drift apart. **Hand mutations of the reader: 17,
    all killed by a test**; two checks (`is_empty` of the effect key and of the ref) were redundant with `plain` (an equivalent mutant), so they
    were removed and not tested. (The first mutation of a name twice was built wrongly, `false && insert(..)` skips the insert, so other tests
    killed it for another reason; rebuilt as `insert(..).is_some() && false`, it is killed by the corpus.) `cargo clippy -p sagat-infra-lake
    --all-targets -- -D warnings` clean. No mutation pass (OD-2026-11-06-2).
  - *Not done.* The server that builds the document (T035-056; its test reads this corpus), the driver that reads it
    (`apps/database-runner/src/lake`), and the sentence in `contracts/worker-protocol.md`, which still names the content in prose (a contract
    edit selects the whole stack in CI: it goes with the driver). The other modes and operations (T035-080, T035-085, revert, probe, browse).
- **D035-122 — `infra-lake` holds the secrets zeroising (closes G035-063).** Thirteenth slice of T035-035. **Decided by: delegate**. It changes
  `Cargo.lock` by one line (`zeroize` in `sagat-infra-lake`, already in the lock), which selects the whole CI: it goes with the client's PR.
  - *What.* `Connection` keeps the catalog's and the storage's secrets as `Zeroizing<String>` (the caller's `String` is **moved** in, not
    copied, so the signature of `Connection::new` is unchanged and a caller that holds `Zeroizing` text hands over the string inside, the same
    allocation); `RelayToken` is a `Zeroizing<String>` and `generate` builds it digit by digit into its final allocation (the old way left
    two-digit temporary strings and the 32 random bytes on the stack); `Body` zeroes the text of every string of its members when it is
    dropped (`scrub`), and `into_map` hands the members over whole; `encode_request` returns a **`Zeroizing<Vec<u8>>` made at its exact
    size** (measured first, then written, so it never grows and never leaves the credentials in a freed block) and scrubs the copy of the body
    that the line is made from, on the error path too (`line_of`).
  - *Tests.* Five unit tests in `protocol.rs`: the scrub empties every string at any depth and leaves numbers, booleans, nulls and the names of
    the members; the line holds the secrets and the document is scrubbed after; the document is scrubbed when the line is too long; the line's
    capacity is its length; `into_map` gives the members back whole. **Five hand mutations, all killed:** the document not scrubbed after the
    line, arrays not scrubbed, objects not scrubbed, the line allocated without its size, the limit not checked before the line.
    `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo test -p sagat-infra-lake`: all suites pass.
  - *What no test can show, and what remains.* That the memory is zeroed **is the `zeroize` crate's and the `Drop` impls'**: reading a freed
    block needs `unsafe` (a spying allocator, or a pointer kept past the drop), and the workspace forbids it (`unsafe_code = "forbid"`), so
    `Drop for Body` and the `Zeroizing` fields are held by their types and by review, not by a test. A mutation that removes `Drop for Body`
    would survive; it is an unobservable one, not an equivalent one. **Residual, written down:** `serde_json` writes a string that holds an
    escape (`"`, `\`, a control character) through a scratch buffer of its own that is freed unscrubbed; a secret with one leaves a short-lived
    copy in freed memory. A hand-written writer or a scrubbing allocator could close it; neither is in this slice.
- **D035-123 — The plan for the lake attempt driver, in the order that keeps each piece testable (the rest of T035-035).** Documentation only.
  **Decided by: delegate**. What is built is in D035-109..122; this is how the pieces are joined, so that the next slices do not have to decide it
  again after a context is lost. Nothing here is built.
  - *Edge and lock.* `apps/database-runner` depends on `sagat-infra-lake` (one `Cargo.lock` line); added with the first slice that uses it, because
    an unused dependency is a lint failure.
  - *Slice A, settings (`apps/database-runner/src/lake/settings.rs`).* A `lake` section of the runner's file configuration (`deny_unknown_fields`;
    absent means the lake profile is off and the lake activities are not registered): `workRoot` (absolute, exists), `sandboxSocket` (absolute) and
    the user the sandbox runner must run as (`SocketSandbox::expecting_server`), `slots` (how many; the lake runner picks the slot, ADR-0079 rule 5),
    `relayAllowCidrs` (**required and non-empty outside development: an empty list reaches nothing**, fail closed, because the relay's own denial list
    removes loopback, link-local and metadata but cannot say which public ranges a deployment trusts), `maxBytes` and `maxRecords` of one attempt's
    staging, the host's ceilings (`maxCommitRetries` at most 16, `maxDecodedBytes`, `maxMetadataBytes`), `hostStartSecs`, `killLimitSecs`, and the byte
    count above which an attempt asks for the `large` class (when the node has one).
  - *Slice B, prepare (`lake/prepare.rs`).* From the lease input (`AttemptInput`, D035-121), the connection (`connection::convert`, D035-115), a
    source of blobs (a port over `ReadBlob`) and the settings, to a `Prepared` (the staged attempt directory, the request line as a `Zeroizing`
    buffer, the relay's configuration for the user of the slot, the plan) or a failure. Order: the mode is one the connection allows; the limits are
    the **intersection** of the step's, the connection's and the settings'; the token is generated and the directory created; the sources are staged
    (a `LimitExceeded` is `ICEBERG_LIMIT_EXCEEDED` before any container); the connection is built and the policy derived; the request is encoded. Every
    path that returns drops the directory and the buffer.
  - *Failure mapping (closed).* An input that is not an attempt input, a connection the runner cannot open, an endpoint policy that refuses, a mode the
    connection does not allow: `DATABASE_CONFIGURATION_INVALID`. Staging over the limit: `ICEBERG_LIMIT_EXCEEDED`. A `ReadBlob` that fails:
    `DATABASE_CAPABILITY_UNAVAILABLE`. A directory that cannot be made or written: `WORKER_ERROR`. The sandbox: `LAKE_SANDBOX_UNAVAILABLE` (retryable);
    a refusal for `ceiling` is the same code with the reason in the metric, not in the failure.
  - *Slice C, driver (`lake/driver.rs`).* Behind ports: the control plane (`resolve`, `read_blob`, heartbeat) and `LakeSandbox`. It prepares, builds the
    `Relay` for the user of the slot (`with_peer_uid(uid_for_slot(slot))`, G035-056, which `run_attempt` already refuses to run without), calls
    `run_attempt`, takes `Disposition`, removes the directory, and returns what to report. A start refused for the slot is tried once with another
    slot; any other refusal ends the attempt. Tested with a scripted host on the real socket, as `crates/infra-lake/tests/attempt.rs` does; that
    scaffold moves behind a `test-support` feature of `infra-lake` rather than being copied.
  - *Slice D, the shell.* The heartbeat and the report loop of `attempt.rs` (`Heartbeat`, `report_until_ack`, `upload`) are made shared, not copied; the
    poll loop dispatches on `lease.activity_name`; the registration offers the lake activities only when the section is present and the negotiated
    protocol minor includes them. The `contracts/worker-protocol.md` sentence about the lease input goes in this slice (a contract edit selects the
    whole stack in CI, so it is made once).
  - *Not decided here.* The server's builder of the input and the resolution of two pinned secrets (T035-044, T035-056; a migration, because the SQL
    runner pins one secret per attempt), the admission of a lake run (T035-055; `runs.rs`, which the open 018 branches change), and the completion of the
    step (T035-059).
- **G035-068 — The per-record size limit is carried and enforced nowhere.** `maxRecordBytes` is in the connection's limits (`LakeLimits`), in the
  step's, and now in the attempt input, but staging counts the **total** bytes and the number of records, and the host's `maxDecodedBytes` is a
  total too. A record of 1 GiB inside a 2 GiB total passes every check. *Exit condition:* the staging of inline records and of NDJSON blobs refuses a
  line over the limit (`ICEBERG_LIMIT_EXCEEDED`), with a test at the edge; a Parquet row is not parsed by the runner and stays with the host's total.
- **D035-124 — The `lake` section of the runner's configuration (slice A of D035-123).** **Decided by: delegate**. It changes `Cargo.lock` by one
  line (the edge `sagat-database-runner` → `sagat-infra-lake`, which this slice uses for the slot constants).
  - *What.* `apps/database-runner/src/lake/settings.rs`: `LakeSettings` (the file's section, `deny_unknown_fields`) and `Settings` (validated and
    parsed), wired into `Config` as an optional member that `Config::validate` checks whole. Absent is off. What a deployment must say has no
    default (`workRoot`, `sandboxSocket`, `sandboxUid`, `slots`, `relayAllowCidrs`); the numbers have the contract's defaults and are bounded on
    both sides.
  - *Decisions.* Paths are absolute and may not climb. The sandbox runner's user is neither root nor one of the containers' users (the lake range,
    both edges tested). `slots` is 1 to 4096. **`relayAllowCidrs` is required and non-empty outside development** (an empty list reaches nothing,
    so a configuration that left it out could not work and is refused instead of being left to fail at the first attempt); `0.0.0.0/0` and
    `::/0` are an explicit, valid answer (*corrected by D035-127:* the relay itself refuses loopback, link-local, metadata and other special-use addresses, and no private
    network; the platform's own ranges are the operator's `relayDenyCidrs`), and at most 256
    ranges. A bare address is not a range. A section that is not valid stops the runner at start.
  - *Evidence.* 9 unit tests in `settings.rs` and one in `config.rs` (absent means off; present is validated whole; an unknown member, a bad
    value and an empty allowance stop the runner; development allows the empty allowance); `database-runner` library tests: 35 passed;
    `cargo clippy -p sagat-database-runner --all-targets --locked -- -D warnings` clean. **Nine hand mutations, all killed** (the section not
    validated by `Config`, the empty allowance taken, a path that climbs, each edge of the lake users, root as the sandbox user, zero slots, too
    many ranges, a threshold above the ceiling). The runbook `docs/runbooks/database-runner.md` documents the section and says plainly that
    nothing serves lake activities yet.
  - *Not done.* Slices B, C and D of D035-123 (prepare, driver, shell). Whether `workRoot` exists and is a directory and whether the socket answers is
    checked when the lake runner starts serving, not by this validation (it is pure).
- **D035-125 — Preparing an attempt: staged, built and refused with a code before any container (slice B of D035-123).** **Decided by:
  delegate**. No `Cargo.lock` change (the edge came with D035-124).
  - *What.* `apps/database-runner/src/lake/prepare.rs`: `prepare(input, connection, blobs, settings, attempt)` takes the lease input
    (`AttemptInput`, D035-121), the connection `convert` read (D035-115, **consumed**: its secrets are moved into the request's connection), a `Blobs`
    port (`ReadBlob` under the lease in production, `open(blob) -> reader`), the settings (D035-124) and what is particular to the attempt (`Attempt`:
    its id, the slot, when the host must be done, the development flag), and returns a `Prepared` (the staged directory, the request line as a
    `Zeroizing` buffer, the `RelayConfig` for the user of the slot, the bytes staged for the choice of the class) or a `PrepareError`. Order: the mode is
    one the connection allows; the relay's allowance is derived from the connection (a connection the host cannot open is refused before anything is
    made); the token and the directory; the sources are staged against the effective limit; the request is built from the input with
    `write_append` and encoded. Every path out drops the directory, which removes itself.
  - *Decisions.* **Effective limits** (`effective`) are the strictest of the step's, the connection's and the settings': records are the least of the
    three, the bytes are the settings' (neither the step nor the connection has a total), the timeout is the least of the step's and the connection's
    (none when neither says: the lease's own deadline is then the only one). **Codes** (`PrepareError::code`, a closed table, none retryable): a mode the
    connection does not allow, a connection the host cannot open, a request the host would refuse (and a slot outside the range): `DATABASE_CONFIGURATION_INVALID`;
    over the limit or more than 32 files: `ICEBERG_LIMIT_EXCEEDED`; a blob that cannot be opened or a file that cannot be written (and a directory that is
    there already, which is never reused or removed): `WORKER_ERROR`. The attempt directory that was there is not ours: it is left alone.
  - *Evidence.* 14 unit tests in `prepare.rs`, against a real temporary work root and a held-in-memory set of blobs: inline records staged and the
    request line is the one the host reads (the effect key, the table, the inputs, the deadline to the millisecond, the credentials, the relay socket
    path, and **the token in the request is the token in the file the relay checks**); blobs staged in the order written with their formats and
    counted; a mode not allowed, an over-the-limit source, a blob that fails, an endless blob (stopped at the limit), a connection the relay cannot
    allow and a slot out of range all leave **nothing on disk**; the step's and the connection's limits apply before the settings'; the relay serves only
    the user of the slot; a `Debug` of the attempt shows the size of the request and not the request. `cargo test -p sagat-database-runner`: 49 passed;
    `cargo clippy -p sagat-database-runner --all-targets --locked -- -D warnings` clean. **Twelve hand mutations, all killed** (the mode unchecked, each
    limit ignored in turn, the loosest timeout, any peer served, another token, milliseconds dropped, a limit read as a source failure, the secrets
    swapped, bytes not counted, blobs backwards, a retryable limit failure).
  - *Not covered.* That the development flag reaches the relay: `RelayConfig` does not expose it, so only a relay that connects to a loopback target
    can show it (the driver's test, slice C). The per-record size limit (G035-068). A blob source over the real `ReadBlob` (slice D).
- **D035-126 — The lake driver: one attempt from the lease's input to what is reported (slice C of D035-123).** **Decided by: delegate**. No
  `Cargo.lock` change (`tokio`'s `net` feature is a dev-dependency of the database runner, already in the lock through `infra-lake`).
  - *What.* `apps/database-runner/src/lake/driver.rs`: `Driver::run(job, connection, signals, on_heartbeat) -> Finished`. It reads the input
    (`AttemptInput`), takes a slot from the `SlotPool`, derives the deadline, prepares the attempt (D035-125), builds the `Relay` for the user of the
    slot, calls `run_attempt` behind the `LakeSandbox` port and says what to report (`Report::{Complete, Fail, Abandon}`) with the facts the runner
    keeps (the request was sent, the container was confirmed gone, the relay's denials, the sandbox's refusal, where the directory was left). It
    does **not** talk to the control plane: the connection is already resolved, the signals are the lease's, and the report is the caller's to send.
    `infra-lake` gains `AttemptDir::keep` (first written as `release`, D035-127), which lets go of a directory without removing it.
  - *Decisions (the deadline, the slot and the dropped future were corrected by D035-127).* **The directory is removed only when the sandbox confirmed the container gone**; otherwise it is released and its path is returned in
    `Finished::left`, because the container may still have it mounted and its credentials are good until its deadline (the sweep, slice D, takes it
    once its deadline is past). A removal that fails is also released, not forgotten. **The slot is free when the attempt is over,** whatever the
    outcome (the sandbox is the one that refuses a slot still in use). **Slots are taken in turn,** not the lowest free, so that the user of the
    container that has just ended is not the user of the next. **The deadline is the least of the lease's end and the step's timeout** (the strictest
    of the step's and the connection's). **The class is `large`** when the staged bytes are *over* `largeAboveBytes` and the settings name one; the
    threshold itself is `small`. **No retry with another slot** (this amends D035-123 slice C, which said to try once more): the code
    `LAKE_SANDBOX_UNAVAILABLE` is retryable, the server retries the step, and the next attempt takes the next slot; a retry inside would need a second
    relay and a second preparation of the same sources for a case that, with one lake runner to a node, comes only from a leftover container whose
    label deadline will remove it. The peer of the relay is `peer_for_slot(slot)`: `40000 + slot` in production (`Driver::new`), the test's own user in a
    test, where the scripted host is in the same process.
  - *Evidence.* 17 tests in `driver.rs` against a scripted sandbox whose host plays the wire on the **real relay socket** (token file, `CONTROL`,
    request line, heartbeat, one ending): an attempt that commits reports the receipt and leaves the work root empty, the slot free and the request the
    host read; a failure of the host arrives with its code and column; a heartbeat reaches the caller; an input that is not an attempt input, no free
    slot, a slot with no user, a mode the connection does not allow, a sandbox that cannot start or refuses (the reason kept), a cancellation before the
    start and a lost lease each leave nothing taken or made; a container not confirmed gone leaves its directory; the deadline is the least of the
    lease and the step; the large class at the threshold and a byte over. `cargo test -p sagat-database-runner -p sagat-infra-lake`: 338 passed;
    clippy for both crates clean. **Twelve hand mutations, all killed** (the directory removed or kept whatever the sandbox said, the slot never given
    back, each side of the deadline, the threshold, the slots always the lowest, a failure abandoned, the request reported unsent, the refusal lost, the
    code of no slot, the large class never asked).
  - *Not covered.* That the development flag reaches the relay (needs a relay that connects to a loopback target). A directory whose removal fails (it
    needs a subdirectory the runner cannot enter, made by another user: G035-060). The metrics of an attempt. The sweep of released directories, the
    shared heartbeat and report loop, the registration and the real `ReadBlob` source (slice D).
- **D035-127 — Corrections after the independent review of the stack D035-120..126.** **Decided by: delegate**. A security and a correctness
  reviewer were given the patch and the documents, not my conclusion (the rule of D035-119), and every claim was read against the code before it was
  taken. **No Blocker; six Majors between them, all taken.** No `Cargo.lock` change. As in D035-119, the findings were fixed together with their cases,
  so those were not seen failing first; **nineteen hand mutations of the fixes stand in for that, and all are killed.**
  - *The relay had no deny list (Major, security).* `prepare` never called `with_deny`, and the network check the relay uses refuses loopback,
    `0.0.0.0/8`, multicast and reserved ranges, link-local, `192.0.0.0/24`, 6to4 and Teredo, and the metadata addresses: **not the private ranges, not
    the pod and service networks, not the platform's own state.** A tenant declares the endpoints of its connection, so a catalog at an address inside the platform would have been reached by a compromised
    host through a raw TCP tunnel. The SQL runner closes this with `deployment.denyCidrs`, required outside development; the lake relay had no
    counterpart, and the sentence of the contract ("deny … platform state endpoints") and of D035-124 that said the relay did it by itself was wrong:
    the operator's list is the only thing that can name the platform. **`relayDenyCidrs`**, required and non-empty outside development, is applied
    with `with_deny`; a deny range wins over an allow range. Tested with a real relay: a declared host that resolves to `10.0.0.5` is refused
    (`403`, counted as `address_forbidden`) and, with the list empty, the same name and address are let through, so the list is what refuses it.
    Found with it: **the development flag was never shown to reach the relay**; now it is (loopback refused, and opened only in development).
  - *Staging had no deadline and ignored the lease (Major, both).* A `ReadBlob` that stalls held a slot, a blocking thread and the resolved
    credentials for as long as it liked, and a cancellation or a lost lease was not seen until staging ended. `prepare` now runs under a `select!`
    with the lost lease, the cancellation and the step's deadline; dropping it removes the directory. Three tests over a source that never answers.
  - *The lease's end was the attempt's deadline (Major, correctness).* A lease is renewed by its heartbeats, so a step of ten minutes would have been
    cut at the first lease TTL, and reported `OUTCOME_UNKNOWN` (not retried) once the request was sent. **The deadline is `now + the step's timeout`
    (the connection's may lower it)**, staging included; a lease that is lost is its own signal. `Job` is gone: `run(input_json, connection, …)`.
  - *Permanent refusals of the sandbox were retryable (Major, correctness).* A step whose timeout is above the node's ceiling, a class the node does
    not have and a directory the node does not accept were all `LAKE_SANDBOX_UNAVAILABLE`: the step was retried, restaged and refused again until its
    policy ran out. `Deadline`, `Class` and `Directory` are now `DATABASE_CONFIGURATION_INVALID`; `Ceiling` and `Slot` stay retryable.
  - *A dropped `run` left a container running, removed its directory and freed its slot (Major in effect, both).* A `Running` guard now owns the
    directory and the slot from the moment a container may be running: settled, it removes the directory only if the container is confirmed gone
    (once more if the removal fails) and gives the slot back; **dropped, it keeps the directory and holds the slot back until the container's deadline
    and the sandbox's sweep have passed** (a minute and a half). The container is still not killed by a drop (that needs an `await`; the label
    deadline is the backstop, as `attempt.rs` says). The same hold applies to a container the sandbox did not confirm gone.
  - *The receipt was not held to the request (security, Minor).* A receipt of another effect key, another ref, another mode or another kind is
    `LAKE_HOST_PROTOCOL`, not a commit of this step: a compromised host could otherwise have the server record a snapshot the step did not write.
  - *Settings.* `sandboxUid` refuses 65534, the user the kernel reports for an unmapped one; `workRoot` is bounded so that
    `<workRoot>/<attempt id>/relay.sock` fits a socket path (longer, every attempt would fail to bind, which is retryable and would retry for ever).
  - *Also taken.* The attempt id is `UUIDv4` (ADR-0079 rule 10: opaque and random; v7 is time-ordered). The inline records are **moved** out of the
    input (`AttemptInput::take_records`), not cloned. `Driver`'s fields are private (`Driver::new` for production; the peer function is a test
    seam inside the module). `Finished::cause` says why no container was asked for (`Input`, `Over`, `NoSlot`, `NoUser`, `Prepare(kind)`, `Relay`).
    Four tests that could not fail for the thing they named are replaced: the relay is shown to serve the user it was built for and no other, the
    host is told the end of the step (`deadlineAt`, parsed and compared), and the peer function is shown to matter.
  - *Not taken, with the reason.* `Body::into_map` stays public: it hands over what the caller already holds (the caller built the secrets), the
    tests of another crate use it, and a feature gate would be a seam of its own. `SocketSandbox::expecting_server` stays optional in the type,
    because the driver is generic over the sandbox: **the shell (slice D) must build it with `settings.sandbox_uid`** (an obligation, not code).
  - *Said for the next slices.* The egress ceiling is carried and not re-checked by the runner: the lease input has no classification, and
    publication checks it against the pinned revision, which cannot change under a run (T035-055 pins it). `ReadBlob` must be scoped to the run by
    the server, or an author could load another workflow's output into a lower-ceiling table (T035-056/slice D). The resolved secrets live across the
    staging awaits; the shell may resolve **after** staging to shorten that window, which is a decision for slice D.
  - *Evidence.* `cargo test -p sagat-database-runner -p sagat-infra-lake`: 352 passed, 0 failed; `cargo clippy --workspace --all-targets --locked -- -D
    warnings` and `cargo fmt --check` clean; the three repository checkers clean.
- **G035-069 — A transient fault while staging is a permanent failure.** A `ReadBlob` network error or `ENOSPC` while writing becomes
  `StagingError::Io`, then `WORKER_ERROR`, which the contract does not retry; and `Blobs::open` returns `Err(())`, so a missing blob (permanent) and a
  timeout (transient) look alike. Nothing was sent to the host, so a retry would be safe. One tenant filling the work volume fails other tenants'
  steps for good. *Exit condition:* a typed error from `Blobs::open`, `ENOSPC` told apart in staging, and a retryable code for them in
  `contracts/errors.md` (a contract change, so the whole CI): decided with the real `ReadBlob` source in slice D.
- **G035-070 — Numbers in inline records go through `f64`.** `serde_json` without `arbitrary_precision` or `float_roundtrip` reads an integer above
  `u64::MAX` as a float and some decimals one unit off in the last place; `1e5` is staged as `100000.0`. The server builds the document with the same
  crate, so this is the platform's behaviour already, not a regression of the reader. *Exit condition:* a corpus case for each, and either the two
  features turned on workspace-wide or a sentence in the guide that exact numbers travel in a blob.
- **G035-071 — What cancellation can leave behind, and what a big document costs.** An `AttemptDir` dropped while a `spawn_blocking` writer of
  `stage_*` is still running can lose to the file it creates (`ENOTEMPTY`, ignored by `Drop`): the directory, with the token and part of the
  records, stays with no handle. A 4 MiB document of `{},` is about a million and a half values in the reader, tens of megabytes, and the input is
  read before a slot is taken, so it is bounded by the runner's `maxConcurrent` (64) and not by the slots. *Exit condition:* the sweep of slice D
  (by age, past the attempt's deadline) takes the leftovers; the memory is measured there, and the parse moves after the slot if it matters.
- **D035-128 — The sweep of attempt directories nobody removed (a piece of slice D of D035-123).** **Decided by: delegate**. No `Cargo.lock` change.
  - *What.* `apps/database-runner/src/lake/sweep.rs`: `sweep(work_root, now, older_than)` and `keep_sweeping(…)`. The driver keeps a directory when the
    container may still be running, and a dropped or aborted attempt can leave one (G035-071); each holds the relay token and the author's records on
    a volume the container shares. The sweep removes **by age**: a directory unchanged for longer than the longest attempt can last (one hour, the
    longest `timeoutMs` the input accepts, plus the 90 s the slot is held back, plus five minutes: `SWEEP_AFTER`) belongs to no running attempt,
    because its container ended by its deadline label and nothing else touches it.
  - *Decisions.* It touches **real directories whose name is an attempt id** (ADR-0079's rule) and nothing else; a link is never followed, a file or
    another name is left and counted as `foreign`. The age is the directory's last change and the edge itself is young. A directory it cannot remove
    (the container's user left in it a subdirectory the runner cannot enter: G035-060, O035-006) is counted `failed` and tried again by the next pass.
    The removal runs on a blocking thread. The pass is reported to a callback, because the metrics are not registered for it yet.
  - *Evidence.* 7 unit tests against a real temporary directory (the set of ages, the edge, the threshold, a link to a directory outside that stays
    intact, a file and a foreign name left alone, a directory the runner cannot enter counted and then taken once it can be, an unreadable root, the
    loop that reports and stops); `cargo clippy -p sagat-database-runner --all-targets --locked -- -D warnings` clean. **Eight hand mutations, all
    killed** (a link followed, the age ignored, the edge old, any name an attempt, a file an attempt, a threshold under the longest step, a failure
    counted as removed, a sweeper that never stops).
  - *Not done.* Nothing calls it: the shell (the rest of slice D) starts `keep_sweeping` with `SWEEP_AFTER` and the runner's stop token, once per minute or
    so, and counts what it reports. The test of a directory the runner cannot enter is skipped in effect when the tests run as root, which can enter it.
- **D035-129 — The blobs of an attempt arrive whole or not at all (a piece of slice D of D035-123).** **Decided by: delegate**. No `Cargo.lock` change.
  - *What.* `apps/database-runner/src/lake/blobs.rs`: `LeaseBlobs`, the `Blobs` port over `ReadBlob` under the lease (the server checks the lease and
    audits the read), and `BlobReader`, the `AsyncRead` under it, generic over the stream of messages so that it is tested without a server. The port's
    error is now a type of its own, `BlobError` (it does not say why: G035-069 is about telling the causes apart).
  - *Decisions.* **What arrives is what was named, whole.** The first message is the header and it is for the blob that was asked for; the size is a
    non-negative number and the digest 64 lower-case hexadecimal digits; the chunks add up to the declared size, and **not a byte past it is handed
    on** (a stream that does not stop is cut at the declared size, not at the staging limit); at the end the SHA-256 of what was read is the header's
    and the count is the size. A truncated or altered blob is an error at the end of the read, and a second header, an empty message or a failing
    stream is an error where it happens; after an error every read says so again. Staging removes the partial file when its source fails (D035-112),
    so a corrupt blob is never left in `in/` as if it were whole: shown with the real `stage_blob`.
  - *Evidence.* 12 unit tests (whole in pieces of every size, empty, a read buffer smaller than a piece, altered bytes, short, past the size,
    failed-stays-failed, a stream that fails, a second header, a header for another blob, eight kinds of start that is not a header, a header whose size
    is not the size of what arrives while its digest matches, and the staging of a corrupt and of a whole blob); the runner's library: 97 tests;
    `cargo clippy -p sagat-database-runner --all-targets --locked -- -D warnings` clean. **Ten hand mutations, all killed**; the first run of the
    mutations left one alive (the size check, when the digest matches what arrived), and the case that kills it was added.
  - *Not covered.* The `ReadBlob` call itself (`LeaseBlobs::open`, a request built from the lease and the call): it is exercised by the end-to-end
    canary against a real server, not here. Slice D must still scope the blob to the run on the server (D035-127).
- **D035-130 — The server side of a lake step: what exists, what blocks it, and the order to build it.** Documentation only. **Decided by: delegate**.
  Written from a read-only reading of the code (2026-11-07, `Explore`); each slice re-reads what it touches before it starts, because the 018 pull
  requests (#91, #98) change `crates/application/src` while they are open. Nothing here is built.
  - *What exists.* The node (`Node::iceberg`, root-only, the reserved activity `sagat.database.iceberg.write`, `IcebergWrite` with its accessors),
    publication (access, modes, egress, `lake.enabled`: `definitions.rs`), the stored connection (`infra-postgres/src/database/iceberg.rs`), the
    proto messages (`worker.proto`, `ResolvedDatabase` fields 13 to 15) and the lake loop of `check_definition` (`database/admission.rs:278-340`), which
    already returns lake pins. **Nothing on the lease, claim, resolve or report paths knows about a lake step.**
  - *Why a lake run cannot start today, and why that is a safety.* `check_definition` refuses a run with a lake write at creation
    (`admission.rs:160-165`, `DatabaseUnavailable`, audited as a database admission refusal), and `managed_capabilities_allowed` (`worker.rs:88-99`, again in
    `infra-postgres/src/workers.rs:284`) lets a database runner register only for exactly `sagat.database.postgresql`, so no worker can claim the
    activity. **If either were opened alone the step would be leased as an ordinary activity**: the "managed" predicate is
    `database_operation().is_some()` in about ten places (the ready filter `worker.rs:1052`, the lease input `:1119`, the lease actor `:1316`, `:2202`,
    `scoped.rs:161`, `scoped_claims.rs:123`, `scoped_reports.rs:49`, `api-http/dto_exec.rs:576`), so the lease would carry the whole run input and every
    step output as its `input_json` and an `input_blob_ref`, and a database runner's heartbeat, `CompleteStep`, `FailStep` and `ReadBlob` would be
    refused (`current_worker_authorized` excludes the role). The two doors must be opened in one slice, behind the same predicate.
  - *What blocks it, by place.* (1) **The predicate**: a lake step must be managed, of a kind (`Sql` or `Lake`), everywhere above. (2) **Registration**: the
    profile (`database` or `lake`) and the activities per profile do not exist; `worker_bindings` has no profile column (a migration); the protocol
    ceiling is flat at minor 6 and the contract's 1.7 is not implemented (`worker-protocol.md` says "only those the negotiated minor includes"). (3)
    **Resolve**: `DatabaseInput` and `ResolvedDatabase` carry a mandatory `DatabaseOperation` and one secret; `database_snapshot`, `database_admission`,
    `database_output_limits` and `database_pinned_secret` call `database_operation()` and `sql_secret()`; `api-grpc/src/database.rs` refuses a non-Postgres
    connection and sets `iceberg: None`, and no conversion from `IcebergConfig` to `pb::IcebergConnection` exists. (4) **Two secrets**:
    `step_database_credentials` has `PRIMARY KEY (tenant_id, run_id, node_id, attempt, generation)` and one `provider_operation`, so catalog and
    storage cannot both be pinned (a migration; `/migration`). (5) **Pins**: the lake loop of `check_definition` ignores the run's pins and resolves by
    the node's own revision or the current one, so with the guard gone a floating write would re-resolve at claim or replay instead of using the
    pinned revision. (6) **Quotas**: the concurrency counts (`database_admission`, `quotas.rs:1109`) are blind to the kind, so lake attempts would
    consume SQL concurrency, and there are no lake quota resources (T035-055 asks for 8 per namespace and 32 per tenant). (7) **`ReadBlob`** is scoped to
    the tenant and namespace (`BlobAccess`, `blobs.rs:99`), not to the run, and `check_blob_access` (`worker.rs:1730`) refuses it for a managed step; the
    precedent for run scope is `collection_source.rs:247` (the run's input blob or a `StepOutput` of this run). (8) **Creation audit**: `audit_database_denial`
    (`runs.rs:934`) audits only `DatabaseUnavailable` and `DatabaseLimit`, not `LakeRefused`.
  - *The input document (T035-056).* Built where the SQL path sets `b"{}"` (`worker.rs:1119-1131`, `claim_root_run`), as the document the reader of D035-121
    accepts; **the shared corpus is the check** (a test in the `tests` crate, which depends on both ends, reads every builder output through
    `AttemptInput::read`). Facts: the run, node and step ids (`tx.info()`, `instance_step_id`), the attempt (`step.attempt() + 1` before `lease_step`), the
    workflow name (`tx.parts().0.name()`), the **database clock** (`RunTx::now()`, `clock_timestamp()`; `WorkerService` itself is on `SystemClock`). Gaps in
    the facts: the **workflow version number** is not on `RunInfo` (only the version id; `workflow_versions.version_number`, read elsewhere in
    `telemetry.rs:151`), **the first attempt's start is stored nowhere** (only the journal event `step.leased` with `attempt=1`), and `allowEmpty`, which the
    runner's reader requires, **has no field in `IcebergWriteParts`** (*corrected by D035-131:* the schema names it only in the overwrite
    section; an append never allows an empty write, so it is `false` for the one mode that exists). The effect key defaults to `"{run_id}:{node}"`
    (`task_identity`, stable across attempts). Records: `run_result` / `collection_source` already evaluate an expression to canonical JSON or a
    `$blob` descriptor; an inline source must be an array of objects, a blob source is mapped to `[{"id","format"}]`, and a 017 result is G035-067.
    The builder must refuse a document over 4 MiB (the runner's bound) with a named error, instead of leasing it for the runner to refuse as a
    permanent configuration failure; the server's own cap is 8 MiB (`MAX_TASK_INPUT_BYTES`).
  - *The order.* (A) **Predicate and kinds**: `managed_kind()` replaces `database_operation().is_some()` in the places above, SQL behaviour unchanged,
    with a test that proves it on the existing SQL suites. (B) **Registration**: profiles, per-profile activities and the minor, with a migration for the
    binding's profile. (C) **Resolve for a lake step**: the sum type for `ResolvedDatabase`, the conversion to `pb::IcebergConnection`, and the
    migration for two pinned secrets, with a test against the runner's `convert` through the shared wire fixture. (D) **The input builder** (T035-056)
    against the corpus, with the three facts above stored or read. (E) **Admission** (T035-055: pins on the five paths, the quotas, the guard opened,
    `LakeRefused` audited), which waits for the 018 pull requests. (F) **Completion** (T035-059) and `ReadBlob` scoped to the run. (G) **The runner's
    shell** (slice D of D035-123), then the end-to-end canary. A and B are safe to land alone (no lake step can start); C to F are one feature
    from the point of view of a run, and only E opens the door.
- **D035-131 — The server's half of the attempt input: the effective limits and a builder held to the runner's corpus (slice D of D035-130).** **Decided
  by: delegate**. No `Cargo.lock` change. Nothing calls the builder yet: the lease path does not know a lake step (D035-130).
  - *What.* (1) `LakeLimits::effective(connection)` in the domain (`EffectiveLakeLimits`, three defaults). (2) `lake_attempt_input(write, facts,
    records, limits)` in `crates/application/src/worker/lake_input.rs`: a pure function from what the server knows to the document the runner's
    strict reader accepts (D035-121). It reads nothing and stores nothing: the **records come already evaluated** (an inline array of objects or a
    list of blobs), the first attempt's instant comes as an `OffsetDateTime` (it formats it in UTC to the millisecond, so a malformed instant cannot
    be written), and the ids, the workflow and its version come as text. (3) `tests/contract/iceberg-writes/attempt_input_builder.rs`: the suite that
    depends on both ends, which is where the two are held together.
  - *Decisions.* **Each limit is the strictest of the step's and the connection's, and the platform's default when neither says** (1 MiB a record,
    1,000,000 records, 600,000 ms): the corpus's base document uses the same three, and ten minutes is the sandbox runner's default ceiling, so a
    default write is never refused by it. **A default is a default and not a ceiling**: the domain's comment said "the minimum of the platform's, the
    namespace's, the connection's and the step's", but there is no platform or namespace policy for lake limits (G035-072), so a step may name more
    than the default when its connection does not forbid it. **The builder refuses what it controls, with a name, before the step is leased**: a
    record that is not an object, no blob or more than 32 or one twice, an effect key the host would refuse, and a document over the runner's 4 MiB
    (the server's own cap of a lease input is 8 MiB, so without this the runner would refuse a permanent configuration failure after the step was
    claimed); a test holds the bound equal to the reader's. **`allowEmpty`** is `false`: the schema names it only in the overwrite section
    (T035-080), an append never allows an empty write (D035-130 had said it had no home; corrected there).
  - *Evidence.* 5 tests in the domain (strictest, symmetric, one side only, default not a ceiling, defaults inside the accepted bounds) and 10 in the
    contract suite: **the builder reproduces the corpus's base document member for member**; every document it builds in a grid of shapes (a
    namespace of eight levels, nested fields, inline, empty, blobs, every combination of the two flags) is accepted by the reader and read back as what
    was put in, the flags checked in the document because the reader has no accessor; the limits at both ends of every range, and every effective
    limit of the domain over the extremes of both sides, are accepted; the bound on a document is exact (a document of exactly 4 MiB is built and
    read, one byte more is refused by the builder); an instant in another offset is written in UTC; the document carries no credential, relay,
    connection or deadline. `cargo test -p sagat-domain -p sagat-application`: 1144 passed; clippy for the crates and the test crate clean. **Sixteen
    hand mutations, all killed.**
  - *Not done.* Everything that feeds it: evaluating the records from the run's data (`run_result` / `collection_source`, D035-130), reading the
    workflow version number and the first attempt's start (stored nowhere), the claim path that calls it, and the connection's limits as the
    server reads them.
- **G035-072 — There is no platform or namespace policy for lake limits.** The doc of `LakeLimits` says the effective value is the minimum of the platform's,
  the namespace's, the connection's and the step's, and the SQL runner has such a policy (`database policy`, `DatabaseLimits`). For a lake write only the
  step's and the connection's exist; the platform contributes a default (D035-131). *Exit condition:* a decision (product) on whether an operator caps
  lake writes per namespace, with a home for it next to the SQL policy; until then the only ceilings are the contract's absolute bounds and the runner's
  own (`maxBytes`, `maxRecords`).
- **D035-132 — The wire form of an Iceberg connection, held to the runner's reading of it (slice C of D035-130, the pure half).** **Decided by:
  delegate**. `Cargo.lock` changes by one line (the test crate depends on `sagat-database-runner`, so that the two ends can be compared).
  - *What.* `crates/api-grpc/src/lake_connection.rs`: `iceberg_connection(config, secrets) -> pb::IcebergConnection`, the server's end of what
    `ResolveDatabase` answers a lake runner with, and `LakeSecrets`, the two resolved secrets (`catalog`, `storage`) as bytes, whose `Debug` shows only
    whether each is there. It writes **every shape the proto carries** (REST with OAuth2, bearer or SigV4; Glue; Hive; JDBC; S3, GCS, ADLS; vended or
    static storage); which of them the lake host can open is the **runner's** to refuse with a name (G035-048), not the server's to hide: a connection the
    platform stored is answered as stored. Nothing calls it yet (the resolve path for a lake step is slice C, D035-130).
  - *Decisions.* An absent optional setting is the empty string and the runner reads it as absent; `limits` is absent when none is set (zero means "not
    set" for the others); the egress ceiling is always set (the runner refuses a connection without one); the modes keep their order; only the secrets
    that were resolved are in `credentials`, under `catalog` and `storage`.
  - *Evidence.* 10 tests in `crates/api-grpc/tests/lake_connection_wire.rs` (the message field by field for each shape, the flags in both places, the
    optional settings, the order of the modes, the three ceilings, the limits, the credentials map, and that neither the message nor the secrets show a
    value; the oneofs of the message have no `Debug` on purpose, so they are compared with `PartialEq`), and 4 in `tests/contract/iceberg-writes/
    connection_wire.rs`, which depends on both ends: **the message the server writes, read by the runner's `convert`, is exactly the stored catalog and
    storage documents, the warehouse locations and the two secrets of the wire fixture that the Python host parses**; a catalog that delegates with
    vended storage and a compatible store with its endpoint and style come out as the documents the platform stores; and a Glue, a Hive, a JDBC catalog,
    GCS and ADLS storage, answered as stored, are each refused by the runner as `Unsupported`, never misread. Clippy for the crates and the test crate
    clean; the architecture check passes. **Thirteen hand mutations, all killed.**
  - *Not done.* The sum type for `ResolvedDatabase` (it carries a mandatory SQL operation and one credential), the two-secret pin and its migration, the
    resolve path that resolves both secrets in two phases and answers with this message, and the profile check that only the lake profile may resolve the
    kind (D035-130 B and C). The stored form is checked here against the wire fixture only; the store's own writer is private to `infra-postgres`.
- **D035-133 — The append builders in the six SDKs, held to one build-only scenario.** **Decided by: delegate**. No `Cargo.lock` change. The
  task text (T035-064, T035-067..072) is amended, not silently reinterpreted: it says five SDKs and names `database.ts`, `database.py`,
  `database.go`, `FlowDatabase.cs` and `tests/contract/sdk-conformance/database.rs`; there are **six** official SDKs (Rust is the sixth,
  `sdks/AGENTS.md`), the builders live in files of their own (below), and the conformance runners need no Rust file of the harness.
  - *What.* `icebergWrite(id, options)` in TypeScript (`src/iceberg.ts`), Python (`iceberg.py`, `Flow.iceberg_write`), Go
    (`workflow/iceberg.go`), Java (`IcebergWriteOptions.java`, `Flow` and `WorkflowBuilder`), C# (`IcebergWriteOptions.cs`, `FlowIceberg.cs`) and
    Rust (`workflow/iceberg.rs`, `Builder::iceberg_write`). The node is `{id, type: "icebergWrite", mode: "append", connectionRef, table,
    records, ref?, mapping?, ignoreUnknownFields?, limits?, compensation?, onError?}`; **the mode is not an option**: overwrite and upsert are not
    built (T035-080, T035-085, T035-089), so no SDK can say them. `connectionRef` is the one check of a managed step, now a single function in each
    SDK that `database` uses too.
  - *Decisions.* One fixed message, `invalid iceberg write` (code `VALIDATION_FAILED`), with no value in it; a supplied empty `mapping` or `limits`
    stays `[]` or `{}`, an absent option is omitted; the bounds are the schema's (namespace 1..8 levels, names and `ref` 1..255 bytes without a
    control character, `.` and `..` not a table part, a pointer of at most 512 bytes and 16 segments with `~0` and `~1` the only escapes, at most 1,024
    rows, a field or a column used once, `maxRecordBytes` 1,024..2,147,483,648, `maxRecords` 1..100,000,000, `timeoutMs` 1,000..3,600,000), except
    **an expression is held to 1..1,024 UTF-8 bytes where the schema says 4,096**, because that is what the engine takes (the same rule as every
    other expression an SDK builds). A typed SDK cannot express an unknown member the same way in six languages, so the shared scenario holds no
    refusal for one; Rust refuses it when the options are decoded, the other runners ignore it, and the server's codec refuses it (T035-053).
  - *Evidence.* `tests/contract/sdk-conformance/scenarios/iceberg-write-builders.json` (build only: eight valid workflows, among them every option, each
    record source, the edge of every bound and non-ASCII names, and 29 refusals, one for each rule, that must carry the fixed message) and its golden
    `expected/iceberg-write-builders.workflows.json`, **written by `SAGAT_SDK_BLESS=1` from TypeScript, Python and Go together after the harness had
    compared them, then read in full, and the scenario rerun without the switch in all six languages (`SAGAT_SDK_LANGS=typescript,go,python,dotnet,
    java,rust`: passed)**. The shared valid document `tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json` is rebuilt by each SDK's
    own unit test. Each SDK's checks pass: TypeScript `npm test` (138), Python pytest (319), mypy and ruff, Go `go test -race`, vet and gofmt, Java
    `./mvnw -q verify`, C# `dotnet test` (487) and `dotnet format --verify-no-changes`, Rust fmt, clippy `-D warnings` and `cargo test --locked`.
    Because the connection check of `database` was moved into the shared function in every SDK, the `database-connections` scenario (the real
    database runner, PostgreSQL and OpenBao on the project's own daemon) was rerun in all six languages and passed.
  - *Honest limit.* The builders and their tests were written together; no test of them was seen failing first, and **no mutation pass was made**
    (it is off until a release closes, OD-2026-11-06-2). What stands in for it is that the scenario holds both sides of each bound, which an
    off-by-one in any of the six languages would fail.
  - *Not done.* The receipts and the errors of a real run, and an unsupported capability failing explicitly (T035-064's other two bullets) need the
    server's lease path (D035-130 E). The Studio editor and the `recordSchema` and overwrite options.
- **D035-134 — The managed kind: one predicate for "an isolated runner executes this step" (slice A of D035-130).** **Decided by: delegate**. No
  `Cargo.lock` change, no migration, no contract change. A lake step still cannot start (the guard of `check_definition` and the registration are
  untouched), so this lands alone and changes nothing a run can observe.
  - *What.* `sagat_domain::database::ManagedKind { Sql, Lake }` and `Node::managed_kind()`. The seven places of the worker paths that asked
    `database_operation().is_some()` (or `.is_none()`) now ask the kind: the ready filter and the protocol-minor gate, the lease input (`{}` for a
    managed step: the run's data does not travel), the lease actor, `managed_database` (who may complete or fail it), the scoped binding, the scoped
    claim and the scoped refusal (the step itself and the forward of a reconciliation), in `worker.rs` and `worker/scoped*.rs`. SQL behaviour is the
    same by construction (a SQL node is `Some(Sql)`); a lake node is now managed in all of them, so that opening the door later cannot lease it as an
    ordinary activity.
  - *Left as it is, on purpose.* `api-http/dto_exec.rs`: the `type` of a run node reads `database` for SQL and `activity` for a lake step; changing it
    changes the contract's enum and belongs with admission (E) and the Studio. `database_write` (`worker.rs`) reads the SQL operation for what only SQL
    has (whether it writes). The domain's own rules (root-only, the collection shape, the IR codec) already treat the lake node by their own tests and
    its effect policy.
  - *The rule is kept by a check* (`architecture-principles.md`, rule 21): `crates/application/tests/managed_kind_sites.rs` reads `worker.rs` and
    `worker/` without their `mod tests` and fails if one asks the narrow question. **It was seen failing first, and the first version was wrong**: it
    listed three files, not four, because it cut each file at its first `#[cfg(test)]` and `worker.rs` has a test helper long before its managed
    paths. The cut is now the `mod tests` module, with a test of its own for that case. It is a source-text check, not a call-graph proof.
  - *Evidence.* Domain test `a_step_is_managed_by_the_kind_of_runner_that_executes_it…` (SQL `Some(Sql)`, lake `Some(Lake)`, an ordinary activity and an
    end node `None`; it did not compile before `managed_kind` existed, which is its red). `cargo fmt --check` and clippy `-D warnings` for the domain,
    application and api-http crates. All 82 test binaries of `sagat-domain` and `sagat-application` pass, and `contract_iceberg_writes` (35). The SQL
    paths, on the changed code: `integration_database_connections` 7 and again as the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE=1`) 7,
    `integration_database_runner` 1 (1 ignored), the `database-connections` SDK scenario in Go, C#, Java and Rust against the real runner, PostgreSQL and
    OpenBao (35 s), and **`integration_workflow_primitives`: 758 tests, 691 passed in the first run and 67 failed with `No space left on device`
    in the setup of a test database or, for one, in a commit (the 29 GB disk of the project's Docker VM filled during the 12-minute suite: 64 of the 67
    panics are at `create database` or `migrate` and name `No space left on device`, 2 are PostgreSQL's `could not write init file` at the admin
    connection, which is the same symptom without the words, and 1 is a store error, `scope compensation could not be committed`, at
    `collection_recovery.rs:274`, which I take to be the same cause because it passed on the rerun); the 67 names, rerun alone in a fresh container,
    passed (67 of 67, 90 s)**. 22 of that suite are ignored.
  - *Not run.* `e2e_database_connections` (a real browser: it builds the Studio, and the frontend dependencies and Chromium are not installed here;
    it is unrelated to the worker paths). The `database-connections` scenario in TypeScript and Python (their toolchains were removed after the
    builders were checked). No test lets a lake step through the lease paths, since none can start: the proof for the lake side is the check, the
    domain test and the reading above; the end-to-end test waits for admission (E).
  - *Not done.* B (registration profiles), C (resolve for a lake step and the two-secret pin), E, F and G of D035-130.
- **D035-135 — A lake runner may register: one profile per registration, by activity set (slice B of D035-130, the first half).** **Decided by:
  delegate**. No migration, no contract file of the API, no `Cargo.lock` change. A lake step still cannot start (the admission guard is closed), so a
  runner that registers waits for nothing, and the SQL profile is unchanged.
  - *What.* `managed_capabilities_allowed` (`crates/application/src/worker.rs`, called by the application and by the store's `create_session`) accepts a
    `DatabaseRunner` that offers `sagat.database.postgresql` alone or `sagat.database.iceberg.write` alone, from the managed minor (1.6), and refuses
    a mix of the two, an ordinary activity beside either, an empty list, and every activity the server does not serve yet: the revert, the
    destinations, the probe and the browse. Every other role is still refused the reserved prefix. The binding's patterns decide, as always, whether the
    subject may register that activity at all.
  - *What this is not.* **It does not replace D035-057.** That decision persists the profile on the binding (`worker_bindings.profile`, T035-025), so
    that an identity is of one profile whatever it is bound to. This slice judges the profile per registration: a runner bound to a wildcard can still
    register as one profile in one session and as the other in the next, which the persisted profile would forbid, and which matters for "a stolen lake
    token cannot reach PostgreSQL credentials" (D035-058). With no lake work in existence that costs nothing today, and it is **a condition on the
    next step**: admission of a lake run (D035-130 E) does not open before the profile is persisted (T035-025, T035-026) or a decision accepts this form
    with its consequence written. `contracts/worker-protocol.md` says so. *Closed on 2026-11-08 by D035-136: the profile is persisted.*
  - *Decisions.* The minor stays 1.6 and is not raised for the lake profile: the `ResolvedDatabase` fields it needs are already in the 1.6 schema, the lake
    runner is first-party, an old server refuses the new activity at registration (`NotBound`: the reverted rule answered it in the red run) and an old runner never
    offers it. *Exit condition:* the first lake wire difference, or a second lake runner of another version, takes the next free minor (the drafts 020, 021, 025
    and 026 also name 1.7). The revert is not accepted yet because the compensation node it runs is not a managed step (`managed_kind` is `None` for a
    derived compensation node), so it would be leased as an ordinary activity: that belongs with completion and compensation (D035-130 F).
  - *Evidence.* `crates/application/tests/managed_registration.rs` (4 tests: both profiles accepted at the managed minor and above, a mix, an ordinary
    activity beside either, an empty list, every unserved activity and the minor below the managed one refused, no other role admitted; **seen failing
    first** on the lake acceptance, the refusals already held) and `lake::registration` in `tests/integration/database-connections/lake.rs` (the
    real store, the membership role, the binding patterns, the negotiated minor: unbound, bound to the SQL activity only, too old a minor, the SQL
    activity under a lake binding, a mix, an ordinary activity, the revert and the probe refused; the write activity accepted at 1.6; an ordinary worker
    and a runner identity refused; **seen failing** with the rule reverted, at the lake registration that must succeed). fmt and clippy `-D warnings` for the
    application, store, gRPC and test crates; the tests of `sagat-application` and `sagat-api-grpc`; `integration_database_connections` 7 and again as the
    runtime role 7, `integration_database_runner` 1 (1 ignored), `integration_iceberg_writes` 50; `contract_iceberg_writes` after the contract note.
  - *Not done.* The persisted profile and `ResolveDatabase` refusing a kind outside it (T035-025, T035-026, slice C); the lease of lake work, which is
    admission (E); the runner's own offering of the activity by the agreed minor (`apps/database-runner/src`, slice G).
- **D035-136 — The profile of a dedicated runner is persisted on its binding (slice C1 of D035-130; the condition of D035-135).** **Decided by:
  delegate**. One migration, one REST field, no new minor, no `Cargo.lock` change. A lake step still cannot start.
  - *What.* `worker_bindings.profile` (`database` by default, or `lake`; CHECK `worker_bindings_profile_valid`), migration
    `20270109000000_worker_binding_profile` (additive: a constant default, so no rewrite; the `down` refuses while any binding is not `database`, with
    `object_not_in_prerequisite_state`). Every row of one identity in one namespace carries the same value: the store replaces them together under
    the identity's advisory lock, and a registration that finds two refuses. `BindingProfile` in the domain; `managed_profile(activities)` in the
    application is the profile an offer is for (`managed_capabilities_allowed` is now written with it). **A dedicated runner registers only when the
    profile of its binding is the profile of what it offers**, whatever its patterns cover, so a runner bound to `*` is of one profile and the
    reasoning of D035-058 (a stolen lake token cannot reach PostgreSQL credentials) holds. A replacement that changes the profile closes the
    sessions that offer the other one, counted in `revoked`, and releases their attempts, as it does for a pattern that no longer covers.
  - *REST.* `PUT /namespaces/{id}/worker-bindings/{subject}` takes an optional `profile` (`database` or `lake`; absent keeps the one in force, `database`
    for a new binding; another value is 422), and a binding in any answer carries `profile` (required in the schema: the server always sends it). The
    Studio's generated types follow (`npm run api`, `npm run check:api`), the audit record of the change names the profile when it is given.
    Bindings made before this read as `database`, which is what every dedicated runner has been, and an older engine ignores the column.
  - *Merge note.* The upgrade oracles (`release_upgrade*.rs`) now say `20270109000000`. The 018 branches carry migrations to `20270108000000` and their own
    oracle edit; whichever merges second keeps the larger stamp (`scripts/check-upgrade-boundary.py` fails otherwise).
  - *Evidence.* Domain test of `BindingProfile` (it did not compile before the type); `managed_registration.rs` gained the profile of an offer. The
    persisted rule in the real store is `lake::registration` of `tests/integration/database-connections`: a new binding is `database`; naming the lake
    activity under it is refused; naming `lake` registers it; an absent profile keeps it; a wildcard under `lake` refuses the SQL activity and under
    `database` refuses the lake one; changing the profile closes the other profile's session (1) and the binding lists the new one. **Written after the
    code, so three hand mutations stand in for the red run, and all three were killed**: registration ignoring the persisted profile (`lake.rs:1024`), a
    profile change that closes nothing (`lake.rs:1101`, "the lake session is closed…", left 0) and an absent profile that resets to the default
    (`lake.rs:1060`, left `Database`). Six REST scenarios in `contract_rest_security_multitenancy` (17 pass): `lake` and `database` are 200, `sql`,
    `Lake` and `5` are 422, a viewer is 403. fmt and clippy `-D warnings` for the domain, application, store, HTTP, gRPC and test crates; the tests of
    `sagat-domain`, `sagat-application` and `sagat-api-http`; `integration_database_connections` 7 and as the runtime role 7;
    `contract_iceberg_writes` 35; `contract_developer_experience` 8; `integration_platform_foundation` 101 before the migration test and 102 with it (the migrations and the privilege audit);
    `integration_security_multitenancy` 126; the storage classification has the column; the Studio typechecks against the regenerated types.
  - *The migration.* `migrations::the_binding_profile_rolls_back_only_while_every_binding_is_of_the_database_profile` (in
    `integration_platform_foundation`, 102 tests): a binding written without a profile reads `database`; `sql` is refused by the CHECK; with a `lake`
    binding the rollback is refused with the guard's message and leaves the column; after the binding is removed it rolls back, and forward
    restores the default. A hand mutation of the guard (`IF false`) is killed at the refused rollback. **The first version of the test took twenty
    minutes**: when a revert fails, `Migrator::undo` returns before it unlocks, so the advisory lock stays on that pooled connection and the next
    rollback waits for the pool to drop it. The test now runs the refused rollback on a connection of its own and releases the lock (1.6 s).
  - *Not run.* The release-upgrade scenarios (`release_upgrade*`), which need the released producer build; the Studio's browser journeys.
  - *Hardened the next day, after an independent security review (the first version overclaimed).* It said an identity is of one profile whatever it
    is bound to; it was one profile **per namespace**. The same machine identity bound as `lake` in one namespace and as `database` in another could,
    with a stolen lake token, register as a SQL runner in the second. Now an identity has **one profile in the whole tenant**: a binding that names
    another profile than the identity has in any other namespace is refused (`StateConflict`, 409) and leaves nothing behind, an absent profile follows
    the identity (its other namespaces) before it falls to the default, and the change is serialised on a key of its own (`binding-profile:{tenant}:
    {subject}`), taken after the namespace's. Rows of one binding that disagree are not guessed at (a replacement that names no profile is refused;
    an unbind still clears them). The audit record of every binding now names the profile that came to be in force, named or kept, so the quiet
    downgrade of an unbind followed by a bind that names none (the profile is forgotten with the last binding and `database` comes back) is on record.
    The `down` migration takes a `SHARE ROW EXCLUSIVE` lock before its guard, so a lake binding cannot be committed between the check and the drop. The
    audit of a refused registration no longer says only that the binding changed. The runbook says an identity has one profile, that an unbind forgets
    it, and that during a rolling upgrade an older replica ignores the profile, so a lake runner is bound to the exact activity and not to `*`.
    Test first and seen failing (`lake::profile_is_the_identitys`: the cross-namespace bind succeeded), then three hand mutations, all killed: the
    other namespaces not compared (`lake.rs:1197`), an absent profile that ignores them (`lake.rs:1213`) and an audit that does not name the profile
    (`lake.rs:1226`). *Left as it is:* the response of a bind that names no pools or profile is read after the transaction (a concurrent change can show
    in it; enforcement is not affected, and pools already behave so), and the registration refusals still return `NotBound` without telling the caller why.
  - *Not done.* `ResolveDatabase` refusing a kind outside the caller's profile (slice C), the runner offering the lake activity by the agreed minor (G), and
    a `--profile` for `sagat worker bind` (that command binds the identity of a worker's own token for development and keeps the profile in force).
- **D035-137 — A run may carry an Iceberg write when the deployment turns the lake on (slice E of D035-130, the first half).** **Decided by:
  delegate**. No migration, no contract change, no `Cargo.lock` change. **Off by default**, and with it off nothing a run can see changes.
  - *What.* `lake.enabled` (`SAGAT_LAKE_ENABLED`) now reaches the processes that admit and lease runs: `PgRunStore::with_lake_enabled`, set from
    `ServeConfig.lake_enabled` in the control plane (the store that starts runs) and from the new `EngineConfig.lake_enabled` in the execution engine
    (the store that admits a claim; `SAGAT_LAKE_ENABLED` is parsed strictly there too, a mistyped value is refused). The guard in `check_definition`
    that refused every run carrying an Iceberg write now refuses it only when the flag is off. **The lake loop of the same function now honours the run's
    pins like the SQL loop**: for an existing run the connection and the revision are the ones it was admitted with, a run with no pin for the node is
    refused, and an explicit revision on the node that disagrees with the pin is refused, so a floating write does not re-resolve at claim or replay.
    **The pin wins once a run exists** (*corrected the same day after an independent review*): the check that judges a step naming a revision by the
    revision in force too (so that naming an older one cannot loosen a tightened mode or ceiling) applies at publication and at creation only. My first
    version let it run on every replay and claim as well, which refused the replay of an admitted run after the connection was tightened, and made a
    claim fail the whole lease call (`LakeRefused` is not a denial). A run keeps the revision it was admitted with, as a SQL run does, and the way to
    stop it is to pause the connection, which both refuse. `database_writes` now counts an Iceberg write as a write, so the run of a lake-only
    workflow started by a schedule takes the same locks and the same authority re-check as one with a SQL step.
    The quickstart for `LAKE_DISABLED` says the setting belongs on the engine replicas too, and no longer says publication waits for checks that exist.
    The setting is read by the shared `flag()` (`true`, `false`, `yes`, `no`, `1`, `0`; empty is false; anything else is refused), as D035-087 says loosely.
  - *What it does not do.* A run of an Iceberg write can now be created and pinned, and then waits: nothing leases its step. `database_claim` and the
    resolve path read the SQL operation of the node, so a lake runner that registers has its claim refused at every poll. That is the work of C (resolve,
    two secrets), D (the input fed from the run), F (completion) and G (the runner's shell). **A deployment must not turn the lake on to write to a
    table yet.** *(D035-138 lets a lake runner lease the step; it still cannot get its credentials.)*
  - *Not done (T035-055).* The lake quota rows (8 per namespace, 32 per tenant; `QuotaResource` is a domain-wide enum and the concurrency counts are blind
    to the kind), the `LakeRefused` audit at creation (`runs.rs`, which the 018 pull requests change), and a test of each creation path: the direct start
    and its replay are tested; child, event and rerun use the same store function and ad hoc refuses a lake write by construction (it asks for a SQL
    operation), none of them tested separately.
  - *Evidence.* In `lake::run` of `integration_database_connections`: the start of a published Iceberg workflow is refused with the lake off (unchanged)
    and admitted with it on, with one pin (`save`, revision 1); the replay of the start is the same run; with the pin deleted the replay is refused.
    **Written after the code, so three hand mutations stand in for the red run, all killed**: the switch ignored so a run is always refused
    (`lake.rs:629`), the switch ignored so a run is always admitted (`lake.rs:607`), and the lake loop not consulting the pins (`lake.rs:660`). The
    fixes of the review were written test first and seen failing: a connection tightened after a run was admitted (revision 2 no longer allows an
    append) leaves the replay of the run's start as that run with its pin on revision 1, and a run started afterwards is refused by name
    (`ModeNotAllowed(Append)`), red with the old behaviour (`InvalidDefinition(LakeWriteRefused …)` on the replay); an unpinned replay answers the exact
    `NotFound` a refused SQL run does; `database_writes` has a unit test (`None` where `Some(true)` was expected, red). The
    engine's setting has a test that did not compile before the field existed. fmt and clippy `-D warnings` for the store, application, engine, control
    plane and test crates; the engine's and the control plane's tests; `integration_database_connections` 7 and as the runtime role 7;
    `integration_database_runner`; `integration_iceberg_writes` 50; `contract_iceberg_writes` 35; `contract_developer_experience`;
    `integration_platform_foundation` 102.
- **D035-138 — A lake runner leases its step, with the attempt input made from the run's data (the lease half of D035-130: the feed of D, and the claim of C/E).**
  **Decided by: delegate**. No migration, no contract file, no `Cargo.lock` change. With the lake off (the default) nothing a run can see changes.
  - *What.* A lake runner of the `lake` profile, registered for `sagat.database.iceberg.write`, polls and is handed the step of a run whose
    connection it may serve, with `input_json` = the document its own strict reader accepts, made **from the run's own data**:
    `lake_lease_document` (`worker/lake_lease.rs`, pure) evaluates the record source over the run's data (`collection_source`, the same expression
    semantics as the rest of the workflow), takes the strictest limits of the step and of the connection, and calls `lake_attempt_input` (D035-131).
    The facts come from `RunTx::lake_lease_facts` (default `Err`, fail closed): the connection's limits **at the revision the run was admitted with**
    (the pin, D035-137), the number of the workflow version (`workflow_versions.version_number`), and the instant of the first lease of the step. The
    effect key is the task's own idempotency key (`{run}:{node}`, stable across attempts), a test holds the two equal.
  - *The first attempt's instant is read from the journal and no column is added.* `min(occurred_at)` of the `step.leased` events of the node is the
    instant by the database's clock that wrote it, bit for bit; for the first attempt, which has no event yet, it is the transaction's own time. An attempt
    after the first with no such event fails closed (`Internal`) instead of guessing, because a retry that looked for what an earlier attempt wrote in the
    wrong window could miss it. `priorAttemptMayHaveLeftFiles` is true when an earlier lease is in the journal.
  - *The claim.* `database_admission` has a lake variant inside the one function, chosen by the managed kind: the role, `admit_node`, the pool refusal over
    **both** secrets of the connection (catalog and storage), and a budget of its own, **8 attempts in a namespace and 32 in a tenant as constants until the
    quota rows exist** (T035-055); the per-connection pool and the SQL limits are the SQL step's and are left out. The SQL concurrency count now excludes
    lake attempts, so they no longer use up its budget. `LakeRefused` at a claim is a denial like a refused SQL claim, not an error of the whole poll.
  - *What cannot be made fails the step for good.* The document is made **after** the claim was admitted, so that a runner which fails the role, pool or
    concurrency checks cannot cause a permanent failure; then, if the data cannot make it, the step fails with a closed code, permanent (the data of a run
    does not change, so a retry would fail the same way), the journal gets `step.leased` then `step.failed`, and no lease token exists
    (`reject_input`, the general form of `reject_large_input`, with a metric label per reason). `DATABASE_CONFIGURATION_INVALID`: the record source does not
    select a list of objects (a missing value, a scalar, a record that is not an object, a blob where inline was asked). `DATABASE_CAPABILITY_UNAVAILABLE`:
    a blob source or the result of a SQL step, named instead of a step that hangs (the lease would have to mint the blob descriptors as a capability, which
    waits for the `ReadBlob` scoped to the run; G035-067 for the SQL result). `ICEBERG_LIMIT_EXCEEDED`: a document over the 4 MiB the runner reads.
  - *Evidence.* Pure: `lake_lease_document` (7 tests in `contract_iceberg_writes`, 42 in all; the suite did not compile without the module): the records are
    the selected values and the reader reads them back, an empty selection, the strictest limits, each non-list-of-objects, the two unserved sources, an
    oversized document is exactly `ICEBERG_LIMIT_EXCEEDED`. Real store (`lake_lease::run` in `integration_database_connections`, **written first and seen
    failing: the poll returned nothing**): the runner is handed the step and its own reader accepts the document, and every lake task it is handed; the
    effect key equals the task's key; the records are the selected values; no secret name travels; no `DatabaseDenied` audit for a healthy claim; a
    **second attempt** after a retryable `CATALOG_UNAVAILABLE` has attempt 2, the same effect key, the same `firstAttemptStartedAt` and
    `priorAttemptMayHaveLeftFiles: true`; a run without `rows` is not leased and fails once with `DATABASE_CONFIGURATION_INVALID` and the run is `Failed`; a
    process with the lake off leases nothing, a paused connection stops the claim of a run admitted on it, and released the run is leased; and **eight
    lake attempts at once in a namespace**, counting those leased before. The first lease test was written before the code; the edge cases after it, so **six hand mutations stand in for their red runs, all killed**:
    no prior attempt reported, the first instant made up again, an unleasable input retryable, the namespace budget unbounded, the count of a lake attempt the
    other kind's, and the claim ignoring the leasing process's switch. fmt and clippy `-D warnings`; the tests of `sagat-application` and the store;
    `integration_database_connections` 7 and as the runtime role 7; `integration_database_runner`; `integration_iceberg_writes` 50;
    `integration_durable_execution` 179; `integration_security_multitenancy` 126; the `database-connections` SDK scenario in Go, C#, Java and Rust against
    the real runner.
  - *Closes* G035-075 (a healthy lake claim is not a denial) and the SQL half of G035-074 (the SQL count excludes lake attempts).
  - *Not done.* `ResolveDatabase` for a lake step: a runner that is handed the step cannot yet get its credentials (slice C, with the two-secret pin and
    the sum type); the completion and failure reports of a lake step and the revert (F); `ReadBlob` scoped to the run, and with it the blob and
    database-result record sources; the lake quota rows and the kind-aware `quotas.rs` and `database_results.rs` (G035-074's other half, T035-055);
    the runner's own shell (G). Not tested: a secret pool that refuses the catalog secret or the storage secret separately, a role revoked between
    registration and poll, and the document's limits after the connection was tightened (they come from the pinned revision by construction).
- **D035-139 — The credential pin of an attempt is keyed by the role of the secret (slice C of D035-130, the first half: the vocabulary and the schema).**
  **Decided by: delegate**. One migration. Nothing a SQL attempt can see changes, and no lake credential is resolved yet.
  - *What.* `SecretRole` (`primary`, `catalog`, `storage`) and `ConnectionConfig::secret_slots()` in the domain: the secrets of a connection with the role each
    plays, in a fixed order (the primary one of a SQL connection; the catalog's then the storage's of a lake one). `ManagedKind::profile()`: the binding
    profile of the runner that executes a kind. Migration `20270110000000_step_credential_roles`: `step_database_credentials.secret_role`
    (`primary` by default, so every existing pin reads as it always did; a CHECK that makes it one of the three names; the primary key gains the role, so
    an attempt can pin one secret per role and no role twice). The pin stays immutable by the trigger it already has. The `down` takes a
    `SHARE ROW EXCLUSIVE` lock, then refuses while any pin is not `primary`, because an older engine reads one pin per attempt and would read the wrong one
    or none. A SQL attempt reads **only its primary pin** (`database_pinned_secret`), so a pin of another role beside it can never be read as its
    credential. The column is classified and the upgrade oracles follow the stamp.
  - *A fact the design check turned up.* A valid lake connection always has at least one secret (REST needs the catalog's, Hive needs the static
    storage's, vended storage needs REST), so the "zero secrets" case of the plan cannot occur; the code that comes will still treat an empty list as a
    resolve with nothing to read.
  - *Evidence.* Domain: 3 tests (`SecretRole` round trip and refusals; `secret_slots` for every valid shape of connection and equal in order to `secrets()`; each kind
    has one profile), red by compile first. Real store, in `executor.rs` (the SQL resolve flow with OpenBao): a pin written without a role reads
    `primary`; a catalog and a storage pin of the same attempt are accepted; a role twice, `primary` again, `sql` and the empty name are refused; the role is
    immutable; with the primary pin moved to the end of the table and the others one version apart, a SQL attempt still reads version 1; the rollback is
    refused (on a connection of its own, releasing sqlx's migration lock) while a catalog pin exists, and the extra pins are removed by an operator. **Red first
    (the column did not exist); then four hand mutations, all killed**: the rollback guard removed; the key without the role; the role not a closed list; and
    the SQL read of any pin. **The last one survived the first version of the test** (the primary row happened to come first in the heap, so an unfiltered
    `fetch_optional` still found it); the test now moves the primary pin to the end and gives the others another version, and the mutation is killed.
    fmt and clippy `-D warnings`; the domain and store tests; `integration_database_connections` 7 and as the runtime role 7; `integration_database_runner`;
    `integration_platform_foundation` (the migrations: a rollback to empty and forward again include the new pair; the classification and the privilege audit);
    `contract_iceberg_writes`; the `database-connections` SDK scenario in Go, C#, Java and Rust.
  - *The fence on the profile (the next step of the design, done).* The transaction that delivers a secret (`database_owner`, under the shared binding lock and
    the session's row lock) now refuses unless the session offers the managed activities of the profile of the step and its binding is of that profile alone
    (`profile_fence`, pure, in `worker.rs`): a binding with no row, or rows that disagree, is not a binding of the profile. Registration and the lease already
    reach only such a session; this is defence in depth in the one place the secret leaves, which does not trust that the rows it reads are the ones
    registration saw. The refusal is `NotBound`, the one a missing grant gives. Pure test (every pairing of kind, session and binding, a mix, an empty list,
    disagreeing rows; red by compile first); real store (`executor.rs`): with the binding changed to `lake` by hand a SQL step is not resolved, nor with the
    session changed to offer the lake activity, and put back the same runner resolves again (red first: the resolve succeeded). **Three hand mutations, all
    killed**: the store not applying the fence, the fence ignoring the binding, the fence ignoring what the session offers.
  - *Not done: the rest of T035-044.* The order of the design (an independent read-only design of 2026-11-08): the sum type for `DatabaseInput` and `ResolvedDatabase` with SQL
    unchanged (done, D035-140); the lake snapshot (done, D035-141), the two pins and the two audits in the second transaction after both reads (sequential, catalog then storage; either secret
    rotated between the phases makes the resolve fail and pins nothing); `ResolvedDatabase.iceberg` built by `iceberg_connection`; the retention and
    cleanup characterisation. Until then a lake runner that is handed its step cannot get its credentials.
- **D035-140 — What a worker is handed for a managed step is a sum of the two kinds (slice C of D035-130, the second step: the types).**
  **Decided by: delegate**. A refactor: nothing a SQL attempt can see changes, and the store does not yet select a lake step.
  - *What.* `DatabaseInput` is a connection and a `DatabaseWork`: `Sql { operation, secret }` or `Lake { secrets }`, each secret a `SelectedSecret`
    (its `SecretRole`, the stored secret the attempt is to use, and the version now in force). `DatabaseInput::secrets()` is the one list the resolve
    reads from the provider and pins, whatever the kind. `ResolvedDatabase` is a connection, a lease expiry and a `ResolvedGrant`: `Sql { operation,
    credential }` or `Lake(ResolvedLake { catalog, storage })`, each secret optional because a connection has the ones its shape names (D035-139). The
    store pins a list (`pin_database_credentials`) under the role of each secret, and reads a pin by role (`database_pinned_secret(node, role)`); the
    SQL paths ask for `primary`. The application refuses a lake input before it reads anything from the provider, and the wire mapping refuses a lake
    grant, both marked as the pending half of T035-044.
  - *Why a sum and not optional fields.* A struct with `operation: Option<_>` and a list of credentials would let a SQL step be handed no statement, or a lake
    step a statement, and every consumer would have to remember which combination is possible. The compiler now holds that. The SQL secret is boxed: a
    `SelectedSecret` is 360 bytes against the 24 of the lake list (clippy `large_enum_variant`).
  - *Alternatives not taken.* A second trait method and a second struct for the lake (`lake_input`, `ResolvedLake` beside `ResolvedDatabase`): two resolve
    paths in the application, the fences and the two-transaction shape written twice, and the gRPC handler choosing between them. One path that reads a list
    is smaller and has one place to be wrong.
  - *Evidence.* The existing SQL suites are the oracle, changed only where they read a field that moved (a helper `sql_grant` in the suite): fmt and clippy
    `-D warnings` on the application, store, gRPC and test crates; the lib and crate-level tests of the application, gRPC and store crates; `integration_database_connections` 7 and as the
    runtime role 7; `integration_database_runner` 1 (1 ignored, as before); `integration_platform_foundation` 102 (the privilege audit reads the changed pin
    statements); `contract_iceberg_writes` 42. The architecture and agent-configuration checks.
    *Not run for this step:* the `database-connections` SDK scenario, which the mapping change does not reach (the SQL wire answer is the same message); it
    runs with the pull request and before the step that opens the lake arm.
- **D035-141 — A lake step is handed its credentials: the snapshot, the reads, the pins and the audit (slice C of D035-130, the third step).**
  **Decided by: delegate**. The store selects a lake step, the worker service resolves it and the wire answers it; what carries out the attempt (the runner shell, slice G)
  is not built, so nothing yet uses the answer.
  - *What.* The store (`lake_snapshot`, beside the SQL path, which keeps its order of checks) refuses a subject with none of the pools of any of the secrets
    of the connection, selects each secret by role (`selected_secret`, shared with SQL: the version the attempt pinned under that role, else the one in
    force; the name and the identity of the secret judged as before) and returns them in the order of `secret_slots()`. The worker service reads them from the
    provider **one after the other, catalog then storage**, holding no lock; the second transaction selects again, compares **all** of it (`before != after`),
    pins the list, writes the audit and commits; the grant is built after the commit, each secret under its role. Nothing is pinned and nothing is handed over if
    either secret changed between the phases.
  - *One audit record for each secret* (contracts/worker-protocol.md: every secret is "compared, pinned and audited separately"). `Effects` carried one
    record, so it gains `further_audits`, written after `audit` by the three persist paths of the store, in the transaction that pins. The reason of a record
    is `<role> <name> v<version>` after the node, attempt and generation (`... generation 1: catalog a v1`), never what the secret holds. A SQL step writes the
    one it always wrote (`<name> v<version>`, no role). The role is in the text on purpose: an identical event is recorded once, so a connection whose
    catalog and storage name one secret would otherwise leave one record for two disclosures. *A mistake of this step, recorded:* the first version wrote one
    record listing both secrets, because `Effects` carried one; I read the contract again before the pull request, and the test was changed first (red: one
    record where two were expected), then the field and the writers.
  - *Empty list refused, in both layers.* A valid lake connection names at least one secret (D035-139), so a connection read back with none is not one this
    capability serves. The store and the service refuse it before anything is read, **which amends D035-139**, which said the code would treat an empty list as
    a resolve with nothing to read: a runner handed nothing is a runner that was not meant to run.
  - *A connection whose catalog and storage name one secret* reads it twice and pins it twice, once per role: two reads of the same version is the price of
    one rule for both roles. Not deduplicated.
  - *Sequential, not concurrent, reads.* Two reads at once would halve the latency of a slow provider; the verdict is the same either way. Sequential keeps the
    failure of the first read from starting the second, and makes the held-read tests deterministic (the barrier holds the first read while the second secret
    changes, then the second while the first changes). Revisit if the provider latency shows in the metric `database_capability_seconds`.
  - *Evidence.* New scenario `lake_resolve.rs` in the lifecycle of `integration_database_connections` (real PostgreSQL, real OpenBao): a connection with two
    secrets is resolved, each value and version under its role, nothing printed (`Debug`) and nothing in the audit; two pins, one per role; one audit record
    for each; the pin wins over a later rotation of the catalog's secret, and the second attempt reads the new version and pins it beside the old
    rows; a connection with one secret for both roles gives two pins and two audit records (an identical resolve of the same attempt adds none); with the first read held and the storage secret rotated, and with the second held and the
    catalog rotated, the resolve fails, pins nothing and is audited as a denial (waited for, the denial is written off the request path); a secret limited
    to a pool is refused to a runner outside it, the storage's and then the catalog's, and handed over once the pool is granted; a name deleted and written
    again does not regain the pin of the deleted secret. **Red first**: the resolve was refused (`NotFound`) because the store selected only the SQL secret.
    **Eight hand mutations, all killed, each at its own assertion**: the pools of the secrets not applied; the second transaction not judging again; the pin
    ignored; the identity of the secret not checked; only the first secret pinned; the roles missing from the audit; the records after the first not set; the
    store not writing them. fmt, clippy `-D warnings`;
    `integration_database_connections` 7 and as the runtime role 7; `integration_database_runner` 1 (1 ignored); `contract_iceberg_writes` 45 (3 new); the lib and
    crate-level tests of the application, store and gRPC crates; `check-architecture`, `check-agent-config`, `check-design-docs`.
  - *A test that could pass for the wrong reason.* The pool cases assert only that the resolve fails (the answer to a refused secret is the opaque `NotFound`,
    by design). They are not vacuous: the mutation that removes the pool check is killed there, which is the proof that the check is what refuses.
  - *The wire answer* (`crates/api-grpc/src/database.rs`, `response`, now public for the contract suite; the handler in `service.rs` was already generic).
    A lake grant is answered with the connection as stored, `iceberg` built by `iceberg_connection` with the secrets under `catalog` and `storage`, the
    lease expiry, and nothing of SQL (no `postgres`, no `credential`, no statement, no limits message). **The secrets that travel are exactly the ones the
    connection names**: a grant with one the connection does not name, or without one it does, is refused (the store selects them from the connection, so it
    cannot happen; the check is for the day it does), as is a lake grant over a PostgreSQL connection. Contract tests (`connection_wire`, through the
    runner's own `convert`): the answer is read by the runner, each secret as the one sent; a vended storage sends only the catalog's; the mismatches are
    refused. Three hand mutations, all killed: the match of the secrets removed; the two secrets swapped; the revision not answered.
  - *Retention, characterised, not changed.* The release of the pins of a terminal run deletes them by run, so every role of every attempt goes together
    and is audited once (`lake_resolve.rs`; the mutation that restricts the release to `primary` is killed). The cleanup of provider versions asks
    only whether any pin names the operation, whatever its role, so a version a catalog or storage pin holds is kept.
  - *The process that serves the worker gRPC needs `SAGAT_LAKE_ENABLED`*, as the one that leases does: with it off the resolve of a lake step is `NotFound`
    and nothing is pinned (tested). Written in `docs/runbooks/database-runner.md` and `docs/quickstarts/errors.md`.
  - *Not done, and why T035-044 is ticked anyway.* What the task asks for is built and each part has its test. Not exercised: the answer through the real gRPC
    server with a lake step (the platform harness does not turn the lake on, and nothing would call it before the runner shell, slice G, which will be its
    first caller); `check_database_blob_upload` and the output scan still read the `primary` pin, so a lake step that uploaded would be refused (read in the
    code, not exercised; its results go through slice F).
- **D035-142 — The server completes a lake step from a receipt it rebuilds (slice F of D035-130; T035-059, T6 of the plan).**
  **Decided by: delegate**. Designed by an independent read-only pass over the code (2026-11-08), whose findings are the ground of this record. Steps S1 to S4
  below (the design proposed a scan of the receipt for the secrets and a second transaction; see *No scan* for why they are not here); S5 (the operator
  decisions on an unknown outcome) is **not** taken here: it is G035-077 and O035-007, right after this record.
  - *What a lake `CompleteStep` met before this slice* (read in the code): the managed branch of `complete` takes the primary pin of the step, which a lake
    step does not have, and answers `Unavailable`; it would also have met the SQL result validator, which refuses unknown members, and the SQL credential
    codec, which refuses a lake secret. So a committed write could not be reported at all.
  - *S1, the receipt is rebuilt* (done). The port `LakeReceiptCodec` (application) reads the runner's receipt into typed fields and returns the receipt written again
    from those fields; the server stores that and never the bytes it was sent. The adapter `JsonLakeReceiptCodec` lives with the typed receipt in `sagat-infra-ir`
    (`lake_receipt.rs`, moved from `sagat-infra-lake`, which re-exports it, so the runner is unchanged): the control plane and the engine must not link the
    lake runtime, which carries the database drivers. It refuses a receipt over 16 KiB, a member the contract does not name, a count that is not a canonical
    decimal string, a kind other than `iceberg`, the outcome `REVERTED` (the receipt of a compensation, not of the step) and any receipt whose effect key, ref
    or mode is not the step's (`ExpectedReceipt`, built by the server from what it knows of the step). *Evidence:* 6 tests in `crates/infra-ir/tests/lake_receipt.rs`
    over the fixtures of the contract (red by compile first); **eight hand mutations, all killed, each at its own test**: each of the three comparisons removed,
    the runner's bytes stored as they came, a revert accepted, the object-storage kind answered as another error, the contract's members forwarded, the size
    not bounded. fmt, clippy `-D warnings`, `check-architecture`, the tests of the application, IR, lake and runner crates, `contract_iceberg_writes` 45.
    Nothing calls the codec yet.
  - *S2, the step is completed from the rebuilt receipt* (done). In the transaction of the completion (`worker/lake_complete.rs`, `output.rs`): the service
    has the codec (`with_lake_receipts`, wired in the engine and in the test harness); the attempt has **resolved its credentials** (the store says every secret
    its connection names has a pin for this attempt and generation: `lake_attempt_resolved`; a completion no resolve preceded is refused, as the SQL path
    refuses one with no pin); the receipt is rebuilt against the step as the server knows it (the effect key from the same `task_identity` the lease used, the ref
    and the mode from the node); the output the run keeps, the journal, the successors and the outbox are then the ones the completion already writes, with
    **the rebuilt bytes in place of the runner's**. The idempotency of the report is still of the bytes sent, so a replay of the same report is the same
    completion and the same key for other bytes is `IdempotencyKeyReused`. A refusal is `Unavailable` (no codec, no resolve) or `NotAllowed("invalid lake
    receipt")` and leaves the step leased, for the runner to report the failure it has. A blob output for a lake step is refused by name.
  - *No scan of the receipt for the secrets of the step* (the design pass proposed one, with a second transaction around the vault reads, as the SQL path has).
    The receipt has no member that can hold text the server did not supply: the effect key and the ref are compared with what the server knows, and every
    other member is a count, an identifier, a flag or a member of a closed set. The scan of a SQL result exists because rows are arbitrary; here the typed
    rebuild is the mechanism (D035-062), and not scanning means the completion needs no read of the vault, so a vault that is down cannot fail a completion
    after the commit it reports. A count can carry a few bytes of a secret in its digits; no scan finds that, and the SQL scan does not claim to either.
  - *The SQL result route does not serve a lake step* (G035-074, the result half): `database_results.rs` excludes the lake activity, so the receipt of a lake
    step is `NotFound` there instead of an unaudited failure of the validator of rows.
  - *Evidence.* In `lake_resolve.rs`, inside the lifecycle of `integration_database_connections` (real PostgreSQL and OpenBao): no completion before the resolve; no
    completion with a service that has no codec; seven receipts that are not the step's (another effect key, another ref, another mode, `REVERTED`, another kind, a
    member the contract does not name, not a receipt) are refused and leave the step `LEASED` with no output; the receipt of the step, sent with other whitespace
    and the members the other way round, completes it, the run is `SUCCEEDED` and `steps.output_digest` is the digest of the canonical bytes and not of the sent
    ones; the same report is `already_applied`, the same key for other bytes is `IdempotencyKeyReused`, a new key finds the step done (`StaleAttempt`); the SQL
    result route answers `NotFound` for the step; and `OUTCOME_UNKNOWN`, reported by the runner, leaves the step `WAITING` for the `OPERATOR` with that code and
    no output (existing behaviour, now pinned for a lake step). **Red first** (the refusals were `Unavailable`, not `NotAllowed`; the route served the receipt). **Four hand
    mutations, all killed**: the pin check removed; a service with no codec storing what was sent; the sent bytes stored instead of the rebuilt; a refused receipt
    stored all the same. fmt, clippy `-D warnings`; `integration_database_connections` 7 and as the runtime role 7; `integration_database_runner` 1
    (1 ignored); `contract_iceberg_writes` 45; `integration_platform_foundation` 102 (the privilege audit reads the changed result query);
    `integration_durable_execution` 179 (the completion of every ordinary step goes through the code that changed); the lib and crate tests of the application,
    store, IR, lake, gRPC, scheduler and runner crates; the architecture, agent-configuration and design-doc checks.
  - *The wire, exercised afterwards* (`lake_grpc.rs`, in the same lifecycle; the harness gained `Options.lake`). A real control plane over real PostgreSQL and OpenBao: the secrets and
    the Iceberg connection are created through the REST API, the workflow of an Iceberg write is published and started through it, a runner identity of the `lake` profile is bound through it, and
    a gRPC client stands in for the lake runner: it leases the step (the lake activity, the run asked for), resolves its connection (the message of the lake with `catalog` and `storage` and
    nothing of SQL, no secret in its `Debug`), has a receipt of another effect refused as `InvalidArgument` with the run still `RUNNING`, completes the step with the receipt of the step (the run is
    `SUCCEEDED`), and finds a replay of the same report `already_applied`. It passed on its first run, so it was shown to be able to fail: with the storage secret left out of the answer, it fails
    at the list of secrets. This closes the gap D035-141 and the first draft of this record named (the completion through the real gRPC server); what it does not cover is the runner's own
    requests against this server (D035-143 tests them against a scripted service).
  - *What two more independent reviews found (2026-11-08, security and correctness of the server side, read-only; nothing that lets another principal obtain a secret) and what was done.*
    Fixed, test first, each with its mutation killed: a completion that no resolve preceded was `Unavailable`, which reads as retryable though it never will succeed: it is `NotAllowed`
    now (the missing codec stays `Unavailable`); a receipt the runner sent that is not the step's left no trace, against rule 10: a collapsing denial audit for the run now; a commit whose
    receipt the run has no room to keep failed as `STORAGE_QUOTA_EXCEEDED` instead of `OUTCOME_UNKNOWN`, because `database_write` knew only SQL statements: every Iceberg write is a managed
    write now (a test with the tenant's content quota at its usage, lifted again); the receipt was checked for identity and not for consistency: a commit without a snapshot, an empty receipt
    with a snapshot or rows, and an empty receipt when the lease did not allow one (`APPEND_ALLOWS_EMPTY`, one value for the lease and the check) are refused, as the host writes them
    (`runners/lake/sagat_lake/modes/append.py`); a failure to write a denial audit replaced the refusal the runner was told; the grant is built before the commit, so a failure to build it
    leaves no pin and no record for a credential never delivered; `lake_snapshot` refuses a connection that is not Iceberg before reading a secret, and the SQL result route also excludes
    a lake step by the kind of the connection, the authority, and not only by the denormalised activity. The tests: the held-read cases now include the run cancelled, the lease lapsing
    and the attempt failed and leased again while a read is held (each fails, pins nothing and is audited), and the assertions that passed for any error name the error.
  - *Decided, not changed (the reviews asked).* **A lake completion does not repeat the capability fence of a SQL completion** (lease unexpired, no cancel, no stop reason): the receipt is
    the proof that a write happened, and a late receipt of a write that happened is better kept than refused; the domain's own fence (token, attempt, generation) holds, and what a cancel or
    an expiry does to the step is the domain's to judge (a lease that lapsed under a lake step is `WAITING` for an operator with `LEASE_EXPIRED`, tested). **A repeated resolve of the same
    attempt is not audited again** (an identical event is recorded once, as for SQL): the first disclosure is. **A pinned version outlives a rotation** for the attempt that holds it, until
    its lease ends: rotating a leaked secret does not cut an attempt in flight (the runbook says so).
  - *Not done in this slice.* S5 (G035-077, O035-007). The per-write audit and the `lake_*` metrics (T035-061). `OUTCOME_UNKNOWN` evidence on the waiting step (O035-007). T035-059 stays unticked: it asks
    for T035-050 and T035-051 to pass, which need the runner and a container.
- **G035-077 — D035-069 says the code already requires `saga:override` to retry an unknown outcome. It does not.** `review_instance` (`runs.rs`) maps
  `Retry` to `run:operate` and only `Skip` and `Resolve` to `saga:override`, and the domain refuses a retry of an unknown outcome outright
  (`retry_review`: `OutcomeUnresolved`, pinned by `operator_actions.rs`). The authorization matrix, FR-035-045 and D035-069 promise an operator can
  `RETRY_STEP` a lake write whose outcome is unknown, with `saga:override`. Found by the design pass of D035-142. Until O035-007 is answered the only ways
  out of an unknown outcome are the ones the code has: resolve it by decision (`saga:override`), or let the run go on to its compensation.
- **O035-007** — May an operator retry a lake write whose outcome is unknown? *Recommendation: yes, for Iceberg writes only.* The effect key lets the next
  attempt find the commit or prove it absent (D035-030), so the retry does not write twice; it needs `run:operate` **and** `saga:override`, and the step is
  leased again with `priorAttemptMayHaveLeftFiles`. SQL steps and everything else stay refused. It reverses, for one kind of step, a rule of the domain that
  its tests pin ("an unknown outcome is never retried"), and it changes `retry_review` and the check in `review_instance`, so it is the owner's call. The
  evidence on the waiting step (the target ref, the window searched, whether the key was found elsewhere) needs a closed reason on the wire, which is a
  `worker.proto` change and a minor version: the same question.
- **D035-143 — The lake runner's shell: the part of the runner that talks to the control plane around the driver (slice G of D035-130; T035-058).**
  **Decided by: delegate**. Designed by an independent read-only pass over the code (2026-11-08), whose findings are the ground of this record.
  - *What was found.* The driver (`run_attempt`, `Driver`) already took a resolved connection and the two signals, but nothing in the binary called it: `Driver::new`
    appeared only in its own tests, `keep_sweeping` and `LeaseBlobs::new` were never called, and the validated lake section was discarded. The SQL shell hard-coded its
    activity, mapped a cancel, a lost lease and a failed heartbeat to one token, and sent `retryable: false` for every failure.
  - *What was built, in the order it was built (each a commit, tests first where the code was new).* (1) A scripted worker service on a loopback port, and a test that pins what
    the SQL shell registers and polls with, before anything was built beside it. (2) The heartbeats and the terminal report, moved out of the SQL attempt to a module both
    use: the end of a lease says why (`CancelRequested` or `Lost`) and leaves what it means to the attempt, a report carries its own `retryable` flag, and the key names
    the profile (`database-complete-…` is what it always was). (3) The driver splits admission (a readable input, a free slot, the slot's user) from the run, so the
    connection, which discloses and audits the secrets, is asked for after it; `with_slots` lets the driver made for each lease share the runner's slots. (4) The lake
    attempt: the lease is checked (a root forward task of the lake activity with an expiry), the heartbeats start, the attempt is admitted, the connection is resolved
    under the same signals and read by a reader that zeroises every secret of the answer on every path, the driver runs, and the report is sent. (5) The serve loop and the
    production runtime. (6) The sweeper. (7) The metrics.
  - *Two tokens, and a drain that is a lost lease.* A cancel the control plane asked for is `cancelled`; a lease that is lost is `lease_lost`; a drain gives the leases up as
    lost and **never** cancels them, because the supervisor's table makes a cancel after the request is sent `OUTCOME_UNKNOWN`, and a rollout would make every write under way
    unknown. What a lost lease leaves is the lease expiring and the next attempt's lookup by the effect key. The grace is 30 s for the attempts and up to 30 s to settle
    the kills; a pod's termination grace has to be longer (the runbook says so; it is G035-066's kind of obligation, which no test can hold).
  - *The configuration decides the profile.* A `lake` section makes a lake runner that offers the lake activity and no SQL, and a section that is wrong, or a `workRoot` that
    is not a directory, stops the runner at start; it is not left off or served as SQL. The lake section still needs the SQL `deployment` and `namespaces` blocks of the file
    (only the namespace ids and slugs are used): a default, recorded rather than changed.
  - *A refused completion is a breach of the protocol.* The server (D035-142) refuses a receipt that is not the step's as `NotAllowed`, which is `InvalidArgument` on the wire;
    the runner then fails the step with `LAKE_HOST_PROTOCOL` (retryable). It cannot happen while the driver holds the host to the request, so it is a check, tested with a scripted refusal.
  - *Evidence.* 132 tests in the runner (it had 98): the scripted service, the SQL pin, the shared lease machinery (5), admission (4), the answer reader (4), the attempt (9 against the
    scripted service and the scripted host on the real relay socket, plus 5 of the metrics), the serve loop (6: the activity offered, the slots, no poll without a free slot, a drain that
    reports nothing, the profile chosen by the configuration, the sweep), the exit reasons (1). **The milestone: a lake step goes from the poll to `CompleteStep` through the shell**
    (`a_lake_step_goes_from_the_poll_to_the_receipt_through_the_shell`). Hand mutations, every one killed, each at its own test: 6 of the lease machinery (one survived at first, a heartbeat
    with no expiry passing because the old expiry ran out; the test now ends before it could), 10 of the attempt and the answer reader, 7 of the serve loop, 4 of the sweep wiring, 8 of the
    metrics (one survived at first, the memory kill, because the scripted sandbox never reported one). Two lessons about the tests themselves: a mutation hung every shell test because they
    waited for the runner with no bound, so every wait is bounded; a plain `sleep` is only where nothing wakes a test (the disk). `integration_database_runner` (the SQL runner against a real
    PostgreSQL) passes as before after every step that touched the SQL attempt.
  - *Honest limits.* The tests of the moved SQL code (2) and of the split (3) were written after the change, against the scripted service; (3) had no mutation runs, the existing 25 driver
    tests being its oracle. That the SQL attempt cancels on a lost lease is not pinned: it needs a database under the runner. **No test runs the shell against the real sandbox runner and a
    container**: the sandbox client and server are tested against each other (`sandbox-runner`), the driver against a scripted sandbox, and the shell against the same scripted one.
  - *What two independent reviews found, and what was done (2026-11-08, security and correctness, each read-only over the code and given the spec and not my conclusions).* Nothing that
    leaks a secret or crosses the tenant boundary. Fixed, test first and each with its mutation killed: **(1)** one heartbeat that failed for a moment ended the lease, so one network blip
    killed a write that was under way: the lake policy now retries a heartbeat that fails in a way a retry may mend until the lease expires, and a refusal or the expiry still ends it
    at once (`Policy::Lake`; the SQL attempt keeps `Policy::Sql`, which is what it was); **(2)** after a cancel the lease was no longer renewed, so a slow kill could let it run out before
    the `OUTCOME_UNKNOWN` report: a lake lease keeps being renewed after the cancel is signalled, once, and a lease that is lost after it is signalled as lost; **(3)** a resolve that
    failed because the control plane could not answer (unavailable, slow, rate limited, conflict, an error of its own) failed the step with a code that is not retryable: nothing was
    written and no secret disclosed, so the lease is now given up, and expires, and the step is leased again; a refusal for good still fails it; **(4)** the free slots the loop counted
    were not exact when a slot was held back, because the attempt took its slot inside the task, after the spawn, so the loop could spawn more attempts than free slots and fail one with
    `LAKE_SANDBOX_UNAVAILABLE`, a retry spent on a step that was not at fault: the lease is admitted when it is launched; **(5)** a lease that was given up was counted as reported, and a
    completion the control plane refused was counted as a commit before it became a failure: an `abandoned` outcome, and the write is counted when it is settled; **(6)** a kept attempt
    directory was not counted (`database_runner_lake_directories_total{outcome="kept"}`) and the tenant was not recorded in the span; **(7)** a test that could not fail (it asserted that the
    slots of a driver it had just made were free) now looks at the slots and the sandbox of the lease; and the tests the reviewer found missing were added (a refusal that is not
    `InvalidArgument`, a cancel after the request is sent through the whole shell).
  - *Found and not fixed: G035-079.* The host names the column of a schema mismatch and the runner drops it: `FailStep` carries only a closed code (`safe_message` is discarded by the
    server, as `worker.rs` says for every managed failure), so the supervisor's "its column when it is a plain name" has nowhere to go. Same exit as G035-078 and O035-007: a
    `worker.proto` field and a minor version. Residual copies of the secrets in the buffers of the transport (tonic and hyper keep the frame that carried them) are a limit the
    SQL runner has too and this one does not close: not verified by a test, and said here. The work root has to be dedicated to the runner and not persistent (an `emptyDir`): the sweep
    removes by the shape of a name and by age, and what an aborted attempt leaves is removed only by the next process, after about an hour.
  - *Not done: G035-078 and the rest of T035-058.* Compose and Helm, the container end to end (T035-062), the proof of the shell itself against a real control plane (the server's half is proved on the wire
    by `lake_grpc.rs`, with a gRPC client in the runner's place; running the shell against that server needs the scripted sandbox in a crate the tests crate can use, which the
    design asked to move behind a `test-support` feature), the phase histograms, the lookup counter, `ReadBlob` scoped to the run and the record sources other than inline.
- **G035-078 — What the relay refused is counted, not audited.** The relay counts its denials by reason and the runner has them in the result of every attempt, but the worker protocol has no call to
  put them in the audit trail (`FailStep` and `CompleteStep` carry no such field), and a new field is a `worker.proto` change and a minor version. Until then they are a metric
  (`lake_relay_denials_total`) and the log of the attempt. T035-058 asks for the audit and stays open for it. *Exit condition:* the owner's word on the minor (the same question as the evidence on
  an unknown outcome, O035-007), or a decision that a metric is the record.
- **G035-079 — The column the host names for a schema mismatch is dropped on the way to the control plane.** `ICEBERG_SCHEMA_MISMATCH` carries the column in the host's failure
  line, and the runner keeps it when it is a plain name (`Failure.column`), but `FailStep` has no field for it and `safe_message` is discarded by the server. The person who reads the
  failed run sees the code and not which column. *Exit condition:* a field in `worker.proto` and a minor version (with G035-078 and O035-007), or the decision that the code is enough.
- **G035-073 — A refused run of an Iceberg write is neither opaque nor audited at creation.** `audit_database_denial` (`runs.rs`) audits only
  `DatabaseUnavailable` and `DatabaseLimit`, and `LakeRefused` keeps its detail (the node, the field paths and classification levels) for a caller who holds
  `run:start` and `database:use`, where a refused SQL run is an opaque `NotFound`. The detail is the publisher's to see; a starter is not that person.
  *Exit condition:* T035-055, in `runs.rs` once the 018 pull requests have landed: audit it, and answer a run start with the opaque refusal while
  publication keeps the detail.
- **G035-074 — The SQL queries do not tell a lake step from a SQL one.** The concurrency counts (`execution/database.rs`, `quotas.rs`) and the SQL result
  route (`database_results.rs`) join `run_database_connections` without the kind of the connection, so a lake step that leases would consume the SQL
  budgets and a finished one would be readable through `database:result:read`. Latent: no lake step leases yet. *Exit condition:* before the first
  lake lease (slices C and F): filter by the managed kind, and give lake attempts their own quota rows (T035-055).
  *2026-11-08, half closed by D035-138:* a lake step now leases; the SQL concurrency count excludes lake attempts and lake attempts have a budget of their
  own (constants). Still open, and now reachable: the count in `quotas.rs` and the quota rows.
  *2026-11-08, the result route closed by D035-142:* `database_results.rs` excludes the lake activity.
- **G035-075 — With the lake on, a lake runner's claim is refused and audited at every poll.** The snapshot and the admission of a claim read the SQL
  operation and the SQL secret of the node. Bounded by the poll rate, and a deployment is told not to turn the lake on yet. *Exit condition:* slice C.
  *2026-11-08, closed by D035-138:* a lake claim reads the pool refusal over the connection's secrets and the lake budget, and a healthy one writes no denial.
- **G035-076 — With the lake off, the replay of the start of a run that exists answers `NotFound`.** The run was created while the lake was on and is
  still running; a client retry is told the creation failed. It is what "refused as before" does to a run that exists. *Exit condition:* decide with
  T035-055 whether the replay of an existing run bypasses the switch (the pins exist), or the switch is documented as one that is not turned off while
  lake runs are in flight.
- **G035-065 — The slot range is not configurable.** ADR-0079 rule 5 has the node configure the range of slots; any slot below 4096 is
  accepted. One lake runner to a sandbox runner (the owner label) makes that harmless. *Exit condition:* a range in the configuration the day
  two lake runners share a node.
- **G035-066 — Deployment obligations of the node that no test can hold.** The drain is a fixed 30 s, so an orchestrator must give the process
  at least that and the 10 s grace before it kills it (a Compose `stop_grace_period`, a pod's `terminationGracePeriodSeconds`); the node's memory
  is what the operator says it is and is not compared with the machine's; hyper's header buffer, not the 4 KiB policy, bounds what a request
  head can hold. *Exit condition:* the Compose service and the Helm shape of T035-029 set the grace and size the memory from the machine.
- **G035-067 — A 017 database result as the source of a lake write has no wire yet.** The reader of the attempt input (D035-121) refuses
  `records.databaseResult` as unsupported by name. A 017 result is retained as a blob in the SQL runner's JSON form, which the host does not
  read (G035-059), so either the driver reads that blob and turns the rows into JSON values for `stage_inline` (then the document names the
  blob, for instance `{"databaseResult": {"blob": "<id>"}}`), or the server turns it into an NDJSON blob when it builds the input (then the
  runner never sees the source). *Exit condition:* decided with the conversion (G035-059) and T035-056; the corpus case "a database result is a
  source this runner does not serve" becomes an accepted case and a refused one. *Narrowed (D035-145, 2026-11-08):* the second way, in its inline form: the server turns a result kept
  inline (up to 256 KiB) into inline records at the lease. What remains of this gap is a result kept as a blob, which is refused by name; the first way (the runner reads the blob)
  and a blob of NDJSON made by the server are both still open, and the reader keeps refusing `records.databaseResult`.
- **D035-144 — A lake lease reads exactly the blobs its document names: the catalog's word, a pin per attempt, and a `ReadBlob` that asks the pin (slice R of D035-130; T035-056).**
  **Decided by: delegate**. Designed by an independent read-only pass over the code (2026-11-08), whose findings are the ground of this record; it is a trust boundary,
  so the steps below each carry their own tests and the last one an independent review before it is merged.
  - *What was found.* A blob source failed the lease with `DATABASE_CAPABILITY_UNAVAILABLE` (D035-138) and a managed lease is refused `ReadBlob` altogether
    (`check_blob_access`, `worker.rs`): the refusal covers SQL and lake, read and write. The scope of a read is the tenant and the namespace (`BlobAccess`), and
    `BlobService::read` checks no run, so the lake gate has to be stricter than the ordinary one: an ordinary read also serves an `Uploaded` step output and a committed run
    input. The lease transaction holds the tenant's advisory lock and cannot do object I/O or ask the catalog through its own pool (`BlobCatalog::find`), which is why
    the lookup is a method of the run's transaction. A 017 result is the SQL step's output: inline JSON up to 256 KiB and a blob up to 4 MiB above that, so it stays
    G035-067 and is not part of this step.
  - *Decided.* (1) **The descriptor in the workflow value proves nothing.** The `blobs` expression names one to 32 descriptors (a list, or a lone descriptor read as a
    list of one), each once; the catalog says what the run's own steps committed (`RunTx::lake_record_blobs`: tenant, namespace, this run as the owner of a
    `STEP_OUTPUT`, `COMMITTED`), and the lease takes the blob only when the descriptor of the data equals the catalog's in size, digest and media type. The media type of
    the catalog decides the format (`application/x-ndjson`, `application/vnd.apache.parquet`, nothing else), and the sizes together are at most 2 GiB (plan.md,
    summed without wrapping; the runner holds the same bound while it streams). Every case that is not the run's (another run, another namespace, another tenant, a run
    input, an upload not committed, a blob being deleted, an id that does not exist) answers the same, nothing, so the answer says nothing about what exists. (2) **The
    failure codes are the closed ones that exist, no new code:** a blob the catalog does not vouch for is `DATABASE_RESULT_UNAVAILABLE`, a shape the step does not say
    or a media type that is not a record file is `DATABASE_CONFIGURATION_INVALID`, a set over the bounds is `ICEBERG_LIMIT_EXCEEDED`; all permanent, as the data of a run does
    not change. (3) **Pin, do not re-derive.** `step_lease_blobs` (migration `20270111000000_lake_blob_pins`) holds one immutable row for each blob the lease named,
    keyed by the attempt and the generation it granted, written by the lease transaction from the catalog's word and not from the caller's list, all or none; `ReadBlob`
    will be allowed a blob when a row names it for the exact fence of the request, so the document and the allow-list cannot disagree, a lapsed, retried or replaced
    attempt finds none, and the check is one indexed read where re-deriving would load the run's data for each of up to 32 reads. The foreign key to the blob is `RESTRICT`
    (no sweep or retention removes bytes a live attempt may read) and `down` refuses while any pin exists.
  - *Defaults taken, none of them product decisions.* A lone descriptor is a list of one; a blob of the run's input is not a record source of a lake write (it is not the
    output of a step, and the contract says "committed by earlier steps of the same run"); the classification of a blob's content is the flat `pii` the egress check
    already gives every blob field (FR-016-015: the bytes are never inspected, so a finer level cannot be proven, and this record does not pretend it can).
  - *Done in this slice so far (each its own commit, tests first).* The pure admission (`named_records`, `lake_lease_document`, 16 tests in the file, 12 hand mutations killed; two
    checks of `named_records` are repeated by the document builder, so they have a test of their own to be killed by) and the catalog's word (`lake_lease_facts` beside
    it: `lake_record_blobs`, one scenario over every kind of row that is not the run's, 6 mutations killed; the tenant predicate survives by construction, because the
    foreign key of `blobs` ties a run to its tenant and no row can exist that only that predicate excludes). The pin table and its two methods (`pin_lake_blobs`,
    `lake_blob_pinned`): the roll-back test was written after the migration (the scenario of the pins was written first and seen failing against a stub that accepted everything);
    8 hand mutations killed (the attempt, the generation and the node of the check, the all-or-none list, the repeated pin, the `RESTRICT` of the foreign key, the immutability
    trigger and the guard of `down`). The three upgrade tests that pin the schema boundary moved to the new stamp.
  - *The gate (done, test first).* `WorkerService::check_blob_read(principal, task, blob, trace)` replaces `check_lease` in the `ReadBlob` handler, so the check names the blob and
    runs **before** the store is asked about it. For an ordinary step nothing changes. A managed step is still refused everything except one door: a **lake** attempt reading a blob
    the lease of exactly its attempt and generation pinned; the check that names no blob, the upload and every SQL step stay `NotAllowed`. The lake read is held to the stricter fence
    of a report (`attempt_authority_fence`: a run told to stop, a cancel requested or a finished run is given nothing more) on top of the lease, the binding and the owner of the
    session that the ordinary check already makes. A blob the pin does not name is `NotFound`, the same answer as one that does not exist (another run, a run's unnamed blob, an id
    that was never made), and **the refusal is audited once for the run, without naming the blob** (`database.denied`, one constant reason: an identical event is recorded once, so a
    runner that tries one blob after another cannot fill the trail; the test sees six refusals of three blobs written as one record). A failure to write the refusal down is logged
    and the answer is still `NotFound`, as for a denied registration. The scenario (`lake_read.rs`, application level, real store and worker service) covers: the named blob read and
    audited once however often; the unnamed, another run's and a missing blob; the lease check and the upload; a wrong attempt, generation and token; a sibling lake runner and an
    ordinary worker (`NotBound`); the attempt that reported, then the retry whose pin is not its own, then its own pin; a lapsed lease; a cancelled run; and a SQL attempt naming any blob
    (`executor.rs`). 9 hand mutations: 7 killed (the pin check, the fence of the pin, the stricter fence, SQL allowed, the answer, the audit, and a distinct audit per refusal, whose
    first pattern did not test that and was corrected). **Two survive by construction:** "a lake attempt may upload" (`check_blob_upload` never carries a blob, so the `false` of the arm is a second
    line and not the first) and "the handler uses the old check" (the scenario calls the service, not the gRPC handler; the wire test of the next step is the one that holds it).
  - *The lease (done, test first).* `lake_lease_input` takes the names from the source, asks the catalog in the same transaction and builds the document from its word; the poll loop
    pins the blobs to the fence of the lease it just granted (`task.fence`), after `lease_one` and before the transaction commits, so a lease that is not granted pins nothing and a
    granted one never lacks its pins. The scenario (`lake_blob_lease.rs`; the blob rows are written between the start of a run and its lease, since a run's input is fixed at the
    start and a step commits the blob afterwards; the connection has the `pii` ceiling every blob needs, which takes the reveal grant for as long as it is written) leases a run of two
    blobs (the order, the formats read back by the runner's own reader, the pins of the attempt, a read of each and the refusal of the run's unnamed blob) and fails for good, leasing
    nothing and pinning nothing, a descriptor of another run's blob, one that lies about the size, a blob that is not a file of records, two gibibytes and a byte, and thirty-three
    blobs, each with its closed code; a retry is a lease of its own, with pins of its own, and the first attempt's token reads nothing. 6 hand mutations, all killed (the catalog not
    asked, the pin missing, for the previous attempt, for another generation and of an empty list, and the document made from the run's data: that last one is caught by the pin's own
    check, a second line, which refuses what the catalog does not vouch for and fails the poll, so a forged descriptor could not be leased even if the document trusted it).
  - *What two independent reviews found (2026-11-08, security and correctness, read-only, given the diff and the record and not my conclusions) and what was done.* Neither found a
    way for another run, namespace or tenant, a run input, an upload not committed or a blob being deleted to be read; for a read to outlive the lease at the next request; for the
    answer or its audit to tell what exists or to fill the trail; or for a pin to be written that the catalog does not vouch for. They verified, by reading, that the pin and the
    read use the same fence with no off-by-one, that a lease without its pins or a pin without its lease cannot be committed (the pin sits in the transaction of the lease, so a
    failure of it rolls back the run's batch and no other run's), that no state change of a committed blob exists between the lookup and the pin, that the sums cannot wrap, and that
    an ordinary step's `ReadBlob` is exactly what it was. What they found, all low, and what was done with each: (1) *the migration's comment was wrong and its `down` can never be
    satisfied*: pins go with their run and nothing purges a run, so once a lake step that names blobs was leased the rollback is closed. The text now says so and the guard's error
    carries the operator's way out (delete the pins of attempts that are over, as the owner: they authorize nothing and the reads are in the audit trail); the guard stays strict, as
    for the credential pins; **G035-080**. (2) *`DELETE` was granted to the runtime role and nothing deletes a pin*: dropped, and a test holds `SELECT` and `INSERT` granted and
    `UPDATE` and `DELETE` not (two mutations). (3) *the refusals of the pin's constraints were asserted as "an error"*: they are pinned to their SQLSTATE now (`23514`, `23503`). (4)
    *each lake read took the namespace's secrets lock and the run's lock, and the gate repeated the binding, owner and lease checks it had just made*: the repeat is gone (the read
    keeps what a report is held to beyond them: a cancel of the step, a stop reason, a finished run); a limit on refused reads per attempt is **G035-081**. (5) *two comments claimed
    more than their assertions*: the 33-blob case does hold the order (the store would refuse a list of 33 itself and fail the poll, so the poll that must not fail is the proof) and
    its comment now says so; the assertion about the first attempt's token no longer claims to prove that the pin is keyed by the attempt (`lake_read.rs` does). (6) *a blob the run's
    child committed is not this run's*: it fails as unavailable; "earlier steps" is enforced by what the run's data holds, since an expression can name only an output that
    exists. Of the three conditions a read is held to beyond the lease (the cancel of the step, the stop reason of the run, a finished run), **the three mutations that drop one each survive**:
    a cancel sets both the cancel of the step and the stop reason, so either refuses it alone (dropping both is killed), and a finished run has no live lease for the checks before to
    accept. They are kept because a report is held to the same three, and the survivors are the price of two conditions that overlap, not a path that no test reaches.
  - *The wire (done, after #169 gave the harness).* `lake_blob_grpc.rs`: a real control plane, real PostgreSQL, a blob store holding the bytes, the REST API that writes the `pii`
    connection (the administrator holds the reveal grant for as long as it takes) and publishes and starts the run, and a gRPC client as the lake runner. The lease names the blob; `ReadBlob`
    returns its header and its bytes as the catalog and the store hold them; a blob of the same run with a row and bytes that the lease did not name, one whose object is gone and an id that
    was never made are all `NOT_FOUND` with the same words (the object that is gone proves that the decision comes before the store, which would say `INTERNAL`: `BlobReadError::Lost`, read from `blobs.rs` and not run); the upload is shut. The
    mutation that makes the handler use the old check, which survived the application-level scenario, is killed here. The order of the check and the store is held by the lost object
    and was not mutation-tested (the handler cannot be reordered without the check's own answer).
  - *What CI found, and I had not (2026-11-09).* The rollback of the pins dropped a table with a foreign key to `runs` without taking the run lock first, as the other rollbacks of tables
    that hang from a run do (`leaf_success_certification`, `scoped_finish_binding`): a lease holds its run and then writes the pins, so a rollback that took the pins first and the run after
    could cross it, and `integration_workflow_primitives::leaf_certification::downgrade_waits_on_actual_own_run_prefix_before_retained_fact_check` (which waits for the first rollback that
    blocks on a run) timed out. The first fix, the lock after a long comment, was not enough: that test reads the **first 1024 bytes** of the waiting statement from `pg_stat_activity`, so the
    lock statements are the first lines of the file now. I had not run that suite because I had touched three of its files (the stamp of the schema boundary) and took it for a suite of
    ignored tests; CI selects a suite for the files it contains. Run the suites of the files you edit, and the whole of a suite that undoes migrations when you add one.
  - *Not done in this slice.* A SQL result as a source (G035-067); the container end to end (T035-062); the limits of G035-080 and G035-081.
- **G035-080 — Pins of lake blobs are never deleted.** `step_lease_blobs` rows go with their run, and nothing purges a run, so up to 32 rows per lake attempt that names blobs stay for
  ever (the credential pins of a lake or SQL attempt do the same), and the rollback of the migration that makes the table stays closed until an operator deletes them. *Exit condition:*
  the retention of runs (or of their pins) when it exists; the `down` guard can then let the pins of finished runs through.
- **G035-081 — The reads of a lake attempt are not limited, and a stream outlives its lease.** A compromised lake runner with a live lease can call `ReadBlob` as often as it likes: each
  call takes the run's lock and the namespace's secrets lock (the one every heartbeat of a managed attempt already takes) and a refused one writes an audit transaction (one record,
  collapsed). And the lease is checked when a read starts, so a download in flight goes on after a cancel or a lapse, for up to the transfer timeout; that is the ordinary `ReadBlob`
  too. *Exit condition:* a budget of refused reads per attempt (after N, answer as stale without a write), and cutting the stream when the lease ends, with the transfer limits of
  the blob service.
- **D035-145 — A lake write takes the result a SQL step retained: the server makes the records at the lease (slice R6 of D035-130; T035-056; narrows G035-067).**
  **Decided by: delegate**, but one part of it is a question of the owner (O035-008, below) and the default taken is stated. Designed from the code of 2026-11-08.
  - *What was found.* Publication already holds that a `databaseResult` source names a SQL node that runs before the write (`workflow.rs`, `InvalidIcebergNode`), and the egress check
    gives every result field `pii` (`lake_egress.rs`). The runner's reader refuses `records.databaseResult` by name (D035-121). The retained result is the typed document of
    feature 017 (`columns`, `rows` of `{type, value}`, `affectedRows`, `truncated`), kept inline in the step output up to `MAX_PAYLOAD_BYTES` (256 KiB) and, above that, as a blob.
  - *Decided.* (1) **The server turns the result into inline records when it makes the lease document** (`worker/lake_result.rs`, called by `named_records`), so the runner never sees
    the source and its reader keeps refusing `records.databaseResult`: there is no new wire. (2) **A row is an object keyed by its column, and each value is the cell's own**: an
    integer and a decimal stay the strings of their digits (which the host reads exactly, `docs/guides/iceberg-type-mapping.md`, and which a JSON number would not hold past 2^53, G035-070),
    a timestamp and a date are text, a JSON value is itself, and a `binary` cell stays the typed cell, which is how the host reads a binary column. (3) **The closed codes, all
    permanent** (a result does not change): a node that produced no output (it failed, or an error edge went round it), a document that is not a whole result, a truncated one, a row
    of the wrong width or a member more are `DATABASE_RESULT_UNAVAILABLE`; a statement that selects no column, two columns of one name or a column of no name are
    `DATABASE_CONFIGURATION_INVALID` (the author wrote them, and nothing can be written); a result kept as a blob is `DATABASE_CAPABILITY_UNAVAILABLE`, by name, until the lease reads
    it. (4) **The read is audited once with the lease** (`database.result_read`, for the run, naming the node and no value, as reading the result through the API is); a lease that
    is not granted reads nothing and records nothing. (5) **No new right is asked of the initiator**, which is the question below.
  - *Done (test first, each seen failing).* The converter and `named_records`: 7 cases in `lake_result_records.rs` (the order and the kinds of value, the binary cell, no row, 11 refusals
    with their codes, a node without output, a blob, and that whatever the validator of the platform accepts the converter reads or names), 11 hand mutations, all killed. The wire
    (`lake_result_grpc.rs`): the SQL runner leases the statement, resolves its credentials and completes it with a typed result; the lake runner is leased the write with the rows as its
    records (the digits of an integer past 2^53 intact, no credential in the lease), once-audited; a result of two columns of one name fails the write for good with its code and leases
    nothing; a write with inline records records no read. 4 hand mutations of the audit, all killed. The old test that expected `DATABASE_CAPABILITY_UNAVAILABLE` for any result source
    was replaced by the blob case.
  - *Not done.* A result above the inline bound (a blob of JSON up to 4 MiB), which stays refused by name: G035-067. The container end to end (T035-062).
- **O035-008** — Must the initiator (or the publisher) of a run also hold `database:result:read` for a lake write to take a SQL result? *Recommendation: no.* The result is read by the
  platform, for the run, inside the authority the workflow was admitted with: the SQL step was admitted for the connection it queried, the lake connection admits the publisher and the
  initiator, every field of a result is `pii` for the egress check so the connection's ceiling is `pii` (which took the reveal grant of the namespace to set), and the read is audited.
  `database:result:read` is the right to *see* the result through the API, which a lake write does not give; ADR-0035 item 10 forbids a bulk export through that route. *Cost of yes:* a
  check of that permission for the initiator at the lease (the principal is known there: `database_claim` already admits the initiator for the connection), and a refusal code for it. Decided here as
  "no, and audited" until the owner says otherwise.
