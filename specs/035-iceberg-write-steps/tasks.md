# Tasks: Apache Iceberg writes and step-output destinations

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/README.md), [quickstart.md](quickstart.md).

**Status**: Not started (generated 2026-10-28). Nothing below is built. A task is ticked only with
the evidence that closes it: a named test and its result, or a command and its output (AGENTS.md).

**Tests**: required. Constitution VIII and AGENTS.md rule 18 make tests come first: every test task
is written before its implementation task and seen failing for the right reason. Races use held locks
or barriers, never sleeps. A test that cannot start its infrastructure fails; it never passes or
skips silently.

**IDs**: feature-scoped `T035-NNN`, in execution order. Decisions `D035-…`, gaps `G035-…` and owner
questions `O035-…` live in research.md and are never renumbered. `[P]` marks a task that touches
files no other open task touches and depends on no unfinished task. `[USnn]` ties a task to a user
story of spec.md.

**Increments**: the phases follow the plan's delivery sequence (I0–I6). Phase 2 is I0; Phases 3–5
are I1; Phase 6 is I2; Phases 7, 8 and 10 are I3; Phase 9 is I4; Phase 11 is I5; Phase 12 is I6.

## Format: `[ID] [P?] [Story?] Description with file path. Refs.`

## Path conventions (this repository)

- Rust crates: `crates/<crate>/src/…`, their unit tests beside the code, and crate tests in
  `crates/<crate>/tests/`.
- Binaries: `apps/database-runner`, `apps/sandbox-runner`, `apps/control-plane` and
  `apps/execution-engine`.
- New: `crates/infra-lake/` (adapter crate, `lake` cargo feature only) and `runners/lake/` (Python host
  `sagat_lake/`, `tests/`).
- Workspace tests: `tests/{contract,integration,fault,e2e,performance}/<feature>/`, registered in
  `tests/Cargo.toml`; helpers in `tests/support/`.
- SDKs: `sdks/{typescript,python,go,java,dotnet}`. Studio: `frontend/apps/studio/src/features/`;
  Playwright: `frontend/e2e/tests/`.

---

## Phase 1: Setup (decisions, harness, fixtures, contracts)

**Purpose**: settle the owner gates, register the test targets, declare the fixtures and merge the
contracts, so every later task has a place to put its failing test.

