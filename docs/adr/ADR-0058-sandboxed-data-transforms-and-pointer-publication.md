# ADR-0058: Engine-local data transforms run in per-attempt sandboxes over immutable snapshots, and assets are published by an atomic PostgreSQL pointer

**Status:** Accepted (2026-11-26, delegate under Owner decision 2026-11-24-4; recorded as Owner decision 2026-11-26-164). Proposed 2026-10-26.
**Decided by:** agent proposal (2026-10-26; revised 2026-10-27 after the architecture review and coordinator decisions). Revised against `develop` on 2026-11-26 after an independent audit of feature 024: the data containers are requested through the sandbox runner's attempt API of [Architecture decision 0079](ADR-0079-the-lake-runner-asks-the-sandbox-runner-for-an-attempt-never-for-a-container.md) instead of being started by the process that claims the task, feature 035's lake runner path is reused wherever it applies, and Owner questions 1240 to 1243 are decided (Owner decisions 2026-11-26-160 to 2026-11-26-163). Accepted the same day by the coordinating session under the owner's delegation; the owner may overrule it by a later decision. Corrected 2026-11-27 after the correctness review of feature 024 (Decision 024-016 of its research): decision 4 now states what binds the audit to the bytes and what the digest comparison proves, decision 9 how a step source is classified and that a `dataQuery` never reads a feature 017 database result, and decision 10 which other records are protected and that the API justification stays charged (Decisions 024-016 and 024-017; delegate row OD-2026-11-27-242, requested); the direction of the decision is unchanged.
**Refines / supersedes:** **Supersedes in part** [ADR-0023](ADR-0023-blob-store-claim-check.md) ("committed blobs are never deleted", for the `DATA_SNAPSHOT_PART` purpose only) and [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) ("does not authorize deletion of customer data", for snapshot bytes under the retention an asset manager declares only), both on Owner decision 2026-11-26-163. **Refines** [ADR-0013](ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md) (a `data` container profile with declared mounts, as Architecture decision 0071 did with the `lake` profile; the `runner` role and its permission set are unchanged), Architecture decision 0079 (a third exclusive mode of the sandbox runner, `data`, served through the same three calls, with its own slot and user range and one data-mode request member, see decision 1), [Architecture decision 0071](ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md) (decision 1: a third profile-bound deployment of the managed executor, `warehouse`; decisions 7 and 8: a recovery rule and an egress ceiling for every new driver), ADR-0023 (an additive ranged read on the `BlobBackend` port; a separate pending-upload ceiling for the new purpose), ADR-0027 (a `data` accounting class, the `tenant_data_bytes` aggregate, and the publication event as a bounded `protected_system` record), ADR-0033 (the `dataQuery` output joins the feature 016 carrier set with a platform-derived schema) and [ADR-0035](ADR-0035-database-connections-and-managed-execution.md) decision 4 (the warehouse deployment). Leaves unchanged ADR-0035 decision 10 and [ADR-0036](ADR-0036-integrated-scoped-managed-and-classified-execution.md) decisions 1 and 6: DuckDB is a data node, not a feature 017 driver. Coordinates with ADR-0054 (feature 020), which excludes `DATA_SNAPSHOT_PART` blobs from run purge; this decision owns their retention.
**Dependents to update** (each by its own owner; listed so that none is forgotten): ADR-0013 (a "refined by ADR-0058" note for the data profile); Architecture decision 0079 (rule 1 names the `data` mode beside `lake` and script; the data-mode request member `layer` of decision 1 below is that decision's `/v1` data path, or a `/v2` path if its owner prefers it); `docs/architecture/security-and-sandbox.md` (data profile, `data-runner` role); ADR-0023 (note on the purpose and the port method); ADR-0027 (the `data` class); `crates/infra-postgres/storage-classification.json`; the feature 016 carrier list (coordinator decision C6); feature 020's content-store registry (coordinator decision C8); feature 023's Requirement 023-027 (feature 024 does not need an image catalog for `python-data`, Owner decision 2026-11-26-162, so the image-catalog parts of that requirement are not built); design 046 (the `python-data` language uses its Python path unchanged: the `pypi` ecosystem, its gateway, policy, layer and lease block); the register of worker-protocol minors in `sdks/SPEC.md` section 9 (the minor this feature takes when its contract merges).
**Enforced by:** planned; until each exists the rule it names is review only:
- `scripts/check-architecture.py`: the data runner (`sagat-data-runner`) joins the packages that reach neither `bollard` nor `sagat-infra-sandbox` (the rule of Architecture decision 0079 rule 2, `LAKE_RUNNER_PACKAGES`); only `sagat-infra-data` may depend on `parquet`; only `sagat-infra-database` may depend on a target-database driver crate; no Rust package depends on DuckDB, Polars or Arrow compute (Task 024-005, decision 1);
- the mount-profile unit test in `crates/infra-sandbox` and the container canaries (Tasks 024-028, 024-039, decision 1); the data-mode tests of the attempt API against the in-memory engine double (Task 024-073, decision 1);
- Task 024-031 (pinned-only reads through `step_lease_blobs`, decision 2); Task 024-033 including its fuzz target (decision 3); Task 024-035 (audit isolation and digests, decision 4); Tasks 024-034, 024-036 and 024-060 (pointer atomicity, decision 5);
- Tasks 024-010 and 024-038 (the guarded `COMMITTED → DELETING` trigger, run-purge exclusion and retention, decision 7);
- Task 024-022 (warehouse deployment NetworkPolicy and profile binding, decision 8) and Task 024-071 (managed kinds and profiles, decisions 1 and 8);
- Task 024-041 (classified `dataQuery` output, including step sources, decision 9); Tasks 024-034 and 024-046 (publications at full quota, decision 10); Task 024-078 (parts of an ended attempt claimed for deletion, decision 10); Task 024-077 (run purge leaves this purpose alone, decision 7, once feature 020's purge exists);
- Task 024-070 (instance-registry capabilities, decision 12).

## Context

Feature 024 adds data pipelines: DuckDB SQL and Polars transformations over namespace data, declared
assets with schema contracts, lineage, freshness and write-audit-publish (Windmill's Pipelines
publish into managed DuckLake tables; its Enterprise write-audit-publish runs checks inside the
write transaction so readers keep the previous version). The forces:

- DuckDB and Polars have no remote server: the transformation *is* tenant-authored computation.
  DuckDB's own security guidance says its settings "are not a substitute for proper sandboxing" and
  recommends an isolated container. Constitution Principle V forbids untrusted code in the control
  plane or scheduler; the feature 017 database runner holds the credentials of every concurrent
  attempt it serves.
- ADR-0013's container takes code and input on standard input and returns one line; it has no mounts,
  because a writable disk has no per-container quota without XFS project quotas. Datasets of hundreds
  of MiB cannot travel that way.
- Feature 035 (merged) already runs a platform-owned Python runtime in per-attempt containers for its
  lake writes, and Architecture decision 0079 settles how a runner gets such a container: it asks a
  sandbox runner in an exclusive mode for an *attempt* (an attempt id, a slot, a budget class and a
  deadline) over a node-local Unix socket, and can never name an image, a mount, a command or a
  limit; the process that handles tenant data never holds the host-root container-engine socket.
  The lake runner stages an attempt directory, serves the container over a Unix socket that admits
  only the slot's user (`crates/infra-lake`: `staging`, `relay`, `attempt`, `supervisor`,
  `sandbox_api`), and reads its inputs with lease-authorized `ReadBlob` calls that the lease
  transaction allows per attempt (`step_lease_blobs`, migration `20270111000000_lake_blob_pins`).
- Readers must never see a half-written dataset, and "publish" must be decided by the authority.
  PostgreSQL is the authority (ADR-0002); object stores reached through the provider-neutral port of
  ADR-0023 share no compare-and-set primitive and cannot commit with a step transition and its outbox.
- ADR-0023 keeps committed blobs forever and ADR-0027 does not authorize deleting customer data (G42).
  Snapshots are large and superseded routinely; without a retention rule they grow without bound. The
  owner's delegate decided that they may be destroyed under the retention an asset manager declares
  (Owner decision 2026-11-26-163).
- BigQuery and Snowflake are reached over public HTTPS; NetworkPolicy cannot list host names, so
  public egress in a pod that also routes to private customer databases mixes two exposures. Feature
  035 already binds each deployment of the managed executor to a profile (`BindingProfile`,
  `worker_bindings.profile`), so a stolen token of one profile cannot ask for another's credentials.

## Decision

1. **Where data code runs.** DuckDB SQL and Polars code MUST execute only in a container per attempt
   that a sandbox runner in **data mode** starts when the **data runner** asks it for an attempt
   through Architecture decision 0079's three calls (start with a slot, a budget class `small` or
   `large` and a deadline; wait; end), never in a process the data runner chooses the shape of:
   - The **data runner** (`apps/data-runner`, machine role `data-runner` holding `worker:execute` and
     `data:execute`, bound with `BindingProfile::Data`) claims the reserved activities
     `sagat.data.transform` and `sagat.data.query`, resolves them with `ResolveData`, stages the pinned
     inputs into the attempt directory, serves the attempt socket, uploads parts and reports. It holds
     no container-engine access, receives no secret and no credential of any kind, and is not a
     profile of `apps/database-runner` (whose role holds `database:execute`).
   - The **data mode** of the sandbox runner is exclusive as the lake mode is (Architecture decision
     0079 rule 1): no runner token, no script, build or lake configuration, its own slot range and
     user range so that a lake peer and a data peer are never confused. It builds the container from
     its own configuration: the pinned data runtime image by digest, the budget of the class (memory,
     CPUs, processes, `/out` size, `/spill` size) and the mounts below. Its one data-mode request
     member is `layer`: for a Polars attempt, the digest of a design 046 layer record that this
     sandbox runner itself materialized and verified, which selects that volume and nothing else
     (like a slot selects a user); a digest it does not hold is `refused`.
   - The container profile is ADR-0013's (no network interface but loopback, read-only root, all
     capabilities dropped, `no-new-privileges`, default seccomp, the slot's unprivileged user, cgroup
     limits) with exactly these mounts: the attempt directory, read-only (`ro,noexec,nosuid,nodev`) as
     a sub-path of the work volume, whose `in/` holds the staged inputs; `/out` and `/spill`, tmpfs
     `noexec,nosuid,nodev` with a `size=` charged to the container's memory cgroup; and, for a Polars
     attempt only, design 046's signed layer read-only at `/opt/sagat/deps` (executable, because
     native extensions are mapped executable; design 046 section 11). Nothing else is mounted.
   - The request, the parts and the result travel over the attempt's Unix socket in the attempt
     directory, which the data runner binds before the start and serves only to the peer user of the
     attempt's slot (the lake relay's `SO_PEERCRED` check). The host program streams each part of
     `/out` over it in bounded length-prefixed frames before it exits; the data runner bounds the
     stream, uploads each part as it arrives and keeps no copy. The socket carries no credential and
     reaches no network.
   - Data code MUST NOT execute in the control plane, the execution engine, the scheduler, the
     database runner, the data runner or any process that holds another attempt's credential. DuckDB
     hardening settings are applied and locked as defence in depth only. The data runner offers the
     data activities only after a probe attempt of the configured image passes.
