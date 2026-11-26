# Tasks: Data engines and pipelines

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`.

**Status**: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decision 0058 accepted (delegate, Owner decision 2026-11-26-164); not
started. No task is checked: each closes only with its named test and result, or a command and its
output (AGENTS.md). Tests are written first and seen failing for the right reason. IDs are
feature-scoped; `[P]` = parallel with other `[P]` tasks of its phase (different files, no
dependency); `[USn]` = user story of this feature (User story 024-0n). Revised 2026-11-26 against
`develop` at `e5bda041` after an independent audit; tasks 024-070 to 024-076 were added then. An
indented `Owner-gate:` line names the owner decision a task builds on. How the tasks are cut into a
foundation slice and two parallel lanes is in [Build slices](#build-slices).

## Phase 1 — Setup and governance

- [ ] T024-001 Confirm at build start that Owner decisions 2026-11-26-160 to 2026-11-26-165 (the answers to Owner questions 1240 to 1243, the acceptance of Architecture decision 0058, the checklist dispositions) are rows of `docs/owner-decisions.md`; record at that date the status of Architecture decision 0079 and its Kubernetes half (Task 035-029), design 046 and Architecture decision 0120, features 018 (instance registry, reset-rule registry), 020, 021, 022, 023, 031 and design 044, and the next free worker-protocol minor in the register of `sdks/SPEC.md` section 9. Refs: all FR, SC. Evidence: the rows and statuses linked from `research.md`; `checklists/requirements.md` dispositions unchanged or revisited.
  Owner-gate: OD-2026-11-26-164, OD-2026-11-26-165
- [ ] T024-002 Update, each through its owner, the dependents Architecture decision 0058 lists (an ADR-0013 "refined by ADR-0058" note, the data mode in Architecture decision 0079 rule 1 and its data request member, `docs/architecture/security-and-sandbox.md` with the `data-runner` role, ADR-0023 and ADR-0027 notes, feature 023's Requirement 023-027 (no image catalog needed), design 046 (the `python-data` language on its Python path), the 017 out-of-scope note) and confirm with 018 (reset-rule registry, ADR-0050 slots 4a and 4b, instance registry), 020 (content-store registry, run-purge exclusion, the `pii`-egress rule), 021 (connection catalog), 023 (`python-data` reservation) and 031 (encoded-field registry); record whether PostgreSQL and the lake kinds already meet the recheck and pause clauses of the `pii`-egress rule (Gap 024-005). Refs: FR-024-018, 021, 028, 034, 042. Evidence: linked notes or owner rows, `python3 scripts/check-agent-config.py` output.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-003 Register `contract_data_engines_pipelines`, `integration_data_engines_pipelines`, `fault_data_engines_pipelines` and `e2e_data_engines_pipelines` in `tests/Cargo.toml` with harnesses in `tests/{contract,integration,fault,e2e}/data-engines-pipelines/main.rs`; a skipped container counts as failure, not success. Refs: NFR-024-002. Evidence: each target runs and reports its (initially failing) cases.
- [ ] T024-004 [P] Pin disposable test images by digest (MySQL 8.4, MariaDB 11, SQL Server 2022 with its EULA variable) in the test fixtures and an `infra/docker/compose.yaml` overlay with loopback ports and health checks; add fake BigQuery/Snowflake HTTP servers under `tests/support/`. The data runtime image is pinned by T024-028. Refs: FR-024-040. Evidence: `docker compose config` and fixture start-up test.

## Phase 2 — Foundational (blocks every story)

- [ ] T024-005 Extend the package rules of `scripts/check-architecture.py` (+ `scripts/tests/`), beside `ICEBERG_TEST_ONLY_PACKAGES` and `LAKE_RUNNER_PACKAGES`: `sagat-data-runner` reaches neither `bollard` nor `sagat-infra-sandbox` (Architecture decision 0079 rule 2 for the data runner); only `sagat-infra-database` may depend on target-database driver crates; only `sagat-infra-data` on `parquet`; no Rust package depends on DuckDB, Polars or Arrow compute; the domain forbids the new driver and format crates. Coordinate the shape with feature 021's Task 021-016 if it adds client rules. Refs: FR-024-021, NFR-024-001, SC-024-004. Evidence: script tests with a planted violation per rule fail; clean tree passes.
- [ ] T024-006 Add `SqlDriver`, four `ConnectionConfig` variants for the five drivers (MySQL and MariaDB share one) beside `Postgres` and `Iceberg`, each with closed validation and `egress_ceiling`; `driver()`, `sql_secret()`, `sql_limits()`, `secret_slots()` and `requires_development()` for them; `DatabaseLimits::max_for(driver)`; `DATABASE_COST_LIMIT` in `crates/domain/src/database.rs` and `database/failure.rs`; tests in `crates/domain/tests/database_engines.rs`. Refs: FR-024-001, 002, 009, 037. Evidence: domain tests; PostgreSQL and Iceberg fixtures unchanged.
- [ ] T024-007 Generalize the `DatabaseCredentialsCodec` port per driver in `crates/application/src/database.rs` and implement the codecs beside `JsonDatabaseCredentials` in `crates/infra-secrets/src/database.rs` (password, service-account key with fixed token URI/universe, key pair); redaction and refusal tests in `crates/infra-secrets/tests/database_credentials.rs`. Refs: FR-024-003, 013. Evidence: codec unit tests incl. hostile `token_uri`.
- [ ] T024-008 Merge the draft fragments of `specs/024-data-engines-pipelines/contracts/` into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json,worker.proto,events.asyncapi.yaml}` at the next free worker-protocol minor (field numbers rechecked against the canonical file), add its row to the register of `sdks/SPEC.md` section 9, implement the minor negotiation for new-driver and data tasks in `crates/api-grpc/src/service.rs` and the mapping of the new connection messages in `crates/api-grpc/src/database.rs`; regenerate `worker.baseline.json` and `events.baseline.json` after reading the diffs; regenerate Studio types. Refs: FR-024-038. Evidence: `contract_*` compatibility suites, `npm run check:api`, the register row.
- [ ] T024-009 Add `data:asset:manage`, `data:asset:use`, `data:asset:publish`, `data:execute`, the machine role `data-runner` (`worker:execute`, `data:execute`), audit actions and the seven quota resources in `crates/domain/src/{identity.rs,audit.rs,quota.rs}`; permission matrix tests in `crates/domain/tests/data_permissions.rs`. Refs: FR-024-033, 035. Evidence: matrix test asserts the four new permissions, the agreed set of built-in roles (021 adds `integration-runner`, 024 `data-runner`, 025 `agent-runner`) and their exact grants — sets, never counts; `runner` unchanged.
- [ ] T024-010 The whole migration set of this feature in `crates/infra-postgres/migrations/`, versions after the latest of `develop` at that time: the data model (tables incl. `run_data_snapshot_refs`, triggers, blob purpose `DATA_SNAPSHOT_PART` with the guarded `COMMITTED → DELETING` trigger enabled, `inbox_events` CHECK and delta-trigger exemption for source `sagat.data`, quota class `data`, grants), `worker_bindings_profile_valid` widened to `warehouse` and `data` (T024-071), the three closed capability CHECKs of the instance registry widened (T024-070); entries in `crates/infra-postgres/storage-classification.json` and 020's content-store registry; `config_from` in `crates/infra-postgres/src/database.rs` decodes the new drivers; guarded downgrade. Refs: FR-024-014, 018, 027, 034, 035. Evidence: migration integration test vs `pg_catalog`, runtime-role privilege sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`), downgrade refusal test, a 020 run purge that leaves `DATA_SNAPSHOT_PART` blobs and all snapshot metadata of the purged producer run untouched while the pointer still names its snapshot.
  Owner-gate: OD-2026-11-26-160, OD-2026-11-26-163, OD-2026-11-26-164
