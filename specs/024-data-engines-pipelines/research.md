# Feature 024 — Baseline, sources, decisions, gaps and owner questions

Investigation date: 2026-10-26, on the worktree branch `feature/018-032-parity-specs` cut from
`origin/develop` at `70f09435` (release 0.2.0). Nothing below is implemented by this feature.
Numbers used: D1240–D1279, G1240–G1256, O1240–O1243 (allocated range 1240–1279).

## Repository evidence (verified paths)

| Observation | Where |
|---|---|
| `ConnectionConfig` has one variant, `Postgres`; the reserved activity is `sagat.database.postgresql` under the prefix `sagat.database.`; `DatabaseLimits::MAX` caps the query deadline at 30 s and results at 10,000 rows / 4 MiB | `crates/domain/src/database.rs` |
| The managed failure vocabulary is closed (18 codes, `OUTCOME_UNKNOWN` and `WORKER_ERROR` included) | `crates/domain/src/database/failure.rs` |
| The target adapter is PostgreSQL only (`tokio-postgres =0.7.18`, `sqlparser =0.63.0`); its `NetworkPolicy` holds IP networks only ("never unresolved host patterns") | `crates/infra-database/Cargo.toml`, `crates/infra-database/src/network.rs` |
| The runner is a separate binary with no platform SQL pool; it registers only the PostgreSQL activity at protocol 1.6 | `apps/database-runner/src/`, `specs/017-database-connections/contracts/managed-executor.md` |
| `listDatabaseDrivers` returns at most one item with `id: postgresql`; connection bodies use `PostgresConnectionConfig` with `driver: const postgresql`; the quota enum lists nine `database_*` operation ceilings and three count resources | `specs/001-platform-foundation/contracts/openapi.yaml` |
| `ResolveDatabase` (1.6) returns `ResolvedDatabase` with a `PostgresConnection postgres = 3` field and twelve fields in all | `specs/001-platform-foundation/contracts/worker.proto` |
| The IR `database` node is closed and root-only (no `forEach`, `while`, `repeatUntil`, `chainPolicy`) | `specs/001-platform-foundation/contracts/workflow.schema.json`; 017 D601; ADR-0036 decision 3 |
| The permission catalog has 48 entries and seven built-in roles, including `database-runner` (`worker:execute`, `database:execute`) and `runner` (`worker:execute`, `script:build`, `script:execute`) | `crates/domain/src/identity.rs` |
| Database catalog tables, immutable revision triggers and per-run pins | `crates/infra-postgres/migrations/20261123000000_database_connections.up.sql`; the latest migration is `20261220000000_reveal_grants_role_change` |
| The blob catalog distinguishes `purpose` `STEP_OUTPUT` and `RUN_INPUT`, each with its exact fence | `crates/infra-postgres/migrations/20261020000000_run_input_blobs.up.sql` |
| ADR-0023 keeps committed blobs forever and leaves retention to a separate decision (G69) | `docs/adr/ADR-0023-blob-store-claim-check.md`, Trade-offs |
| The sandbox runner configures one pinned image per language; TypeScript/Python are framework-only and refuse package-bearing locks; only the curated C# catalog admits packages | `apps/sandbox-runner/src/config.rs`, `crates/infra-sandbox/src/lib.rs`, `specs/005-inline-code-sandbox/research.md` (D105, G97, G98) |
| The alert monitor holds an `engine_leases` row, keeps one alert row per namespace, kind and subject, and resolves nothing when a look fails; alert kinds are a closed enum of five | `docs/runbooks/alerts.md`, `openapi.yaml` (`kind: [OUTBOX_STUCK, …]`) |
| The inbox key is `(tenant, namespace, source, event_id)`; event triggers filter on `eventType`, `source` and a condition over `event`/`data`; slugs cannot contain a dot, so a source named `sagat.data` cannot collide with an endpoint or connector | `crates/infra-postgres/migrations/20261012000000_events.up.sql`, `workflow.schema.json` (`triggers`), `crates/domain/src/identity.rs` (`Slug`) |
| Webhook subscriptions accept `run.finished`, `ops.alert.fired`, `ops.alert.resolved` | `openapi.yaml` (subscription `events` enum) |
| The guarded HTTP client checks each resolved address inside its resolver and ignores proxies | `crates/infra-egress/src/lib.rs` |
| `scripts/check-architecture.py` checks the domain allowlist, layer edges and cycles; it has no per-crate client allowlist (ADR-0023's "only infra-blobstore may depend on a storage client" is therefore review-only today) | `scripts/check-architecture.py` |
| Classification uses `x-sagat-classification` with `public < internal < pii < secret`; carriers are closed and exclude the database node | `specs/016-payload-classification/spec.md` (FR-016-001/002), ADR-0036 decision 6 |
| Tenant content quota default is 10 GiB | ADR-0027 (`tenant_content_bytes`) |
| A stored connection row whose driver is not `postgresql` makes the 0.2.0 store raise `corrupt("database driver")`, i.e. `StoreError::Internal`, so the whole connection list fails after a binary rollback | `crates/infra-postgres/src/database.rs` (`config_from`), `crates/infra-postgres/src/identity.rs` (`corrupt`) |
| The ADR-0013 container has no mounts and only a small `noexec,nosuid,nodev` size-limited `/tmp`; it rejected a writable disk because the engine has no per-container quota without XFS project quotas; the `runner` role holds exactly `worker:execute`, `script:build`, `script:execute` | `docs/adr/ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md` |
| ADR-0054 (020, proposed) decision 6 sends a purged run's committed blobs to ADR-0023 deletion; 020 is excluding `DATA_SNAPSHOT_PART` from that purge (coordinator decision) | `docs/adr/ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md` |
| ADR-0055 (021, proposed) keeps database connections in the 017 catalog and runs resource operations, including S3, only in `apps/integration-runner` | `docs/adr/ADR-0055-integration-resources-and-inbox-connector-kinds.md` decisions 1 and 3 |
| ADR-0057 (023, proposed) decision 4: closed offline catalogs, catalog digest in the build key, a runtime image MUST NOT carry the catalog | `docs/adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md` |

## External sources consulted (2026-10-26)

- Windmill, *Data pipelines* — <https://www.windmill.dev/docs/core_concepts/data_pipelines>: a
  pipeline is a DAG of jobs that read, compute and write datasets; large data is not passed through
  Windmill's database but through workspace object storage as `s3object` pointers; DuckDB is the
  main engine.
- Windmill, *Pipelines* — <https://www.windmill.dev/docs/core_concepts/pipelines>: DuckDB
  transformations materialize into managed DuckLake tables, steps are wired by asset lineage, data
  tests (`unique`, `not_null`, `accepted_values`, `relationships`) run against the materialized
  slice; "freshness is a consumer guarantee, not a schedule"; write-audit-publish (Enterprise) runs
  checks inside the write transaction so readers keep the previous version, while Community
  publishes then tests.
- Windmill, *Assets* — <https://www.windmill.dev/docs/core_concepts/assets>: assets (S3 objects,
  resources, volumes, dbt models) are detected by static analysis or at run time, shown as graph
  nodes with read/write modes.
