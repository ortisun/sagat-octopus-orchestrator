# ADR-0058: Engine-local data transforms run in per-attempt sandboxes over immutable snapshots, and assets are published by an atomic PostgreSQL pointer

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26; revised 2026-10-27 after the architecture review and coordinator decisions)
**Refines / supersedes:** **Supersedes in part** [ADR-0013](ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md) (for the data profile only: "no mounts" and the small `/tmp`; and the exact permission set of the `runner` role, which gains `data:execute`), [ADR-0023](ADR-0023-blob-store-claim-check.md) ("committed blobs are never deleted", for the `DATA_SNAPSHOT_PART` purpose only) and [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) ("does not authorize deletion of customer data", for snapshot bytes under declared asset retention only, pending owner question O1243). **Refines** ADR-0023 (an additive ranged read on the `BlobBackend` port; a separate pending-upload ceiling for the new purpose), ADR-0027 (a `data` accounting class, the `tenant_data_bytes` aggregate, and the publication event as a bounded `protected_system` record), ADR-0033 (the `dataQuery` output joins the 016 carrier set with a platform-derived schema) and [ADR-0035](ADR-0035-database-connections-and-managed-execution.md) decision 4 (a separate warehouse deployment of the database runner). Leaves unchanged ADR-0035 decision 10 and [ADR-0036](ADR-0036-integrated-scoped-managed-and-classified-execution.md) decisions 1 and 6: DuckDB is a data node, not a 017 driver. Coordinates with ADR-0054 (020), which excludes `DATA_SNAPSHOT_PART` blobs from run purge; this ADR owns their retention. **Dependents to update** when accepted: ADR-0013 (a "partially superseded by ADR-0058" note), `docs/architecture/security-and-sandbox.md` (data profile, `runner` permissions), `docs/runbooks/scripts.md`, the 005 spec and research (D97 role set), ADR-0023 (note on the purpose and the port method), ADR-0027 (the `data` class), `crates/infra-postgres/storage-classification.json`, the 016 carrier list (C6) and 020's content-store registry (C8).
**Enforced by:** planned — `scripts/check-architecture.py` with a per-crate dependency allowlist (one allowlist shared with feature 021: only `crates/infra-data` may depend on `parquet`; no package linked into `apps/control-plane`, `apps/execution-engine`, `crates/scheduler` or `apps/database-runner` depends on DuckDB, Polars or Arrow compute) (T024-005, decision 1); the mount-profile unit test in `crates/infra-sandbox` and the container canaries (T024-028, T024-039, decision 1); T024-031 (pinned-only reads, decision 2); T024-033 including its fuzz target (decision 3); T024-035 (audit isolation and digests, decision 4); T024-034, T024-036 and T024-060 (pointer atomicity, decision 5); T024-010 and T024-038 (the guarded `COMMITTED → DELETING` trigger, run-purge exclusion and retention, decision 7); T024-022 (warehouse deployment NetworkPolicy, decision 8); T024-041 (classified `dataQuery` output, decision 9); T024-046 (publication at full quota, decision 10). Until they exist every rule is review only.

## Context

Feature 024 adds data pipelines: DuckDB SQL and Polars transformations over namespace data, declared
assets with schema contracts, lineage, freshness and write-audit-publish (Windmill's Pipelines
publish into managed DuckLake tables; its Enterprise write-audit-publish runs checks inside the
write transaction so readers keep the previous version). The forces:

- DuckDB and Polars have no remote server: the transformation *is* tenant-authored computation.
  DuckDB's own security guidance says its settings "are not a substitute for proper sandboxing" and
  recommends an isolated container. Constitution V forbids untrusted code in the control plane or
  scheduler; the 017 database runner holds the credentials of every concurrent attempt it serves.
- ADR-0013's container takes code and input on standard input and returns one line; it has no mounts,
  because a writable disk has no per-container quota without XFS project quotas. Datasets of hundreds
  of MiB cannot travel that way.
- Readers must never see a half-written dataset, and "publish" must be decided by the authority.
  PostgreSQL is the authority (ADR-0002); object stores reached through the provider-neutral port of
  ADR-0023 share no compare-and-set primitive and cannot commit with a step transition and its outbox.
- ADR-0023 keeps committed blobs forever and ADR-0027 does not authorize deleting customer data (G42).
  Snapshots are large and superseded routinely; without a retention rule they grow without bound.
- BigQuery and Snowflake are reached over public HTTPS; NetworkPolicy cannot list host names, so
  public egress in a pod that also routes to private customer databases mixes two exposures.

## Decision