2. **Pinned inputs.** A data node MUST read only immutable inputs pinned in the transaction that
   grants its first lease (published or same-run audited snapshots, or committed outputs of earlier
   steps of the same run). The lease transaction of every attempt MUST write the allow-list rows of
   feature 035 (`step_lease_blobs`) for exactly the pinned parts, so the data runner stages them
   through lease-authorized `ReadBlob` calls of exactly the pinned objects; any other data object MUST
   look missing to that lease. The audit phase may read back only the parts that this attempt and
   generation uploaded.
3. **Engine-verified snapshots.** Snapshot bytes MUST be Parquet parts in the platform's ADR-0023 blob
   backend under the namespace (Owner decision 2026-11-26-160) with the purpose `DATA_SNAPSHOT_PART`,
   uploaded with the exact attempt fence. A snapshot record MUST be created only in the completion
   transaction, after `apps/execution-engine` itself verified each part's fence, measured size and
   SHA-256, and Parquet footer (read by a bounded ranged request, decoded with bounded limits on a
   panic-isolated task) and schema against the pinned contract. The sandbox's report is never
   sufficient.
4. **Audits apart from the transform.** Audits declared on the asset MUST run in a fresh attempt of
   the data mode, in the transform's budget class, over exactly the uploaded parts, which the data
   runner reads back from the platform with `ReadBlob` (verified against the digests the server
   measured), after the transform container ended. That read-back, in platform code, is what binds
   the audit to the uploaded bytes. The engine MUST refuse a completion whose digests of the parts the
   data runner staged differ from the committed parts; this is a self-check that catches a data runner
   staging other bytes, and it proves nothing about what the untrusted audit container read: audit
   outcomes rest on the sandbox (Gap 1249). Outcomes MUST be payload-free. A `FAILED` check, or an
   `ERROR` of a check the asset manager authored, MUST commit the snapshot as `QUARANTINED`; a failure
   of the audit attempt itself (refused, unavailable, killed, timed out, crashed, or an `ERROR` of a
   platform-generated check) MUST commit no snapshot and fail the step retryably. A quarantined
   snapshot MUST NOT become current or feed a same-run input.
