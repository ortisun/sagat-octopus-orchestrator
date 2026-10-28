# ADR-0071: Lake writes and step-output destinations are managed effects recovered at the destination

**Status:** Proposed
**Decided by:** agent proposal (2026-10-27, `/speckit-plan` of feature 035, revised after independent
architecture and security reviews the same day), pending owner review. The scope it serves is the
owner's (`docs/owner-decisions.md`, 2026-10-27, items 1–11).
**Refines / supersedes:**
- **Refines [ADR-0035](ADR-0035-database-connections-and-managed-execution.md):** new connection kinds;
  an egress ceiling on every kind; a second, profile-bound deployment of the managed executor;
  ClickHouse and object-storage writers.
- **Refines [ADR-0013](ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md):** a
  `lake` container profile that runs pinned platform code (not tenant code), mounts an attempt
  directory and one Unix socket, and may open sockets only to that relay. The sandbox runner of a lake
  deployment serves only `lake` containers and may serve several tenants, since each attempt has its
  own container and UID; tenant-dedicated lake pools remain a deployment option (009 pools by trust
  level). ADR-0013's request for evidence before a container gains a network path other than the
  supervisor is met by T-isolation and SC-035-009 before this ADR is accepted.
- **Refines [ADR-0010](ADR-0010-compensations-are-derived-steps.md):** a third derived node kind,
  destinations, which, unlike compensation and reconciliation nodes, sit *in* the forward graph
  between the source and its successors; broker-stream destination nodes run no activity (the engine
  settles them); a reserved revert activity is admitted as a compensation.
- **Refines [ADR-0025](ADR-0025-outbound-brokers-share-the-durable-delivery-pipeline.md):** targeted
  `STEP_OUTPUT` deliveries on dedicated subscriptions, a payload by reference, and an outbox hint from
  the delivery worker's settle transaction.
- **Refines [ADR-0036](ADR-0036-integrated-scoped-managed-and-classified-execution.md) decision 5:** the
  permission catalog grows from 48 to 49 (`database:browse`).
- **Refines ADR-0036 decision 6 and [ADR-0033](ADR-0033-payload-values-are-classified-per-field.md):**
  `x-sagat-classification` annotations are accepted on the reserved `sagat.database.*` nodes, but only
  as egress annotations (never as disclosure carriers); and at egress, and only there, a written
  field's level is the stricter of the level inherited from its source and the annotated one. That is
  a bounded use of the source level, not general taint tracking; disclosure policies are unchanged.

Leaves unchanged ADR-0036 decision 3 (root-only managed execution, applied to the new nodes). Extends the Delivery
Constraints' technology baseline with a platform-owned Python runtime confined to sandbox containers
of the lake deployment.

**Enforced by:** planned. Until these checks exist, every rule is review-only:
- `integration_step_destinations` cases for the broker subscription ceiling re-check at completion and
  for the retry/skip permission split (`run:operate`, `saga:override`);
- `scripts/check-architecture.py`: `crates/infra-lake` and the `lake` feature absent from the
  control-plane, engine and scheduler graphs, checked on `cargo tree -e features`;
- the pinned `DatabaseFailure` set test;
- T-commit-once and the conditional-commit race test (`fault_iceberg_writes`);
- T-isolation, the relay and hostile-catalog canaries (`integration_iceberg_writes security::`);
- the stricter-classification test;
- the derived-node and profile-binding tests (`integration_step_destinations`).

## Context

Feature 035 lets workflows write to customer Apache Iceberg tables (append, overwrite and upsert, on
REST, Glue, Hive and JDBC catalogs over S3, GCS and ADLS). It also lets any root-level step save its
output synchronously to Iceberg, PostgreSQL, object storage, ClickHouse or a broker stream. Each of
these is an external effect: the destination's commit and the platform's completion are separate
commits (constitution III).

Facts (035 `research.md`, Phase 0 and review round 1, checked 2026-10-27):

- **Iceberg clients:** iceberg-rust 0.10.1 only fast-appends to `main` and cannot commit through Hive.
  PyIceberg 0.12.0 covers every mode, catalog and storage in scope except format v3. Iceberg Java
  1.12.0 covers everything but needs a JVM.
- **Idempotency marker:** every Iceberg catalog commits by compare-and-swap of the table's metadata
  pointer, and snapshot summaries accept writer properties. Flink's committer stores its checkpoint id
  there to avoid duplicate commits.
- **Client retries:** both libraries retry commits internally by re-applying the changes to a newer
  table state.
- **Isolation:** a Python process that shares the runner's UID, network and volume could read the
  runner's identity token and other attempts' data. The isolation that `security-and-sandbox.md`
  requires — namespaces, seccomp and cgroups — exists in the feature-005 container driver.
- **Catalog overrides:** a REST catalog can return configuration overrides (FileIO class, endpoints,
  proxies, signers) that clients merge over their own settings.

## Decision

1. **Where it runs.** Iceberg writes and destination writes MUST run only as reserved
   `sagat.database.*` activities of `apps/database-runner`, under `Role::DatabaseRunner` identities
   bound to a deployment profile (`database` or `lake`). The server MUST allow only that profile's
   activities and connection kinds, and only under the negotiated protocol minor. No API, engine or
   scheduler process may contact a catalog, object store, ClickHouse server or destination database.
