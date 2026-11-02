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
  environment of SC-035-009; the plan proposes values, the owner approves them.
- **O035-004** — Confirm `hold` as the default failure policy of synchronous destinations (D035-023).
- **O035-005** — Should 020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference the new 017 connection
  kinds (`object_storage`, `clickhouse`) instead of carrying their own settings and secret, so a
  destination is configured once for both modes? Recommendation: yes (020's `POSTGRESQL` sink already
  uses a 017 connection, and its `ICEBERG` sink will).

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
  (ADR-0036 decision 5: 48 → 49).

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