- [ ] T024-011 [P] Closed telemetry vocabulary (spans, metrics, labels; the `driver` label on the existing `database_runner_*` metrics; the `data_sandbox_*` metrics mirroring `lake_sandbox_*`) in `crates/telemetry/src/{metrics.rs,span.rs}` with tests in `crates/telemetry/tests/data_vocabulary.rs`. Refs: FR-024-036. Evidence: label-set tests.
- [ ] T024-012 [P] Error reasons (`DRIVER_NOT_ENABLED`, `DATA_PIPELINES_NOT_ENABLED` and the rest of `contracts/README.md`) with remediation rows in `crates/api-http/src/error.rs`; the data failure set in `crates/domain/src/data/failure.rs`; `DATABASE_COST_LIMIT` and every data failure code as `worker`-origin entries of `sdks/error-catalog.json`, `sdks/SPEC.md` section 2 and each SDK's embedded copy. Refs: FR-024-037. Evidence: `errors::every_reason_the_table_introduces_has_a_remediation_and_is_in_the_openapi` extended; each SDK's catalog-copy test.
- [ ] T024-013 Driver catalog route and the `DRIVER_NOT_ENABLED` gate at catalog write, publication and admission, decided by the registry capabilities and the deployment's served drivers, in `crates/api-http/src/routes/database.rs`, `crates/api-http/src/dto_database.rs` (the union grows) and `crates/application/src/database.rs`; regression proving PostgreSQL-only deployments behave exactly as before; a test with a binary without these drivers showing a new-driver row fails the connection list (`StoreError::Internal`), recorded for the release notes. Refs: FR-024-001, 038, NFR-024-003. Evidence: `integration_database_connections` unchanged plus new capability and rollback tests.
- [ ] T024-014 Per-driver registration and the two deployment profiles (`database`: postgresql/mysql/mariadb/sqlserver; `warehouse`: bigquery/snowflake) in `apps/database-runner/src/{config.rs,runner.rs}`; a runner below the new minor never receives a non-PostgreSQL SQL task (the negotiation is T024-008's). Refs: FR-024-001, 004, 038. Evidence: gRPC test with an old-minor runner and a mixed backlog; a profile never registers the other profile's drivers and a `warehouse` token is refused a private connection's credential.
- [ ] T024-070 Instance-registry capabilities `database-engines-v1`, `database-warehouses-v1` and `data-pipelines-v1` (kinds `CONTROL_PLANE`, `EXECUTION_ENGINE`) in `Capability::ALL` (`crates/application/src/instances.rs`), listed by a process only when its build serves them; the publication, admission and catalog gates that refuse an unrecorded one (`DRIVER_NOT_ENABLED`, `DATA_PIPELINES_NOT_ENABLED`); the migration is T024-010's. Refs: FR-024-001, 020, 038. Evidence: registry tests (a process without a recorded capability is refused, Architecture decision 0082); gate tests before and after the operator records each.
  Owner-gate: OD-2026-11-26-161, OD-2026-11-26-164
- [ ] T024-071 Managed kinds and profiles: `ManagedKind::Sql(SqlDriver)` with `activity()` `sagat.database.<driver>` and `profile()` `Database`/`Warehouse`; `ManagedKind::{DataTransform, DataQuery}` (`sagat.data.transform`, `sagat.data.query`, profile `Data`); `BindingProfile::{Warehouse, Data}` in `crates/domain/src/binding.rs`; `Node::managed_kind` for data nodes; `crates/application/tests/managed_kind_sites.rs` extended so no worker path tests a driver name or a `sagat.data.` prefix. Refs: FR-024-001, 004, 020, 021, 033. Evidence: domain tests, `managed_kind_sites`, a binding test where a `data` token is refused a SQL step and a `database` token a data step.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-075 The uniform `pii`-egress rule for the new drivers in `crates/application` (driver-agnostic): a `pii` ceiling saved only by a reveal-grant holder (`ConnectionConfig::authorize_egress_ceiling`), rechecked at each `ResolveDatabase` of a write that carries a `pii` value, the connection paused when the saver's grant is revoked, each such attempt audited, `CLASSIFICATION_EGRESS_REFUSED` above the ceiling; unit tests over the application ports here, the end-to-end case against MySQL in T024-015. Refs: FR-024-002, 042, SC-024-017. Evidence: application tests; the end-to-end case.

## Phase 3 — US01: MySQL, MariaDB and SQL Server (P1)

**Independent test**: shared typed fixture against real containers from six SDKs and the Studio.

- [ ] T024-015 [US01] Write failing integration tests in `tests/integration/data-engines-pipelines/operational_engines.rs`: typed fixture, wrong host name, untrusted CA, privileged login, denied destination, unsupported type, a `pii` write to a connection whose ceiling is `internal` (T024-075). Refs: FR-024-002, 005..008, 042, SC-024-001, 003, 017.
- [ ] T024-016 [US01] MySQL/MariaDB adapter `crates/infra-database/src/mysql.rs` (pinned crate inspected per D1252): rustls verify-full, `READ ONLY` transaction, `KILL QUERY` cancellation, privilege check, UTF-8/UTC session, type table. Refs: FR-024-004..008, 012. Evidence: T024-015 cases pass for both servers.
- [ ] T024-017 [US01] SQL Server adapter `crates/infra-database/src/sqlserver.rs`: strict/verify-full encryption, always-rollback query transaction, attention cancellation, role checks, type table. Refs: FR-024-004..008, 012. Evidence: T024-015 cases pass on SQL Server 2022.
- [ ] T024-018 [US01] Fixed probes per driver (through design 043's gate at row 13, unchanged) and Studio connection forms (`frontend/apps/studio/src/features/connections/`) driven by `listDatabaseDrivers`, write-only credential fields and the egress ceiling. Refs: FR-024-011, 039, 042. Evidence: probe integration tests; `ConnectionsPage.test.tsx` per driver.
- [ ] T024-019 [US01] Fault tests in `tests/fault/data-engines-pipelines/operational_writes.rs`: SIGKILL of the runner after dispatching a write → `OUTCOME_UNKNOWN` and manual review (the recovery rule "none"); an explicit compensating write on the error path that fails; restart. Refs: FR-024-004, 010, NFR-024-002, SC-024-006.
- [ ] T024-020 [US01] [P] Conformance scenario `database-engines` in `tests/contract/sdk-conformance/scenarios/database-engines.json` with expected outputs. The SDK builders need no change for drivers (a database node names its connection, not its driver); the `assets` property is T024-056's. Refs: FR-024-038, SC-024-001, 012. Evidence: `SAGAT_SDK_SCENARIO=database-engines` passes in six languages (TypeScript, Go, Python, C#, Java, Rust).

## Phase 4 — US02: BigQuery and Snowflake (P1)

**Independent test**: fake servers in CI; real-service suite before advertising.

- [ ] T024-021 [US02] Fake protocol servers and failing tests in `tests/integration/data-engines-pipelines/warehouses.rs`: auth, binding, pagination, 202 polling, cost refusal, cancel, lookup. Refs: FR-024-009..012, SC-024-002.
  Owner-gate: OD-2026-11-26-161
- [ ] T024-022 [US02] Public-endpoint profile `crates/infra-database/src/endpoints.rs` and runner config: driver-fixed hosts, deployment allowlist narrowed by namespace policy, public-address check, private-endpoint networks, no redirect/proxy; the deployment bound to the `warehouse` profile and its NetworkPolicy in `infra/helm/` (DNS, engine, TCP 443 to non-private ranges or listed private-endpoint networks only). Refs: FR-024-004, 005, NFR-024-001, SC-024-002. Evidence: endpoint unit tests, a resolver canary, and live socket tests: the warehouse pod cannot reach a private customer network and the private pod cannot reach a public endpoint.
- [ ] T024-023 [US02] BigQuery adapter `crates/infra-database/src/bigquery.rs`: JWT-bearer token at the fixed endpoint, jobs with deterministic ID, dry-run query check, `maximumBytesBilled`, result pages within bounds, cancel. Refs: FR-024-002, 006, 008, 009, 012. Evidence: T024-021 BigQuery cases.
  Owner-gate: OD-2026-11-26-161
- [ ] T024-024 [US02] Deterministic request identity and in-attempt lookup for both warehouses in `crates/infra-database/src/{bigquery.rs,snowflake.rs}` (the recovery rule "lookup"); manual-review record carries the opaque identity. Refs: FR-024-010, NFR-024-002. Evidence: fake-server faults (lost response before/after commit) report the true outcome or `OUTCOME_UNKNOWN`.
- [ ] T024-025 [US02] Snowflake adapter `crates/infra-database/src/snowflake.rs`: key-pair JWT ≤ 1 h per attempt, SQL API `requestId`/`retry=true`, `MULTI_STATEMENT_COUNT=1`, bindings, 202 polling, partitions, cancel, administrative-role refusal. Refs: FR-024-002, 006..008, 012. Evidence: T024-021 Snowflake cases.
  Owner-gate: OD-2026-11-26-161
- [ ] T024-026 [US02] Credential and output canaries for warehouse codecs (private key PEM/DER, passphrase, `private_key_id`) through the 017 output guard. Refs: FR-024-003, 013, SC-024-011. Evidence: canary tests in `tests/integration/data-engines-pipelines/credential_canaries.rs`.
- [ ] T024-027 [US02] Real-service contract suite `tests/contract/data-engines-pipelines/warehouses_real.rs` (`#[ignore]`, credentials from the environment) on accounts the owner provides (no agent creates an account or spends money); record its run (date, scope, results) before the operator records `database-warehouses-v1`; resolve G1241 facts. Refs: FR-024-001, 010, SC-024-002. Evidence: recorded run with real results; drivers stay hidden otherwise.
  Owner-gate: OD-2026-11-26-161

## Phase 5 — US03: assets, DuckDB transforms, write-audit-publish (P1)

**Independent test**: concurrent readers and producer over a seeded asset; audit failure; crash matrix.

- [ ] T024-028 [US03] Data runtime image `runners/data/{Dockerfile.runtime,host.py,README.md}` (Python line of design 046's dependency runtime, DuckDB, the data contract host, SBOM) and the data container profile `crates/infra-sandbox/src/data.rs` beside `lake.rs` (the mounts of ADR-0058 decision 1, class budgets with tmpfs memory accounting, DuckDB settings). Refs: FR-024-021, 040. Evidence: image build with SBOM; the mount-profile unit test in `crates/infra-sandbox` (every writable mount tmpfs `noexec,nosuid,nodev` with `size=`, every bind read-only, the signed layer the only executable mount, nothing else); container tests that executing a file from `/out` or `/spill` fails `EACCES` and writing past `size=` fails `ENOSPC`.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-073 [US03] Data mode of the sandbox runner in `apps/sandbox-runner/src/data/` (exclusive: no runner token, no script, build or lake configuration; its own slot and user range), serving Architecture decision 0079's three calls over a node-local Unix socket with the peer check, limits, label state, sweep and drain; the lake engine's list, inspect, wait and kill by label generalized in `crates/infra-sandbox/src/lake/engine.rs`; the data request member `layer` in `crates/infra-lake/src/sandbox_api.rs`, refused for a digest the mode did not materialize. Refs: FR-024-021. Evidence: tests against the in-memory engine double (mode exclusivity, unknown request member, ceiling, slot, class, deadline refusals, restart rebuild from labels, drain), one test of the real client against the real server over a real socket.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-072 [US03] Data runner `apps/data-runner` (package `sagat-data-runner`, role `data-runner`, profile `data`): claim loop for `sagat.data.*` at the new minor, `ResolveData`, attempt directory and whole-blob staging (`crates/infra-lake` `staging`), attempt socket with the slot's peer check, attempt client, table of endings (`memory_killed` → `DATA_RESOURCE_EXHAUSTED`, refusal or unavailable → `DATA_RUNTIME_UNAVAILABLE`), heartbeat and cancellation, a start-up probe attempt before offering the activities, the host's bounded log kept as a script step's log and no step-log carry (Requirement 041-016); a `compose.data.yaml` service beside the data-mode sandbox runner. Refs: FR-024-021, 042. Evidence: runner tests against a scripted host (the lake runner's test pattern); `python3 scripts/check-architecture.py` shows no engine dependency; output-channel tests (too many parts, oversized frame, truncated stream, wrong peer user) fail the attempt and commit nothing.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-029 [US03] Asset catalog: domain `crates/domain/src/data.rs` (contract types, compatibility, classification, producer), service `crates/application/src/data_assets.rs`, store `crates/infra-postgres/src/data_assets.rs`, routes `crates/api-http/src/routes/data_assets.rs` + `crates/api-http/src/dto_data.rs`. Refs: FR-024-014..017. Evidence: domain compatibility pairs; integration catalog tests (receipts, revisions, tombstones, `secret` refused, `DATA_PIPELINES_NOT_ENABLED` before the capability).
- [ ] T024-030 [US03] IR codec `crates/infra-ir/src/data.rs` and domain node kinds for `dataTransform`; publication validates graph-model and root-only placement, no `destinations`, the registry capability, asset kinds, producer binding, output contract, limits that a configured class covers, and `step`/`from` predecessors, and refuses ordinary activities using the reserved `sagat.data.` prefix; writes `definition_data_edges`. Refs: FR-024-017, 019, 020. Evidence: `crates/infra-ir/tests/data_nodes.rs` acceptance/refusal fixtures.
- [ ] T024-031 [US03] First-lease pins and `ResolveData` in `crates/application/src/data_execution.rs`, `crates/infra-postgres/src/data_execution.rs`, `crates/api-grpc/src/data.rs`; every data lease writes the `step_lease_blobs` rows of its pinned parts and `ReadBlob` is limited to them (feature 035's gate). Refs: FR-024-019, 038. Evidence: retry reuses pins after the pointer moved and gets rows for its own attempt and generation; foreign/unpinned blob looks missing; a `from` input on a quarantined snapshot fails `DATA_INPUT_QUARANTINED`.
  Owner-gate: OD-2026-11-26-164
- [ ] T024-032 [US03] Transform flow of the data runner: staging with digest verification, the request file, the DuckDB wrapper and part split in `runners/data/host.py`, the framed output channel over the attempt socket, `WriteBlob` purpose `DATA_SNAPSHOT_PART` as frames arrive, in `apps/data-runner/src/`. Refs: FR-024-021, 022. Evidence: transform integration test produces parts through the data mode.
- [ ] T024-033 [US03] Additive ranged read on `BlobBackend` (`crates/application/src/blobs.rs`, `crates/infra-blobstore/src/backend.rs`) and `crates/infra-data` bounded Parquet footer reader and contract verifier, run by `apps/execution-engine` on a blocking task under `catch_unwind` with decoder limits (≤ 1 MiB footer, ≤ 256 columns, ≤ 10,000 row groups, bounded container/string lengths), plus a fuzz target `crates/infra-data/fuzz/` with a hostile-footer corpus; "lying sandbox" tests (wrong schema, encrypted footer, oversized footer, truncated part, reported rows ≠ footer rows). Refs: FR-024-023, SC-024-007. Evidence: unit, integration and fuzz runs (duration and corpus recorded); a panic becomes `DATA_CONTRACT_VIOLATION`, never a crash (G1256).
- [ ] T024-034 [US03] Completion transaction (asset heads locked first, snapshots, parts commit without a second data charge, provenance, auto publication CAS, `data.snapshot_committed`/`data.published` events, protected-system inbox event, deliveries) in `crates/infra-postgres/src/data_execution.rs`; failing-statement injection per write. Refs: FR-024-018, 023, 025, 027, 035. Evidence: atomicity tests in `tests/integration/data-engines-pipelines/completion.rs`, including a completion that publishes while content quota and `events_per_second` are exhausted.
  Owner-gate: OD-2026-11-26-160
- [ ] T024-035 [US03] Audit attempt (a second attempt of the data mode, class `small`) over the read-back uploaded parts and checks (row count, one generated not-null, unique key, accepted values, custom statement) with payload-free outcomes, audited digests and quarantine. Refs: FR-024-024. Evidence: audit pass/fail/error tests; mismatched audited digests refused; quarantined snapshot never current.
- [ ] T024-036 [US03] Concurrency tests with barriers (no sleeps): readers pin and read whole snapshots while publications commit; stale CAS loses; two overlapping producer runs. Refs: FR-024-018, 019, 025, SC-024-005. Evidence: 100% of rounds consistent in `tests/integration/data-engines-pipelines/wap_concurrency.rs`.
- [ ] T024-037 [US03] Studio asset catalog and `dataTransform` editor in `frontend/apps/studio/src/features/data-assets/` and `features/workflows/` (DuckDB dialect in Monaco, contract view, snapshots, audits). Refs: FR-024-039. Evidence: component tests; no sample values (FR-007-010).
- [ ] T024-038 [US03] Retention loop on `engine_leases` in `crates/application/src/data_retention.rs` and `crates/infra-postgres/src/data_retention.rs`; mark phase, then two-phase byte deletion of `DATA_SNAPSHOT_PART` blobs under the asset's declared retention once no `step_lease_blobs` row names a part, audited; `tenant_data_bytes` released after confirmed deletion. Refs: FR-024-034, 035, SC-024-014. Evidence: pinned/in-window, lease-pinned and `run_data_snapshot_refs`-referenced snapshots kept; crash between mark and confirm retried; bytes of a retired snapshot deleted and the charge released only after confirmation.
  Owner-gate: OD-2026-11-26-160, OD-2026-11-26-163
- [ ] T024-039 [US03] Security tests: sandbox canaries (file outside mounts, extension install, socket, fork bomb, memory, a container connecting to another attempt's socket) in `tests/security/data-runtime/` beside the existing `tests/security/sandbox/`; the asset routes and data RPCs added to the tenant battery `tests/contract/security-multitenancy/rest/battery.rs`. Refs: FR-024-021, 033, NFR-024-001, SC-024-004, 009. Evidence: every canary refused; battery green.
- [ ] T024-074 [US03] Statement-budget shapes of the data first lease (pins, `step_lease_blobs` rows, journal) and the data completion in `tests/integration/claim-transition-efficiency/work_budget.rs` (target `integration_claim_transition_efficiency`), recorded at first measurement; the ordinary lease and completion shapes unchanged. Refs: FR-024-042, SC-024-017. Evidence: the budget target passes; a hand mutation adding a statement to the ordinary path fails it.

## Phase 6 — US04: DuckDB queries over assets (P2)

- [ ] T024-040 [US04] `dataQuery` node: IR codec and domain kind (graph-only, root-only, `sagat.data.query`, `ManagedKind::DataQuery`), first-lease pins, one attempt, 017 typed-cell encoding within 256 KiB and the row limit, inline completion validated by the 017 result validator; DuckDB type table of the data runtime contract. Refs: FR-024-029. Evidence: integration join over two assets; oversize result fails `DATA_OUTPUT_LIMIT`; unsupported type fails `DATA_TYPE_UNSUPPORTED`; the 017 result view returns `NOT_FOUND` for the node.
- [ ] T024-041 [US04] `dataQuery` output as a 016 carrier with a platform-derived schema at the highest pinned column level; Studio query panel in `frontend/apps/studio/src/features/data-assets/` using 016 disclosure; ad hoc use through 022 standalone steps when present (022 adds that creator row to design 043's table). Refs: FR-024-016, 029, 039. Evidence: e2e disclosure — `internal` inputs need `payload:read-internal`, `pii` inputs the namespace reveal grant, audited as 016 reveals.
- [ ] T024-042 [US04] Resource exhaustion and canary tests for queries (memory, spill, rows/bytes limits). Refs: FR-024-021, 029, SC-024-004, 010. Evidence: typed failures, other attempts unaffected.

## Phase 7 — US05: lineage, freshness, publication events (P2)

- [ ] T024-043 [US05] Lineage query (declared + recorded, bounded, opaque producers) in `crates/application/src/data_assets.rs` and store; route `getDataLineage`. Refs: FR-024-030. Evidence: fixture graph equality and truncation tests.
- [ ] T024-044 [US05] Freshness evaluation in `crates/domain/src/data/freshness.rs` (lag and cron deadline, DST cases) and monitor conditions `DATA_ASSET_STALE`, `DATA_SNAPSHOT_QUARANTINED` in `crates/application/src/operations.rs` / store. Refs: FR-024-031. Evidence: database-clock tests; fire/resolve; failed look resolves nothing.
- [ ] T024-045 [US05] Studio lineage graph with table alternative and freshness badges in `frontend/apps/studio/src/features/data-assets/`. Refs: FR-024-039, NFR-024-006. Evidence: component tests and axe checks.
- [ ] T024-046 [US05] Publication inbox event (protected-system, ≤ 1 KiB, not rate-charged, no column value) and webhook deliveries; e2e event trigger starts one run per publication under duplicate delivery, through design 043's gate at row 9; a rollback emits a new event (`<asset id>:<pointer version>`). Refs: FR-024-027, 042, SC-024-008, 010. Evidence: `tests/e2e/data-engines-pipelines/publication_trigger.rs` and a publication at full content quota and exhausted `events_per_second`.

## Phase 8 — US06: manual publication, approval gating, rollback (P2)

- [ ] T024-047 [US06] `dataPublish` engine node in `crates/domain/src/workflow/`, `crates/infra-ir/src/data.rs` and the scheduler apply path (`crates/scheduler/`); already-current idempotency. Refs: FR-024-025. Evidence: reducer and integration tests.
- [ ] T024-048 [US06] API publish/rollback with receipts, justification and audit (`publishDataSnapshot`), taking the membership guard before the asset head (D1273), with an interleaving test against a concurrent admission and role change. Refs: FR-024-026. Evidence: replay after revocation denied; changed body conflicts; quarantined refused.
- [ ] T024-049 [US06] Races with barriers: approval decision vs run deadline before `dataPublish`; API vs run publication; cancellation vs auto-publishing completion. Refs: FR-024-025, 026, SC-024-005, 006. Evidence: first commit wins, loser audited, in `tests/fault/data-engines-pipelines/publication_races.rs`.
- [ ] T024-050 [US06] Studio publish/rollback actions with justification, permission-gated. Refs: FR-024-039. Evidence: component and e2e tests.

## Phase 9 — US07: Polars (P3)

- [ ] T024-076 [US07] Design, before any Polars code, how a design 046 layer reaches a data attempt under Architecture decision 0079's split (Gap 024-002): which process fetches the locked files with a control-plane token, which builds and verifies the volume with the engine, how the data mode learns that a layer digest is signed, and how the data runner advertises only versions whose layer is held; record it in `research.md` and, if it changes the attempt API, in a note to Architecture decision 0079's owner. Refs: FR-024-028. Evidence: the recorded design reviewed by the security reviewer.
  Owner-gate: OD-2026-11-26-162
- [ ] T024-051 [US07] `python-data` on the data runtime image (no package catalog) and the reviewed starter lock for Polars and PyArrow in `runners/data/python-data/starter/` (a PEP 723 block and a PEP 751 lock as an author's template, with its advisory evidence), and the starter-lock refresh procedure in `docs/runbooks/data-runner.md`; requires design 046 built. Refs: FR-024-028. Evidence: the starter lock resolves through the package gateway and passes the default supply-chain policy; image build unchanged by it.
  Owner-gate: OD-2026-11-26-162
- [ ] T024-052 [US07] With design 046's mechanism: a `python-data` version publishes only as bundle version 5 with its own lock through the gateway and the namespace policy; the lease of a Polars step runs design 046's block check (`DATA_STATEMENT_DENIED`, reason `PACKAGE_BLOCKED`); a `python-data` version is refused as an ordinary script step. Refs: FR-024-028. Evidence: policy refusal at publication; a block after publication stops the lease; an ordinary script step naming a `python-data` version is refused.
  Owner-gate: OD-2026-11-26-162
- [ ] T024-053 [US07] Polars transform execution through the data contract with the layer at `/opt/sagat/deps`; parity with the DuckDB fixture of US03; runtime install attempt fails. Refs: FR-024-021, 028, SC-024-004. Evidence: integration parity test.
  Owner-gate: OD-2026-11-26-162

## Phase 10 — US08: external assets (P3)

- [ ] T024-054 [US08] External declarations with a producer and the `assets` property on `database` and `icebergWrite` nodes; publication refuses a declared write by another node or connection; observation committed with a successful declared write. Refs: FR-024-032. Evidence: integration tests for success, failure, `OUTCOME_UNKNOWN` and the spoofing refusal, for a SQL connection and for an Iceberg connection.
- [ ] T024-055 [US08] Freshness and lineage over observations. Refs: FR-024-030, 031, 032, SC-024-008. Evidence: freshness fixture with observations only.

## Phase 11 — Polish and release gates

- [ ] T024-056 [P] Six SDK builders (TypeScript, Go, Python, C#, Java, Rust: `sdks/rust/src/workflow/` beside `database.rs` and `iceberg.rs`) for `dataTransform`, `dataQuery`, `dataPublish` and the `assets` property; scenarios `data-transform-builders` and `data-pipeline-runtime`; the four Rust SDK lock files move together. Refs: FR-024-020, 028, 029, 038, SC-024-012. Evidence: six-language runs and the per-SDK checks of `sdks/AGENTS.md`.
- [ ] T024-057 [P] Runtime graph for data nodes (metadata only) in `frontend/apps/studio/src/features/runs/`; generated client parity. Refs: FR-024-039, 041. Evidence: `npm run check:api`, component tests.
- [ ] T024-058 [P] Runbooks `docs/runbooks/{database-engines.md,data-runner.md,data-assets.md}` (the data runner, the data mode, retention and copies outside the platform), guide `docs/guides/data-pipelines.md`, alert sections in `docs/runbooks/alerts.md`; the Helm templates of the data runner, the data mode and the warehouse deployment. Refs: FR-024-040. Evidence: documentation link check; rendered chart tests.
- [ ] T024-059 Quickstart demo script and compose overlay reproducing `quickstart.md` happy and failure paths. Refs: FR-024-040, SC-024-016. Evidence: demo run log.
- [ ] T024-060 Fault matrix: crash at every commit point of staging, completion, publication, rollback, retention mark and confirm; `verify_run` (`crates/infra-postgres/src/execution.rs`) and `assert_consistent` (`tests/support/engine.rs`) after each. Refs: FR-024-019, 023..027, 034, SC-024-006. Evidence: `fault_data_engines_pipelines` results.
- [ ] T024-061 Fault: duplicate completion and duplicate event, lost broker hint, engine, data runner and data-mode restarts, warehouse ambiguous writes, failed compensation write. Refs: FR-024-010, 027, NFR-024-002, SC-024-006. Evidence: fault target results.
- [ ] T024-062 Security gate: tenant battery over every new operation and RPC, credential/private-key/asset-content canaries, runtime-role sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`). Refs: FR-024-033, NFR-024-001, SC-024-009. Evidence: suite results.
- [ ] T024-063 Observability gate: real OTLP export and logs free of canaries, closed labels, audit-failure rollback. Refs: FR-024-036, 037, NFR-024-004, SC-024-011. Evidence: suite results.
- [ ] T024-064 Compatibility gate: 017 and 035 suites unchanged; the previous generated Studio client reads a connection list containing a MySQL connection; old-minor runner exclusion; 031 refusals (`ENCODING_READ_BY_EXPRESSION`) for data-node and database parameters; 022 draft test runs refuse unmocked data nodes (`TEST_RUN_UNMOCKED_EFFECT`); six-SDK conformance. Refs: FR-024-038, 041, NFR-024-003, SC-024-012, 015. Evidence: suite results.
- [ ] T024-065 Ceilings suite: exhaust each row of the plan's ceilings table, including bytes billed, tmpfs sizes, class budgets and `tenant_data_bytes` at part upload. Refs: FR-024-009, 035, SC-024-010. Evidence: typed errors before work.
- [ ] T024-066 Performance report `tests/performance/reports/data-engines-pipelines.md` (environment, workload, p50/p95/p99, failures). Refs: NFR-024-005. Evidence: report with measured values only.
- [ ] T024-067 Final regression on the merged tree: `cargo fmt --check`, clippy, workspace tests, architecture and agent-config checks, contract checks, browser e2e (SC-024-013), per-SDK lints; record actual results and remaining gaps. Refs: all FR, SC. Evidence: command outputs.
- [ ] T024-068 Register this feature's journal event types in the reset-rule registry `crates/domain/src/execution/reset.rs` (`data.inputs_pinned` and `data.snapshot_committed` `Rematerialize`, `data.published`, `data.publish_skipped`, `data.observation_recorded` `Copy`; the variants and their rules compile together in the foundation slice) and implement the rematerialization in the reset transaction; tests in `tests/integration/data-engines-pipelines/reset.rs`. Refs: FR-024-019, 025, SC-024-006. Evidence: reset after each event type behaves as declared; a reset referencing a retired snapshot is refused `SNAPSHOT_RETIRED`; no publication is re-executed.
- [ ] T024-069 Amendment to feature 016 (C6): add the `dataQuery` output to the consolidated FR-016-002 carrier list (opened by the first parity feature implemented) and to classification rules version 2. Refs: FR-024-016, 029. Evidence: 016 carrier tests include a `dataQuery` output.

## Dependencies

Phase 1 → Phase 2 → {US01, US02, US03} (US02 also needs T024-022); US03 needs T024-028, T024-072
and T024-073 before T024-032; US04 needs T024-028/031/032; US05 and US06 need US03; US07 needs US03,
design 046 built and T024-076; US08 needs US01 or US02 and T024-029; Phase 11 after the stories it
gates. The order in which the work is built is the build slices below.

## Build slices

The tasks are cut into a foundation slice built first by one builder, then two lanes that two
builders run in parallel without editing the same Rust crate. Lane A is the drivers: the target
adapters, the database runner and their tests. Lane B is the data assets and transforms: the data
domain, the IR, the application and storage, the data runner, the data mode of the sandbox runner,
the data runtime, the scheduler, the SDK builders and the Studio. A slice merges as one pull request
with its verification; a slice that names a dependency in the other lane starts only after that slice
has merged.

### Slice 0 — foundation (one builder, before the lanes split)

- Tasks: 024-001, 024-002, 024-003, 024-005, 024-006, 024-007, 024-008, 024-009, 024-010, 024-011,
  024-012, 024-013, 024-070, 024-071, 024-075, and the event variants and reset rules of 024-068 (the
  registry is an exhaustive match, so they compile together).
- Touches: `specs/001-platform-foundation/contracts/` (OpenAPI, IR schema, `worker.proto` and its
  baseline, AsyncAPI and its baseline); `sdks/SPEC.md` (the register row of the minor and section 2),
  `sdks/error-catalog.json` and the six embedded copies; `crates/domain` (`database.rs`,
  `database/failure.rs`, `binding.rs`, `identity.rs`, `audit.rs`, `quota.rs`, `workflow.rs` for
  `Node::managed_kind`, `execution/reset.rs`, and the data asset types of `data.rs` both lanes name);
  `crates/application` (`database.rs` codec port, driver catalog and gate, the `pii`-egress recheck;
  `instances.rs`; `tests/managed_kind_sites.rs`); `crates/infra-secrets/src/database.rs`;
  `crates/infra-postgres` (the whole migration set, `storage-classification.json`, `config_from` in
  `src/database.rs`); `crates/api-grpc` (generated types, the minor negotiation, the mapping of the
  new connection messages); `crates/api-http` (`error.rs`, `dto_database.rs`, `routes/database.rs`);
  `crates/telemetry`; `scripts/check-architecture.py` and its tests; `tests/Cargo.toml` (the four
  targets declared once here); the Studio's generated client (`npm run api`).
- Depends on: nothing; it fixes the worker-protocol minor number at its merge (the next free one).
- Verification: `cargo fmt --check`, clippy, `python3 scripts/check-architecture.py` and
  `scripts/tests/`, the CI `contracts` lint and compatibility suites, migration up and down with the
  runtime-role sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`) and the storage-classification check,
  `integration_database_connections` and the feature 035 suites unchanged, `managed_kind_sites`, the
  permission and role matrix test, the registry tests, each SDK's catalog-copy test,
  `npm run check:api`.

### Lane A — drivers

- **Slice A1, operational engines.** Tasks 024-004, 024-014, 024-015, 024-016, 024-017, 024-018
  (probes; its Studio forms are built in slice B8), 024-019, 024-020. Touches
  `crates/infra-database` (`mysql.rs`, `sqlserver.rs`, statement checks), `apps/database-runner`, the
  workspace `Cargo.toml` and `Cargo.lock` for `mysql_async` and `tiberius`, the test fixtures of the
  engine images, and the test files `tests/integration/data-engines-pipelines/operational_*.rs`,
  `tests/fault/data-engines-pipelines/operational_writes.rs`,
  `tests/contract/sdk-conformance/scenarios/database-engines*.json`. Depends on slice 0.
  Verification: `integration_data_engines_pipelines` (operational cases, including the `pii` write of
  Task 024-075), `fault_data_engines_pipelines` (operational writes), the driver-crate inspection
  record (D1252), `cargo deny` and the advisory audit.
- **Slice A2, warehouses against fake servers.** Tasks 024-021, 024-022, 024-023, 024-024, 024-025,
  024-026. Touches `crates/infra-database` (`endpoints.rs`, `bigquery.rs`, `snowflake.rs`),
  `apps/database-runner` (the `warehouse` profile), the warehouse Helm templates in `infra/helm/`,
  `tests/support/` (the fake servers only) and `tests/integration/data-engines-pipelines/{warehouses,credential_canaries}.rs`.
  Depends on A1. Verification: fake-server suites, endpoint unit tests and the resolver canary, the
  live network-policy socket tests, credential canaries.
- **Slice A3, real-service gate.** Task 024-027. Touches
  `tests/contract/data-engines-pipelines/warehouses_real.rs`. Depends on A2 and on accounts the owner
  provides (Owner decision 2026-11-26-161). Verification: the recorded run; only then does the
  operator record `database-warehouses-v1`.

### Lane B — data assets and transforms

- **Slice B1, the attempt path.** Tasks 024-028, 024-073, 024-072, 024-032. Touches `runners/data`,
  `crates/infra-sandbox` (`data.rs`, the generalized `lake/engine.rs`), `crates/infra-lake`
  (`sandbox_api.rs`: the data request member and slot range, additive), `apps/sandbox-runner`
  (`src/data/`), `apps/data-runner` (new), `infra/docker/compose.data.yaml`, the workspace `Cargo.toml`
  and `Cargo.lock` for the new application. Depends on slice 0. Verification: the mount-profile unit
  test, the engine-double tests of the data mode, the real client against the real server over a
  real socket, the data runner's scripted-host tests, `python3 scripts/check-architecture.py` (no
  engine in the data runner), the container canaries of Task 024-028.
- **Slice B2, catalog, IR and pins.** Tasks 024-029, 024-030, 024-031, 024-074. Touches
  `crates/domain/src/data*`, `crates/infra-ir`, `crates/application` (`data_assets.rs`,
  `data_execution.rs`), `crates/infra-postgres/src/` (stores only), `crates/api-grpc/src/data.rs`,
  `crates/api-http` (asset routes), `apps/control-plane` and `apps/execution-engine` (wiring),
  `tests/integration/claim-transition-efficiency/work_budget.rs`. Depends on slice 0 (B1 for the end
  to end cases). Verification: domain and IR fixtures, catalog integration tests,
  `integration_claim_transition_efficiency`, the pin and allow-list tests.
- **Slice B3, verification, completion and audits.** Tasks 024-033, 024-034, 024-035, 024-036,
  024-039. Touches `crates/infra-data` (new) and its fuzz target, `crates/infra-blobstore`,
  `crates/application/src/blobs.rs`, `crates/infra-postgres/src/data_execution.rs`,
  `apps/execution-engine`, `tests/security/data-runtime/`, the tenant battery. Depends on B1 and B2.
  Verification: fuzz run with its corpus recorded, atomicity, concurrency and security suites.
- **Slice B4, queries, publication, lineage and freshness.** Tasks 024-040, 024-041 (engine part),
  024-042, 024-043, 024-044, 024-046, 024-047, 024-048, 024-049, and the rematerialization of 024-068.
  Touches `crates/domain`, `crates/infra-ir`, `crates/application`, `crates/infra-postgres/src/`,
  `crates/scheduler`, `crates/api-http`. Depends on B3. Verification: integration, e2e trigger, race
  and reset suites.
- **Slice B5, retention.** Task 024-038. Touches `crates/application/src/data_retention.rs`,
  `crates/infra-postgres/src/data_retention.rs`, `crates/scheduler`. Depends on B3. Verification:
  retention suite and its crash cases.
- **Slice B6, external assets.** Tasks 024-054, 024-055. Touches `crates/infra-ir`,
  `crates/application`, `crates/infra-postgres/src/`. Depends on B2, and for the SQL case on slice A1
  (a working MySQL connection) and for the Iceberg case on feature 035's lake suite. Verification:
  external-asset integration tests.
- **Slice B7, Polars.** Tasks 024-076, 024-051, 024-052, 024-053. Touches `runners/data/python-data/`,
  `apps/data-runner`, `apps/sandbox-runner/src/data/`, `crates/application` (the lease block for
  data steps). Depends on B3 and on design 046 built (Owner decision 2026-11-26-162); Task 024-076
  first. Verification: the design review, policy and block tests, the parity test.
- **Slice B8, Studio.** Tasks 024-018 (connection forms), 024-037, 024-041 (Studio part), 024-045,
  024-050, 024-057. Touches `frontend/apps/studio`. Depends on slice 0 (the generated client) and,
  for the journeys, on B4. Verification: component and accessibility tests, `npm run api && npm run
  check:api`, the real-browser journeys (SC-024-013).
- **Slice B9, SDKs.** Task 024-056. Touches `sdks/` only (and the four Rust SDK lock files together).
  Depends on slice 0 (the IR schema and the catalog). Verification: the per-SDK checks of
  `sdks/AGENTS.md`, unit tests, the byte-identical IR fixtures; the conformance run is a final gate.
- **Slice B10, documentation and deployment.** Tasks 024-058, 024-059. Touches `docs/runbooks/`,
  `docs/guides/`, `infra/helm/` (data runner and data-mode templates), the quickstart script. Depends
  on B4. Verification: the documentation link check, rendered chart tests, the demo run log.

### Cross-lane order points

1. Slice 0 merges before either lane starts; it fixes the contracts, the protocol minor and its
   register row, the error catalog, the migrations, the domain types, the permissions and the role,
   the registry capabilities, the architecture rules and the test targets.
2. Migrations are slice 0's. After it, only lane B (which owns `crates/infra-postgres`) adds a
   migration; lane A asks lane B for one and never writes a migration file.
3. The worker-protocol contract (`worker.proto`, its baseline and the register row) is slice 0's; a
   later change is a separate pull request by lane B, reviewed by lane A.
4. After slice 0, `crates/domain`, `crates/application`, `crates/api-grpc`, `crates/api-http`,
   `crates/infra-postgres`, `crates/telemetry`, `crates/infra-lake`, `crates/infra-sandbox`,
   `crates/scheduler`, `apps/control-plane` and `apps/execution-engine` are lane B's. Lane A edits only
   `crates/infra-database`, `crates/infra-secrets` (a codec fix), `apps/database-runner`, its Helm
   templates and its own test files; it consumes `crates/infra-lake` without editing it.
5. The SDK builders are lane B's (slice B9); lane A owns only the `database-engines` scenario files.
   The six-language conformance run of Tasks 024-020 and 024-056 is a final gate.
6. The Studio is lane B's, including the connection forms of Task 024-018.
7. The workspace `Cargo.toml` and `Cargo.lock`: each lane adds its own dependencies in one slice (A1
   the driver crates, B1 the data runner, B3 `parquet`), merged one at a time; the later rebases on
   `develop` and re-resolves only its own packages (`cargo update -p <package>`), never the whole lock.
8. The `tests` package is shared by design: its targets are declared in slice 0, and afterwards each
   lane adds files only under its own paths (lane A: `operational_*`, `warehouses*`,
   `credential_canaries.rs`, `warehouses_real.rs`, `operational_writes.rs`, the fake servers and engine
   fixtures under `tests/support/`, `database-engines*.json`; lane B: every other file under
   `data-engines-pipelines/`, `tests/security/data-runtime/` and the budget test).
9. The final gates run after both lanes: Tasks 024-060 to 024-067 and 024-069.