5. **Atomic pointer.** The current snapshot of a managed asset MUST be a pointer row in PostgreSQL
   with a monotonic version, moved only by a compare-and-set against the version captured at the
   producer's first lease (or supplied by an API caller), in one transaction with the publication
   record, the inbox event `data.asset.published`, the outbound delivery rows and the audit — and,
   when a run publishes, with the step or node transition, journal, next tasks and outbox. Asset heads
   MUST be locked in ADR-0050's slot 4b at admission (and the namespace data-catalog fence in slot 4a
   by writers), and after the run and authority guards elsewhere, before any row that references
   them. Apart from an explicit, audited API rollback, an older snapshot MUST NOT overwrite a newer
   publication. Readers MUST pin snapshot ids.
6. **Where the guarantee holds.** These guarantees MUST be claimed only for platform-catalogued
   (managed) assets. Tables inside a customer warehouse, and Iceberg tables that feature 035's
   `icebergWrite` steps write, are external assets with observations from their declared producer;
   they MUST NOT be presented as having write-audit-publish.
7. **Destroying superseded snapshots.** Only `DATA_SNAPSHOT_PART` blobs MAY move from `COMMITTED` to
   `DELETING`, only by this feature's retention, only under the retention the asset manager declared
   on the asset (Owner decision 2026-11-26-163), only after a committed decision that rechecked
   live-run pins under the asset row lock and found no allow-list row of `step_lease_blobs` left for
   the part, and audited in the deciding transaction. Deletion follows ADR-0023's tombstone and
   durable-retry rules; bytes are released from `tenant_data_bytes` only after confirmed deletion.
   Run purge (feature 020, ADR-0054) MUST NOT delete this purpose nor snapshot metadata (snapshots,
   parts, provenance, audits, publications), which this decision owns: purging a producer run must
   not remove what a current pointer, a pin or a provenance edge names. Every other purpose keeps its
   feature 020-owned retention. Copies that left the platform are not reached, and the runbook says so.
