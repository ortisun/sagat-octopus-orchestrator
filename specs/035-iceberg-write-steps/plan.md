# Implementation Plan: Apache Iceberg writes and step-output destinations

**Branch**: `feature/035-iceberg-write-steps` | **Date**: 2026-10-27 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/035-iceberg-write-steps/spec.md`

Status: Proposed — not started. Revised the same day after review round 1 (an architecture review and
a security review; findings and decisions D035-048..069 are in research.md). The registers are in
[research.md](research.md): decisions D035-001..069, gaps G035-001..023, open owner questions
O035-002..005. The architecture is proposed as
[ADR-0071](../../docs/adr/ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md),
pending owner acceptance.

## Summary

Extend the feature-017 connection catalog with three kinds — `iceberg`, `object_storage` and
`clickhouse` — and give every kind an egress classification ceiling. Add a root-level `icebergWrite`
node and synchronous `destinations` on root-level nodes. Both are reserved managed activities run by
`apps/database-runner` under `Role::DatabaseRunner`, in two profile-bound deployments:

- **database** (exists): PostgreSQL and ClickHouse destination writes, with fixed,
  platform-generated statements.
- **lake** (new; cargo feature `lake`, image `lake-runner`): Iceberg writes and object-storage
  destinations. The Rust runner owns the lease, the credentials and the record files. Iceberg
  semantics run in a pinned PyIceberg 0.12.0 host inside a **per-attempt sandbox container**, started
  by `apps/sandbox-runner` with a new `lake` profile of the feature-005 driver (ADR-0013). The
  container's only route out is an authenticated Unix-socket relay that enforces 017's destination
  policy (D035-048..050).

**Iceberg recovery.** Every snapshot of a commit carries the step's effect key in its summary. Before
each commit the runtime searches the target ref's history for that key, and the commit is conditional
on the exact head it searched (D035-030). Duplicates between attempts are impossible, and an
inconclusive history becomes `OUTCOME_UNKNOWN`.

**Destinations.** They are derived at parse into nodes `<source>~d.<name>` between the source and its
successors (D035-054), so the completion transaction schedules them and the dependents wait for them.
Broker streams are different: they store the mapped records in `payloads`, insert ADR-0025 delivery
envelopes in that same transaction, and are settled by the scheduler shard owner (D035-052, D035-053).

**Failure policies** map onto existing step states and run operations, and never reopen a terminal
step (D035-055).

**Asynchronous mode.** It stays in draft 020, whose `ICEBERG` sink reuses this feature's connection
kind and lookup.

## Technical Context

**Language/Version**:

- Rust 1.98.0 (the workspace toolchain).
- Python 3.12, inside the lake runtime image only.
- The five SDK languages, for the builders.
- TypeScript/React, for the Studio.

**Primary Dependencies**:

- Existing: Tokio, Axum, tonic, SQLx 0.9, `tokio-postgres` 0.7.18, and `reqwest` 0.13 (ClickHouse over
  HTTPS).
- `object_store` 0.14.2 with `gcp` and `azure` added, under the `lake` cargo feature only, built in a
  separate invocation (D035-058).
- Lake runtime image, hash-locked, with pip and setuptools removed: `pyiceberg` 0.12.0 with
  `pyarrow`, `s3fs`, `gcsfs`, `adlfs`, `thrift` (Hive), and `sqlalchemy` + `psycopg` (JDBC catalog).
- Test-only: `iceberg` 0.10.1 (iceberg-rust), as an independent reader in `tests/`.

**Storage**: PostgreSQL remains the authority, with one additive migration,
`20270115000000_lake_destinations` (D035-044). Customer tables, buckets, databases and topics are
external effects.

**Testing**:

- `cargo test -p sagat-tests` targets: `contract_iceberg_writes`, `integration_iceberg_writes`,
  `fault_iceberg_writes`, `e2e_iceberg_writes`, `perf_iceberg_writes`,
  `integration_step_destinations`, `fault_step_destinations`, `e2e_step_destinations`.
- SDK conformance scenarios `iceberg-writes` and `step-destinations`.
- `pytest`, `mypy` and `ruff` for `runners/lake/`.
- Playwright journeys for the Studio.
- Containers are started by the tests themselves (`tests/support/lake.rs`).

**Target Platform**: Linux containers (Compose; Helm).

**Project Type**: a modular-monolith web service with workers, SDKs and a web Studio.

**Performance Goals** (SC-035-009, criteria to measure, not promises, on a declared reference
environment):

- an append of 1,000,000 narrow rows (about 100 MiB) within 2 min at p95, sandbox start included;
- the lookup adds under 5 s at p95 with 10,000 retained snapshots;
- a 2 GiB, 32-blob append stays within the container's `memory.max`.

**Constraints**:

- No catalog or storage I/O in the API, engine or scheduler.
- The Python host runs only inside the sandbox container.
- At most one commit per search.
- Root-only placement (ADR-0036 decision 3).
- Finite ceilings, listed below.

**Scale/Scope**: 45 functional requirements, 8 user stories, 5 destination kinds, and 4 catalogs × 3
storages, delivered in six increments.

## Constitution Check

*GATE: checked before Phase 0, re-checked after Phase 1 design, and re-checked after review round 1.*

| Principle | Pre-design | Post-design | Evidence / gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | T5 commits the source completion, the destination steps, broker payloads and envelopes, and the outbox together. Delivery settlement stays in the delivery worker's own transaction; the shard owner settles the step from the authoritative rows (T9/T11). No external commit runs inside a platform transaction, and no object I/O runs inside T5 (D035-052). |
| II — modular domain, ports and adapters | Pass | Pass | Domain types need no new dependencies. The Python closure, relay and object-store clients live in `crates/infra-lake` and `runners/lake/` behind the `lake` cargo feature. A fitness check inspects `cargo tree -e features` of the control plane and engine (D035-058). |
| III — at-least-once, compensation | Pass | Pass | Each kind has an effect key and a lookup or deduplication rule; unproven outcomes wait for a person; revert is a derived compensation that can fail and wait; no exactly-once claim. |
| IV — versioned, pinned, five SDKs | Pass | Pass | Connection and subscription use are pinned and re-checked. Contracts are additive. The protocol minor is negotiated, and new activities are offered only under it (D035-057). Five-SDK scenarios. Python and images are pinned by hash and digest. |
| V — isolation, tenant, secrets | Pass | Pass with conditions | Isolation: a per-attempt sandbox container, the relay as its only route, catalog-configuration hygiene, profile-bound runner identities, and ceilings on every kind including broker subscriptions. **Conditions before I1 closes:** spike T-isolation (G035-022), the relay and hostile-catalog canaries (G035-018), and T-commit-once (G035-020). |
| VI — observability and audit | Pass | Pass | Closed-label metrics and spans per phase. Audit of connections, browses, writes, reverts, decisions, subscription changes and relay denials. Host standard error is never stored (D035-062). |
| VII — inbox/outbox | Pass | Pass | Broker destinations reuse ADR-0025 delivery rows written in the causing transaction; settlement uses an outbox hint plus a reconciliation scan; consumers dedupe by stable event id. |
| VIII — failure tested, honest | Pass | Pass | The failure matrix below; fault targets with barriers; emulator-only paths labelled (G035-012, G035-013, G035-014). |
| Delivery constraints — technology baseline | Requires ADR | Pass pending ADR-0071 acceptance | A platform-owned Python runtime, confined to sandbox containers of the lake deployment, extends the V1 baseline. ADR-0071 (Proposed) records the reason and the exit condition. |

No unjustified violation. The baseline extension and the added moving parts are justified under
Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/035-iceberg-write-steps/
├── spec.md              # specification (owner scope, clarifications, amendments)
├── research.md          # registers D035/G035/O035, Phase 0 research, review round 1
├── plan.md              # this file
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1 validation guide
├── contracts/           # Phase 1: IR fragment, receipts, REST, protocol, runtime, destinations, errors
├── checklists/
│   └── requirements.md  # specification quality checklist
└── tasks.md             # Phase 2 (/speckit-tasks; not created by this command)
```

