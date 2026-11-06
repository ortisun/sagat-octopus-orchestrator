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
    `NoResult`; a line over 1 MiB is `LineTooLong`. The attempt driver reads the socket a line at a time under that bound.
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