1. DuckDB SQL and Polars code MUST execute only in a container per attempt started by the sandbox
   runner, with the ADR-0013 profile except exactly these mounts: `/in`, a read-only
   `ro,noexec,nosuid,nodev` bind of a per-attempt directory the runner created, filled and bounded;
   `/out` and `/spill`, tmpfs `noexec,nosuid,nodev` with a `size=` limit charged to the container's
   memory cgroup (the size mechanism). Nothing else is mounted. Because `/out` is discarded with the
   container, the host program MUST stream each part to the runner over standard output in bounded
   length-prefixed frames before it exits (this extends ADR-0013's "one line out" for the data
   profile only); the runner bounds the stream, uploads each part as it arrives and keeps no copy. They MUST NOT execute in the control
   plane, the execution engine, the scheduler, the database runner or any process that holds another
   attempt's credential. DuckDB hardening settings are applied and locked as defence in depth only.
   The `runner` role gains `data:execute` and advertises data activities (`sagat.data.*`) only after
   the data image's probe passes.
2. A data node MUST read only immutable inputs pinned in the transaction that grants its first lease
   (published or same-run audited snapshots, or committed outputs of earlier steps of the same run).
   The runner MUST stage them through lease-authorized reads of exactly the pinned objects; any other
   data object MUST look missing to that lease.
3. Snapshot bytes MUST be Parquet parts in the ADR-0023 blob store under the purpose
   `DATA_SNAPSHOT_PART`, uploaded with the exact attempt fence. A snapshot record MUST be created only
   in the completion transaction, after `apps/execution-engine` itself verified each part's fence,
   measured size and SHA-256, and Parquet footer (read by a bounded ranged request, decoded with
   bounded limits on a panic-isolated task) and schema against the pinned contract. The sandbox's
   report is never sufficient.
4. Audits declared on the asset MUST run in a fresh container over exactly the uploaded parts, which
   the runner reads back from the platform with `ReadBlob` (verified against the digests the server
   measured), after the transform container exited; the engine MUST
   refuse a completion whose audited digests differ from the committed parts. Outcomes MUST be
   payload-free. A failed or errored audit MUST commit the snapshot as `QUARANTINED`; a quarantined
   snapshot MUST NOT become current or feed a same-run input.
5. The current snapshot of a managed asset MUST be a pointer row in PostgreSQL with a monotonic
   version, moved only by a compare-and-set against the version captured at the producer's first
   lease (or supplied by an API caller), in one transaction with the publication record, the inbox
   event `data.asset.published`, the outbound delivery rows and the audit — and, when a run
   publishes, with the step or node transition, journal, next tasks and outbox. Asset heads MUST be
   locked in ADR-0050's slot 4 at admission, and after the run and authority guards elsewhere, before
   any row that references them. Apart from an explicit, audited API rollback, an older snapshot MUST
   NOT overwrite a newer publication. Readers MUST pin snapshot ids.
6. These guarantees MUST be claimed only for platform-catalogued (managed) assets. Tables inside a
   customer warehouse are external assets with observations from their declared producer; they MUST
   NOT be presented as having write-audit-publish.
7. Only `DATA_SNAPSHOT_PART` blobs MAY move from `COMMITTED` to `DELETING`, only by this feature's
   retention after a committed decision that rechecked live-run pins under the asset row lock, and
   only once the owner has accepted destructive deletion (O1243); until then retention labels
   snapshots `RETIRED` and keeps their bytes. Deletion follows ADR-0023's tombstone and durable-retry
   rules. Run purge (020, ADR-0054) MUST NOT delete this purpose nor snapshot metadata (snapshots,
   parts, provenance, audits, publications), which this ADR owns: purging a producer run must not
   remove what a current pointer, a pin or a provenance edge names. Every other purpose keeps its
   020-owned retention.
8. Remote engines (MySQL, MariaDB, SQL Server, BigQuery, Snowflake) remain in ADR-0035's model and
   its `database-runner` role. BigQuery and Snowflake MUST run in a separate deployment of
   `apps/database-runner` whose NetworkPolicy allows DNS, the engine and TCP 443 to non-private ranges
   or operator-listed private-endpoint networks and no route to private customer networks; the
   private deployment has no public egress. The process also enforces the driver-fixed host form and
   an operator host allowlist that namespace policy can only narrow.
9. DuckDB queries MUST be `dataQuery` data nodes, never a 017 database driver. A `dataQuery` output is
   inline (≤ 256 KiB) in the 017 typed-cell shape and is a 016 classification carrier annotated at the
   highest level among its pinned input columns, disclosed only through the existing 016 disclosure;
   the 017 result view and its rules are unchanged.
10. Snapshot bytes MUST be charged once, to `tenant_data_bytes` under a `data` class, when each part
    upload is reserved and measured; `DATA_SNAPSHOT_PART` uploads have their own pending ceiling. The
    publication event MUST be a closed document of at most 1 KiB stored as a `protected_system` record:
    it is not charged to content quota and does not spend `events_per_second`, so a full quota never
    rolls back a completion or a publication.