### Source boundaries and planned additions

| Path | Change |
|---|---|
| `crates/domain/src/database.rs` | `ConnectionConfig::{Iceberg, ObjectStorage, ClickHouse}`; `egress_ceiling` on every variant; `IcebergWrite`, `Destination`, limit types |
| `crates/domain/src/database/failure.rs` | new `DatabaseFailure` codes ([errors](contracts/errors.md)) and a test pinning the set |
| `crates/domain/src/workflow.rs` | `Node.iceberg`, `Node.destinations`, `NodeOrigin::Destination`, derivation at parse before region analysis, root-only validation, `revert` admitted in `compensate_with` (D035-056) |
| `crates/domain/src/classification.rs` | `within(level, ceiling)` and the stricter-of rule (D035-051) |
| `crates/domain/src/{outbound.rs,identity.rs}` | `OutboundKind::StepOutput`, subscription purpose and ceiling, permission `database:browse` (49 entries), runner binding profile |
| `crates/application/src/{database.rs,definitions.rs,publishing.rs}` | validation per kind; publication checks (access, modes, ceilings, root-only, broker sources, filter depth, `lake.enabled`) |
| `crates/application/src/worker/{database.rs,output.rs}`, `worker.rs` | profile-bound activities and resolution, both secrets pinned; completion schedules destination steps and broker payloads and envelopes; shard-owner settlement of `WAITING(DELIVERY)` steps |
| `crates/application/src/{lake.rs,runs.rs,runs/ad_hoc.rs}` | browse and probe admission (`CATALOG_BROWSE` origin), receipt rebuilding, skip widened to destination steps, ad hoc destination redrive |
| `crates/infra-ir/src/{convert.rs,lake.rs}` | codecs |
| `crates/infra-postgres/src/{database.rs,execution.rs,outbound.rs}`, `migrations/20270115000000_lake_destinations.{up,down}.sql`, `storage-classification.json` | stores, migration, classification |
| `crates/infra-database/src/{network.rs,destination.rs,clickhouse.rs}` | IPv6 metadata addresses; PostgreSQL destination statements; ClickHouse writer and probe with parameterised identifiers |
| `crates/infra-lake/` (new adapter crate, `lake` feature) | sandbox requests to the sandbox runner, `sagat.lake.v1` codec, authenticated Unix-socket relay on `infra-database::network`, policy-bound `object_store` client |
| `apps/database-runner/src/{profile.rs,lake.rs,destinations.rs}` | profiles, activity registration by negotiated minor, attempt drivers |
| `apps/sandbox-runner` | the `lake` container profile (namespaces, no network but loopback, mounts, cgroup, seccomp) and the start/kill API for the lake runner |
| `runners/lake/` (new) | `host.py` and modules (configuration hygiene, forwarder, modes, lookup), `requirements.lock`, `Dockerfile`, `tests/` (pytest), `pyproject.toml` (mypy, ruff) |
| `crates/scheduler/src/deliveries.rs` | `STEP_OUTPUT` sends compose the body from `payloads`; the settle transaction adds the outbox hint |
| `crates/api-http/src/routes/{database.rs,catalog_browse.rs,execution.rs,events.rs}`, `dto_database.rs` | config variants, drivers list, browse, skip on destination steps, subscription purpose and ceiling |
| `specs/001-platform-foundation/contracts/{openapi.yaml,worker.proto,workflow.schema.json}` | merged contract fragments |
| `sdks/{typescript,python,go,java,dotnet}` | builders; error catalog |
| `frontend/apps/studio/src/features/{connections,workflows,runs,events}` | kind-specific forms, catalog picker, destination panel, derived steps, subscription purpose and ceiling |
| `infra/docker/{Dockerfile,compose.lake.yaml}`, `infra/helm/templates/{lake-runner.yaml,networkpolicy*.yaml}`, `values.yaml` | images, fixtures, deployment, egress |
| `tests/support/lake.rs`, `tests/{contract,integration,fault,e2e,perf}/{iceberg-writes,step-destinations}/`, `tests/contract/sdk-conformance/scenarios/` | tests |
| `docs/runbooks/lake-destinations.md`, `docs/guides/iceberg-writes.md`, `docs/adr/ADR-0071-…`, `docs/architecture/security-and-sandbox.md`, `docs/traceability.md`, `scripts/check-architecture.py` | operations, guide, decision, the `lake` profile's rights and walls (a socket only to the relay; a lake sandbox runner serving only lake containers), traceability, fitness function |

