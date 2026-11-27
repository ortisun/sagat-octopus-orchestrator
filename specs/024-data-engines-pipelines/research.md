# Feature 024 — Baseline, sources, decisions, gaps and owner questions

Investigation date: 2026-10-26, on the worktree branch `feature/018-032-parity-specs` cut from
`origin/develop` at `70f09435` (release 0.2.0). Nothing below is implemented by this feature.
Numbers used: D1240–D1279, G1240–G1256, O1240–O1243 (allocated range 1240–1279).

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decision 0058 accepted (delegate, Owner decision 2026-11-26-164); not started.

Revised 2026-11-26 against `develop` at `e5bda041` after an independent audit. The evidence rows marked
*(2026-11-26)* were read on that revision and replace the older rows they contradict. Register entries
added on that date use the feature-scoped form (D024-001 and G024-001 onward), because the bare numbers
after D1279 and G1256 belong to other features. Owner questions 1240 to 1243 are decided (Owner
decisions 2026-11-26-160 to 2026-11-26-163).

Revised again on 2026-11-27 after the correctness review (`reviews/2026-11-27-correctness.md`): the
evidence rows marked *(2026-11-27)* were read on that date, and every finding and what changed is
Decision 024-016.

## Repository evidence (verified paths)

| Observation | Where |
|---|---|
| *(2026-11-26)* `ConnectionConfig` has two variants, `Postgres` (with `egress_ceiling: EgressCeiling`) and the boxed `Iceberg` of feature 035; `driver()` answers `postgresql` or `iceberg`; `secret_slots()` names each secret with its `SecretRole` (`Primary`, `Catalog`, `Storage`); `ManagedKind::{Sql, Lake, LakeProbe}` with `activity()` and `profile()`; `MANAGED_DATABASE_ACTIVITY` is still `sagat.database.postgresql` | `crates/domain/src/database.rs` lines 37-38, 188-201, 245-266, 403-430, 445-479 |
| *(2026-11-26)* `BindingProfile::{Database, Lake}`; the column `worker_bindings.profile` with `CHECK (profile IN ('database', 'lake'))`; a dedicated runner registers only as its own profile, so a stolen lake token cannot ask for PostgreSQL credentials | `crates/domain/src/binding.rs` lines 39-61; migration `20270109000000_worker_binding_profile.up.sql` |
| *(2026-11-26)* Step credential pins are keyed by the secret's role (`primary`, `catalog`, `storage`) | migration `20270110000000_step_credential_roles.up.sql` |
| *(2026-11-26)* The lease of a lake attempt writes `step_lease_blobs` rows (tenant, run, node, attempt, generation, blob), and `ReadBlob` of that attempt is allowed only a blob named there; the rows are released with the terminal run's retention a day after it ended; the foreign key restricts deleting a pinned blob | migrations `20270111000000_lake_blob_pins.up.sql`, `20270112000000_lake_blob_pin_release.up.sql` |
| *(2026-11-26)* The lake runner (`apps/database-runner`, lake profile) stages an attempt directory, binds a Unix socket that admits only the slot's user, and asks a lake-mode sandbox runner for an attempt over a node-local socket (`start`, `wait`, `kill`; slot, class `small`/`large`, deadline; never an image, mount, command or limit); the wire types and `uid_for_slot` live in `crates/infra-lake/src/sandbox_api.rs`; the container profile in `crates/infra-sandbox/src/lake.rs`, the engine in `crates/infra-sandbox/src/lake/engine.rs`, the server in `apps/sandbox-runner/src/lake/` | `crates/infra-lake/src/{attempt.rs,staging.rs,relay.rs,supervisor.rs,sandbox_api.rs}`; `apps/database-runner/src/lake/`; Architecture decision 0079 (Proposed) |
| *(2026-11-26)* `scripts/check-architecture.py` has package rules, not a general allowlist: `ICEBERG_TEST_ONLY_PACKAGES` (lines 61-62) and `LAKE_RUNNER_PACKAGES` with `CONTAINER_ENGINE_CRATES` (lines 64-67, Architecture decision 0079 rule 2) | `scripts/check-architecture.py` |
| *(2026-11-26)* The canonical OpenAPI already has the union `DatabaseConnectionConfig` (`postgresql`, `iceberg`, `object_storage`, `clickhouse`, discriminated by `driver`), `EgressCeiling` (`public`, `internal`, `pii`, default `internal`) on `PostgresConnectionConfig`, and `DatabaseDriverList` (at most 4 items, `DatabaseDriver` with `id: [postgresql]` or `LakeDatabaseDriver`) | `specs/001-platform-foundation/contracts/openapi.yaml` lines 6341-6359, 6523-6535, 6615-6646 |
| *(2026-11-26; corrected 2026-11-27)* `ResolvedDatabase` uses fields 1-15: 13 `iceberg`, 14 `object_storage`, 15 `clickhouse` (feature 035); `WriteBlobHeader` uses 1-10; `CompleteStepRequest` and `FailStepRequest` use 1-13, 13 being feature 041's `StepLogCarry step_logs` (protocol 1.7). The 2026-11-26 row said 1-12, which was already false | `specs/001-platform-foundation/contracts/worker.proto` (`CompleteStepRequest`, `FailStepRequest`, `WriteBlobHeader`, `ResolvedDatabase`) |
| *(2026-11-27)* An uploaded blob that nothing refers to is kept `DEFAULT_UPLOAD_TTL` (24 hours) before the sweep claims it, an unfinished one `DEFAULT_ABANDONED_AFTER` (1 hour); ADR-0027 releases finalized garbage only after confirmed deletion | `crates/application/src/blobs.rs` (constants and `sweep`); ADR-0027, "Blob upload reserves …" |
| *(2026-11-27)* The lake pin writer `lake_blob_pin` inserts `step_lease_blobs` rows only for blobs of the same run, purpose `STEP_OUTPUT`, state `COMMITTED`; `check_blob_access` admits a managed read only for `ManagedKind::Lake`. The table's foreign key to `blobs` is by tenant, namespace and blob id, so a cross-run row is possible in shape | `crates/infra-postgres/src/execution/database.rs` (`lake_blob_pin`); `crates/application/src/worker.rs` (`check_blob_access`); migration `20270111000000_lake_blob_pins.up.sql` |
| *(2026-11-27)* An 018 reset copies the outputs its restored prefix names by payload digest, so a restored step output still refers to the source run's content; `step.completed` is `Copy` | `crates/infra-postgres/src/execution/resets.rs`; `crates/domain/src/execution/reset.rs` |
| *(2026-11-27)* Feature 020's run purge is not built: ADR-0054 is accepted and "its implementation waits for the principle-I amendment" | `docs/adr/ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md` (status line) |
| *(2026-11-27)* A lease that never reached a handler can be withdrawn without spending its attempt (`withdraw_lease`, at most 4 per attempt), but only before the first heartbeat; a data attempt heartbeats while it stages, so the data path bounds its uncounted attempts in its own retry rule instead | `crates/domain/src/execution/command.rs` (`withdraw_lease`, `MAX_WITHDRAWALS_PER_ATTEMPT`) |
| *(2026-11-26)* The register of worker-protocol minors: 1.7 is feature 041's (step logs); each later draft takes the next free minor when its contract merges | `sdks/SPEC.md` section 9; Owner decision 2026-11-15-8 |
| *(2026-11-26)* The IR has 19 node kinds, among them `database` (with optional `destinations`, feature 035) and `icebergWrite`, whose `records.blobs` names 1 to 32 blobs of `application/x-ndjson` or `application/vnd.apache.parquet` committed by earlier steps of the same run | `specs/001-platform-foundation/contracts/workflow.schema.json` (`$defs.node`, `database`, `icebergWrite`, `lakeRecords`, `stepDestination`) |
| *(2026-11-26)* `Permission::ALL` has 55 entries; seven built-in roles; `runner` holds `worker:execute`, `script:build`, `script:execute`; `database-runner` holds `worker:execute`, `database:execute` | `crates/domain/src/identity.rs` lines 401, 541, 611-612, 702-703 |
| *(2026-11-26)* The instance registry's closed capability lists are three CHECKs (`serving_instances_capabilities_closed`, `platform_requirements_capability_valid`, `platform_requirement_history_capability_valid`), extended by later migrations; `Capability::ALL` has four entries | migrations `20270113000000_instance_registry.up.sql`, `20270118000000_run_input_validation.up.sql`; `crates/application/src/instances.rs` line 121; Architecture decision 0082 |
| *(2026-11-26)* The reset-rule registry is an exhaustive `match` over `RunEvent` with the rules `Copy`, `Rematerialize`, `Exclude` and `Refuse` | `crates/domain/src/execution/reset.rs` lines 25-50 |
| *(2026-11-26)* The latest migration is `20270210000100_step_logs_validate` | `crates/infra-postgres/migrations/` |
| *(2026-11-26)* The credential codec port is `DatabaseCredentialsCodec` (`decode`, `contains_credential`) with a `{username, password}` shape; its adapter is `JsonDatabaseCredentials` | `crates/application/src/database.rs` lines 260-280; `crates/infra-secrets/src/database.rs` |
| *(2026-11-26)* The SDK error catalog has 44 codes, among them the `worker`-origin Iceberg and destination codes of feature 035 and `OUTCOME_UNKNOWN`; `DatabaseFailure` adds feature 035's codes to feature 017's | `sdks/error-catalog.json`; `crates/domain/src/database/failure.rs` lines 42-75 |
| *(2026-11-26)* Database runner metrics are `database_runner_execution_total{outcome}`, `database_runner_attempts_total{outcome}` and `database_runner_attempt_seconds`; the lake sandbox's are `lake_sandbox_*` | `apps/database-runner/src/attempt.rs` line 79, `runner.rs` lines 209-210 and 487-489 |
| *(2026-11-26)* Every path that creates a run goes through one run-input gate: an event trigger is row 9, an ad hoc database execution and a connection probe are row 13 | `docs/designs/043-run-input-validation.md` (rows 1 to 14); `crates/infra-schema/src/run_input/`; Architecture decisions 0090 and 0092 |
| *(2026-11-26)* The statement budget of a lease and a completion is enforced by `integration_claim_transition_efficiency` | `docs/designs/038-claim-transition-efficiency.md`; `tests/integration/claim-transition-efficiency/work_budget.rs` |
| *(2026-11-26)* Steps the platform runs keep their output where their path keeps it and are refused step logs; the ingest refuses the runner roles and managed steps (check C0) | `docs/designs/041-step-logs.md` (Requirement 041-016, section 2a) |
| *(2026-11-26)* Design 046 (approved on `origin/docs/windmill-parity-readiness`, Architecture decisions 0120 and 0121 accepted there): a Python script with packages is bundle version 5 with its own PEP 751 lock; the runner materializes a signed read-only layer before it offers the activity and mounts it at `/opt/sagat/deps` (not `noexec`); a block stops the lease with `PACKAGE_BLOCKED` | `docs/designs/046-script-dependencies.md` sections 8, 11 and 12 |
| *(2026-11-26)* ADR-0050 slot 4 is split: 4a namespace fences (the feature 024 data-catalog fence for writers), 4b catalog heads (feature 024 asset heads after feature 021's variable heads) | `docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md` line 91 |
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
| `scripts/check-architecture.py` checks the domain allowlist, layer edges and cycles; it has no per-crate client allowlist (ADR-0023's "only infra-blobstore may depend on a storage client" is therefore review-only today). *Superseded 2026-11-26: it now has package rules for the Iceberg crate and the lake runner (row above).* | `scripts/check-architecture.py` |
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
  connections`). *(revised 2026-11-26)* The catalog already holds feature 035's lake kinds, so the new
  drivers join the existing `ConnectionConfig` beside `Postgres` and `Iceberg` and the existing OpenAPI
  union `DatabaseConnectionConfig`; each new variant carries feature 035's `egressCeiling` (Decision
  024-005).
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
  *Superseded in part 2026-11-26 by Decision 024-001:* the claimant is the data runner, which asks a
  data-mode sandbox runner for an attempt (Architecture decision 0079); `/in` is the read-only attempt
  directory, the output channel is the attempt socket, and ADR-0058 now refines ADR-0013 as the lake
  profile did.
- **D1243 — Warehouses use a public-endpoint profile in their own runner deployment.** CIDR
  allowlists cannot describe Google or Snowflake public endpoints, and a NetworkPolicy cannot list
  host names, so mixing public HTTPS egress with private customer-database routes in one pod is what
  ADR-0055 option 4 rejects. BigQuery and Snowflake run in a separate deployment of
  `apps/database-runner` (warehouse profile): its NetworkPolicy allows DNS, the engine and TCP 443 to
  non-private ranges (or listed private-endpoint networks for PrivateLink/PSC) and nothing private
  otherwise; the process additionally enforces the driver-fixed host form and the deployment host
  allowlist, which namespace policy can only narrow, and refuses reserved, metadata and platform
  addresses. The private profile keeps the 017 policy and has no public egress. Same binary, same
  `database-runner` role, so ADR-0036 decision 1 is unchanged. *(revised 2026-11-26)* The warehouse
  deployment is bound to a third `BindingProfile`, `Warehouse` (Architecture decision 0071 decision 1),
  so the server hands it only BigQuery and Snowflake tasks and never a private database's credential
  (Decision 024-002).
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
  and manual review shows the opaque identity (automation is G1240). *(revised 2026-11-26)* This is
  each driver's recovery rule in Architecture decision 0071 decision 7's vocabulary (Decision 024-006).
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
  ceiling (64 MiB); Parquet is the only format of the first delivery (O1240, decided by Owner decision
  2026-11-26-160: the platform blob backend under the namespace, counted in `tenant_data_bytes`).
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
- **D1263 — No eighth role.** *Superseded 2026-11-26 by Decision 024-001: a machine role `data-runner`
  holds `data:execute`; the `runner` role is unchanged.* The `runner` role gains `data:execute`; a sandbox runner advertises
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
  *(revised 2026-11-26)* `Permission::ALL` holds 55 on `develop`; this feature adds four permissions
  and one machine role, `data-runner` (Decision 024-001); tests still assert sets, never counts.
- **D1265 — No new endpoint; `dataQuery` outputs are 016 carriers.** Asset values leave the platform
  only as `dataQuery` step outputs (≤ 256 KiB inline). That output is a classification carrier whose
  schema the platform derives (the 017 typed-cell result shape) and annotates with the highest level
  among the pinned input columns, because a query can project any column and no taint is tracked
  (ADR-0036 decision 6). The existing 016 payload disclosure then applies its audience and audit
  rules. The 017 result view and ADR-0035 decision 10 are untouched. Snapshot metadata (row counts,
  bytes, schema) is not a value disclosure. *(revised 2026-11-27, Decision 024-016)* A step source has
  no declared columns and counts as `pii` unless its source output is a 016 carrier declaring a level
  for every field. *(revised again 2026-11-27, Decision 024-017)* A `dataQuery` may not read a 017
  database result at all; publication refuses such a step source.
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
  for O1243; until then retention only marks snapshots `RETIRED`. *(revised 2026-11-26)* Owner decision
  2026-11-26-163 answers O1243 (a): retention deletes the bytes under the declared retention, audited.
- **D1270 — Polars runs on 023's image-catalog runtime `python-data`; 024 owns its content.**
  *Superseded 2026-11-26 by Decision 024-008 (Owner decision 2026-11-26-162): no image catalog; the
  packages come from the script version's own lock and design 046's layer.* 023
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
- **D1271 — New worker messages need a protocol minor.** *Revised 2026-11-26 by Decision 024-014:
  1.7 is feature 041's; this feature takes the next free minor when its contract merges.* `ResolveData` and the data completion
  fields are additive in the next minor after 1.6 (written 1.7 here; the coordinator allocates the
  number across the parallel drafts). Engine-specific connection messages for `ResolveDatabase` are
  added in the same minor; a 1.6 runner never receives a new task kind.
- **D1272 — Fake servers in CI, real services as a release gate.** BigQuery and Snowflake are
  exercised in CI by protocol-faithful fake HTTP servers written from the documented API; the
  real-service suite is `ignore`d until accounts exist, and the drivers stay hidden until it passes
  (the G70 pattern of ADR-0023; O1241, decided by Owner decision 2026-11-26-161).
- **D1273 — Lock order.** Run admission follows the merged admission lock order that ADR-0050
  (feature 018) holds for every draft (coordinator decision C4): authority → request receipt →
  run-creating aggregates → catalog heads and pins (slot 4b: 019 fences → 017 connection heads → 021
  resource and variable heads → **024 asset heads**, ascending id → 025 → 030; a writer of the asset
  catalog also takes the 024 data-catalog fence in slot 4a) → secrets and ACLs →
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
  accounting. *(revised 2026-11-26)* The frames travel over the attempt's peer-checked Unix socket,
  not standard output, and the audit is a second attempt of the data mode (Decision 024-001).
  *(revised 2026-11-27, Decision 024-016)* The binding is the data runner's read-back verification;
  the digest comparison is a self-check of the data runner, and audit outcomes rest on the sandbox
  (Gap 1249).
- **D1277 — `from` reads only audited snapshots.** A same-run staged input must be `AUDITED` or
  `PUBLISHED`; a quarantined one fails `DATA_INPUT_QUARANTINED`, so an error route cannot derive
  publishable data from data that failed its audit.
- **D1278 — Data bytes are charged once, at upload.** As ADR-0027 does for every external blob, a
  part is charged to `tenant_data_bytes` when its upload is reserved and measured; committing the
  snapshot does not charge again, and quota exhaustion therefore surfaces during upload.
- **D1279 — DuckDB is a data node, not a database driver (coordinator decision on Blocker 2 of the
  coordinator's architecture review of 2026-10-27).** A
  `duckdb` driver would have put a managed database task in the sandbox runner (against ADR-0036
  decision 1) and needed a classification gate on the 017 result view (against ADR-0035 decision 10
  and ADR-0036 decision 6). A `dataQuery` node runs in the data sandbox like `dataTransform`, writes
  nothing, returns the 017 typed-cell shape as an inline, classified step output, and leaves the 017
  catalog, role rules and result view untouched.

### Decisions added on 2026-11-26 (audit against develop)

| ID | Decision | Why |
|---|---|---|
| D024-001 | **The data runner asks a data-mode sandbox runner for an attempt.** A new application `apps/data-runner` (package `sagat-data-runner`, machine role `data-runner` with `worker:execute` and `data:execute`, bound with `BindingProfile::Data`) claims `sagat.data.transform` and `sagat.data.query`, resolves them with `ResolveData`, stages the pinned inputs into an attempt directory (the lake runner's `AttemptDir` and whole-blob reader), binds the attempt's Unix socket with the peer check of the lake relay (`uid_for_slot`), and asks a sandbox runner in the exclusive **data mode** for an attempt through Architecture decision 0079's three calls (attempt id, slot, class `small`/`large`, deadline). The data mode builds the container from its own configuration (image by digest, class budget, the mounts of Architecture decision 0058 decision 1) and has its own slot and user range. The request, the framed parts and the result travel over the socket; the request file and the socket live in the attempt directory, mounted read-only. An author's `limits` choose the smallest class that covers them and lower what the host program and the data runner enforce in software (DuckDB `memory_limit`, staged and output bytes); admission refuses limits no configured class covers. The data runner links neither `bollard` nor `sagat-infra-sandbox` (checked). Replaces the sandbox-runner claim of D1242 and the `runner` grant of D1263. | Owner decision 2026-11-26-164 requires the attempt API. A process that parses untrusted frames and stages other tenants' inputs must not hold the host-root engine socket (Architecture decision 0079); a separate binary makes that a dependency fact, not a configuration. A role of its own keeps a stolen data token from claiming script steps and their declared secrets, which the `runner` role could. |
| D024-002 | **Managed kinds and profiles.** `ManagedKind::Sql` gains the driver (`ManagedKind::Sql(SqlDriver)`, `activity()` is `sagat.database.<driver>`, `profile()` is `Database` for `postgresql`, `mysql`, `mariadb`, `sqlserver` and the new `Warehouse` for `bigquery`, `snowflake`); `ManagedKind::DataTransform` and `ManagedKind::DataQuery` (activities `sagat.data.transform`, `sagat.data.query`, profile `Data`). `BindingProfile` gains `Warehouse` and `Data`; one migration widens `worker_bindings_profile_valid`. `crates/application/tests/managed_kind_sites.rs` is extended so a worker path asks `Node::managed_kind`, never a driver name or a `sagat.data.` prefix. A runner is handed a step only when it offers that kind's activity (D035-150). | One predicate for "which runner runs this step" already exists; a second one per feature is how a data step would reach an ordinary worker. |
| D024-003 | **Instance-registry capabilities.** `database-engines-v1` (MySQL, MariaDB, SQL Server), `database-warehouses-v1` (BigQuery, Snowflake) and `data-pipelines-v1` (assets, data nodes, retention), each for the kinds `CONTROL_PLANE` and `EXECUTION_ENGINE`, are added to the three closed CHECKs of the registry and to `Capability::ALL` by one migration (Architecture decisions 0082 and 0050 decision 9). A process lists a capability only when its build serves it; publication, admission and catalog writes that need an unrecorded one are refused (`DRIVER_NOT_ENABLED`, `DATA_PIPELINES_NOT_ENABLED`). `database-warehouses-v1` is recorded only after the real-service suite passes (Owner decision 2026-11-26-161). Within a recorded capability, a deployment still lists which drivers its runners serve. | A per-process flag lets one replica accept a connection row another cannot decode; the registry makes the requirement fleet-wide and refuses a process that lacks it, which is also the binary-rollback guard of Requirement 024-038. |
| D024-004 | **Principle V's two sets, no amendment.** The five remote drivers are feature 017 managed database capabilities: their credential is disclosed only to the lease-owning `database-runner` with `database:execute` (Principle V's second set), through the existing two-phase `ResolveDatabase` and the role-keyed credential pin (`secret_role = 'primary'`). The data runner and its containers receive no secret and no credential: they read only pinned blobs through lease-authorized reads, and `ResolveData` carries no secret. | Both cases fit the constitution as written; a third set is not needed. |
| D024-005 | **The `pii`-egress rule.** Every new driver connection carries feature 035's `egressCeiling` (default `internal`); a written field's level is the stricter of its source and any annotation (Architecture decision 0071 decision 8). Under the uniform rule of Owner question 1081 (decided 2026-11-24 for feature 020, Decision 1617): `pii` is set only by an administrator holding the namespace reveal grant (`ConnectionConfig::authorize_egress_ceiling`), rechecked at each `ResolveDatabase` of a step that writes a `pii` value, revocation pauses the connection, each such attempt is audited, `secret` never leaves. The publication event and its webhook deliveries carry asset metadata only (names, ids, counts), never a column value. | One control for every place `pii` leaves the platform. |
| D024-006 | **Recovery rules in feature 035's vocabulary.** Each new driver declares the recovery rule of Architecture decision 0071 decision 7: `bigquery` lookup by deterministic job id, `snowflake` lookup by deterministic `requestId`, both within the lease (D1248); `mysql`, `mariadb`, `sqlserver` none, so an ambiguous write ends `OUTCOME_UNKNOWN` and waits for a person. The new drivers are not destination kinds of feature 035 in this delivery (Gap 024-004). | One vocabulary for "what can the platform prove after a lost response". |
| D024-007 | **Pinned reads reuse `step_lease_blobs`.** The lease transaction of every data attempt writes one `step_lease_blobs` row per pinned part (snapshot parts of any producing run in the namespace, or a step-output blob of the same run); the existing `ReadBlob` gate of feature 035 admits exactly those; the audit phase's read-back of the attempt's own uploads is admitted by the upload fence (same attempt and generation). The rows are released with the run's retention a day after it ends, which matches the 24-hour pin grace of retention; until then the foreign key keeps the blob row. A data attempt pins at most 32 inputs of at most 32 parts. *(revised 2026-11-27, Decision 024-016)* The table and the gate are reused, but the writer and the gate branch are new code: feature 035's `lake_blob_pin` admits only committed `STEP_OUTPUT` blobs of the same run and `check_blob_access` only `ManagedKind::Lake`, so a data pin writer (cross-run `DATA_SNAPSHOT_PART` parts, never `DELETING`; a reset run's source-run step output through the reset link) and a `DataTransform`/`DataQuery` branch are built and tested by Task 024-031. | One allow-list for every managed attempt that reads blobs; the table and its gate are tested, the data writer and branch are new and get their own tests. |
| D024-008 | **Polars through design 046's package layer (Owner decision 2026-11-26-162, answering O1242).** There is no platform-curated Python catalog image. A Polars transform references a published script version of language `python-data` (reserved by feature 023), whose artifact is design 046's bundle version 5 (the Python text bundle plus `dependencies`, ecosystem `pypi`) with the version's own PEP 751 lock, resolved through the package gateway, checked by the namespace's supply-chain policy and blocked at lease with `PACKAGE_BLOCKED` like any script. The `python-data` runtime (`runners/data/Dockerfile.runtime`, Python line of design 046's dependency runtime) carries the interpreter and the data contract host only; Polars, PyArrow and any library come from the version's signed read-only layer at `/opt/sagat/deps`. The repository ships a reviewed starter lock for Polars and PyArrow (`runners/data/python-data/starter/`) as a template. A `python-data` version runs only as a Polars transform, never as an ordinary script step. Feature 023's image-catalog parts of Requirement 023-027 are not needed. | Users pin their versions under the namespace's policy; the platform keeps no catalog cadence and strands no version at a re-pin. |
| D024-009 | **Overlap with feature 035's Iceberg writes.** A managed snapshot's authority stays the PostgreSQL pointer (Architecture decision 0058 option 5). An Iceberg table written by an `icebergWrite` step is an external asset: the external declaration may name an Iceberg connection and a table locator, and the closed `icebergWrite` node gains the same optional `assets` property as the `database` node, so a successful declared write records an observation. Exporting a snapshot to an Iceberg table is an `icebergWrite` step whose `records.blobs` names the snapshot's Parquet parts (Gap 024-004, not built here). Data nodes declare no `destinations` in this delivery. Warehouse-native write-audit-publish on an Iceberg branch (`ref`) is recorded under Gap 1244. | No second lake writer and no second catalog. |
| D024-010 | **Graph model only.** Data nodes and the `assets` property exist only in graph-model versions. A procedural workflow (Principle IV, design 044, not yet written) reaches them only through a child graph workflow its envelope declares (Gap 024-003). Producer bindings name a graph node. | The procedural envelope rules are not written; a producer binding needs a node id. |
| D024-011 | **No new run creator.** A run started by an event trigger on `data.asset.published` passes design 043's gate at row 9; a connection probe and an ad hoc execution of a new driver pass at row 13. A standalone `dataQuery` (feature 022) is a creator row that feature 022 adds to design 043's table. | Architecture decision 0090: every path that creates a run passes one gate. |
| D024-012 | **Claim and completion budgets.** The lease and completion of ordinary steps keep their recorded statement budget unchanged; the first lease of a data node (pins, `step_lease_blobs` rows, journal) and the data completion get budget shapes of their own in `integration_claim_transition_efficiency`, recorded at the first measurement and moved only with a reason (design 038). | The hot path's cost is a tested number, not a hope. |
| D024-013 | **Step logs.** Data steps are managed steps run by the platform: they are refused step logs (Requirement 041-016, check C0). The host program's bounded, redacted log reaches the data runner over the attempt socket and is kept as a script step's log is (Requirement 005-007, its quota), never as step logs. | A tenant line on a platform runner's report would bypass the managed paths' redaction. |
| D024-014 | **Protocol minor and field numbers.** The worker-protocol additions take the next free minor of the register in `sdks/SPEC.md` section 9 when their contract merges (1.7 is feature 041's, Owner decision 2026-11-15-8); the merge adds the register row. `ResolvedDatabase` gains its new fields from number 16 (13 to 15 are feature 035's); `WriteBlobHeader` gains `purpose = 11`; `CompleteStepRequest` gains `data_result` (written 13 on 2026-11-26, a number feature 041's `step_logs` already held; 14 since 2026-11-27, Decision 024-016; 15 since design 044's contract took 14 the same day, Decision 044-256 of `specs/044-workflow-as-code/research.md`); the numbers are confirmed against the canonical file at merge by a contract test (Task 024-008). | Two drafts naming one minor, or one field number, is a wire break. |
| D024-015 | **Error catalog.** No new REST `Error.code`. `DATABASE_COST_LIMIT` and the data failure codes become `worker`-origin entries of `sdks/error-catalog.json`, of `sdks/SPEC.md` section 2 and of every SDK's embedded copy (the Iceberg codes of feature 035 are the precedent); new REST meanings are `details.reason` values with remediation rows in `crates/api-http/src/error.rs`. | An SDK must be able to name every code a step can end with. |

### Decisions added on 2026-11-27 (correctness review)

| ID | Decision | Why |
|---|---|---|
| D024-016 | **Revision after the correctness review of 2026-11-27** (`reviews/2026-11-27-correctness.md`: 1 Blocker, 12 Majors, 12 Nits; every claim about code was checked against the code it cites, and none was judged wrong). **Blocker 1** (a `dataQuery` over a step source had no level): a step source counts as `pii`, as an undeclared column does, unless the source output is a 016 carrier declaring a level for every field; a 017 database result always counts as `pii`; the first-lease pin records each input's `level` (FR-024-016, 029, the 016 amendment, `step_data_inputs.level`, SC-024-009, Task 024-041, ADR-0058 decision 9). **Major 2** (field 13 collides with feature 041): the evidence row corrected, `CompleteStepRequest.data_result = 14`, a contract test against the canonical proto added to Task 024-008. **Major 3** (full quota rolled back API and run publications): the `dataTransform` and `dataPublish` output documents are closed protected records of at most 4 KiB (`data_protected_outputs`, measured as zero by the `payloads` content-delta trigger) and the justification is `protected_system`, at most 1,024 bytes without control characters; Tasks 024-034 and 024-046 test an auto-publishing completion, a `dataPublish` and an API publication at full quota; a `dataQuery` output stays charged. **Major 4** (uncommitted parts held for 24 hours): the transaction that fails, retries under a new generation, cancels or refuses an attempt claims its uploads for deletion, and a late finalization of an ended fence is claimed at once (FR-024-035, new Task 024-078). **Major 5** (the digest check could not fail as tested): restated as a data-runner self-check; the binding is the read-back verification and the outcomes rest on the sandbox (Gap 1249); the test is a data-runner fault injection (FR-024-024, data runtime contract, the proto field renamed `staged_part_sha256`, Task 024-035, ADR-0058 decision 4). **Major 6** (no Parquet mapping): a normative encoding table, `OPTIONAL` accepted for `nullable: false` with nulls caught by the generated not-null audit, both decimal encodings accepted, User story 024-07 parity defined as equal multisets of rows and contract fingerprint (Tasks 024-033, 024-053). **Major 7** (`ABANDONED` hit a live run's snapshot): a live-run pin never moves to `ABANDONED`; a `dataPublish` on `ABANDONED` is stale under `onStale`, a `from` on it fails `DATA_CONFIGURATION_INVALID`; both races in Task 024-049. **Major 8** (reset rematerialization): a reset row in the transactions table (asset heads `FOR SHARE` in slot 4b, state read under the lock); a restored step-source pin admitted through the reset link and the restored id and digest; `from` and `dataPublish` resolved through `run_data_snapshot_refs`; three cases and a barrier race in Task 024-068; 020 asked to keep such a source output (Task 024-002). **Major 9** (external observation write): a "declared external write completion" transaction (external asset heads `FOR NO KEY UPDATE`, recheck, observation, head, journal; a failed recheck records no observation but completes the step), its budget shape in Task 024-074 and a race in Task 024-054. **Major 10** (two outputs on one asset): unique output assets and input aliases per node, `DATA_NODE_SCOPE` at publication, fixtures in Task 024-030. **Major 11** (purge evidence impossible in slice 0): Task 024-010 proves only the purpose CHECK and the guarded trigger; the purge proof is new Task 024-077, gated on feature 020's purge (owner row OD-2026-11-27-240 requested). **Major 12** (freshness from publication time): lag is measured from the current snapshot's `created_at`; rollback cases in Task 024-044. **Major 13** (audit class and infrastructure failures): the audit runs in the transform's class; refusal, unavailability, memory kill, timeout, crash or a generated check's `ERROR` commit nothing and fail `DATA_RUNTIME_UNAVAILABLE` retryably; only `FAILED` or a custom check's `ERROR` quarantines (`DATA_AUDIT_FAILED`, now not retryable). **Nits**: 14 a rollback to a retired snapshot is `CONFLICT SNAPSHOT_NOT_PUBLISHABLE` (User story 024-06, OpenAPI, Task 024-048); 15 the resolve condition of `DATA_SNAPSHOT_QUARANTINED` stated (FR-024-031, quickstart, Task 024-044); 16 Decision 024-007 says the pin writer and gate branch are new code (Task 024-031); 17 a stale auto-publication that no later node reads is created `ABANDONED`, and the unpublished window counts from creation; 18 an object-store outage during footer reads is a transient `UNAVAILABLE` with no state change; 19 the tombstone clears the pointer with a `TOMBSTONE` publication record and the snapshot trigger refuses retiring a pointed-at snapshot; 20 the duplicate tested is a dispatcher crash between run creation and the processed mark and a redelivered webhook row (Tasks 024-046, 024-061); 21 `DATA_RUNTIME_UNAVAILABLE` before any container started does not count against `maxAttempts` (bounded at 10 per step), and a restarted data runner ends the attempts it finds; 22 `RUN_NODE` checks the binding against the `from` node, and `from`/`dataPublish` resolve the successful completion's snapshot; 23 the event payload schema is closed, `timeoutMs` (default 15 minutes) covers the whole attempt and publication refuses one that leaves the transform less than a minute. | A review that finds a disclosure below protection, a wire collision and guarantees no test can fail is fixed in the set before any code; the safer option was taken at each choice (a step source counts as `pii`; parts released in the deciding transaction; a live-run pin is never abandoned). |
| D024-017 | **The stricter option for the two residual risks of Decision 024-016** (coordinating session, 2026-11-27). (1) A `dataQuery` MUST NOT name a feature 017 database step as a step source: publication refuses it (`VALIDATION_FAILED`, reason `DATA_NODE_SCOPE`), so a database result never becomes readable under the 016 `pii` reveal grant instead of `database:result:read`; the `pii`-by-default rule stays for other step sources, and a `dataTransform` may still read a database result into an asset whose manager declares the column levels (FR-024-016, FR-024-029, User story 024-04 scenario 2, SC-024-009, Tasks 024-040 and 024-041, the schema fragment, ADR-0058 decision 9). (2) The API publication's justification stays in the charged `content` class, bounded at 1,024 bytes, as ADR-0027 requires of tenant text; an API publication or rollback therefore obeys the content quota and is refused `QUOTA_EXCEEDED` at a full quota with nothing written, while run publications, which carry no free text, stay uncharged, and a `TOMBSTONE` record carries no justification so a deletion still works at a full quota (FR-024-026, FR-024-027, SC-024-010, data model, OpenAPI fragment, ADR-0058 decision 10, Tasks 024-034, 024-046 and 024-048). Supersedes the parts of Decision 024-016 that said a database result counts as `pii` in a `dataQuery` and that the justification is `protected_system`. The ADR-0058 correction is to be recorded by the delegate row OD-2026-11-27-242. | ADR-0035 decision 10 keeps one audience for database results, and ADR-0027 does not let free text become protected by naming it; both hold without exception this way. |

## Gaps

| ID | Gap | Status |
|---|---|---|
| G1240 | A warehouse write whose attempt is lost (runner crash) cannot be looked up automatically; the reviewer sees the opaque job ID / requestId and resolves by hand. A lookup-only managed attempt would close it. | Open |
| G1241 | Unverified provider facts: BigQuery's answer to a second insert with an existing job ID; how long Snowflake keeps a `requestId`; the dry-run statement-type field for every statement form. Contract tests must establish them before the drivers are advertised. | Open |
| G1242 | BigQuery IAM cannot be introspected with a scoped key; least privilege (and cross-project reads) is the customer's control. | Accepted limitation |
| G1243 | SQL Server Entra ID service principals and Windows authentication. | Open |
| G1244 | Warehouse-native write-audit-publish (Snowflake `SWAP WITH`, BigQuery copy/clone, or an Iceberg branch written by feature 035's `icebergWrite` with `ref` and fast-forwarded after audit) and verification of a warehouse table's schema against a contract. | Open |
| G1245 | Partitioned assets, incremental append, range backfill (Windmill offers these). | Open |
| G1246 | Customer-owned buckets as snapshot storage, and DuckDB reads of customer object storage. Under ADR-0055 a 021 S3 resource's credentials resolve only in `apps/integration-runner`, never in the sandbox or the engine, so this needs that runner to stage objects for the data sandbox or a new decision. Ingestion of one object per step through a 021 S3 get operation already works (D1274). | Open, depends on 021 |
| G1247 | Snapshot bytes pass through the engine process (ADR-0023); ceilings are set below what that path can carry and must be measured. Direct or presigned transfer needs a new ADR. | Open |
| G1248 | Asset declarations are not in 011 GitOps manifests; cross-namespace asset reads (030). | Open |
| G1249 | Row-level audit outcomes are computed in a sandbox; a sandbox escape (G98) could forge them. Footer-level checks are the engine's own. | Accepted limitation |
| G1250 | Erasure of individual records inside retained snapshots; only whole snapshot or asset deletion exists. | Open |
| G1251 | The DuckDB extension set is fixed by the image (Parquet, JSON); spatial, Iceberg, Delta and DuckLake are not offered. Iceberg tables are reached through feature 035, not through DuckDB (Decision 024-009). | Open |
| G1252 | After a regional failover (032) the pointer can be ahead of replicated objects; readers must verify part presence. | Open, depends on 032 |
| G1253 | Nested Parquet types (list, struct, map) are not in the contract vocabulary. | Open |
| G1254 | Snapshot metadata (snapshot, part, provenance and audit rows, publication history) is excluded from 020's run purge and owned by ADR-0058, because a published snapshot's pointer, pins and provenance reference it whatever happens to its producer run. Pruning metadata of `RETIRED` snapshots that nothing references is not designed yet; the rows are bounded in size and grow with completed transforms. Snapshot *bytes* are deleted by retention (Owner decision 2026-11-26-163). | Open |
| G1255 | Re-pinning the `python-data` catalog strands older script versions (023 G1219): the procedure of D1270 keeps the previous image for 90 days, but republication stays a manual author action. | Closed 2026-11-26: there is no catalog (Decision 024-008); each version keeps its own lock and layer |
| G1256 | The Parquet footer decoder runs in the engine process (bounded, panic-isolated, fuzzed); if fuzzing finds an unbounded case, it moves to a separate unprivileged verifier process. | Open |
| G024-001 | Architecture decision 0079 is Proposed, and its Kubernetes half (Task 035-029: whether a node-local socket and work volume can be shared between two processes, and whether `SO_PEERCRED` separates them) is open. The data mode and the data runner inherit both; a change of that decision changes Architecture decision 0058 decision 1 with it. | Open, depends on 035; blocks the data deployment on Kubernetes, not on Compose |
| G024-002 | Design 046 materializes a layer in the runner that holds a control-plane token and the engine; under Architecture decision 0079's split the data runner holds the token and the data-mode sandbox runner holds the engine. Which process fetches the locked files, which builds and verifies the volume, and how the data mode learns that a layer digest is signed, is designed in Task 024-076 before Polars is built (the data-mode request member `layer` of Architecture decision 0058 decision 1 assumes it). | Open; blocks User story 024-07 only |
| G024-003 | The procedural model (Principle IV, design 044) is not yet written; whether its envelope gains a data entry is undecided. Data nodes are graph-only (Decision 024-010). | Open, depends on design 044 |
| G024-004 | Exporting a managed snapshot to a customer Iceberg table (an `icebergWrite` step reading the snapshot's parts, which needs `step_lease_blobs` to admit `DATA_SNAPSHOT_PART` parts for a lake attempt), and the five new drivers as feature 035 destination kinds. | Open, not in this delivery |
| G024-005 | Whether PostgreSQL and the lake kinds of features 017 and 035 already meet the recheck-at-resolution and pause-on-revocation clauses of the uniform `pii`-egress rule is recorded at build start (Task 024-002); a difference goes to their owners and is not changed by this feature. | Open |

## Owner questions

| ID | Question | Options | Recommended default | Decision |
|---|---|---|---|---|
| O1240 | Where do managed snapshots live? | (a) platform blob backend under the namespace; (b) a customer bucket per namespace, which under ADR-0055 means the integration runner stages objects; (c) both from the start | (a), with `tenant_data_bytes`; (b) as G1246 | **(a)**, delegate, Owner decision 2026-11-26-160; a customer bucket stays Gap 1246 |
| O1241 | How are BigQuery and Snowflake released without free emulators? | (a) hidden until the real-service suite passes on owner-provided accounts; (b) advertised as preview on fake-server evidence; (c) out of the feature | (a) | **(a)**, delegate, Owner decision 2026-11-26-161: built and tested against the fake protocol servers, not offered (`DRIVER_NOT_ENABLED`, `database-warehouses-v1` unrecorded) until the real-service suite passes on accounts the owner provides; agents create no account and spend no money (Owner decision 2026-11-25-1) |
| O1242 | Does the project maintain a curated Python data catalog for Polars (content specified here, mechanism 023's `python-data`)? | (a) yes, one hash-pinned catalog re-pinned on a stated cadence; (b) no, DuckDB only | (a); if (b), US07 is dropped | **Neither: decided through design 046's package layer**, delegate, Owner decision 2026-11-26-162: no platform-curated catalog image; the `python-data` runtime carries the interpreter and the data contract host; Polars, PyArrow and any library come from the script version's own lock and signed read-only layer (design 046, Architecture decision 0120); a reviewed starter lock ships as a template; User story 024-07 stays (Decision 024-008) |
| O1243 | May the platform destroy superseded snapshot data (ADR-0027 does not authorize deleting customer data; ADR-0023 never deletes committed blobs)? | (a) yes, under the retention the asset manager declares, audited; (b) only on an explicit, audited operator action; (c) never in this delivery (bytes kept, `RETIRED` is a label) | (a); until decided, (c) | **(a)**, delegate, Owner decision 2026-11-26-163; Architecture decision 0058 supersedes ADR-0023 and ADR-0027 in part for that purpose only |

Note (2026-11-20, from Architecture decision 0099): worker-protocol minor 1.8 is reserved for delivery acknowledgement (feature 006, Owner decision 2026-11-20-12; Owner question 006-002); minors are cumulative. An internal runner of this draft at 1.9 or later either implements the 1.8 host procedure (a flagged lease is acknowledged before it runs) or registers with `WorkerCapabilities.declines_acknowledgement = true` (field 10), and the server then flags none of its leases (Decision 006-063 of `specs/006-sdk-multilanguage/research.md`). `TaskLease` fields 23 and 24 and `WorkerCapabilities` field 10 are taken. Its contract `contracts/worker-data.md` takes the next free minor of the register in `sdks/SPEC.md` section 9 when it merges (1.9 is named by design 045, 1.10 is design 044's and 1.11 to 1.13 are named by draft 019, Decision 044-255 of `specs/044-workflow-as-code/research.md`, 2026-11-27).