- DuckDB, *Securing DuckDB* — <https://duckdb.org/docs/2.0/operations_manual/securing_duckdb/overview.html>
  (the `stable` URL redirects there): `enable_external_access=false`, `allowed_directories`,
  `allowed_paths`, `disabled_filesystems`, `autoload_known_extensions=false`,
  `autoinstall_known_extensions=false`, `lock_configuration`, `memory_limit`, `threads`,
  `max_temp_directory_size`; these "are not a substitute for proper sandboxing"; it recommends an
  isolated container, network isolation, query timeouts and not running as root.
- BigQuery `jobs.query` — <https://docs.cloud.google.com/bigquery/docs/reference/rest/v2/jobs/query>:
  `requestId` gives idempotent behaviour for duplicate requests for 15 minutes, `maximumBytesBilled`
  fails a query that would exceed it without charge, `dryRun` validates and reports statistics,
  `timeoutMs` is a hint, `labels` are bounded keys/values, positional or named parameters.
- BigQuery *Running jobs* — <https://docs.cloud.google.com/bigquery/docs/running-jobs>: a client may
  set the job ID; status is read with `jobs.get` by ID and location; `DONE` does not mean success,
  an absent `errorResult` does.
- Snowflake SQL API, *Submitting requests* — <https://docs.snowflake.com/en/developer-guide/sql-api/submitting-requests>:
  `requestId` with `retry=true` resubmits without executing a statement that already succeeded;
  bind values are strings typed per placeholder; binding is not supported in multi-statement
  requests; `timeout` defaults to `STATEMENT_TIMEOUT_IN_SECONDS`.