2. **Isolation.** Iceberg semantics MUST be delegated to a pinned, hash-locked PyIceberg runtime, run
   once per attempt in a sandbox container. The container MUST have its own namespaces, no network
   interface but loopback, a read-only root, no capabilities, seccomp, cgroup limits and a distinct
   UID. It MUST be started by the sandbox runner. The credential-holding runner MUST NOT hold the
   container engine socket. The container MUST receive only that attempt's credentials, served by the
   lake runner over the authenticated relay socket, so credentials never transit the sandbox runner.
3. **Egress.** The container's only route MUST be an authenticated Unix socket to a relay owned by
   the runner. The relay MUST allow only endpoints declared on the pinned connection (or derived from
   its storage type and region), and MUST apply ADR-0035's destination policy to every resolved
   address. The runtime MUST drop catalog-supplied configuration outside a closed allowlist, and MUST
   NOT use ambient credential chains.
4. **Commits.** Every Iceberg commit MUST carry the step's effect key in the summary of each snapshot
   it creates, MUST be preceded by a search of the target ref's history for that key, and MUST be
   conditional on the exact ref head that was searched. Client-internal commit retries MUST be
   disabled. A search that cannot prove absence MUST end in `OUTCOME_UNKNOWN`, never in a write.
5. **Destination nodes.** Synchronous destinations MUST be derived at parse, before region analysis,
   as nodes `<source>~d.<name>` in the source's saga region, scheduled by the transaction that accepts
   the source's completion. Failure policies MUST NOT reopen a terminal step.
6. **Broker streams.** Broker-stream destinations MUST take inline sources only. They MUST store
   record data as run content and insert ADR-0025 delivery envelopes in that same transaction, and the
   scheduler shard owner MUST settle them from the delivery rows.
7. **Recovery rules.** Every destination kind MUST declare its recovery rule (lookup, constraint-backed
   key, effect-key object name with a records checksum, deduplication token with both windows proven,
   or stable message id). A write whose outcome the rule cannot prove MUST wait for a person.
8. **Classification.** A written field's level MUST be the stricter of the level inherited from its
   source and any level a record schema declares; a field that neither the source nor the record
   schema declares is `pii`. It MUST NOT exceed the destination's ceiling, and every kind, broker
   subscriptions included, has one.
9. **Scope of the Python runtime.** It MUST NOT be used outside the lake deployment's sandbox
   containers. Replacing it with a Rust client needs a new ADR, once a Rust client supports overwrite,
   row-level changes, branches and every catalog in scope.

## Considered options

1. **PyIceberg in a per-attempt sandbox container behind an authenticated relay (chosen).** It meets the
   owner's scope today, and the container boundary contains a parser compromise inside one attempt.
2. **PyIceberg as a subprocess of the runner.** Cheaper (no container start), but rejected by the
   security review: same UID, network and volume as the runner, so the runner's token, other attempts'
   records and their relays were reachable.
3. **iceberg-rust in the runner process.** Native and smallest, but cannot overwrite, upsert, delete,
   write branches or commit through Hive.
4. **Iceberg Java in a JVM sidecar.** The most complete, but adds a JVM, a larger image and attack
   surface, and its own retry and status-check logic. Kept as the fallback for v3 and merge-on-read.
5. **Destination writes as a side table drained by a new scheduler loop.** Avoids touching the graph,
   but needs its own leases, recovery, UI and readiness rules; derived nodes reuse all of them.
6. **The delivery worker completing destination steps directly.** One transaction fewer, but it
   bypasses the shard owner's fencing (ADR-0020) and widens what ADR-0025's delivery worker commits.

## Consequences

**Positive:**

- Retried, stale or crashed attempts cannot double-commit an Iceberg write.
- Every destination kind states what it can prove, and the rest goes to review.
- Neither ordinary workers nor a compromised host can reach the new credentials or another attempt.

**Negative:**

- A Python dependency closure joins the supply chain (hash lock, scheduled `pip-audit`, SBOM).
- One more deployment, plus a sandbox profile, to operate.
- Sandbox start-up adds 1–3 s per attempt.
- Copy-on-write only, and v3 tables are refused.
- Synchronous destinations produce small Iceberg commits at high volume.
- Glue conflicts and ADLS DFS cannot be tested locally.
- Publishing writes is admin-only, as for 017.

**Runs in flight:** unaffected. The feature is gated at publication and admission by `lake.enabled`,
and an older engine refuses runs that use it with `UNSUPPORTED_NODE`.

**Rollback of the decision:** disable `lake.enabled`, drain, and remove the lake deployment and
profile. Connections and subscriptions of the new kinds stay as inert, paused records.

## Evidence

- 035 [spec](../../specs/035-iceberg-write-steps/spec.md).
- [research](../../specs/035-iceberg-write-steps/research.md): D035-026..069, with sources and dates,
  and the review findings.
- [plan](../../specs/035-iceberg-write-steps/plan.md): transaction boundaries and the failure matrix.
- [contracts](../../specs/035-iceberg-write-steps/contracts/).

No measurement exists yet. The planned evidence is SC-035-003, SC-035-006, SC-035-009, SC-035-013 and
the T-isolation spike.