8. **Remote engines.** MySQL, MariaDB, SQL Server, BigQuery and Snowflake remain in ADR-0035's model
   and its `database-runner` role: their credential is disclosed only to the lease-owning
   `database-runner` with `database:execute`, which is Principle V's second declared set; no
   constitution amendment is needed. BigQuery and Snowflake MUST run in a deployment bound to a
   third profile, `warehouse` (Architecture decision 0071 decision 1: the server allows a profile only
   its own activities and connection kinds), whose NetworkPolicy allows DNS, the engine and TCP 443 to
   non-private ranges or operator-listed private-endpoint networks and no route to private customer
   networks; the private deployment (profile `database`) has no public egress. The process also
   enforces the driver-fixed host form and an operator host allowlist that namespace policy can only
   narrow. Every connection of a new driver carries feature 035's egress ceiling under the uniform
   `pii`-egress rule (Owner question 1081, decided 2026-11-24 for feature 020, Decision 1617), and
   declares its recovery rule in Architecture decision 0071 decision 7's vocabulary: BigQuery and
   Snowflake look the write up by its deterministic identity within the lease; MySQL, MariaDB and SQL
   Server have none, so an ambiguous write ends `OUTCOME_UNKNOWN` and waits for a person.
9. **DuckDB queries.** DuckDB queries MUST be `dataQuery` data nodes, never a feature 017 database
   driver. A `dataQuery` output is inline (≤ 256 KiB) in the feature 017 typed-cell shape and is a
   feature 016 classification carrier annotated at the highest level among its pinned inputs,
   disclosed only through the existing feature 016 disclosure; the feature 017 result view and its
   rules are unchanged. An asset input counts at the highest level of its pinned columns; a step
   source, which has no declared columns, counts as `pii` unless its source output is a feature 016
   carrier that declares a level for every field. A `dataQuery` MUST NOT read a feature 017 database
   result: publication refuses such a step source, so the result stays behind `database:result:read`
   as ADR-0035 decision 10 requires; only a `dataTransform` may stage one, into an asset whose
   manager declares the column levels.