- Snowflake SQL API, *Authenticating* — <https://docs.snowflake.com/en/developer-guide/sql-api/authenticating>:
  key-pair JWT with `iss` = account.user.SHA256 fingerprint, valid at most one hour.
- Snowflake SQL API, *Handling responses* — <https://docs.snowflake.com/en/developer-guide/sql-api/handling-responses>:
  202 while running (also after 45 s), status by statement handle, result partitions, cancel path.
- MySQL 8.4, *START TRANSACTION* — <https://dev.mysql.com/doc/refman/8.4/en/commit.html>: `READ ONLY`
  prevents modifying or locking permanent tables and DDL, but temporary tables remain writable.
- SQL Server, *TDS 8.0* — <https://learn.microsoft.com/en-us/sql/relational-databases/security/networking/tds-8>:
  `Encrypt=strict` (SQL Server 2022 and later, Azure SQL) wraps the whole session in TLS and forbids
  `TrustServerCertificate=true`.

The Temporal parity list has no item in this scope; Temporal documentation was not consulted for
this feature. Facts not confirmed by these pages (the behaviour of a second BigQuery insert with an
existing job ID, the retention of a Snowflake `requestId`) are recorded as gaps, not assumed.

## Decisions

- **D1240 — Extend the 017 catalog, do not invent a second model.** New drivers are variants of
  `ConnectionConfig`, new rows in the driver catalog and new reserved activities under
  `sagat.database.`. Feature 021 decided (ADR-0055 decision 1) that database connections stay in the
  017 catalog, so the drivers extend it; 021 refuses them as resource types (`managedBy:
  connections`).
- **D1241 — MySQL and MariaDB are two driver IDs sharing one adapter.** Their wire protocol is
  compatible but server behaviour differs (JSON is a `LONGTEXT` alias in MariaDB; RETURNING support
  and authentication plugins differ). Separate IDs keep the type table and probe evidence honest.