**Structure Decision**: extend the code where the semantics already live:

- 017's connections and managed execution;
- 005's sandbox driver;
- 003's derived steps;
- ADR-0025's delivery pipeline.

Add one adapter crate (`crates/infra-lake`) and one runtime directory (`runners/lake/`), both behind
the `lake` feature and image, so the Python closure and the relay stay out of every other process.

## Architecture decisions

| Topic | Decision | Refs |
|---|---|---|
| Client and isolation | PyIceberg 0.12.0 runs in a per-attempt sandbox container started by the sandbox runner. The runner never holds the engine socket, and iceberg-rust is used only as a test reader. | D035-026, D035-048 |
| Egress | An authenticated Unix-socket relay is the only route. It allows only declared or derived endpoints and verifies TLS names. Catalog configuration is filtered. The runner's own clients follow the policy. | D035-049, D035-050, D035-061 |
| Commit protocol | Search before every commit, each commit conditional on the searched head, client retries disabled in memory. | D035-030 |
| Row-level changes | Copy-on-write; v1 and v2 tables only. | D035-032 |
| Classification | Record schemas only annotate egress. A field's level is the stricter of its declared and inherited levels. Every kind has a ceiling, including broker subscriptions. | D035-051, D035-052 |
| Destinations | Derived at parse inside the source's saga region; failure policies never reopen a terminal step. | D035-054, D035-055 |