10. **Accounting.** Snapshot bytes MUST be charged once, to `tenant_data_bytes` under a `data` class,
    when each part upload is reserved and measured; `DATA_SNAPSHOT_PART` uploads have their own
    pending ceiling. The publication event MUST be a closed document of at most 1 KiB stored as a
    `protected_system` record: it is not charged to content quota and does not spend
    `events_per_second`. It carries no column value. The output document of a `dataTransform` step
    and of a `dataPublish` node (closed metadata, at most 4 KiB) is a `protected_system` record too,
    so a full content quota never refuses the records an auto-publishing completion or a `dataPublish`
    writes itself; charged rows of the next node (a child workflow's input) still obey ADR-0027. The
    API publication's justification (at most 1,024 bytes) is tenant-authored text and stays in the
    charged `content` class, as ADR-0027 requires: an API publication obeys the content quota and is
    refused `QUOTA_EXCEEDED` at a full quota, with nothing written. The
    transaction that ends an attempt without committing its parts MUST claim them for deletion, so a
    failed or retried attempt does not hold `tenant_data_bytes` until the upload time to live.
11. **Reuse of feature 035, and what differs.** Reused as they are: the attempt API and its wire types
    (`crates/infra-lake/src/sandbox_api.rs`, `uid_for_slot`, the budget classes), the attempt-directory
    staging and its whole-blob checks (`crates/infra-lake/src/staging.rs`,
    `apps/database-runner/src/lake/blobs.rs`), the peer-checked socket and the table of endings
    (`relay`, `attempt`, `supervisor`), the engine's label-held state with list, inspect, wait and
    kill by label (`crates/infra-sandbox/src/lake/engine.rs`, generalized rather than copied), the
    lease allow-list `step_lease_blobs`, `ManagedKind` and `BindingProfile` (decision 8 and
    `ManagedKind::{DataTransform, DataQuery}`), the egress ceiling, the recovery-rule vocabulary, and
    the error catalog precedent of the Iceberg codes. What differs, and why: the data container
    receives no credential and its socket relays nothing to a network (the lake relay forwards to a
    catalog and storage); it runs tenant code, not pinned platform code, so its output tmpfs is
    `noexec` and its audits run in a separate attempt; the authority over a managed snapshot is a
    PostgreSQL pointer, not an Iceberg catalog (option 5 below), so an Iceberg table is only ever an
    external asset; and the claimant is an application and a role of its own, not a profile of the
    database runner, because it must never hold `database:execute`.
12. **Activation.** The drivers and the data layer are switched on by instance-registry capabilities
    (Architecture decision 0082): `database-engines-v1` (MySQL, MariaDB, SQL Server),
    `database-warehouses-v1` (BigQuery, Snowflake; recorded only after the real-service suite passes,
    Owner decision 2026-11-26-161) and `data-pipelines-v1` (assets, data nodes, retention), each for
    the kinds `CONTROL_PLANE` and `EXECUTION_ENGINE`. Publication and admission that need a capability
    not yet recorded are refused (`DRIVER_NOT_ENABLED`, `DATA_PIPELINES_NOT_ENABLED`).

## Considered options

1. **A data runner that asks a data-mode sandbox runner for an attempt, engine-verified parts, a
   PostgreSQL pointer (chosen)** — keeps tenant code out of every privileged or credential-holding
   process and the engine socket out of the process that parses tenant output, bounds the extra mounts
   by the memory cgroup and `noexec`, makes the contract the engine's check rather than the sandbox's
   claim, gives an atomic publication that commits with the run's transition and outbox, and reuses
   the attempt path feature 035 already built and tested.
2. **The sandbox runner claims data tasks and starts the containers itself (the script-mode shape;
   the 2026-10-27 draft of this decision)** — one process fewer, but the process that reads untrusted
   framed output and stages other tenants' inputs would hold the host-root engine socket, which
   Architecture decision 0079 refuses for the lake runner for the same reason; rejected.
3. **The data runner as a profile of `apps/database-runner` (the lake precedent)** — one binary
   fewer, but its role holds `database:execute` and its process links the driver crates; a data
   attempt needs neither, and a profile would leave a stolen data token one role away from database
   credentials; rejected.
4. **Keep ADR-0013 unchanged and stream datasets through standard input and output** — no new mounts,
   but a GiB-sized input and output through one pipe and one result line has no seekable access for
   Parquet and no bound other than the pipe; rejected.
5. **Publish by an object-store manifest or a lake catalog (DuckLake, Iceberg)** — the ADR-0023 port
   has no common compare-and-set and the manifest cannot commit with the step transition and outbox.
   Feature 035's Iceberg tables stay customer-owned external assets; exporting a snapshot to one is
   an `icebergWrite` step whose record source is the snapshot's Parquet parts (not in this delivery,
   Gap 024-004), not a second authority.