- [ ] T035-001 Put to the owner, and record each answer in `docs/owner-decisions.md` and
  `specs/035-iceberg-write-steps/research.md`:
  - acceptance of ADR-0071, by setting its status line in
    `docs/adr/ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md`;
  - O035-003 (the plan's ceilings table and the reference environment);
  - O035-004 (`hold` as the default policy);
  - O035-005 (020's sinks reuse the new connections);
  - O035-002 (ORC/Avro and Kerberos priority);
  - whether developers may publish writes. That changes ADR-0036 decision 5; register it as the next
    free O035 id when asking.

  No code task starts before ADR-0071 is accepted (plan.md, Constitution Check, technology-baseline
  row). Refs: D035-045, D035-047, D035-066, O035-002..005.
- [ ] T035-002 Before the first code commit, check every number the plan reserved:
  1. Rebase `feature/035-iceberg-write-steps` on `origin/develop`.
  2. Check every open branch and worktree (`git branch -a`, `git worktree list`) for:
     - migration `20270115000000` still sorting after every branch's latest migration;
     - ADR-0071 still free;
     - 48 permissions before this feature;
     - protocol minor 6 on develop.
  3. Record any change as a new D035 decision in research.md.

  Refs: D035-043, D035-044.
- [ ] T035-003 Register the targets in `tests/Cargo.toml` and create each harness `main.rs`:

  | Target | Harness |
  |---|---|
  | `contract_iceberg_writes` | `tests/contract/iceberg-writes/main.rs` |
  | `integration_iceberg_writes` | `tests/integration/iceberg-writes/main.rs` |
  | `fault_iceberg_writes` | `tests/fault/iceberg-writes/main.rs` |
  | `e2e_iceberg_writes` | `tests/e2e/iceberg-writes/main.rs` |
  | `perf_iceberg_writes` | `tests/performance/iceberg-writes/main.rs` |
  | `integration_step_destinations` | `tests/integration/step-destinations/main.rs` |
  | `fault_step_destinations` | `tests/fault/step-destinations/main.rs` |
  | `e2e_step_destinations` | `tests/e2e/step-destinations/main.rs` |

  A harness that cannot start its containers fails the test. Refs: NFR-035-DUR, SC-035-010.
- [ ] T035-004 [P] Declare the fixtures in `infra/docker/compose.lake.yaml`: the D035-041 images, each
  pinned by digest, with a tag at least two weeks old when added; loopback-only ports; a healthcheck
  per service. Add `tests/support/lake.rs`, which:
  - starts its own disposable containers;
  - provides the fixture admin client that creates tables (the platform never creates them);
  - never uses a hand-started service or another project's container.

  Refs: FR-035-032, SC-035-010, D035-041, AGENTS.md rule 16.
- [ ] T035-005 [P] Add the independent reader:
  - `iceberg = "=0.10.1"` as a test-only dependency in `tests/Cargo.toml`;
  - `tests/support/lake_reader.rs`, returning rows, snapshots, snapshot summaries and refs;
  - a check in `scripts/check-architecture.py` that fails if the `iceberg` crate appears in any
    package other than `tests`.

  Refs: NFR-035-INTEROP, D035-026.
- [ ] T035-006 [P] Create the lake runtime skeleton in `runners/lake/`:
  - `pyproject.toml`, with mypy strict and ruff;
  - `requirements.in`;
  - `requirements.lock`: the closure of `pyiceberg[pyarrow,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0`,
    with hashes;
  - `Dockerfile` stage `lake-runtime`:
    - Python 3.12 slim pinned by digest;
    - `--require-hashes --no-deps`;
    - pip and setuptools removed and no compiler;
    - `/opt/sagat/lake` read-only;
    - an SBOM;
  - `sagat_lake/__init__.py` and `tests/conftest.py`.

  Add pytest, mypy, ruff and `pip-audit` (on every lock change and on a schedule) to
  `.github/workflows/ci.yml`. Refs: D035-026, G035-023, contracts/lake-runtime.md "Image".
- [ ] T035-007 Write the failing contract tests in `tests/contract/iceberg-writes/`, with their
  fixtures under `tests/contract/iceberg-writes/fixtures/`:
  - `ir.rs`: the canonical `workflow.schema.json` accepts the valid IR samples and refuses the invalid
    ones. Use at least the 14 IR cases checked at planning.
  - `receipts.rs`: the receipt schema refuses an injected extra field (`unevaluatedProperties: false`)
    and accepts the 5 planning samples.
  - `openapi.rs`:
    - `DatabaseConnection.config` is a `oneOf` with `discriminator.propertyName: driver`;
    - `GET /database-drivers`, catalog-browse and the subscription `purpose`/`egressCeiling` exist with
      `x-implemented: '035'`;
    - every existing operation is byte-identical.
  - `proto.rs`: the new `ResolvedDatabase` fields are additive, and no number is reused.
  - `errors.rs`: every code and reason of contracts/errors.md appears in the OpenAPI `Error.code`
    reasons and in `sdks/error-catalog.json`.

  Refs: FR-035-008, FR-035-029, FR-035-033.
- [ ] T035-008 Merge the fragments into the canonical contracts, then regenerate the derived files.
  Merges:
  - [contracts/workflow-lake.schema.json](contracts/workflow-lake.schema.json) into
    `specs/001-platform-foundation/contracts/workflow.schema.json`;
  - [contracts/connections.md](contracts/connections.md) and the reasons of
    [contracts/errors.md](contracts/errors.md) into `specs/001-platform-foundation/contracts/openapi.yaml`;
  - [contracts/worker-protocol.md](contracts/worker-protocol.md) into
    `specs/001-platform-foundation/contracts/worker.proto`, assigning field numbers now.

  Regeneration:
  - `docs/reference/workflow-document.md`: run
    `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience`, read the
    diff, then rerun without the switch;
  - `frontend/apps/studio/src/api/schema.d.ts`: `npm run api && npm run check:api`;
  - each SDK's generated protobuf stubs.

  T035-007 passes. Refs: FR-035-008, FR-035-033, D035-043.

---

## Phase 2: Foundational (I0 — blocks every user story)

**Purpose**: the domain types, permission, failure set, migration, runner profiles, build separation,
the `lake` sandbox profile, the relay, the host protocol and the rollout flag. This phase also closes
the three constitution-V conditions: T-isolation (G035-022), relay and hostile-catalog canaries
(G035-018), and T-commit-once (G035-020).

**⚠️ CRITICAL**: no user story starts until this phase is complete, and I1 cannot close before the
three conditions pass.

### Tests first

- [ ] T035-009 [P] Write the failing tests in `crates/domain/tests/database_connections.rs`. Every
  connection variant, `Postgres` included, carries `egressCeiling` with these rules, quoted from
  data-model.md §1:
  - "values `public`, `internal` or `pii`; `secret` is not a valid value";
  - "default `internal`, and a revision without the field reads as `internal`";
  - "setting `pii` requires the actor to hold the 016 reveal grant for the namespace".

  Refs: FR-035-005, D035-040.
- [ ] T035-010 [P] Write the failing tests in `crates/domain/tests/classification.rs`:
  - `within(level, ceiling)` holds for every pair of the four levels;
  - a written field's level is the stricter of its inherited and declared levels, so a `public`
    `recordSchema` annotation never lowers an inherited `pii` field;
  - a field with neither level is `pii`;
  - database-result fields are `pii`.

  Refs: FR-035-025, D035-051.
- [ ] T035-011 [P] Write the failing tests in `crates/domain/tests/identity.rs` and
  `crates/domain/tests/audit.rs`:
  - the catalog has 49 permissions;
  - `database:browse` is held by developer and admin only;
  - a permission added to `ALL` without a role decision fails a test;
  - the audit actions `DatabaseCatalogBrowsed`, `DestinationDecided` and `IcebergRevertRequested`
    exist, with their collapse policy.

  Refs: FR-035-024, D035-066.
- [ ] T035-012 [P] Write the failing pin test in `crates/domain/src/database/failure.rs`: the full set
  of contracts/errors.md codes, each with its retryable flag, is identical in `code()` and in
  `parse()`, so no code can be added to one list only. Refs: FR-035-029, D035-042.
- [ ] T035-013 [P] Write the failing migration tests in `tests/integration/iceberg-writes/migration.rs`:
  - the up migration applies on the develop schema;
  - existing rows read unchanged: `purpose = RUN_EVENTS`, binding profile `database`, ceiling
    `internal`;
  - the grants work under `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
  - the down migration refuses while any `DESTINATION` step, `STEP_OUTPUT` delivery, `STEP_OUTPUT`
    subscription or connection of a new kind exists;
  - every new column is classified.

  Refs: D035-044, D035-059.
- [ ] T035-014 [P] Write the failing profile tests in `tests/integration/iceberg-writes/profiles.rs`:
  - a `database` binding cannot lease a lake activity, and a `lake` binding cannot lease a database
    activity;
  - `ResolveDatabase` refuses a connection kind outside the caller's profile;
  - a runner at minor 6 still registers for PostgreSQL and never sees the new activities;
  - an ordinary SDK worker cannot claim `sagat.database.*`.

  Refs: FR-035-023, D035-057.
- [ ] T035-015 [P] Write the failing T-isolation tests in `tests/integration/iceberg-writes/isolation.rs`.
  From inside a `lake` sandbox container, each of these fails:
  - reading the runner's token file;
  - reading another attempt's `/work/<attempt-id>`;
  - reaching the pod network or the metadata service (IPv4 and IPv6);
  - reaching the container engine socket;
  - running with any capability or a writable root.

  Refs: FR-035-023, D035-048, G035-022.
- [ ] T035-016 [P] Write the failing relay tests in `crates/infra-lake/tests/relay.rs`, plus unit tests
  in `crates/infra-database/src/network.rs` for `fd00:ec2::254` and `fd20:ce::254`. The relay:
  - refuses a missing or wrong token;
  - refuses `CONNECT` to an undeclared host, and counts the refusal;
  - allows declared endpoints and the regional endpoints derived from the storage type and region;
  - resolves once and connects to the checked address (a DNS rebinding test with a scripted resolver);
  - refuses loopback, link-local, metadata and platform state addresses;
  - supports TCP mappings for non-HTTP destinations;
  - never terminates TLS.

  Refs: FR-035-004, D035-049, D035-061.
- [ ] T035-017 [P] Write the failing `sagat.lake.v1` codec tests in `crates/infra-lake/tests/protocol.rs`:
  - one request line;
  - progress lines at most once per second;
  - exactly one result line;
  - a malformed line, a missing result or an extra field yields `LAKE_HOST_PROTOCOL`;
  - the receipt is rebuilt from typed fields and validates against
    [contracts/receipts.schema.json](contracts/receipts.schema.json);
  - host standard error is reduced to a closed exception class and a line count, and is never stored
    or logged.

  Refs: D035-062, D035-067.
- [ ] T035-018 [P] Write the failing configuration-hygiene tests in
  `runners/lake/tests/test_config_hygiene.py`, and the hostile REST catalog fixture in
  `tests/fixtures/lake/hostile-catalog/`. The fixture returns:
  - `py-io-impl`, `s3.endpoint`, `s3.proxy-uri` and signer settings;
  - a table location at `169.254.169.254`, and one at the platform database.

  Expect: every override dropped, every such location refused, and each denial counted. Refs:
  FR-035-004, D035-050.
- [ ] T035-019 [P] Write the failing T-commit-once test in
  `tests/integration/iceberg-writes/commit_once.rs`: a commit-counting relay in front of the REST
  fixture sees exactly one commit request per search, even when the catalog answers 409, 5xx or times
  out. Refs: FR-035-018, D035-030, G035-020.
- [ ] T035-020 [P] Write the failing rollout tests in `tests/integration/iceberg-writes/rollout.rs`:
  with `lake.enabled` off (the default), both publication and admission refuse with
  `VALIDATION_FAILED`, reason `LAKE_DISABLED`. Refs: D035-059.
- [ ] T035-021 [P] Write the failing telemetry tests in `crates/telemetry/src/metrics.rs`: every label
  of the plan's Observability metrics takes values only from its closed set, and none takes a
  connection, table, run or tenant identifier. Refs: FR-035-030, NFR-035-OBS.

### Implementation

- [ ] T035-022 Implement `egressCeiling` on every connection variant in `crates/domain/src/database.rs`,
  and `classification::within` with the stricter-of rule in `crates/domain/src/classification.rs`.
  Add no new domain dependency. T035-009 and T035-010 pass. Refs: FR-035-005, FR-035-025, D035-040,
  D035-051.
- [ ] T035-023 Add the permission and the audit actions:
  - `Permission::DatabaseBrowse` and its role tables in `crates/domain/src/identity.rs`;
  - the three audit actions in `crates/domain/src/audit.rs`;
  - `database:browse` in the `AuthContext` of `crates/api-http/src/routes/identity.rs`;
  - the browse operation in `crates/application/src/api_admission.rs`.

  T035-011 passes. Refs: FR-035-024, D035-066.
- [ ] T035-024 Add the contracts/errors.md codes to `DatabaseFailure` in
  `crates/domain/src/database/failure.rs`. Add the publication refusal reasons to
  `crates/api-http/src/error.rs`, and the catalog entries to `sdks/error-catalog.json` and every
  per-SDK copy (Go, .NET, Java and the others its equality test names). T035-012 and the copy-equality
  tests pass. Refs: FR-035-029, D035-042.
- [ ] T035-025 Write `crates/infra-postgres/migrations/20270115000000_lake_destinations.{up,down}.sql`
  with the `/migration` checklist, covering data-model.md §3:
  - `steps.kind` accepts `DESTINATION`;
  - `waiting_kind` accepts `DELIVERY`;
  - `webhook_subscriptions.purpose` (`RUN_EVENTS` default | `STEP_OUTPUT`) and nullable
    `egress_ceiling`;
  - `webhook_deliveries` gains kind `STEP_OUTPUT` and nullable `run_id`, `step_key` and
    `payload_digest`, indexed on `(tenant_id, run_id, step_key)` where not null;
  - `worker_bindings.profile` (`database` | `lake`);
  - run origin `CATALOG_BROWSE`;
  - the quota resources;
  - the `sagat_app` grants.

  Update `crates/infra-postgres/storage-classification.json` as data-model.md §6 says. T035-013 passes.
  Refs: D035-044, D035-059.
- [ ] T035-026 Implement profile binding:
  - persist `profile` in the worker-binding store of `crates/infra-postgres/src/`;
  - in `crates/application/src/worker/database.rs`, `managed_capabilities_allowed` accepts only the
    profile's activities, and only under the negotiated minor;
  - `ResolveDatabase` in `crates/api-grpc/src/service.rs` refuses a kind outside the profile;
  - `apps/database-runner/src/{profile.rs,config.rs,runner.rs}` offer activities by the agreed minor.

  T035-014 passes. Refs: D035-057.
- [ ] T035-027 [P] Separate the lake build:
  - a `lake` cargo feature in `apps/database-runner/Cargo.toml`;
  - a new workspace crate `crates/infra-lake/`, the only crate that enables `object_store` 0.14.2
    `gcp` and `azure`;
  - an `infra/docker/Dockerfile` target `lake-runner`, built in its own
    `cargo build --features lake` invocation, with `USER 10001` and a read-only root.

  In `scripts/check-architecture.py`, run `cargo tree -e features` for `apps/control-plane` and
  `apps/execution-engine`, and fail if `infra-lake`, `object_store/gcp` or `object_store/azure`
  appears. Refs: D035-058, constitution II.
- [ ] T035-028 Implement the `lake` container profile in `crates/infra-sandbox/src/docker.rs`:
  - own user, pid, mount and network namespaces, with loopback only;
  - a distinct UID per slot;
  - a read-only root, all capabilities dropped, `no-new-privileges` and the default seccomp profile;
  - core dumps off;
  - `memory.max`, `pids.max`, a CPU quota and a deadline;
  - mounts limited to `/opt/sagat/lake` (read-only) and a size-limited `/work/<attempt-id>`.

  In `apps/sandbox-runner/src/{api.rs,config.rs}`, add the start, stop and kill API. A start request
  carries no credential. A lake sandbox runner serves only lake containers and admits at most the
  "lake containers per node" ceiling. Refs: FR-035-023, D035-048, ADR-0013.
- [ ] T035-029 Run spike T-isolation. Choose the co-location of the lake runner and the sandbox runner
  (one pod sharing an `emptyDir`, or a node-local socket) in Compose and in
  `infra/helm/templates/lake-runner.yaml`. Run T035-015 under both, the Helm one in the Kubernetes
  validation profile. Record the result and the evidence in research.md, closing G035-022, and in
  `docs/architecture/security-and-sandbox.md`.

  If no co-location passes, stop and ask the owner to choose between a node-local deployment and a
  per-attempt Kubernetes Job (plan.md Risks). Refs: G035-022, D035-048.
- [ ] T035-030 Implement the relay in `crates/infra-lake/src/relay.rs`:
  - an authenticated Unix socket at `/work/<attempt-id>/relay.sock`;
  - a per-attempt token at `relay.token`, mode 0400, owned by the container's UID;
  - `CONNECT` policy over `crates/infra-database/src/network.rs`;
  - TCP mappings;
  - a denial count returned with the result.

  Add the IPv6 metadata addresses to `network.rs`, and the in-container forwarder in
  `runners/lake/sagat_lake/forwarder.py` with pytest. T035-016 passes. Refs: D035-049, D035-061.
- [ ] T035-031 Write the relay canaries for the I1 clients in
  `tests/integration/iceberg-writes/relay_canaries.rs`. The PyIceberg REST client and the S3 FileIO,
  run inside a lake container, work through the relay, and every other route fails. Record the result
  against G035-018 in research.md. Refs: G035-018, D035-049.
- [ ] T035-032 Implement the `sagat.lake.v1` codec in `crates/infra-lake/src/protocol.rs`, and the host
  skeleton `runners/lake/sagat_lake/host.py`:
  - `PR_SET_DUMPABLE=0` is its first action;
  - it authenticates on the relay;
  - it reads one request line, including credentials served over the relay;
  - it writes heartbeats and one result line, then exits 0.

  Add the pytest `runners/lake/tests/test_host_protocol.py`. T035-017 passes. Refs: D035-062, D035-067.
- [ ] T035-033 Implement configuration hygiene in `runners/lake/sagat_lake/config_hygiene.py`:
  - an allowlist of `warehouse`, `prefix` and the declared storage type's vended credential keys;
  - `py-io-impl` and the FileIO implementation pinned;
  - remote signing refused unless declared;
  - `AWS_EC2_METADATA_DISABLED=true`;
  - GCS and Azure credentials passed explicitly, failing closed when missing.

  T035-018 passes. Refs: D035-050.
- [ ] T035-034 Implement the conditional commit in `runners/lake/sagat_lake/commit.py`: set
  `commit.retry.num-retries = 0` in memory on the loaded table, and assert `assert-table-uuid` and
  `assert-ref-snapshot-id(ref, H)`. If T035-019 shows the override has no effect, build the updates and
  call the catalog's commit directly. Record the result in research.md, closing G035-020. Refs:
  D035-030, G035-020.
- [ ] T035-035 Implement the lake attempt driver in `apps/database-runner/src/lake.rs`:
  - create `/work/<attempt-id>/`;
  - stream inline records, `ReadBlob` blobs and 017 results into `in/`, checking the effective limit
    while streaming; going over fails `ICEBERG_LIMIT_EXCEEDED` before any container starts;
  - open the relay and request the container;
  - serve the request line;
  - map heartbeats to lease heartbeats;
  - kill the container on lease loss, cancellation, deadline, a missing heartbeat or `memory.max`;
  - parse the result and rebuild the receipt;
  - zeroise the credentials and remove the directory;
  - fail `LAKE_SANDBOX_UNAVAILABLE` when the sandbox runner is unreachable.

  Refs: FR-035-009, FR-035-023, contracts/lake-runtime.md "One attempt".
- [ ] T035-036 [P] Add the `lake.enabled` setting, off by default, to the control-plane and
  execution-engine configuration. Check it in `crates/application/src/publishing.rs` and at run
  admission in `crates/application/src/runs.rs`. T035-020 passes. Refs: D035-059.
- [ ] T035-037 [P] Register the metric families of plan.md "Observability" in
  `crates/telemetry/src/metrics.rs`. T035-021 passes. Refs: FR-035-030.

**Checkpoint**: G035-018 (REST and S3 clients), G035-020 and G035-022 are closed with evidence in
research.md. User stories can start.

---

## Phase 3: User Story 1 — Register an Iceberg connection and test it (Priority: P1), REST + S3 (I1)

**Goal**: an administrator registers an Iceberg connection (REST catalog, S3-compatible storage) with
OpenBao-backed secrets, allowed modes and a ceiling, and tests it with a fixed probe.

**Independent Test**: create a REST + S3 connection against the fixtures, run the probe, reload the
form, and check that no credential value ever comes back (quickstart scenario 1). Phase 6 extends the
test to every catalog and storage type.

### Tests for User Story 1

- [ ] T035-038 [P] [US01] Write the failing tests in `crates/domain/tests/database_connections.rs` for
  every Iceberg configuration rule of data-model.md §1, quoted:
  - `catalog.type` is "`rest` | `glue` | `hive` | `jdbc`";
  - REST `uri` is "HTTPS outside development";
  - Hive `uris` are "1–4 entries of `thrift://host:port`", with `tls: none` "only when the deployment
    sets `allowUnauthenticatedMetastore`";
  - JDBC `engine` is "`postgresql` (others refused in this delivery)";
  - `storage.credentials: vended` is "REST with `vended-credentials` only", and "no `storageSecret`
    when credentials are `vended`";
  - `warehouseLocations` are "1–16 URI prefixes (`s3://`, `gs://`, `abfss://`)";
  - `allowedModes` is "a subset of `append`, `overwrite`, `upsert`; default `[append]`";
  - unknown settings are refused;
  - a URI with user info or query credentials fails `VALIDATION_FAILED`, reason `CREDENTIAL_IN_URI`.

  Refs: FR-035-001..005.
- [ ] T035-039 [P] [US01] Write the failing tests in `tests/integration/iceberg-writes/connections.rs`:
  - creating a REST + S3 connection with a new OAuth2 secret stores the value in OpenBao, and the
    connection keeps only references;
  - reload and edit return metadata only;
  - `GET /database-drivers` lists `iceberg` with its catalog and storage types and modes;
  - a Lakekeeper connection with vended credentials needs no storage secret, and vended values never
    appear in the store, logs, audit or responses;
  - another tenant's connection is `404`;
  - a paused connection admits no probe or run;
  - a `pii` ceiling needs the reveal grant;
  - every change is audited as `DatabaseConnectionChanged`.

  Refs: FR-035-001..005, FR-035-026, SC-035-001.
- [ ] T035-040 [P] [US01] Write the failing tests in `tests/integration/iceberg-writes/probe.rs`:
  - the probe runs in the managed executor;
  - it reports `CATALOG_AUTH`, `WAREHOUSE` and `STORAGE_ACCESS` with the tested revision and time;
  - a wrong secret, an untrusted TLS certificate, a host outside the allowed networks, an unavailable
    OpenBao and unreachable storage each give a distinct typed result with no provider text;
  - the probe writes no business data;
  - a write outside the prefix that succeeds yields a warning (G035-021);
  - a result arrives in under 30 s on the fixtures.

  Refs: FR-035-006, SC-035-001.

### Implementation for User Story 1

- [ ] T035-041 [US01] Add `ConnectionConfig::Iceberg`, with every catalog and storage shape, to
  `crates/domain/src/database.rs`. T035-038 passes. Refs: FR-035-001..005, D035-040.
- [ ] T035-042 [US01] Validate and save the new kind in `crates/application/src/database.rs`. A
  revision is audited, and a `pii` ceiling requires a current reveal grant inside T1. Refs: FR-035-005,
  FR-035-024.
- [ ] T035-043 [US01] Expose the kind over REST:
  - the `oneOf` config with a `driver` discriminator in `crates/api-http/src/dto_database.rs`;
  - `GET /database-drivers` in `crates/api-http/src/routes/database.rs`;
  - the reasons `UNKNOWN_CONNECTION_KIND` and `CREDENTIAL_IN_URI`.

  Refs: FR-035-001, FR-035-002.
- [ ] T035-044 [US01] Resolve the Iceberg connection. `ResolvedDatabase.iceberg` is built in
  `crates/api-grpc/src/service.rs` and `crates/application/src/worker/database.rs`. `catalogSecret` and
  `storageSecret` are each compared, pinned and audited separately (017 two-phase), and only the lake
  profile may resolve the kind. Refs: FR-035-003, FR-035-026, D035-057.
- [ ] T035-045 [US01] Implement the probe activity `sagat.database.lake.probe`:
  - wire it in `apps/database-runner/src/lake.rs`;
  - write the host operation in `runners/lake/sagat_lake/probe.py`, with pytest: authenticate, load
    the warehouse configuration, list one namespace page and try one write outside the prefix;
  - admit it through 017's fixed-probe admission.

  T035-039 and T035-040 pass. Refs: FR-035-006, G035-021.
- [ ] T035-046 [P] [US01] Add the Iceberg connection to the Studio, in
  `frontend/apps/studio/src/features/connections/{ConnectionForm.tsx,ConnectionProbe.tsx,api.ts}`:
  - an Iceberg form with REST and S3 sections;
  - 017's inline OpenBao secret create and select;
  - warehouse locations and allowed modes;
  - the ceiling, with `pii` disabled when the actor has no reveal grant;
  - the vended-credentials choice;
  - per-check probe results; saving is never shown as proof of connectivity.

  Add the vitest cases to `ConnectionsPage.test.tsx`: no credential value after a reload. Refs:
  FR-035-001..006, SC-035-001.

**Checkpoint**: US01 on REST + S3 works and is tested on its own.

---

## Phase 4: User Story 2 — Append workflow records to a table, never twice (Priority: P1) 🎯 MVP (I1)

**Goal**: an `icebergWrite` step appends records from inline values, blobs or a 017 result, as one
commit carrying the effect key, and recovers by lookup after any crash.

**Independent Test**: quickstart scenario 2. Append 10,000 records, kill the executor at each critical
point, and check with the independent reader that the rows are there exactly once and that exactly
one commit carries the key.

### Tests for User Story 2

- [ ] T035-047 [P] [US02] Write the failing domain tests in `crates/domain/tests/iceberg_write.rs`:
  - `Node.iceberg` parses;
  - it is refused inside forEach, while, repeatUntil, scopes and continuation combinations
    (`ICEBERG_NODE_NOT_ROOT`);
  - an ordinary activity node cannot forge `sagat.database.iceberg.write`;
  - `records` holds exactly one of `inline`, `blobs` or `databaseResult`, with at most 32 blobs;
  - mapped columns and nesting stay within their ceilings;
  - the step is `high_risk` and irreversible unless it declares `compensation: revert`.

  Refs: FR-035-008, FR-035-009, FR-035-034, G035-019.
- [ ] T035-048 [P] [US02] Write the failing publication tests in
  `tests/integration/iceberg-writes/publication.rs`:
  - publishing needs `database:use` and `database:write` (admin);
  - a developer is refused as if the connection were missing, and the refusal is audited;
  - an `upsert` step on an append-only connection fails `MODE_NOT_ALLOWED` (US01 scenario 3);
  - `CLASSIFICATION_EGRESS_REFUSED` is raised for a `pii` field against an `internal` ceiling, for any
    `secret` field, and for undeclared fields against a ceiling below `pii`;
  - publication never contacts the catalog.

  Refs: FR-035-005, FR-035-024, FR-035-025, SC-035-007.
- [ ] T035-049 [P] [US02] Write the failing host tests in `runners/lake/tests/`:
  - `test_lookup.py`, the D035-030 walk:
    - found;
    - absent and conclusive;
    - inconclusive when the chain is broken;
    - reaching the first snapshot;
    - the 5-minute clock margin;
    - more than 10,000 snapshots is inconclusive;
    - the key found on another ref is a failure for review.
  - `test_mapping.py`, the lossless mapping table, naming the column and reason and never the value:
    - refused: decimal scale overflow, integer overflow, a timestamp without zone into a zoned column,
      NaN into a non-floating column, and a string into binary;
    - nested struct, list and map map recursively;
    - a missing optional column becomes null;
    - a missing required column is refused;
    - an unknown field is refused unless `ignoreUnknownFields` is set; then it is dropped and counted.
  - `test_compat.py`, all refused before any file: v3, a non-Parquet default format, an unsupported
    transform, a location outside `warehouseLocations`, metadata above `maxMetadataBytes`, and a
    missing ref.
  - `test_append.py`:
    - streamed batches;
    - an empty batch makes no commit and the receipt says so;
    - the decoding bounds hold (D035-063).

  Refs: FR-035-010..014, FR-035-018, FR-035-019.
- [ ] T035-050 [P] [US02] Write the failing tests in `tests/integration/iceberg-writes/append.rs`:
  - a 10,000-record blob append makes one snapshot on the target ref;
  - its summary holds exactly `sagat.effect-key`, `sagat.run-id`, `sagat.step-id`, `sagat.workflow` and
    `sagat.workflow-version`;
  - the independent reader sees exactly 10,000 rows, and the receipt fields match receipts.schema.json;
  - a non-`main` target ref works;
  - inline and 017 database-result sources work;
  - a conflict with an external writer is retried within the bound, and both writers' rows exist once;
  - an `amount` scale error is typed;
  - a `pii` field fails at execution with zero relay connections;
  - going over a limit fails before the container starts;
  - a missing table or ref is typed;
  - an added column passes, a dropped mapped column fails, and the schema id is recorded;
  - the receipt carries no file locations.

  Refs: FR-035-009..014, FR-035-027, SC-035-002.
- [ ] T035-051 [P] [US02] Write the failing fault tests in `tests/fault/iceberg-writes/append.rs`, each
  held with a barrier at the relay or in the runner:
  - a kill before the commit request;
  - a kill after the request, before the response;
  - a kill after the response, before `CompleteStep`;
  - a stale lease racing its successor (at most one commit);
  - a catalog 5xx or timeout on commit;
  - snapshot expiry inside the window, giving `OUTCOME_UNKNOWN` and `WAITING(OPERATOR)` with the lookup
    evidence;
  - a timeout racing a cancellation;
  - cancellation before the commit request (nothing committed) and after it (looked up, never
    reported as cancelled);
  - a `memory.max` kill and a missing heartbeat (retryable);
  - the sandbox runner down;
  - OpenBao sealed;
  - the connection paused or the credential revoked mid-run.

  Every run ends with exactly one keyed commit or `OUTCOME_UNKNOWN`, and zero duplicate rows. Refs:
  FR-035-018..020, FR-035-022, SC-035-003.

### Implementation for User Story 2

- [ ] T035-052 [US02] Add the `IcebergWrite` type and `Node.iceberg` to
  `crates/domain/src/{database.rs,workflow.rs}`, with root-only validation and the reserved activity.
  T035-047 passes. Refs: FR-035-008, D035-038.
- [ ] T035-053 [US02] Add the codec in `crates/infra-ir/src/lake.rs` and `crates/infra-ir/src/convert.rs`.
  A field outside the contract is refused, and canonical bytes are stable across two publications.
  Refs: FR-035-008, FR-035-033.
- [ ] T035-054 [US02] Add the publication checks in
  `crates/application/src/{definitions.rs,publishing.rs,database.rs}`:
  - access;
  - allowed modes;
  - egress through `within`, using the stricter of the source `outputSchema` and the `recordSchema`;
  - root-only placement;
  - `lake.enabled`.

  T035-048 passes. Refs: FR-035-005, FR-035-024, FR-035-025.
- [ ] T035-055 [US02] Pin admission (T3) in `crates/application/src/runs.rs` and
  `crates/infra-postgres/src/database.rs`:
  - connection pins on direct, child, event, rerun and ad hoc paths;
  - quota rows for concurrent lake attempts per namespace and tenant (defaults 8 and 32, from the
    plan's ceilings).

  Refs: FR-035-024, FR-035-028.
- [ ] T035-056 [US02] Build the attempt input in `crates/application/src/worker/database.rs`:
  - `effectKey`;
  - `firstAttemptStartedAt` from the database clock;
  - `snapshotProperties`;
  - the record references.

  Credentials never travel in the lease. Refs: FR-035-013, FR-035-018, contracts/worker-protocol.md.
- [ ] T035-057 [US02] Implement the host's append path in
  `runners/lake/sagat_lake/{lookup.py,mapping.py,compat.py,modes/append.py}`:
  - lookup and commit follow D035-030;
  - the host deletes its own files when a commit is definitively refused;
  - `priorAttemptMayHaveLeftFiles` is set after a crash.

  T035-049 passes. Refs: FR-035-010..014, FR-035-018, FR-035-020, D035-031.
- [ ] T035-058 [US02] Add the activity `sagat.database.iceberg.write` to
  `apps/database-runner/src/lake.rs`: rebuild the receipt, report `CompleteStep` or `FailStep`, and
  audit relay denials. Refs: FR-035-023, FR-035-027.
- [ ] T035-059 [US02] Complete the step on the server, in
  `crates/application/src/worker/output.rs` and `crates/application/src/runs.rs`:
  - T6 stores the receipt as output, writes the journal, sets successor readiness and the outbox;
  - `OUTCOME_UNKNOWN` goes to `WAITING(OPERATOR)` with the lookup evidence (target ref, window
    searched, key found elsewhere);
  - `RETRY_STEP` and `RESOLVE_STEP` on it need `saga:override`.

  T035-050 and T035-051 pass. Refs: FR-035-019, D035-069, constitution III.
- [ ] T035-060 [US02] Add ad hoc Iceberg writes. Write the failing test
  `tests/integration/iceberg-writes/ad_hoc.rs` first: the same lookup, refusals and unknown outcomes as
  a published step. Then implement `crates/application/src/runs/ad_hoc.rs`
  (`POST /namespaces/{ns}/database-executions`, which needs `database:adhoc` and `database:write`).
  Refs: FR-035-031, SC-035-012.
- [ ] T035-061 [US02] Add the observability and its canaries:
  - spans `lake.attempt`, `lake.sandbox`, `lake.load`, `lake.lookup`, `lake.write` and `lake.commit`,
    with the plan's attributes;
  - the `lake_*` metrics;
  - audit once per committed write;
  - a failing-first canary test in `tests/integration/iceberg-writes/canaries.rs`: no value, partition
    value, file location, credential or host error text in logs, spans, audit, journal, receipt or API.

  Refs: FR-035-030, NFR-035-SEC.
- [ ] T035-062 [US02] Add the example `examples/iceberg/orders-append.workflow.json` and the e2e test
  `tests/e2e/iceberg-writes/append.rs` (happy path and a recovered crash, quickstart scenario 2).
  Refs: FR-035-032, SC-035-003.

**Checkpoint**: the MVP. An append on REST + S3 is exactly-once-or-unknown, proven with the
independent reader.

---

## Phase 5: User Story 3 — Author the same write in the Studio and in every SDK (Priority: P1) (I1)

**Goal**: catalog browsing, a Studio editor for `icebergWrite`, and builders in the five SDKs that emit
the same canonical definition.

**Independent Test**: quickstart scenario 6 (`SAGAT_SDK_SCENARIO=iceberg-writes`) and the Studio
journey publish identical definitions and get equal receipts and errors.

### Tests for User Story 3

- [ ] T035-063 [P] [US03] Write the failing browse tests in `tests/integration/iceberg-writes/browse.rs`:
  - developer and admin may browse; viewer and operator may not; a foreign connection is `404`;
  - levels `NAMESPACES`, `TABLES` and `TABLE`, with pages of at most 200 (default 100);
  - 30 browses per actor per minute;
  - `Cache-Control: no-store`;
  - only the actor who started a browse reads it, and anyone else gets `404`;
  - a same-key replay is keyed by tenant, namespace and actor;
  - `CATALOG_BROWSE` runs are absent from run lists and reads for every role;
  - results expire 10 minutes after completion;
  - `DatabaseCatalogBrowsed` is audited without naming tables beyond the requested level;
  - no rows or file locations are returned.

  Refs: FR-035-007, D035-065.
- [ ] T035-064 [P] [US03] Write the failing conformance scenario
  `tests/contract/sdk-conformance/scenarios/iceberg-writes.json`, with its files under
  `tests/contract/sdk-conformance/expected/`:
  - the step built by each SDK gives byte-identical canonical documents;
  - receipts and errors are equal;
  - an unsupported capability fails explicitly.

  Refs: FR-035-008, FR-035-033, SC-035-005.
- [ ] T035-065 [P] [US03] Write the failing vitest
  `frontend/apps/studio/src/features/workflows/IcebergWriteFields.test.tsx`:
  - pick a connection and a table, and see the schema;
  - map fields and choose the mode;
  - a warning appears when a mapped column is missing from the last-read schema;
  - the snapshot-summary visibility notice is shown;
  - refusal looks like a missing connection.

  Refs: FR-035-013, US03 scenarios 1–3.

### Implementation for User Story 3

- [ ] T035-066 [US03] Implement browsing:
  - admission with origin `CATALOG_BROWSE` in `crates/application/src/lake.rs`;
  - routes in `crates/api-http/src/routes/catalog_browse.rs`;
  - run-list and run-read exclusion in `crates/application/src/runs.rs`;
  - the activity `sagat.database.lake.browse`;
  - the host operation `runners/lake/sagat_lake/browse.py`, with pytest.

  T035-063 passes. Refs: FR-035-007, D035-065.
- [ ] T035-067 [P] [US03] Add the TypeScript builder in `sdks/typescript/src/database.ts`, with unit
  tests in `sdks/typescript/test/`. Refs: FR-035-008.
- [ ] T035-068 [P] [US03] Add the Python builder in `sdks/python/sagat_octopus_orchestrator/database.py`,
  with tests in `sdks/python/tests/`. Refs: FR-035-008.
- [ ] T035-069 [P] [US03] Add the Go builder in `sdks/go/workflow/database.go`, with tests. Refs:
  FR-035-008.
- [ ] T035-070 [P] [US03] Add the Java builder
  `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/IcebergWriteOptions.java` and
  its use from `Flow.java` and `WorkflowBuilder.java`, with tests. Refs: FR-035-008.
- [ ] T035-071 [P] [US03] Add the C# builder
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/IcebergWriteOptions.cs` and its use from
  `FlowDatabase.cs`, with tests. Refs: FR-035-008.
- [ ] T035-072 [US03] Wire the conformance runners:
  - `sdks/typescript/conformance/runner.ts`;
  - `sdks/python/conformance/runner.py`;
  - `sdks/go/conformance/build.go`;
  - `sdks/java/conformance/src/main/java/dev/sagatoctopusorchestrator/conformance/Runner.java`;
  - `sdks/dotnet/conformance/BuildMode.cs`;
  - and `tests/contract/sdk-conformance/database.rs`.

  Run all five. T035-064 passes. Refs: SC-035-005.
- [ ] T035-073 [US03] Build the Studio editor:
  - `frontend/apps/studio/src/features/workflows/{IcebergWriteFields.tsx,NodeInspector.tsx,AddNode.tsx}`;
  - `frontend/apps/studio/src/features/connections/CatalogPicker.tsx`, which shows the schema,
    partition spec, identifier fields and format version.

  T035-065 passes. Refs: FR-035-007, FR-035-013.
- [ ] T035-074 [US03] Write the Playwright journey `frontend/e2e/tests/iceberg-write.spec.ts` and its
  accessibility checks. Rerun `npm run check:api`. Refs: SC-035-005.

**Checkpoint**: I1 is complete: US01, US02 and US03 on REST + S3.

---

## Phase 6: User Story 1, continued — every catalog and storage type (Priority: P1) (I2)

**Goal**: Glue, Hive and JDBC catalogs; GCS and ADLS storage; vended credentials. Each is verified
against a declared fixture, and emulator-only paths are labelled.

**Independent Test**: the US01 test, repeated for every combination the fixtures support. Each
combination is probed, appended to once, and read back with the independent reader.

- [ ] T035-075 [P] [US01] Write the failing matrix in `tests/integration/iceberg-writes/catalogs.rs`:
  - catalogs: REST, Lakekeeper with vended credentials, Glue on moto, Hive with TLS and with
    `allowUnauthenticatedMetastore`, and JDBC on the repository's PostgreSQL;
  - storages: RustFS, fake-gcs-server and Azurite's blob endpoint;
  - a JDBC catalog that points at the platform's own database is refused by the network policy.

  Refs: FR-035-002, FR-035-003, SC-035-010.
- [ ] T035-076 [P] [US01] Add relay canaries for boto3 Glue, Thrift Hive, psycopg JDBC, gcsfs and adlfs
  to `tests/integration/iceberg-writes/relay_canaries.rs`. Record the rest of G035-018 in research.md.
  Refs: G035-018.
- [ ] T035-077 [US01] Implement the non-HTTP routes and the remaining clients:
  - TCP mappings in `crates/infra-lake/src/relay.rs` and `runners/lake/sagat_lake/forwarder.py`;
  - TLS name verification: psycopg `host=<name> hostaddr=127.0.0.1 sslmode=verify-full`, and Thrift
    TLS `server_hostname`;
  - the Glue, Hive and JDBC catalogs in `runners/lake/sagat_lake/catalogs.py`;
  - GCS and ADLS FileIO with explicit credentials in `runners/lake/sagat_lake/storage.py`.

  T035-075 and T035-076 pass. Refs: FR-035-002..004, D035-049.
- [ ] T035-078 [US01] Add the Glue, Hive, JDBC, GCS and ADLS sections to
  `frontend/apps/studio/src/features/connections/ConnectionForm.tsx`, with vitest cases. Refs:
  FR-035-002, FR-035-003.
- [ ] T035-079 [US01] Add the "verified against" table to `docs/guides/iceberg-writes.md`:
  - Glue conflicts cannot be tested locally (G035-012);
  - ADLS is tested through the blob endpoint only (G035-013);
  - RustFS conditional create is decided by the probe (G035-014).

  Refs: FR-035-032, SC-035-010.

**Checkpoint**: FR-035-002 and FR-035-003 are covered in full.

---

## Phase 7: User Story 4 — Replace a slice of a table (overwrite) (Priority: P2) (I3)

**Goal**: overwrite with three scopes (`batchPartitions`, `filter`, `table`), in one commit, isolated
against concurrent writers.

**Independent Test**: quickstart scenario 3, step 1. Write a slice twice and check that only the
second remains, that rows outside the scope are unchanged, and that a concurrent reader sees one whole
version.

- [ ] T035-080 [P] [US04] Write the failing domain and IR tests in
  `crates/domain/tests/iceberg_write.rs`:
  - `overwrite {scope, filter?, allowEmpty}`, where `scope` is `batchPartitions`, `filter` or `table`;
  - filter parameters are typed and bound to workflow values;
  - a filter deeper than 8 levels or with more than 64 comparisons fails `FILTER_TOO_DEEP`.

  Refs: FR-035-015, FR-035-028.
- [ ] T035-081 [P] [US04] Write the failing host and integration tests
  (`runners/lake/tests/test_overwrite.py`, `tests/integration/iceberg-writes/overwrite.rs`):
  - a batch for two partitions replaces exactly those two;
  - a row outside the filter fails `ICEBERG_SCOPE_VIOLATION` before any write;
  - an empty batch without `allowEmpty` fails;
  - scope `batchPartitions` on an unpartitioned table is refused at execution (see the note under
    "Findings from task generation");
  - a concurrent writer inside the scope causes a re-read and re-validation, and otherwise
    `ICEBERG_COMMIT_CONFLICT`, never deleting an unseen row;
  - a concurrent reader sees one whole version;
  - after a lost report, the lookup prevents a second overwrite;
  - an evolved partition spec is handled as the contract states.

  Refs: FR-035-015, FR-035-017, SC-035-002, SC-035-004.
- [ ] T035-082 [US04] Implement `runners/lake/sagat_lake/modes/overwrite.py`:
  - scope `filter`: check every row, then `Table.overwrite(df, overwrite_filter=…)`;
  - scope `batchPartitions`: `dynamic_partition_overwrite`;
  - scope `table`: `AlwaysTrue()`.

  Add the conflict validation to `commit.py`, the overwrite fields to the domain, IR and publication,
  and the 256 MiB default input. T035-080 and T035-081 pass. Refs: FR-035-015, FR-035-017, D035-046.
- [ ] T035-083 [US04] Add the overwrite options to the Studio (`IcebergWriteFields.tsx`) and the five
  SDK builders. Extend `iceberg-writes.json` and rerun all five. Refs: FR-035-008, SC-035-005.
- [ ] T035-084 [US04] Add the example `examples/iceberg/daily-overwrite.workflow.json`, run from the
  e2e target. Refs: quickstart scenario 3.

**Checkpoint**: US04 works on its own on top of US02.

---

## Phase 8: User Story 5 — Upsert records by key (Priority: P2) (I3)

**Goal**: copy-on-write upsert by the table's identifier fields or declared key columns, with an
optional delete marker, in one commit.

**Independent Test**: quickstart scenario 3, steps 2 and 3. After an upsert, each key has exactly one
live row holding the latest values, and untouched keys are unchanged.

- [ ] T035-085 [P] [US05] Write the failing domain and IR tests in
  `crates/domain/tests/iceberg_write.rs`:
  - `upsert {keyColumns?, deleteMarker?}`;
  - the delete marker is a boolean record field and not a mapped column.

  Refs: FR-035-016.
- [ ] T035-086 [P] [US05] Write the failing host and integration tests
  (`runners/lake/tests/test_upsert.py`, `tests/integration/iceberg-writes/upsert.rs`):
  - 3 existing and 2 new keys give one commit and one live row per key;
  - declared key columns must be required columns, otherwise the step is refused;
  - a duplicate key, marked or not, fails `ICEBERG_DUPLICATE_KEY` and commits nothing;
  - a v3 table is refused before any file;
  - a delete marker gives 2 replaced and 1 deleted, a marked key that is absent counts as not found,
    and only the key fields of a marked record are written;
  - the receipt says `rowLevelMode: COPY_ON_WRITE`, also for a table that prefers merge-on-read;
  - T-upsert-one-commit: exactly one commit.

  Refs: FR-035-016, D035-032, SC-035-002.
- [ ] T035-087 [P] [US05] Write the failing concurrency tests in
  `tests/fault/iceberg-writes/concurrency.rs`:
  - with 2–8 writers (platform runs and a direct PyIceberg writer), the final table equals some serial
    order of the commits;
  - no key ends with two live rows;
  - no overwrite removes a row outside its scope;
  - an executor killed after writing data and delete files, before the commit, leaves nothing
    visible, and the retry starts from a fresh base.

  Refs: FR-035-017, FR-035-020, SC-035-004.
- [ ] T035-088 [US05] Implement `runners/lake/sagat_lake/modes/upsert.py`:
  - split the batch by the marker;
  - stage the upsert and the deletion in one `Table.transaction()`, or compose
    `overwrite`/`delete`/`append` when the transaction has no upsert;
  - apply the 6 GiB `memory.max` for upsert.

  T035-085..087 pass. Refs: FR-035-016, FR-035-017, contracts/lake-runtime.md "Modes".
- [ ] T035-089 [US05] Add the upsert options to the Studio and the five SDK builders, extend the
  conformance scenario, and add the example `examples/iceberg/customers-upsert.workflow.json`. Refs:
  FR-035-008, SC-035-005.

**Checkpoint**: US05 works on its own on top of US02 and US04's conflict validation.

---

## Phase 9: User Story 7 — A finished payment goes to the lake and to PostgreSQL inside the run (Priority: P2) (I4)

**Goal**: synchronous destinations derived at parse as `<source>~d.<name>`, scheduled by the
completion transaction, with failure policies, for the Iceberg and PostgreSQL kinds.

**Independent Test**: quickstart scenario 4, Iceberg and PostgreSQL part. Each destination holds the
payment once, *send receipt* starts only after both writes, and the run graph shows the two derived
steps.

### Tests for User Story 7

- [ ] T035-090 [P] [US07] Write the failing domain tests in `crates/domain/tests/destinations.rs`:
  - at most 8 destinations per node (default 4), with unique names;
  - allowed only on root-level nodes that produce an output, never on derived nodes or `icebergWrite`
    (`DESTINATION_NOT_ROOT`);
  - derived at parse, before region analysis: id `<source>~d.<name>`,
    `NodeOrigin::Destination { source, name }`, and edges source → each derived node → the original
    successors;
  - the derived nodes are in the source's saga region, and parallel joins see them;
  - they are never stored;
  - `instance_destination_key = <source key>~d.<name>`;
  - `records` is `output`, an array path, `blobs` or `inline`;
  - `onFailure` defaults to `hold`.

  Refs: FR-035-034, FR-035-035, D035-054.
- [ ] T035-091 [P] [US07] Write the failing tests in `tests/integration/step-destinations/`:
  - `scheduling.rs`:
    - T5 schedules one destination step per destination, in the same transaction;
    - a stale, failed or cancelled completion schedules nothing;
    - successors wait for `SUCCEEDED` or `SKIPPED`;
    - destinations run in parallel.
  - `classification.rs`: mapping `cardHolder` (`pii`) to a PostgreSQL connection with an `internal`
    ceiling is refused at publication; mapping only `amount` publishes; the check runs again at
    execution.
  - `postgresql.rs`:
    - the key must be covered by a primary key or a non-partial unique constraint, otherwise
      `DESTINATION_KEY_CONSTRAINT_MISSING`;
    - `effectKeyColumn` receives the key;
    - at most 1,000 rows per statement, in one transaction;
    - the default of 10,000 rows per write holds;
    - `DESTINATION_SCHEMA_MISMATCH` names the column;
    - identifiers are quoted only after an exact match.
  - `iceberg.rs`: an `items` array of 500 makes one Iceberg commit and one database transaction.
  - `policies.rs`:
    - `hold` waits in `WAITING(OPERATOR)`; `RETRY_STEP` (`run:operate`) makes it `READY`;
    - a skip (`saga:override`) makes it `SKIPPED`, successors run, and the decision is audited;
    - `fail` makes it `FAILED`, an ordinary forward failure in the saga region;
    - `continue` makes it terminal `SKIPPED` with the failure receipt;
    - an ad hoc redrive (`database:adhoc` + `database:write`) uses the same key and cannot duplicate;
    - `OUTCOME_UNKNOWN` needs `saga:override` to retry or resolve;
    - the source step is never re-executed;
    - receipts are readable as the source's destination results, and the source output is unchanged.

  Refs: FR-035-035..039, FR-035-043, FR-035-045.
- [ ] T035-092 [P] [US07] Write the failing fault tests in
  `tests/fault/step-destinations/{postgresql.rs,iceberg.rs}`:
  - a crash after the database commit, before recording, leaves one row after the retry;
  - a source step retried after a lost lease writes each destination once;
  - crashes before and after the external write, and a restart.

  Refs: SC-035-013.

### Implementation for User Story 7

- [ ] T035-093 [US07] Add `Destination`, `Node.destinations` and `NodeOrigin::Destination` to
  `crates/domain/src/workflow.rs`, with derivation at parse beside `derive_companions`, before region
  analysis. T035-090 passes. Refs: D035-054.
- [ ] T035-094 [US07] Add the destination codec to `crates/infra-ir/src/lake.rs`, and the publication
  checks (per-kind permission, ceiling, root-only, count) to `crates/application/src/publishing.rs`.
  Refs: FR-035-034, FR-035-036, FR-035-045.
- [ ] T035-095 [US07] Implement the engine side:
  - T5 scheduling and readiness in `crates/application/src/{worker.rs,worker/output.rs}` and
    `crates/infra-postgres/src/execution.rs`;
  - T7 policies;
  - T8 decisions in `crates/application/src/runs.rs`: `RETRY_STEP` on destination steps, and
    `ReviewAction::Skip` widened from compensation steps to destination steps, with the route mapping
    in `crates/api-http/src/routes/execution.rs`;
  - `DestinationDecided` audit.

  Refs: FR-035-035, FR-035-037, D035-055.
- [ ] T035-096 [US07] Implement the ad hoc destination redrive `{runId, step, destination}` in
  `crates/application/src/runs/ad_hoc.rs`, reusing the pinned definition and the effect key. Refs:
  FR-035-037, D035-055.
- [ ] T035-097 [US07] Implement the PostgreSQL destination:
  - fixed discovery statements and the insert and upsert statements, with chunking, in
    `crates/infra-database/src/destination.rs`;
  - the activity `sagat.database.destination.postgresql` (database profile) in
    `apps/database-runner/src/destinations.rs`.

  Refs: FR-035-039, D035-034.
- [ ] T035-098 [US07] Add the activity `sagat.database.destination.iceberg` (lake profile) to
  `apps/database-runner/src/lake.rs`, reusing the host with the selected record set as its source. Refs:
  FR-035-038.
- [ ] T035-099 [US07] Add destination observability: the span `destination.write{kind}`, the metrics
  `destination_writes_total{kind,outcome}` and `destination_held_total{kind}`, and the held-destination
  alert. Refs: FR-035-030.
- [ ] T035-100 [US07] Add destinations to the Studio:
  - `frontend/apps/studio/src/features/workflows/DestinationsPanel.tsx`: synchronous destinations, and
    020 sinks once 020 is built, each labelled with its mode (FR-035-044);
  - derived steps and permission-gated retry and skip in
    `frontend/apps/studio/src/features/runs/{GraphView.tsx,RunNodeCard.tsx,Operations.tsx}`;
  - vitest cases.

  Refs: FR-035-034, FR-035-044.
- [ ] T035-101 [US07] Add destinations to the five SDK builders, and write the conformance scenario
  `tests/contract/sdk-conformance/scenarios/step-destinations.json`. Refs: FR-035-034, SC-035-005.
- [ ] T035-102 [US07] Add `examples/iceberg/checkout.workflow.json` (the Iceberg and PostgreSQL
  destinations) and `tests/e2e/step-destinations/checkout.rs`. With the lake down, the destination
  holds; after `RETRY_STEP`, the payment step is not re-executed. Refs: SC-035-013, SC-035-014.

**Checkpoint**: the owner's motivating example works for the lake and PostgreSQL. 020's `ICEBERG`
sink (T020-062) can proceed in parallel since I1.

---

## Phase 10: User Story 6 — Undo a write when a saga compensates (Priority: P3) (I3)

**Goal**: `compensation: revert` undoes exactly one step's commit with a new commit, or waits for a
person when a later commit interferes.

**Independent Test**: a saga appends, overwrites or upserts, then fails. The reverted table must equal
the prior table plus everyone else's commits.

- [ ] T035-103 [P] [US06] Write the failing domain tests in `crates/domain/tests/sagas.rs`:
  - `compensate_with` admits `sagat.database.iceberg.revert` only for `icebergWrite` nodes;
  - the derived `~c` uses the source's connection pin;
  - every other reserved activity is still refused.

  Refs: FR-035-021, D035-056.
- [ ] T035-104 [P] [US06] Write the failing host and integration tests
  (`runners/lake/tests/test_revert.py`, `tests/integration/iceberg-writes/revert.rs`):
  - a revert without interference restores the prior content plus other writers' commits;
  - after a compaction or rewrite, or with a restored file gone, the revert fails
    `ICEBERG_REVERT_BLOCKED` and waits for a person, and the run is not marked compensated;
  - with no compensation declared, the run ends `PARTIAL`;
  - after a lost revert response, the lookup by the revert's own key prevents a second revert;
  - the revert never moves the table to an older snapshot.

  Refs: FR-035-021, SC-035-008.
- [ ] T035-105 [US06] Implement revert:
  - `runners/lake/sagat_lake/revert.py`;
  - the activity `sagat.database.iceberg.revert`;
  - the admission rule in `crates/domain/src/workflow.rs`;
  - reuse of T10 (003);
  - the `IcebergRevertRequested` audit;
  - the `compensation: revert` option in the Studio and the five SDKs.

  T035-103 and T035-104 pass. Refs: FR-035-021.

**Checkpoint**: US06 works on its own on top of US02, US04 and US05.

---

## Phase 11: User Story 8 — Step outputs go to object storage, ClickHouse and broker streams (Priority: P3) (I5)

**Goal**: the three remaining destination kinds, each with its own recovery rule.

**Independent Test**: quickstart scenario 4 in full. Inject crashes and lost responses, then check each
destination's content and the recorded outcomes.

### Tests for User Story 8

- [ ] T035-106 [P] [US08] Write the failing domain tests in `crates/domain/tests/database_connections.rs`
  and `crates/domain/tests/destinations.rs`, quoting data-model.md §1:
  - `object_storage`:
    - `prefix` is "one URI prefix, no `..`";
    - `formats` are `parquet` and `ndjson`;
    - `conditionalCreate` is "found by the probe and recorded with its result, never authored".
  - `clickhouse`:
    - `url` is "HTTPS; plain HTTP only in development";
    - `database` is "the only database destinations may target";
    - `tls` is verify-full.
  - broker sources other than `output`, `outputPath` or `inline` fail `BROKER_SOURCE_NOT_INLINE`.
  - a `STEP_OUTPUT` subscription carries `egressCeiling` (`public` | `internal` | `pii`), and no other
    purpose does;
  - a subscription that also takes run events fails `DESTINATION_SUBSCRIPTION_SHARED`.

  Refs: FR-035-040..042, D035-052.
- [ ] T035-107 [P] [US08] Write the failing tests in `tests/integration/step-destinations/object_storage.rs`:
  - the object name is
    `<prefix>/<workflow>/<destination>/dt=<run admission date UTC>/<hex sha-256(effect key)>.<ext>`;
  - the write uses `PutMode::Create`;
  - an existing object with an equal `sagat-records-sha256` is `ALREADY_PRESENT`; a different one is
    `DESTINATION_OBJECT_CONFLICT`;
  - with `conditionalCreate = false`, the write does a `HEAD` and then a put;
  - the destination never overwrites or deletes;
  - multipart uploads above 64 MiB;
  - the probe's double-create check;
  - the 256 MiB default object size.

  Refs: FR-035-040, D035-036, D035-064.
- [ ] T035-108 [P] [US08] Write the failing tests in `tests/integration/step-destinations/clickhouse.rs`:
  - identifiers are matched against `system.tables` and `system.columns`, and sent as
    `{name:Identifier}` parameters, percent-encoded;
  - the settings are `insert_deduplication_token`, `deduplicate_insert` (or `insert_deduplicate`),
    `async_insert=0` and `input_format_skip_unknown_fields=0`;
  - credentials go in headers;
  - the baseline block numbers are recorded durably before the first insert;
  - a retry happens only within both windows (elapsed time below the window seconds minus 60 s, and a
    block delta below the window blocks), otherwise `OUTCOME_UNKNOWN`;
  - on a table that does not deduplicate, an uncertain insert is unknown;
  - 4xx, 401 and 403 map to their codes;
  - the probe records the deduplication facts.

  Refs: FR-035-041, D035-060.
- [ ] T035-109 [P] [US08] Write the failing tests in `tests/integration/step-destinations/broker.rs`:
  - T5 stores the record in `payloads` and the envelope rows: `kind = STEP_OUTPUT`,
    `event_id = uuid_v5(ns, effectKey + "/" + index)`, `run_id`, `step_key` and `payload_digest`;
  - `payload` holds only the `step.output.v1` envelope;
  - the subscription is re-checked under lock: disabled, deleted or a lowered ceiling fails the step in
    T5;
  - a record above 65,536 bytes, more messages than the destination's limit (100, at most 1,000), or
    more than 4 MiB per completion fails `DESTINATION_DELIVERY_BOUND_EXCEEDED`;
  - the worker composes the body from `payloads`;
  - settlement goes through the T9 hint and the T11 shard owner, and the scan finds lost hints;
  - a `DEAD` delivery applies the policy;
  - under `hold`, `RETRY_STEP` returns the step to `WAITING(DELIVERY)` and re-inserts missing rows
    idempotently;
  - under `continue`, the step stays `SKIPPED` and only the ADR-0025 redrive applies;
  - publication needs `event:manage`;
  - retention and erasure apply to the payloads.

  Refs: FR-035-042, D035-052, D035-053, D035-068.
- [ ] T035-110 [P] [US08] Write the failing fault tests in
  `tests/fault/step-destinations/{object_storage.rs,clickhouse.rs,broker.rs}`:
  - crashes before and after the external write;
  - lost responses, stale leases, restarts and a retried source step;
  - a lost broker acknowledgement delivers the same message id twice, which is documented and never
    called exactly-once.

  Refs: SC-035-013, G035-009.

### Implementation for User Story 8

- [ ] T035-111 [US08] Add `ConnectionConfig::{ObjectStorage, ClickHouse}` to the domain, DTO and
  application validation, and `ResolvedDatabase.{object_storage, clickhouse}`. Resolution is
  profile-bound: object storage to `lake`, ClickHouse to `database`. Refs: FR-035-040, FR-035-041,
  D035-057.
- [ ] T035-112 [US08] Implement the object-storage destination:
  - the writer in `crates/infra-lake/src/object_store.rs`: a resolver backed by
    `NetworkPolicy::resolve`, no redirects, no proxy, explicit credentials, `PutMode::Create` and the
    checksum metadata;
  - NDJSON encoded by the runner, Parquet by the host (`runners/lake/sagat_lake/object_write.py`);
  - the activity `sagat.database.destination.object_storage` and its probe.

  T035-107 passes. Refs: FR-035-040, D035-061, D035-064.
- [ ] T035-113 [US08] Implement the ClickHouse destination:
  - the writer and probe in `crates/infra-database/src/clickhouse.rs`;
  - the activities `sagat.database.destination.clickhouse` and `sagat.database.clickhouse.probe` in
    `apps/database-runner/src/destinations.rs`;
  - the durable per-attempt baseline in `crates/infra-postgres/src/execution.rs`. If no durable
    checkpoint is possible, record the gap and make every uncertain insert unknown.

  T035-108 passes. Refs: FR-035-041, D035-060.
- [ ] T035-114 [US08] Implement broker streams:
  - `OutboundKind::StepOutput`, the subscription purpose and the ceiling in `crates/domain/src/outbound.rs`;
  - the store in `crates/infra-postgres/src/outbound.rs`;
  - `purpose` and `egressCeiling` (`pii` needs the grant; changes audited) in
    `crates/api-http/src/routes/events.rs`;
  - the T5 inserts in `crates/application/src/worker/output.rs`;
  - body composition and the settle hint in `crates/scheduler/src/deliveries.rs`;
  - T11 settlement and the reconciliation scan in `crates/scheduler/src/`.

  T035-109 passes. Refs: FR-035-042, D035-052, D035-053, D035-068.
- [ ] T035-115 [US08] Add the object-storage and ClickHouse connection forms, and the three kinds in
  `DestinationsPanel.tsx`, to the Studio. Add the three kinds to the five SDK builders, and extend
  `step-destinations.json`. Refs: FR-035-034, SC-035-005.
- [ ] T035-116 [US08] Complete `examples/iceberg/checkout.workflow.json` with `archive`, `analytics` and
  `bus`, and extend `tests/e2e/step-destinations/checkout.rs` (quickstart scenario 4). T035-110 passes.
  Refs: SC-035-013.

**Checkpoint**: all five destination kinds work.

---

## Phase 12: Polish and cross-cutting concerns (I6)

**Purpose**: the security battery, the limits and error coverage, the benchmark, the rollback,
deployment, documentation, reviews and gates.

- [ ] T035-117 [P] Add the security battery to `tests/integration/security-multitenancy/`:
  - another tenant's connection, browse or destination is `404`, and the denial is audited;
  - credential, vended-credential, value and partition-value canaries never appear in logs, spans,
    audit, journal or API responses.

  Rerun T035-015, T035-018 and T035-031. Refs: NFR-035-SEC, SC-035-006.
- [ ] T035-118 [P] Add the classification battery in `tests/integration/iceberg-writes/classification.rs`:
  every `secret` field, every field above the ceiling, and every undeclared field against a ceiling
  below `pii` is refused, for every kind, with zero relay or destination connections. Refs:
  FR-035-025, SC-035-007.
- [ ] T035-119 [P] Add `tests/integration/iceberg-writes/limits.rs`:
  - every ceiling in plan.md is refused before a file is written, and the refusal names the ceiling;
  - a table-driven check makes every FR-035-029 category come from at least one test, with no provider
    text, value or file location;
  - exported telemetry carries only the closed label sets.

  Refs: FR-035-028, FR-035-029, SC-035-011.
- [ ] T035-120 Run the benchmark `tests/performance/iceberg-writes/main.rs` on the reference
  environment approved in T035-001:
  - 1,000,000 narrow rows in at most 2 min at p95, sandbox start included;
  - the lookup over 10,000 snapshots in under 5 s at p95;
  - 2 GiB from 32 blobs within `memory.max`;
  - p50, p95 and p99 of probe, lookup, file writing and commit.

  These are benchmark criteria, not promises. Refs: SC-035-009, NFR-035-PERF.
- [ ] T035-121 Add `tests/integration/iceberg-writes/rollback.rs`:
  - an older engine refuses with `UNSUPPORTED_NODE`;
  - the down migration refuses as T035-013 says;
  - `lake.enabled` off stops admission while runs drain.

  Refs: D035-059, FR-035-033.
- [ ] T035-122 [P] Run the existing contract, integration and SDK suites, unchanged. Run the feature
  targets under `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Refs: SC-035-012.
- [ ] T035-123 [P] Deploy:
  - `infra/helm/templates/lake-runner.yaml`, the lake egress in `networkpolicy*.yaml`, and
    `values.yaml` (`lake.enabled`, profiles, ceilings);
  - the lake services in Compose;
  - the canaries in `docs/runbooks/kubernetes-validation.md`.

  Refs: FR-035-023, D035-057.
- [ ] T035-124 [P] Write the documentation:
  - `docs/runbooks/lake-destinations.md`:
    - setup per catalog;
    - credential rotation;
    - unknown-outcome review, revert failures and orphan files;
    - expiry versus the lookup window;
    - small files and sizing;
    - prefix-scoped credentials (G035-021);
    - alerts.
  - `docs/guides/iceberg-writes.md`:
    - the type mapping table and the scope semantics;
    - the ClickHouse replacing-table pattern;
    - broker at-least-once delivery;
    - reruns write again;
    - snapshot-summary visibility.
  - `docs/architecture/security-and-sandbox.md`;
  - the 035 section of `docs/traceability.md`;
  - the 035 row in `docs/roadmap.md`.

  Refs: FR-035-032.
- [ ] T035-125 At merge:
  - take the next free protocol minor and record it in `SPECIFICATION-INDEX.md` (D035-043);
  - update the runner's negotiation constant;
  - tell 020's T020-062 that the connection kind and the lookup are available.

  Refs: D035-043, FR-035-044.
- [ ] T035-126 Run the six scenarios of quickstart.md and record each command and its output in
  `specs/035-iceberg-write-steps/validation-report.md`. Report emulator-only paths and real-cloud
  checks as gaps. Refs: SC-035-001..014.
- [ ] T035-127 Have `security-reviewer`, `correctness-reviewer` and `architecture-reviewer` review the
  diff and the spec. Fix the findings, then run a verification pass on the fixes. Refs: plan.md
  "Verification plan".
- [ ] T035-128 Run the gates and record their output in validation-report.md:
  - `cargo fmt --check` and clippy;
  - `check-architecture.py` and `check-agent-config.py`;
  - the feature targets;
  - the per-SDK checks of `sdks/AGENTS.md`;
  - pytest, mypy, ruff and `pip-audit` for `runners/lake/`;
  - `npm run check:api`;
  - `/verify`.

  Refs: AGENTS.md "Commands".
- [ ] T035-129 Record the demo: the happy path, a recovered crash, and checkout with the lake down,
  held and retried. Add the asynchronous half of SC-035-014 when 020's sink exists. Refs: SC-035-014.

---

## Dependencies and execution order

### Phase dependencies

| Phase | Depends on | Blocks |
|---|---|---|
| 1 Setup | T035-001 (ADR-0071 accepted) before any code | everything |
| 2 Foundational (I0) | Phase 1 | every story; its three conditions block closing I1 |
| 3 US01 (REST + S3) | Phase 2 | US02 (needs a connection), Phase 6 |
| 4 US02 append 🎯 | Phase 3 | US03, US04, US05, US06, US07 |
| 5 US03 | Phase 4 (the IR node) | none (closes I1) |
| 6 US01 continued (I2) | Phase 4 | none; it can run beside Phases 7–11 |
| 7 US04 overwrite | Phase 4 | US05 (conflict validation), US06 |
| 8 US05 upsert | Phase 7 | US06 |
| 9 US07 destinations I | Phase 4 | Phase 11 |
| 10 US06 revert | Phases 7 and 8 | none |
| 11 US08 destinations II | Phase 9 (derived nodes and policies) | none |
| 12 Polish | the stories in the release | release |

### User story dependencies

- **US01 → US02**: a write needs a connection. US01's I2 half (Phase 6) is independent of US02's
  semantics and can run later.
- **US02 → US03, US04, US07**: they reuse the IR node, the attempt driver and the lookup.
- **US04 → US05**: upsert reuses overwrite's conflict validation.
- **US04 + US05 → US06**: revert covers the three modes. An append-only revert could ship after US02 if
  the owner wants it earlier.
- **US07 → US08**: the three new kinds reuse the derived nodes, policies and receipts.
- **Outside this feature**: 020's `ICEBERG` sink (T020-062) depends on Phase 4. Redis Streams broker
  destinations wait for 033 (G035-017).

### Within each story

Tests come first and fail for the right reason. Then domain, codec, application, adapters and runner,
host, API, Studio, SDKs, and finally the example or e2e. A task closes only with its evidence.

---

## Parallel opportunities

- **Phase 1**: T035-004, T035-005 and T035-006 run in parallel.
- **Phase 2 tests**: T035-009..021 all touch different files and run in parallel.
- **Phase 2 implementation**: T035-027, T035-036 and T035-037 run in parallel with each other and with
  T035-022..026.
- **Each story's tests**: every `[P]` test task of a phase runs in parallel. For example, in US02,
  T035-047..051 are five files.
- **The five SDK builders**: T035-067..071 run in parallel, each in its own SDK.
- **After Phase 4**: Phase 6 (I2), Phase 7 (US04) and Phase 9 (US07) can be staffed in parallel.
  Phase 5 (US03) can too, except for the conformance runner wiring (T035-072), which touches all five
  SDKs.
- **Phase 12**: T035-117, T035-118, T035-119, T035-122, T035-123 and T035-124 run in parallel.

### Parallel example: User Story 2

```text
Task: "Failing domain tests for Node.iceberg in crates/domain/tests/iceberg_write.rs"        (T035-047)
Task: "Failing publication tests in tests/integration/iceberg-writes/publication.rs"         (T035-048)
Task: "Failing host tests in runners/lake/tests/test_{lookup,mapping,compat,append}.py"      (T035-049)
Task: "Failing append integration tests in tests/integration/iceberg-writes/append.rs"       (T035-050)
Task: "Failing fault tests in tests/fault/iceberg-writes/append.rs"                          (T035-051)
```

### Parallel example: User Story 3

```text
Task: "TypeScript builder in sdks/typescript/src/database.ts"                                (T035-067)
Task: "Python builder in sdks/python/sagat_octopus_orchestrator/database.py"                 (T035-068)
Task: "Go builder in sdks/go/workflow/database.go"                                           (T035-069)
Task: "Java builder IcebergWriteOptions.java"                                                (T035-070)
Task: "C# builder IcebergWriteOptions.cs"                                                    (T035-071)
```

Agents working in parallel follow `/parallel-agents`: one worktree per agent, a private
`CARGO_TARGET_DIR`, at most two heavy builds at once, and the gates rerun on the merged tree.

---

## Implementation strategy

### MVP first: US01 and US02 on REST + S3

1. Phase 1, which starts only after the owner accepts ADR-0071 (T035-001).
2. Phase 2, closing the three constitution-V conditions.
3. Phase 3 (US01 on REST + S3), then Phase 4 (US02 append).
4. **Stop and validate**: run quickstart scenarios 1 and 2 and the US02 fault target, and read the
   results with the independent reader.

US01 alone is not usable, since a connection with no write is only a probe, so the smallest valuable
cut is US01 + US02. Adding US03 completes I1.

### Incremental delivery

| Step | Phases | Delivers |
|---|---|---|
| I1 | 3, 4, 5 | append on REST/S3, from the Studio and the five SDKs |
| I2 | 6 | every catalog and storage |
| I3 | 7, 8, 10 | overwrite, upsert and revert |
| I4 | 9 | the payment example to the lake and PostgreSQL |
| I5 | 11 | object storage, ClickHouse and broker streams |
| I6 | 12 | hardening and release evidence |

Each increment ends at a checkpoint where the product is coherent and could stop there (plan.md
Risks, "Scope: 45 requirements").

---

## Findings from task generation

- **US04 scenario 4 contradicts US03 scenario 3.** US04 scenario 4 says scope "partitions in the
  batch" on an unpartitioned table is refused at publication. But publication never contacts the
  catalog (US03 scenario 3), so it cannot know whether the table is partitioned. T035-081 tests the
  refusal at execution instead, before any file is written. When T035-082 is implemented, US04
  scenario 4 and FR-035-015 must say "refused before writing". errors.md must also give the refusal a
  code: proposed `ICEBERG_SCOPE_VIOLATION`, reason `UNPARTITIONED_TABLE`. `/speckit-analyze` should
  confirm this.
- **The Studio has no subscriptions screen.** Subscription `purpose` and `egressCeiling` are set
  through the REST API (T035-114), and the Studio only selects a subscription in
  `DestinationsPanel.tsx`. A subscription editor would be new Studio scope, which the owner has not
  asked for.
- **The sandbox driver lives in `crates/infra-sandbox/src/docker.rs`.** The `lake` profile goes there,
  and `apps/sandbox-runner` gains only the API (T035-028). The plan's source table named only
  `apps/sandbox-runner`.

## Notes

- `[P]` tasks touch different files and have no unfinished dependency. `[USnn]` traces a task to a
  story.
- Commit after each task or logical group, in this worktree, with `git -C` and a branch assertion,
  Conventional Commits and a `Refs:` footer.
- Never tick a task without its evidence. A failing or flaky test is a defect to explain, not to retry.