Per-kind recovery rules:

| Kind | Rule | Ref |
|---|---|---|
| PostgreSQL | constraint-backed keys | D035-034 |
| ClickHouse | parameterised identifiers; a retry needs both deduplication windows proven | D035-060 |
| Object storage | record checksums | D035-064 |
| Broker streams | payload by reference, engine settlement | D035-052, D035-053 |

## Transaction boundaries

| # | Transaction | Writes together | Outside it |
|---|---|---|---|
| T1 | Connection or subscription revision | revision row and audit; a `pii` ceiling only with a current reveal grant | OpenBao writes (017 recoverable operation) |
| T2 | Publication | version and validation results; destination nodes are derived at parse and never stored | none |
| T3 | Run admission (`lake.enabled` checked) | run, connection pins for `icebergWrite` and destinations, initial tasks, outbox, quotas, audit | none |
| T4 | Lease and `ResolveDatabase` (017 two-phase, profile-checked) | credential pin metadata for each secret; audit | OpenBao read between the fence checks |
| T5 | Source step completion accepted | step output, journal, destination steps `READY`; for broker streams: the subscription re-check under lock, record `payloads`, delivery envelopes and the step `WAITING(DELIVERY)`, or the step failed with its policy; outbox `task.ready`; successors stay blocked | none; a refused (stale) completion writes nothing |
| T6 | Iceberg or destination step completion | rebuilt receipt as output, journal, successor readiness, outbox | the external commit already happened (or was found) |
| T7 | Destination failure | step `WAITING(OPERATOR)`, `FAILED` or `SKIPPED` with its receipt per policy; journal; audit | — |
| T8 | Operator decision (`RETRY_STEP`, skip, `RESOLVE_STEP` after an unknown outcome) | step transition, journal, audit, outbox | — |
| T9 | Delivery settled (delivery worker) | delivery status, attempt row, outbox hint `destination.delivery.settled` | broker send |
| T10 | Revert compensation scheduled (003 saga) | derived `~c` step, journal | the inverse commit |
| T11 | Destination settlement (shard owner, from the hint or the reconciliation scan) | the step's completion or failure policy, read from the authoritative delivery rows; journal; outbox | — |

## Authorization matrix