## Considered options

1. **Per-attempt sandbox with declared mounts, engine-verified parts, PostgreSQL pointer (chosen)** —
   keeps tenant code out of every privileged or credential-holding process, bounds the extra mounts by
   the memory cgroup and `noexec`, makes the contract the engine's check rather than the sandbox's
   claim, and gives an atomic publication that commits with the run's transition and outbox.
2. **Keep ADR-0013 unchanged and stream datasets through standard input and output** — no new mounts,
   but a GiB-sized input and output through one pipe and one result line has no seekable access for
   Parquet and no bound other than the pipe; rejected.
3. **Embed DuckDB in the database runner** — no container start-up cost, but one hostile query or
   engine bug shares memory with other attempts' credentials; DuckDB's own guidance rejects
   settings-only isolation.
4. **DuckDB as a 017 database driver** — reuses the connection catalog and result view, but puts a
   managed database task in the sandbox runner (against ADR-0036 decision 1) and needs a
   classification gate on the 017 result view (against ADR-0035 decision 10, ADR-0036 decision 6).
5. **Publish by an object-store manifest or a lake catalog (DuckLake, Iceberg)** — the ADR-0023 port
   has no common compare-and-set and the manifest cannot commit with the step transition and outbox;
   a lake format can be added later as an export, not as the authority.
6. **Trust the sandbox's schema and audit report** — the contract becomes advisory.
7. **Presigned or direct object transfer for snapshot bytes** — ADR-0023 requires a new ADR with
   benchmark data first; kept as G1247.
8. **One database-runner deployment with public and private egress** — fewer pods, but a compromised
   warehouse attempt could reach private networks (the reason ADR-0055 rejects its option 4).

## Consequences

- Good: readers see whole snapshots; publication, rollback and retention are ordinary audited
  PostgreSQL transactions; DuckDB and Polars keep the ADR-0013 isolation except three declared,
  bounded, `noexec` mounts; the 017 driver, role and result-view rules stay untouched.
- Bad: two container starts per transform attempt add latency, and the audit reads the parts back
  from the platform (a second transfer of the snapshot); tmpfs output and spill consume the
  container's memory, so memory ceilings must cover them; snapshot bytes pass through the engine
  process (ADR-0023), bounding size (2 GiB hard, provisional) and throughput until measured; the SQL
  data image and, if accepted, the `python-data` image and catalog must be maintained and re-pinned
  (re-pinning strands older Polars versions, G1255); row-level audit outcomes are only as
  trustworthy as the sandbox (G98, G1249); the footer decoder parses tenant bytes in the engine
  process, bounded and fuzzed, with a separate verifier process as the fallback (G1256).
- Operational: two new images, a runner data profile, a second database-runner deployment, a
  retention loop and `tenant_data_bytes` accounting; runbooks for quarantine, rollback, retention
  and re-pinning.
- Compatibility: additive IR node kinds, protocol minor, OpenAPI routes and AsyncAPI message; runs
  without data nodes are unaffected. Every new journal event type declares its 018 reset rule. A
  0.2.0 binary cannot read a new-driver connection row, so new drivers stay disabled until binary
  rollback is no longer planned. Disabling the capability stops new data nodes while pointers,
  snapshots and retention keep working.
- Rollback of the decision: disable data publication and new data nodes; existing snapshots stay
  readable; no bytes are deleted unless O1243 was accepted.

## Evidence

None yet: this is a proposal. Constitution Governance requires, before the superseded parts of
ADR-0013, ADR-0023 and ADR-0027 are accepted as implemented, benchmark data (staging and
footer-verification throughput, container start-up, tmpfs memory use, publication commit latency on
a declared environment; T024-066), failure tests (the mount-profile test, executing from `/out`,
filling `/out` past `size=`, the crash matrix of SC-024-006) and the operational cost of the images,
the second runner deployment and retention. Also required: the tests named in **Enforced by**, the
sandbox canaries of SC-024-004 and the concurrency rounds of SC-024-005, recorded in
`specs/024-data-engines-pipelines/`. Design: [spec](../../specs/024-data-engines-pipelines/spec.md),
[research](../../specs/024-data-engines-pipelines/research.md) (D1242, D1243, D1254–D1262, D1265,
D1267, D1269, D1270, D1279), [data runtime contract](../../specs/024-data-engines-pipelines/contracts/data-runtime.md).
External sources: <https://duckdb.org/docs/2.0/operations_manual/securing_duckdb/overview.html>,
<https://www.windmill.dev/docs/core_concepts/pipelines>.