6. **Embed DuckDB in the database runner** — no container start-up cost, but one hostile query or
   engine bug shares memory with other attempts' credentials; DuckDB's own guidance rejects
   settings-only isolation.
7. **DuckDB as a feature 017 database driver** — reuses the connection catalog and result view, but
   puts a managed database task in the sandbox (against ADR-0036 decision 1) and needs a
   classification gate on the feature 017 result view (against ADR-0035 decision 10, ADR-0036
   decision 6).
8. **A platform-curated Python catalog image for Polars (feature 023's image-catalog runtime)** — one
   reviewed set of packages, but the platform would keep a re-pin cadence and strand older versions
   at each re-pin; rejected by Owner decision 2026-11-26-162 in favour of each script version's own
   lock and design 046's signed layer under the namespace's supply-chain policy.
9. **Trust the sandbox's schema and audit report** — the contract becomes advisory.
10. **Presigned or direct object transfer for snapshot bytes** — ADR-0023 requires a new decision with
    benchmark data first; kept as Gap 1247.
11. **One database-runner deployment with public and private egress** — fewer pods, but a compromised
    warehouse attempt could reach private networks (the reason ADR-0055 rejects its option 4).

## Consequences

- Good: readers see whole snapshots; publication, rollback and retention are ordinary audited
  PostgreSQL transactions; DuckDB and Polars keep the ADR-0013 isolation except declared, bounded
  mounts; the engine socket never enters the process that parses tenant output; the feature 017
  driver, role and result-view rules stay untouched; the data path is the lake path with fewer
  privileges, so its tests and its operations are largely shared.
- Bad: two attempts per transform (transform and audit) add container start-up latency, and the audit
  reads the parts back from the platform (a second transfer of the snapshot); tmpfs output and spill
  consume the container's memory, so the budget classes must cover them; snapshot bytes pass through
  the engine process (ADR-0023), bounding size (2 GiB hard, provisional) and throughput until
  measured; the data runtime image must be maintained; row-level audit outcomes are only as
  trustworthy as the sandbox (Gap 98, Gap 1249); the footer decoder parses tenant bytes in the engine
  process, bounded and fuzzed, with a separate verifier process as the fallback (Gap 1256).
- Dependent on proposals: Architecture decision 0079 is Proposed and its Kubernetes half (Task
  035-029: whether a node-local socket and work volume can be shared between the two processes) is
  open; the data mode inherits both (Gap 024-001). How a data-mode sandbox runner, which holds no
  token, obtains a design 046 layer is designed before Polars is built (Gap 024-002).
- Operational: one image (the data runtime with its host program and the `python-data` interpreter
  line), a data runner deployment and a data-mode sandbox runner per node that runs data steps, a
  warehouse deployment of the database runner, a retention loop and `tenant_data_bytes` accounting;
  runbooks for quarantine, rollback, retention and the data runner.
- Compatibility: additive IR node kinds, the next free worker-protocol minor, OpenAPI routes and an
  AsyncAPI message; runs without data nodes are unaffected. Every new journal event type declares its
  reset rule in `crates/domain/src/execution/reset.rs`. A binary without these drivers cannot read a
  new-driver connection row, so the drivers stay unrecorded in the instance registry until binary
  rollback to such a release is no longer planned. Removing the requirement stops new data nodes while
  pointers, snapshots and retention keep working.
- Rollback of the decision: deactivate `data-pipelines-v1` and stop the data runner; existing
  snapshots stay readable; retention keeps applying the declared policy unless the operator also
  stops it.

## Evidence

None yet beyond the merged feature 035 attempt path this decision reuses. Constitution Governance
requires, before the superseded parts of ADR-0023 and ADR-0027 are accepted as implemented, benchmark
data (staging and footer-verification throughput, container start-up, tmpfs memory use, publication
commit latency on a declared environment; Task 024-066), failure tests (the mount-profile test,
executing from `/out`, filling `/out` past `size=`, the crash matrix of Success criterion 024-006) and
the operational cost of the image, the data deployments, the warehouse deployment and retention. Also
required: the tests named in **Enforced by**, the sandbox canaries of Success criterion 024-004 and
the concurrency rounds of Success criterion 024-005, recorded in `specs/024-data-engines-pipelines/`.
Design: [spec](../../specs/024-data-engines-pipelines/spec.md),
[research](../../specs/024-data-engines-pipelines/research.md) (Decisions 1242, 1243, 1254–1262, 1265,
1267, 1269, 1279 and 024-001 to 024-018),
[data runtime contract](../../specs/024-data-engines-pipelines/contracts/data-runtime.md).
External sources: <https://duckdb.org/docs/2.0/operations_manual/securing_duckdb/overview.html>,
<https://www.windmill.dev/docs/core_concepts/pipelines>.