| Action | Permission | Viewer | Developer | Operator | Admin | DatabaseRunner |
|---|---|---|---|---|---|---|
| Create, edit or pause connections of the new kinds | `database:manage` | — | — | — | ✓ | — |
| Set a `pii` ceiling (connection or subscription) | manage permission + 016 reveal grant | — | — | — | grant holders | — |
| Probe a connection | `database:use` | — | ✓ | — | ✓ | — |
| Browse a catalog | `database:browse` (new) | — | ✓ | — | ✓ | — |
| Publish `icebergWrite` or an Iceberg/PostgreSQL/object-storage/ClickHouse destination | `database:use` + `database:write` | — | — | — | ✓ | — |
| Publish a broker-stream destination | `event:manage` | — | ✓ | — | ✓ | — |
| Ad hoc Iceberg write, or a destination redrive | `database:adhoc` + `database:write` | — | — | — | ✓ | — |
| Retry a destination step that failed (held) | `run:operate` | — | — | ✓ | ✓ | — |
| Retry or resolve a destination step whose outcome is unknown | `saga:override` (as `runs.rs` requires for outcomes that may have happened) | — | — | — | ✓ | — |
| Skip a destination | `saga:override` | — | — | — | ✓ | — |
| Lease activities of its profile; `ResolveDatabase` and `ReadBlob` for them | `database:execute` + profile binding | — | — | — | — | ✓ |

The grants follow the code (D035-066), which reserves `database:write` for admins. Letting developers
publish writes would change ADR-0036 decision 5 and is the owner's call. Every check is repeated at
publication, admission and capability use; foreign resources are `404`; denials are audited.

## Finite ceilings

The effective value is the minimum of the platform, namespace and step values, and zero is not
unlimited. These are proposed values (D035-045); their approval by the owner is pending as O035-003.

| Resource | Default | Hard ceiling | Enforced by |
|---|---:|---:|---|
| Record input per step — append | 1 GiB | 2 GiB | runner, while streaming blobs |
| Record input per step — overwrite, upsert | 256 MiB | 2 GiB | runner (D035-046) |
| Decoded bytes per attempt | 8 × input | 16 GiB | host (D035-063) |
| Metadata file size / manifests per table read | 64 MiB / 10,000 | 256 MiB / 100,000 | host |
| Record blobs per step | 32 | 32 | publication and runner |
| Records per step | 10,000,000 | 100,000,000 | host |
| Mapped columns / nesting depth | 256 / 8 | 1,024 / 16 | publication |
| Overwrite filter depth / comparisons | 8 / 64 | 8 / 64 | publication |
| Data files per commit / target file size | 256 / 128 MiB | 2,048 / 512 MiB | host |
| Commit retries per attempt | 4 | 8 | host |
| Snapshots walked by one lookup | 10,000 | 100,000 | host (beyond: inconclusive) |
| Attempt deadline / request deadline | 10 min / 30 s | 60 min / 120 s | runner, relay |
| Container `memory.max` | 2 GiB (append), 6 GiB (upsert) | 16 GiB | sandbox profile |
| Lake containers per node | 2 | 8 | sandbox runner admission |
| Lake attempts per namespace / tenant | 8 / 32 | 32 / 128 | quota rows |
| Destinations per step | 4 | 8 | publication |
| PostgreSQL rows per destination write | 10,000 | 100,000 | `infra-database` |
| ClickHouse records per destination write | 100,000 | 1,000,000 | runner |
| Object size per destination write | 256 MiB | 2 GiB | runner |
| Broker messages per destination write | 100 | 1,000 (each ≤ 64 KiB) | T5 |
| Broker delivery bytes per completion | 4 MiB | 16 MiB | T5 |
| Browse page / browses per actor per minute | 100 / 30 | 200 / 120 | API |

## Failure and recovery matrix