- **D1242 — Two executors by trust class; the data container's mounts are declared.** Remote
  engines run in `apps/database-runner` (it holds only the attempt's credential). DuckDB and Polars
  run tenant-authored computation, so they run only as data nodes in a per-attempt container started
  by `apps/sandbox-runner`, never in a process that holds other attempts' credentials. Their inputs
  are too large for standard input, so the profile differs from ADR-0013 in exactly three mounts:
  `/in`, a read-only `ro,noexec,nosuid,nodev` bind of a per-attempt directory the runner wrote and
  bounded; `/out` and `/spill`, tmpfs `noexec,nosuid,nodev` with `size=` charged to the container's
  memory cgroup (the size mechanism, since there is no XFS project quota). Parts leave `/out` through
  a framed output channel on standard output before the container exits (D1276). ADR-0058 supersedes ADR-0013
  in part for this profile only; a unit test over the container spec asserts the profile.
- **D1243 — Warehouses use a public-endpoint profile in their own runner deployment.** CIDR
  allowlists cannot describe Google or Snowflake public endpoints, and a NetworkPolicy cannot list
  host names, so mixing public HTTPS egress with private customer-database routes in one pod is what
  ADR-0055 option 4 rejects. BigQuery and Snowflake run in a separate deployment of
  `apps/database-runner` (warehouse profile): its NetworkPolicy allows DNS, the engine and TCP 443 to
  non-private ranges (or listed private-endpoint networks for PrivateLink/PSC) and nothing private
  otherwise; the process additionally enforces the driver-fixed host form and the deployment host
  allowlist, which namespace policy can only narrow, and refuses reserved, metadata and platform
  addresses. The private profile keeps the 017 policy and has no public egress. Same binary, same
  `database-runner` role, so ADR-0036 decision 1 is unchanged.
- **D1244 — Closed credential codecs, fixed authorities.** The BigQuery codec accepts a Google
  service-account key (`type=service_account`, `client_email`, `private_key_id`, `private_key`) and
  ignores no field silently: a `token_uri` or `universe_domain` other than the fixed Google values is
  a refusal, so a secret cannot redirect the runner. Snowflake supports key-pair authentication only
  (password sign-in is not offered). Passphrase-protected PKCS#8 keys are accepted.
- **D1245 — SQL Server: SQL login over verified TLS first.** `Encrypt=strict` (TDS 8.0) where the
  server supports it, otherwise mandatory encryption with full certificate and host-name
  verification; `TrustServerCertificate` is never offered. Entra ID service principals and Windows
  authentication are G1243. The official SQL Server Linux image is x86-64 only; tests on ARM hosts
  run under emulation and say so.
- **D1246 — Per-driver deadline ceilings.** Operational engines keep the 017 query ceiling (10 s
  default, 30 s hard, `database_timeout_ms`). Warehouse drivers (BigQuery, Snowflake) get
  `database_analytic_timeout_ms` (60 s default, 600 s hard), enforced engine-side (job timeout,
  statement `timeout`) and by the lease. Data nodes use the data limits of the plan instead.
- **D1247 — BigQuery bytes billed are always capped.** `maximumBytesBilled` is mandatory in the
  connection, intersected with a namespace quota `database_bytes_billed` (10 GiB default, 1 TiB
  hard). Snowflake cost is bounded by the statement timeout and the customer's warehouse size.
- **D1248 — Deterministic request identities, lookup within the attempt.** Job ID and `requestId`
  derive from (tenant, run, node, attempt) by a keyed hash so they reveal nothing. After an
  ambiguous response the runner looks the work up a bounded number of times inside the same lease.
  Managed database operations keep one attempt (017); a lost attempt still ends `OUTCOME_UNKNOWN`
  and manual review shows the opaque identity (automation is G1240).
- **D1249 — Query mode uses the strongest mechanism per engine, never parsing alone.** MySQL/MariaDB:
  `START TRANSACTION READ ONLY` (temporary tables stay writable, so the statement allowlist refuses
  DDL and the role must lack `CREATE TEMPORARY TABLES`); SQL Server: an explicit transaction always
  rolled back; BigQuery: a dry run must report a `SELECT` statement type before execution, enforced
  only once the contract suite verifies the field (G1241). Snowflake has no read-only mode the SQL
  API exposes: refusing administrative roles does not prove a custom role is read-only, so for
  Snowflake (and BigQuery until G1241 closes) the allowlist plus the customer's grants are the only
  control, and the contract and Studio say so. Execute mode accepts only top-level
  INSERT/UPDATE/DELETE (and MERGE where the dialect supports it).
- **D1250 — Privileged identities are refused where the engine can tell.** MySQL/MariaDB:
  `SUPER`, `FILE`, `PROCESS`, `SHUTDOWN`, `CREATE USER`, `GRANT OPTION` on `*.*`; SQL Server:
  `sysadmin`, `securityadmin`, `serveradmin` server roles or `db_owner`; Snowflake: the current role
  in the administrative set. BigQuery IAM cannot be introspected with a scoped key (G1242).
- **D1251 — Keep the 017 cell vocabulary.** No new cell type is added: zone-less timestamps
  (MySQL `DATETIME`, SQL Server `DATETIME2`, Snowflake `TIMESTAMP_NTZ`, DuckDB `TIMESTAMP`) are
  unsupported and must be cast; MySQL `TINYINT(1)` is an integer, not a boolean; MariaDB JSON arrives
  as text; Snowflake `VARIANT/OBJECT/ARRAY` are JSON. Sessions are set to UTF-8 and UTC.
- **D1252 — Driver crates are chosen and inspected at implementation.** Candidates: `mysql_async`
  (MySQL/MariaDB) and `tiberius` (TDS) with rustls; warehouses through a guarded HTTPS client built
  on `crates/infra-egress` semantics, not vendor SDKs. Each is pinned exactly and its logging,
  framing and allocation behaviour is inspected as 017 did for `tokio-postgres`; an unacceptable
  finding blocks that driver.
- **D1253 — Assets are declared, not inferred.** Windmill detects assets by static analysis and at
  run time. Here an asset is a catalog resource with a revisioned contract and a single producer,
  and lineage comes from declarations and recorded provenance; SQL text is never parsed for lineage.
- **D1254 — Managed snapshots are Parquet parts in the blob store.** Parts use the ADR-0023 catalog
  with a new purpose `DATA_SNAPSHOT_PART` and the exact attempt fence; each part is at most the blob
  ceiling (64 MiB); Parquet is the only format of the first delivery (O1240).
- **D1255 — Separate data quotas.** Snapshot bytes count against `tenant_data_bytes` (10 GiB
  default) instead of `tenant_content_bytes`, and data part uploads have their own pending ceiling,
  so a pipeline cannot starve run payloads or step-output uploads.
- **D1256 — Inputs are pinned at the first lease.** Pinning at admission would read stale data for a
  step that starts late; pinning per attempt would make retries non-deterministic. The claim
  transaction that grants attempt 1 records the snapshot IDs; retries reuse them.
- **D1257 — Publication is a compare-and-set on a pointer row.** The producer captures the pointer
  version at its first lease; publication succeeds only if it is unchanged, otherwise `fail` or
  `skip`. There is no overwrite mode: an older run never replaces a newer snapshot.
- **D1258 — The engine checks what it can; audits run apart from the transform.** Footers, sizes,
  digests, schema and row counts are verified by `apps/execution-engine` (the process that accepts
  `CompleteStep`) with a pure-Rust Parquet metadata decoder in `crates/infra-data`, over a buffer of at
  most 1 MiB read by a ranged request, with decoder limits (container and string lengths, at most 256
  columns and 10,000 row groups per part), on a blocking task under `catch_unwind` (the workspace
  keeps unwinding in release builds); a panic or limit hit becomes `DATA_CONTRACT_VIOLATION` and a
  metric, never a crash. A fuzz target over hostile footers gates the adapter. Moving the decoder to
  a separate unprivileged verifier process is the fallback if fuzzing finds an unbounded case
  (G1256). Row-level audits need a query engine, so they run in a fresh container over the uploaded
  parts; their trust is the sandbox's (G1249).
- **D1259 — Quarantine is final.** A quarantined snapshot is never publishable, by run or by hand;
  the remedy is a new run. This keeps write-audit-publish from becoming write-publish.
- **D1260 — `dataPublish` is an engine node.** The pointer move is internal state, so it needs no
  worker and commits in the reducer transaction. It is not allowed in saga bodies in this delivery;
  undoing a publication is an explicit rollback.
- **D1261 — Data nodes are root-only.** As for 017 database nodes (D601), pins are keyed by root
  step; collections, loops and continuation need scoped pins first.
- **D1262 — Transforms are retryable.** A transform's only effects are staged objects keyed by its
  attempt and a pointer moved in a PostgreSQL transaction, so a retry is safe. Default 2 attempts,
  hard 5. Remote database writes keep one attempt.
- **D1263 — No eighth role.** The `runner` role gains `data:execute`; a sandbox runner advertises
  the data activities only when its data image is configured and its start-up probe passed (the
  FR-005-009 pattern). This changes ADR-0013's exact `runner` permission set, which ADR-0058 declares
  as a partial supersession. A new machine role was rejected: the data container is the same trust
  class as a script container.
  Data activities use the reserved prefix `sagat.data.` (`sagat.data.transform`, `sagat.data.query`)
  so 017's rule that sandbox runners never claim `sagat.database.*` stays true; ordinary workers and
  the database runner cannot claim `sagat.data.*`.
- **D1264 — Permissions.** `data:asset:manage` admin; `data:asset:use` admin and developer;
  `data:asset:publish` admin and operator; metadata through `namespace:read`, lineage through
  `namespace:read` plus `definition:read`. Four permissions are added to the 48 of 0.2.0; the final
  total depends on the order in which the parallel drafts (022, 025, 029 also add permissions) land,
  so the matrix test asserts the set, not a count chosen here. The same holds for roles: 024 adds
  none, 021 adds `integration-runner` and 025 `agent-runner`, so tests assert the agreed set.
- **D1265 — No new endpoint; `dataQuery` outputs are 016 carriers.** Asset values leave the platform
  only as `dataQuery` step outputs (≤ 256 KiB inline). That output is a classification carrier whose
  schema the platform derives (the 017 typed-cell result shape) and annotates with the highest level
  among the pinned input columns, because a query can project any column and no taint is tracked
  (ADR-0036 decision 6). The existing 016 payload disclosure then applies its audience and audit
  rules. The 017 result view and ADR-0035 decision 10 are untouched. Snapshot metadata (row counts,
  bytes, schema) is not a value disclosure.
- **D1266 — Columns are classified, secrets are refused.** The 016 vocabulary applies per column;
  undeclared means `pii`; `secret` is refused. The level is recorded for later surfaces (020, 026,
  027); no taint tracking is performed (ADR-0036 decision 6).
- **D1267 — Publication events go through the inbox as protected system records.** Source
  `sagat.data`, type `data.asset.published`, event id = `<asset id>:<pointer version>` (one per
  publication, so a rollback to an earlier snapshot is a new event and the inbox key still
  deduplicates redelivery); existing event triggers start runs; no new trigger kind. The document is
  closed and at most 1 KiB, so ADR-0027 lets it be `protected_system`: a row-level CHECK bounds rows
  of this source and the content-delta trigger skips them, and the publication does not spend
  `events_per_second` (that bucket admits events from the API, webhooks and connectors). A full
  quota therefore cannot roll back a completion. This is the platform's form of Windmill's "writing
  an asset triggers its readers".
- **D1268 — Freshness is found by looking.** Status is a function of policy, last publication or
  observation and the database clock; the 008 monitor raises alerts. No watchdog re-runs anything
  (Windmill Enterprise does); reacting is the operator's or a trigger's job.
- **D1269 — One owner per blob purpose.** 020 (ADR-0054) owns retention of run-purpose blobs and
  excludes `DATA_SNAPSHOT_PART` from run purge; ADR-0058 owns `DATA_SNAPSHOT_PART` retention. Only
  that purpose may go from `COMMITTED` to `DELETING`, under FR-024-034 and ADR-0023's tombstone/retry
  semantics. Because ADR-0027 does not authorize deleting customer data, the destructive step waits
  for O1243; until then retention only marks snapshots `RETIRED`.
- **D1270 — Polars runs on 023's image-catalog runtime `python-data`; 024 owns its content.** 023
  reserves the language id (FR-023-027, 023 D1229): bundle v4 with `runtimeCatalog`, a hash-pinned
  requirements lock matching catalog entries exactly, catalog digest in the toolchain identity and
  build key, and a runner that refuses an artifact whose `runtimeCatalog` differs from its runtime
  (`SCRIPT_SANDBOX_UNAVAILABLE`). That is ADR-0057's one recorded exception to "no catalog in the
  runtime image". 024 owns the runtime image (`runners/data/python-data/Dockerfile.runtime`), the
  catalog file (`catalog.schema.json`, upstream `pypi`, `language: python-data`): Polars and PyArrow
  at exact versions with their reviewed transitive closure, each with upstream checksum, an
  independent wheel SHA-256 per target, licence and image-build advisory evidence; the execution
  environment (the data mount profile, no network, only locked packages on the import path) and
  their tests. **Re-pin procedure** (023 G1219): a new catalog digest is a new image; runners keep the
  previous image configured for at least 90 days so existing versions keep running; authors are told
  which versions to republish; a version whose catalog image is withdrawn fails
  `SCRIPT_SANDBOX_UNAVAILABLE`, never runs against other packages. No runtime installation, no index
  access (O1242).
- **D1271 — New worker messages need a protocol minor.** `ResolveData` and the data completion
  fields are additive in the next minor after 1.6 (written 1.7 here; the coordinator allocates the
  number across the parallel drafts). Engine-specific connection messages for `ResolveDatabase` are
  added in the same minor; a 1.6 runner never receives a new task kind.
- **D1272 — Fake servers in CI, real services as a release gate.** BigQuery and Snowflake are
  exercised in CI by protocol-faithful fake HTTP servers written from the documented API; the
  real-service suite is `ignore`d until accounts exist, and the drivers stay hidden until it passes
  (the G70 pattern of ADR-0023; O1241).
- **D1273 — Lock order.** Run admission follows the merged admission lock order that ADR-0050
  (feature 018) holds for every draft (coordinator decision C4): authority → request receipt →
  run-creating aggregates → catalog heads and pins (slot 4: 019 fences → 017 connection heads → 021
  resource and variable heads → **024 asset heads**, ascending id → 025 → 030) → secrets and ACLs →
  run rows → 022 semaphores → usage. Transactions on an existing run keep "run row first": the run
  row, then the authority guards it needs (current membership, binding/session for managed
  attempts), then asset head rows in ascending id, then usage (ADR-0027). Asset heads are locked
  *before* inserting any row that references them: `FOR NO KEY UPDATE` by writers (pointer moves,
  declaration changes, retention), `FOR SHARE` by readers that must exclude a concurrent pause,
  tombstone or producer change (pins, non-publishing completion). Taking a foreign-key key-share
  first and upgrading later would let two completions deadlock. An API publication checks current
  membership *before* the asset head. No transaction takes a run or authority lock after an asset
  lock.
- **D1274 — Data enters through step outputs.** The data sandbox has no network, so ingestion is
  an earlier step of the same run: a database step of any remote driver (its 017 typed result,
  ≤ 4 MiB) or any activity, script or 021 S3 get operation (executed in `apps/integration-runner`,
  which alone resolves customer S3 credentials, ADR-0055) that commits a Parquet step-output blob
  (≤ 64 MiB, verified by the engine). Larger or direct ingestion from customer object storage is
  G1246. The provenance records the step, so lineage shows the external source.
- **D1275 — External assets have a producer too.** Without it any workflow declaring
  `assets.writes` could refresh freshness and resolve a stale alert. A declared write is accepted at
  publication only from the asset's producer node using the asset's connection.
- **D1276 — Audits are bound to the uploaded bytes.** The transform container's `/out` is a tmpfs
  discarded with the container, so its host program streams each part to the runner over standard
  output in bounded length-prefixed frames before it exits, and the runner uploads each part as it
  arrives. The runner then reads the attempt's own uploaded parts back with `ReadBlob` (verified
  digests) into a new staged directory for the audit container, and reports the audited digests; the
  engine refuses a completion whose audited digests differ from the committed parts. Reading back
  costs a second transfer of the snapshot; a runner-held copy or a host tmpfs shared by both
  containers was rejected because it would keep tenant output outside the container's memory
  accounting.