| Failure | Behaviour | Proof |
|---|---|---|
| Container or runner crash before the commit request | nothing visible; the next attempt searches, finds nothing, writes; orphan files flagged | fault |
| Crash after the commit request, before the response | the next attempt finds the key (or, with expired history, `OUTCOME_UNKNOWN`) | fault |
| Crash after the response, before `CompleteStep` | the next attempt finds the key and reports `foundByLookup` | fault |
| A stale attempt races its successor | conditional commits: at most one lands, and the other finds it | fault with a barrier |
| Commit conflict with an external writer | re-search, re-validate scope and keys, bounded retries, then `ICEBERG_COMMIT_CONFLICT` | integration |
| Catalog 5xx or timeout on commit | search; found → done; absent → retry asserting the same head; inconclusive → unknown | fault |
| Snapshot expiry inside the window | inconclusive lookup → `OUTCOME_UNKNOWN` → operator | integration |
| Lost broker hint, or engine restart | lease reconciliation re-delivers the task (002); the settlement scan finds settled deliveries | fault |
| Timeout races a cancellation | the first commit wins (constitution I); a commit already sent is looked up and never reported as cancelled | fault |
| Revert blocked by a compaction | the compensation waits for a person; the run is not marked compensated | integration |
| Credential revoked or connection paused mid-run | the next `ResolveDatabase` refuses; the attempt fails typed; no fallback | integration |
| OpenBao sealed | resolution fails closed, retryable | integration |
| A compromised host tries the runner token, another attempt, the pod network, the metadata service or the engine socket | the container's namespaces and mounts refuse each one | T-isolation (I0) |
| A hostile catalog returns `py-io-impl`, endpoint, proxy or signer overrides | the keys are dropped; the relay refuses undeclared hosts; audited | hostile-catalog fixture |
| Relay token missing or wrong | connection refused | integration |
| Container exceeds `memory.max` or stops emitting heartbeats | killed; the attempt fails retryable; the next attempt searches | fault |
| Sandbox runner unavailable | `LAKE_SANDBOX_UNAVAILABLE`, retryable; nothing was sent | integration |
| ClickHouse retry outside either window | `OUTCOME_UNKNOWN`, never a blind insert | integration |
| An object exists with another records checksum | `DESTINATION_OBJECT_CONFLICT`, review | integration |
| Broker subscription disabled or its ceiling lowered at completion | the destination step fails in T5; its policy applies | integration |
| Broker delivery `DEAD` | the shard owner applies the destination's policy | integration |
| Destination held, an admin skips | step `SKIPPED`; successors run; audited | e2e |
| Broker destination held, retried after a redrive | step back to `WAITING(DELIVERY)`; missing rows re-inserted idempotently; T11 settles it | integration |
| An older engine meets a version that uses the feature | `UNSUPPORTED_NODE` | rollback test |

## Observability

- **Metrics** (closed labels):
  - `lake_writes_total{mode,catalog_type,storage_type,outcome}`;
  - `lake_write_phase_seconds{phase}`, phases `sandbox`, `load`, `lookup`, `write`, `commit`;
  - `lake_lookups_total{result}`, results `found`, `absent`, `inconclusive`;
  - `lake_commit_conflicts_total`, `lake_sandbox_exits_total{reason}`,
    `lake_relay_denials_total{reason}`;
  - `destination_writes_total{kind,outcome}`, `destination_held_total{kind}`;
  - `database_catalog_browses_total{outcome}`.
- **Spans:** `lake.attempt`, with children `lake.sandbox`, `lake.load`, `lake.lookup`, `lake.write`
  and `lake.commit`; and `destination.write{kind}`. Attributes are the connection id, table identifier,
  snapshot id and effect-key hash. Never values, partition values, file locations, object keys,
  credentials or host error text.
- **Audit:** connection and subscription changes, probes, browses, writes (once per committed write),
  reverts, operator decisions, relay denials and refusals.
- **Runbook alerts:** held destinations older than N minutes, unknown outcomes, relay denials, sandbox
  start failures and the commit-conflict rate.

## Migration, compatibility and rollback

- **The migration is additive:**
  - new `CHECK` values and nullable columns;
  - defaults that read existing rows unchanged: `purpose = RUN_EVENTS`, binding profile `database`,
    `egressCeiling = internal`;
  - grants for `sagat_app` and storage classification entries.
- **The down migration refuses** while any `DESTINATION` step, any `STEP_OUTPUT` delivery, any
  `STEP_OUTPUT` subscription or any connection of a new kind exists (`/migration` checklist,
  D035-059).
- **Compatibility:**
  - Contracts are additive.
  - A runner offers, and the server allows, the new activities only under the negotiated minor, so
    mixed versions keep working (D035-057).
- **Rollout:** `lake.enabled` is off by default and gates both publication and admission. The lake
  deployment and the sandbox `lake` profile are installed separately.
- **Rollback:**
  1. Turn `lake.enabled` off; admission then stops, so draining finishes.
  2. Wait for the runs that use the feature to end.
  3. Roll the binary back. An older engine refuses runs that use these nodes with `UNSUPPORTED_NODE`,
     and a rollback test proves it.

## Delivery sequence

| Increment | Scope | Closes |
|---|---|---|
| I0 Foundations | domain types; the error set and its pin test; migration; contract merges; `compose.lake.yaml`; the lake image; the sandbox `lake` profile and the **T-isolation spike**; the relay and its canaries; the hostile-catalog fixture; T-commit-once; profile binding; build separation | V conditions; G035-018, G035-020, G035-022 |
| I1 Append MVP | Iceberg connection (REST + S3), probe, browse, `icebergWrite` append with the lookup protocol, Studio and five SDKs, faults | US01, US02, US03 (REST/S3) |
| I2 Catalogs and storages | Glue, Hive, JDBC; GCS, ADLS; emulator labels | FR-035-002/003 in full |
| I3 Row-level modes | overwrite (three scopes), upsert with delete marker, isolation, revert | US04, US05, US06 |
| I4 Destinations I | derived destination nodes, policies, Iceberg and PostgreSQL destinations | US07 |
| I5 Destinations II | object storage, ClickHouse, broker streams (payload by reference, shard-owner settlement) | US08 |
| I6 Hardening | security battery, classification canaries, SC-035-009 benchmark, runbook, guide, demo, reviews | SC-035-001..014 |

020's `ICEBERG` sink (T020-062) can start after I1, once the connection kind and the lookup exist.

## Verification plan

- **Tests first.** They are written first and seen failing (AGENTS.md rule 18). Races use held locks
  or barriers.
- **Independent reader.** iceberg-rust 0.10.1 reads every table the platform wrote.
- **Security proofs:**
  - T-isolation (container boundaries);
  - relay and hostile-catalog canaries;
  - credential and value canaries across logs, spans, audit, receipts and API;
  - the cross-tenant battery.
- **Emulator-only paths are reported as such:** Glue conflicts (moto), ADLS DFS (Azurite), RustFS
  conditional create (decided by the probe).
- **Reviews before declaring done:** independent `security-reviewer`, `correctness-reviewer` and
  `architecture-reviewer`, given the diff and the spec.
- **Evidence:** written in `validation-report.md`; `docs/traceability.md` gains a 035 section.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| A Python runtime (PyIceberg) in a Rust platform | it is the only client covering the owner's modes, branches and catalogs (D035-026) | iceberg-rust 0.10.1 only appends, only to `main`, and has no Hive commit; Java brings a JVM |
| A sandbox container per attempt | contains a parser compromise inside one attempt, with no route to the runner token or other attempts (D035-048) | a subprocess with the runner's UID, network and volume reached all of them (security review H1) |
| An authenticated Unix-socket relay | the only route out for the container; enforces FR-035-004 against catalog redirects (D035-049) | trusting library resolution leaves redirects unenforced |
| A second, profile-bound deployment and cargo feature | different networks, memory profile and image; a stolen lake token cannot reach PostgreSQL credentials (D035-057, D035-058) | one deployment mixes private-database reach with object-storage reach |
| Derived destination nodes | dependents wait on the writes, reusing existing readiness and recovery (D035-054) | a side table of pending writes would need its own scheduler, recovery and UI |

## Risks

| Risk | Mitigation |
|---|---|
| Co-location of the lake runner and the sandbox runner is unproven (G035-022) | T-isolation decides it in I0; if no safe co-location exists in the Helm topology, the owner chooses between a node-local deployment and a per-attempt Kubernetes Job. |
| Sandbox start latency of 1–3 s per attempt | Measured in SC-035-009. The runbook recommends the asynchronous 020 sink for high-volume flows. |
| PyIceberg regressions or semantic drift | Pinned and hash-locked; independent-reader tests on every bump; Java documented as the fallback. Re-check iceberg-rust once overwrite and RowDelta ship. |
| A client bypasses the relay (G035-018) | Contained by the container's network namespace; canaries per client. |
| Memory use of copy-on-write upserts | Lower defaults and `memory.max`; a sizing table in the runbook. |
| Small files at high volume (G035-011) | Use the asynchronous 020 sink. |
| Scope: 45 requirements | Each increment is a coherent product to stop at. |
| Coordination with other work | The protocol minor (D035-043), 020's sink (T020-062), 033's Redis destinations, 021's `s3_bucket` semantics, and ADR-0036 decision 3 if per-instance pins arrive. |