- **D1277 — `from` reads only audited snapshots.** A same-run staged input must be `AUDITED` or
  `PUBLISHED`; a quarantined one fails `DATA_INPUT_QUARANTINED`, so an error route cannot derive
  publishable data from data that failed its audit.
- **D1278 — Data bytes are charged once, at upload.** As ADR-0027 does for every external blob, a
  part is charged to `tenant_data_bytes` when its upload is reserved and measured; committing the
  snapshot does not charge again, and quota exhaustion therefore surfaces during upload.
- **D1279 — DuckDB is a data node, not a database driver (coordinator decision on review B2).** A
  `duckdb` driver would have put a managed database task in the sandbox runner (against ADR-0036
  decision 1) and needed a classification gate on the 017 result view (against ADR-0035 decision 10
  and ADR-0036 decision 6). A `dataQuery` node runs in the data sandbox like `dataTransform`, writes
  nothing, returns the 017 typed-cell shape as an inline, classified step output, and leaves the 017
  catalog, role rules and result view untouched.

## Gaps

| ID | Gap | Status |
|---|---|---|
| G1240 | A warehouse write whose attempt is lost (runner crash) cannot be looked up automatically; the reviewer sees the opaque job ID / requestId and resolves by hand. A lookup-only managed attempt would close it. | Open |
| G1241 | Unverified provider facts: BigQuery's answer to a second insert with an existing job ID; how long Snowflake keeps a `requestId`; the dry-run statement-type field for every statement form. Contract tests must establish them before the drivers are advertised. | Open |
| G1242 | BigQuery IAM cannot be introspected with a scoped key; least privilege (and cross-project reads) is the customer's control. | Accepted limitation |
| G1243 | SQL Server Entra ID service principals and Windows authentication. | Open |
| G1244 | Warehouse-native write-audit-publish (Snowflake `SWAP WITH`, BigQuery copy/clone) and verification of a warehouse table's schema against a contract. | Open |
| G1245 | Partitioned assets, incremental append, range backfill (Windmill offers these). | Open |
| G1246 | Customer-owned buckets as snapshot storage, and DuckDB reads of customer object storage. Under ADR-0055 a 021 S3 resource's credentials resolve only in `apps/integration-runner`, never in the sandbox or the engine, so this needs that runner to stage objects for the data sandbox or a new decision. Ingestion of one object per step through a 021 S3 get operation already works (D1274). | Open, depends on 021 |
| G1247 | Snapshot bytes pass through the engine process (ADR-0023); ceilings are set below what that path can carry and must be measured. Direct or presigned transfer needs a new ADR. | Open |
| G1248 | Asset declarations are not in 011 GitOps manifests; cross-namespace asset reads (030). | Open |
| G1249 | Row-level audit outcomes are computed in a sandbox; a sandbox escape (G98) could forge them. Footer-level checks are the engine's own. | Accepted limitation |
| G1250 | Erasure of individual records inside retained snapshots; only whole snapshot or asset deletion exists. | Open |
| G1251 | The DuckDB extension set is fixed by the image (Parquet, JSON); spatial, Iceberg, Delta and DuckLake are not offered. | Open |
| G1252 | After a regional failover (032) the pointer can be ahead of replicated objects; readers must verify part presence. | Open, depends on 032 |
| G1253 | Nested Parquet types (list, struct, map) are not in the contract vocabulary. | Open |
| G1254 | Snapshot metadata (snapshot, part, provenance and audit rows, publication history) is excluded from 020's run purge and owned by ADR-0058, because a published snapshot's pointer, pins and provenance reference it whatever happens to its producer run. Pruning metadata of `RETIRED` snapshots that nothing references is not designed yet; the rows are bounded in size and grow with completed transforms. Snapshot *bytes* are deleted by retention (after O1243). | Open |
| G1255 | Re-pinning the `python-data` catalog strands older script versions (023 G1219): the procedure of D1270 keeps the previous image for 90 days, but republication stays a manual author action. | Accepted cost, depends on 023 |
| G1256 | The Parquet footer decoder runs in the engine process (bounded, panic-isolated, fuzzed); if fuzzing finds an unbounded case, it moves to a separate unprivileged verifier process. | Open |

## Owner questions

| ID | Question | Options | Recommended default |
|---|---|---|---|
| O1240 | Where do managed snapshots live? | (a) platform blob backend under the namespace; (b) a customer bucket per namespace, which under ADR-0055 means the integration runner stages objects; (c) both from the start | (a), with `tenant_data_bytes`; (b) as G1246 |
| O1241 | How are BigQuery and Snowflake released without free emulators? | (a) hidden until the real-service suite passes on owner-provided accounts; (b) advertised as preview on fake-server evidence; (c) out of the feature | (a) |
| O1242 | Does the project maintain a curated Python data catalog for Polars (content specified here, mechanism 023's `python-data`)? | (a) yes, one hash-pinned catalog re-pinned on a stated cadence; (b) no, DuckDB only | (a); if (b), US07 is dropped |
| O1243 | May the platform destroy superseded snapshot data (ADR-0027 does not authorize deleting customer data; ADR-0023 never deletes committed blobs)? | (a) yes, under the retention the asset manager declares, audited; (b) only on an explicit, audited operator action; (c) never in this delivery (bytes kept, `RETIRED` is a label) | (a); until decided, (c) |
