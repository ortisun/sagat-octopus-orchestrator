# Research: Multi-region replication and active-passive failover (feature 032)

Baseline investigation: 2026-10-26, worktree `sagat-octopus-parity-specs`, branch
`feature/018-032-parity-specs` cut from `origin/develop` at `70f09435` (release 0.2.0). Nothing below is
implemented. Identifier range of this feature: D1560–D1599, G1560–G1599, O1560–O1599; ADR-0067.

## 1. What exists today (verified paths)

- **In-region HA.** ADR-0002, ADR-0009 ("open a new ADR with benchmark data, failure tests, operational
  cost and compatibility with existing runs"), ADR-0020 (64 shards; owner + generation + expiry; fence
  `FOR SHARE` in the action's transaction; the random owner suffix already covers "a promoted
  asynchronous standby that lost the last takeover"). Tables: `20261017000000_scheduler_shards.up.sql`
  (`scheduler_shards`, `scheduler_members`), `20261015000100_telemetry_projection.up.sql`
  (`engine_leases`). Port `crates/application/src/shards.rs`, adapter
  `crates/infra-postgres/src/partitioning.rs`, keeper `crates/scheduler/src/shards.rs`.
- **DR.** `docs/runbooks/disaster-recovery.md` ("The multi-region case is out of scope"), `infra/dr/drill.sh`,
  `infra/dr/journal_digest.sql`; owner decision 8 (`docs/owner-decisions.md`, D241): RPO 0 / failover RTO
  ≤ 5 min / PITR RTO ≤ 1 h in-region, gating releases. Feature 010 gaps G242 (no read replicas), G250
  (no other zone/region, object store, broker, key ring in the drill), G251 (object store), G252 (clocks).
- **Commit timeout.** `crates/infra-postgres/src/pool.rs` sets server `statement_timeout` (5 s,
  `SAGAT_DB_STATEMENT_TIMEOUT_MS`) and `idle_in_transaction_session_timeout` (60 s) on pooled connections.
- **Absent today.** No code calls `pg_is_in_recovery()` or handles SQLSTATE `25006`; no migration declares
  `UNLOGGED`, a sequence, `serial` or an identity column; no `LISTEN/NOTIFY` (grep at `70f09435`).
  Identifiers are application UUIDv7; journal sequences are row counters; advisory locks are
  transaction-scoped.
- **Inbox/outbox.** ADR-0012 (inbox key `(tenant, namespace, source, event_id)`, 202 after commit,
  connector acknowledgment after the inbox commit, stable `X-Octopus-Delivery-Id`), ADR-0025 (one delivery
  pipeline). `webhook_deliveries_event_uniq (subscription_id, event_id)`; subscription kinds constrained in
  `20261015000200_operational_alerts.up.sql`; `command_dedupe` holds API idempotency keys.
- **Worker protocol.** `worker.proto`: `RegisterWorkerResponse {worker_id, session_token,
  accepted_protocol_version}`, `x-sagat-error` trailers; lease tokens stored as SHA-256. `sdks/SPEC.md` §5
  (retry by code; "a `reason` in a server's answer never makes a call repeatable"), §6.2 (re-register on
  `UNAUTHENTICATED`, reconnect on `UNAVAILABLE`).
- **Errors.** `sdks/error-catalog.json`; OpenAPI `Error.details.reason` refines codes without new values.
- **Following runs.** `getRuntimeGraphChanges` and `streamRunChanges` with `GraphChanges.reset`; handler
  `run_stream` in `crates/api-http/src/routes/execution.rs`; Studio `frontend/apps/studio/src/api/follow.ts`.
- **Probes.** `crates/api-http/src/routes/health.rs` (`/v1/ready` pings storage, cached 1 s); `/v1/version`
  needs a token and no permission.
- **Blobs.** `crates/application/src/blobs.rs` (`BackendFailure::NotFound`, `BlobIoError::Corrupt`);
  catalog with `sha256` in `20261018000000_blobs.up.sql`; `crates/infra-blobstore/src/config.rs`.
- **Secrets.** `secret_versions.key_id` (`20261010000200_secrets.up.sql`) lets a key-ring coverage check run
  without decrypting.
- **Operator CLI.** `apps/control-plane/src/cli.rs` (subcommands, environment inputs); exit codes in
  `apps/control-plane/src/lib.rs`. **Audit**: `audit_records` is tenant-scoped and append-only.
- **Guarded egress.** `crates/infra-egress/src/lib.rs` refuses private destinations (D1570).

## 2. External semantics consulted

- **Temporal Cloud High Availability and failover** —
  <https://docs.temporal.io/cloud/high-availability>, <https://docs.temporal.io/cloud/high-availability/failovers.md>,
  <https://docs.temporal.io/cloud/rpo-rto>. Graceful failover drains replication then switches (RPO zero);
  a forced failover "immediately activates the replica" and can lose unreplicated writes; "Completed
  Activities may be re-dispatched … leading to repeated executions"; the namespace endpoint uses DNS to
  redirect to the active region and "can take a few minutes" to reach clients; under a partition "both
  regions will be active simultaneously" and histories are reconciled afterwards. Targets stated by
  Temporal for HA namespaces: sub-1-minute RPO, RTO ≤ 20 minutes.
- **Temporal multi-cluster replication (self-hosted)** —
  <https://docs.temporal.io/temporal-service/multi-cluster-replication>, <https://docs.temporal.io/global-namespace>.
  A global namespace is active in one cluster at a time; a failover version orders conflicting
  failovers; replication is asynchronous; calls to a standby can be forwarded (`dcRedirectionPolicy`) or
  rejected with `NamespaceNotActiveError` naming the active cluster; activities are at least once.
- **Windmill high availability** — <https://www.windmill.dev/docs/advanced/high_availability>. All state in
  PostgreSQL; multi-datacenter = streaming replica in the second datacenter; failover = promote and point
  servers/workers at it; running jobs "will not complete"; triggers may need reprocessing; no fencing,
  split-brain or RPO/RTO statements.
- **PostgreSQL synchronous replication** —
  <https://www.postgresql.org/docs/current/warm-standby.html#SYNCHRONOUS-REPLICATION>: "the application
  will not receive explicit acknowledgment of the successful commit of a transaction until the WAL data
  is known to be safely received by all the synchronous standbys"; "If primary restarts while commits are
  waiting for acknowledgment, those waiting transactions will be marked fully committed once the primary
  database recovers"; `synchronous_standby_names` with `FIRST`/`ANY`; "If the primary is isolated from
  remaining standby servers you should fail over to the best candidate of those other remaining standby
  servers."
- **PostgreSQL source, `src/backend/replication/syncrep.c`** (<https://github.com/postgres/postgres>): on a
  query cancel during the wait it emits WARNING "canceling wait for synchronous replication due to user
  request" with detail "The transaction has already committed locally, but might not have been replicated
  to the standby", and stops waiting without raising an error, because an error "would make the client
  think the transaction aborted". `src/backend/access/transam/xact.c`: the wait happens after the commit
  record and clog update, "but still show as running in the procarray and continue to hold locks", so
  other sessions do not see the transaction while it waits.
- **Recovery functions** — `pg_last_wal_replay_lsn()` and `pg_last_xact_replay_timestamp()` stay static
  at the last replayed record after recovery ends (PostgreSQL "System Administration Functions",
  <https://www.postgresql.org/docs/current/functions-admin.html>); `pg_rewind`:
  <https://www.postgresql.org/docs/current/app-pgrewind.html>.
- **Operator tooling examples (not dependencies)** — CloudNativePG replica clusters, distributed topology
  with controlled demotion/promotion since 1.24:
  <https://cloudnative-pg.io/documentation/current/release_notes/v1.24>,
  <https://cloudnative-pg.io/docs/1.27/replica_cluster>; Patroni standby cluster (a standby leader that
  replicates from a remote primary): <https://patroni.readthedocs.io/en/latest/standby_cluster.html>.
- **Object storage** — S3 Replication Time Control "replicates most objects … in seconds, and 99.9 percent
  of those objects within 15 minutes", with replication metrics:
  <https://docs.aws.amazon.com/AmazonS3/latest/userguide/replication-time-control.html>. Delete-marker
  replication is a separate rule setting (same page, example configuration).
- **OpenBao** — Raft snapshots (`bao operator raft snapshot save|restore`):
  <https://openbao.org/docs/commands/operator/raft/>. No built-in cross-cluster replication was found in
  OpenBao's documentation during this research (G1560); this is an absence of evidence, not a claim.

## 3. Decisions

- **D1560 — Active-passive per cell; one writable authority; no forwarding.** Temporal can forward and
  merge because its histories are mergeable event logs; run rows, quotas, receipts and leases are not
  (Principle I).
- **D1561 — Physical, not logical, replication.** Logical replication does not carry DDL, re-fires
  triggers and `SECURITY DEFINER` functions on apply and makes conflict handling ours.
- **D1562 — The platform does not elect the active region.** During a partition neither database can
  arbitrate; a platform-run witness would be the second authority ADR-0020 (D230, D239) rejected. The
  operator or the HA manager promotes; the platform verifies, records, fences and reports (ADR-0067).
- **D1563 — Authority row and append-only history in the cell's database**, written only by owner-role
  commands (as `quota-ceiling`; feature 010 G260); no row = single-region mode.
- **D1564 — Epoch in the ADR-0020 fence and singleton leases; activation invalidates the previous
  epoch's shard leases, singleton leases and memberships** in its own transaction, so the new region takes
  all 64 shards at once instead of waiting for replicated rows to expire. No new lock.
- **D1565 — Two modes; protection observed, not assumed.** `sync_quorum` counts only standbys outside the
  active region; the monitor uses an optional `pg_monitor` connection and the `application_name`
  convention `<region>-<name>`; without it protection is `UNKNOWN` (G1573).
- **D1566 — A cancelled replication wait is not an acknowledgment.** PostgreSQL returns success with a
  WARNING on a cancelled wait (syncrep.c) and the pools set a 5 s server `statement_timeout`
  (`crates/infra-postgres/src/pool.rs`). In `sync_quorum` cells COMMIT is exempted from the server timeout
  (`SET LOCAL statement_timeout = 0` just before it) and bounded by a client-side deadline that abandons
  the connection and reports `COMMIT_OUTCOME_UNKNOWN`; notice detection is a further defence if the driver
  exposes it. That `statement_timeout` interrupts the wait like an explicit cancel is to be proven by
  T032-023 (G1578). The COMMIT guard alone is not enough: D1590 covers the read paths.
- **D1567 — Behaviour by role** (plan table). Passive refuses before authentication and touches no data;
  draining refuses admissions and keeps reads and reports of granted leases. No redirect: a 307 would
  replay a bearer token to another host.
- **D1568 — Region-local data endpoints, cell endpoints for everyone else.** A former-region process can
  never reach the new authority by a DNS change; clients use cell endpoints the operator's GSLB moves.
- **D1569 — No new worker fence.** Superseding sessions was rejected: what the new authority holds is valid
  there, what it lacks is already refused (`UNAUTHENTICATED`, `STALE_ATTEMPT` by lease-token hash), and it
  would break feature 017's managed session-ownership rule.
- **D1570 — Stop-only peer probe through `crates/infra-egress/src/peer.rs`**, separate from tenant egress
  (which refuses private addresses): configured https URLs, configured CA, no redirect or proxy, bounded.
  Its target is the peers' admin listener (D1598).
- **D1571 — Planned activation evidence is an LSN**: the operator's final LSN of the cleanly stopped former
  primary against `pg_last_wal_replay_lsn()`, static after promotion and NULL after a restart (then the
  unplanned path applies).
- **D1572 — Unplanned activation needs an attestation and a preflight** (blocking vs warning checks of
  `contracts/operator-cli.md`); an unevaluable blocking check blocks.
- **D1573 — Followers reset with the reset that exists.** `GraphChanges.reset` already answers "when
  `since` is ahead of the run or more than 500 events behind it" (OpenAPI `getRuntimeGraphChanges`); the
  feature adds the epoch of the event at the cursor (D1594) as one more reason, so a follower whose
  sequence replicated is not reset. No new SSE event type.
- **D1574 — Blob grace counts from the activation** (`SAGAT_BLOB_REPLICA_GRACE_SECS`, 900 s = the S3 RTC
  threshold, ≤ 3,600 s): `BLOB_NOT_REPLICATED` then `BLOB_LOST`; steady state unchanged; leased verifier.
- **D1575 — Failover notice through the existing pipeline**: outbound kind `ops.region.activated`, event id
  UUIDv5 over `(cell, epoch, namespace)`, deduped by `(subscription, event_id)` (ADR-0025).
- **D1576 — Tenant home cell; residency on the namespace** (tenant-scoped authority stays single).
- **D1577 — No new error code.** Reasons `REGION_PASSIVE`, `REGION_DRAINING`, `REGION_SUPERSEDED`,
  `COMMIT_OUTCOME_UNKNOWN`, `BLOB_NOT_REPLICATED` (`UNAVAILABLE`), `PLACEMENT_RESIDENCY_CONFLICT`
  (`VALIDATION_FAILED`), step failure `BLOB_LOST` (`contracts/errors.md`).
- **D1578 — Fixed metric labels.** Region names are operator-configured, at most three per cell: a bounded
  label like a pool name (feature 010 D237).
- **D1579 — Rewind or reclone the former primary; report divergence; re-admit inbound events only.**
- **D1580 — No stale reads**: the passive region refuses reads too (feature 010 G242 stays open).
- **D1581 — CLI shape of the existing commands**: hyphenated subcommands, environment inputs, one JSON
  line, exit codes 0/1/2 (`apps/control-plane/src/lib.rs`).
- **D1582 — The drill simulates two regions on one host and says so**: two clusters, a TCP proxy for
  RTT/loss/partition, two blob stores with a controllable replicator, two NATS, a resolver override.
- **D1583 — Constitution amendment, stated not applied.** Delivery Constraints, technology baseline:
  "single region with high availability" → "one active region per cell with high availability; an
  optional cross-region standby region with active-passive failover as defined by ADR-0067". "Out of scope
  for V1: multi-region active-active, …" unchanged. 1.0.0 → 1.1.0 (MINOR: guidance expanded, no principle
  removed or redefined). Sync Impact Report: ADR-0009 status, `docs/architecture/overview.md`,
  `capacity-and-topology.md`, `docs/runbooks/disaster-recovery.md`, the "V1 single-region HA" assumption
  lines of specs 001–012 (annotated, not rewritten). The amendment and ADR-0009's line wait for evidence
  (D1592).
- **D1584 — No new crate** (placement in plan §Source placement).
- **D1585 — Commit deadline** 10 s default, 60 s ceiling, at least twice the statement timeout.
- **D1586 — RPO** = acknowledged ledger operations missing (count) and the window from the cut point to the
  last former-region acknowledgment; **RTO** = detection (reported separately) + promotion + activation +
  routing propagation + shard takeover + first successful start through the cell endpoint.
- **D1587 — Contract test against `UNLOGGED` tables and sequences** (empty or skipping after promotion).
- **D1588 — The history row is the platform audit**; tenant `region.activated` records follow by a
  bounded, idempotent fan-out (G1575).
- **D1589 — Permission `placement:manage`**, admin only; reading placement needs `namespace:read`; 029
  treats it as one more catalog permission (coordinator C10).
- **D1590 — Acknowledgment through read paths (review Blocking 1).** A transaction whose replication wait
  was cancelled or terminated, or that was waiting when the primary restarted, is visible locally, so
  a keyed replay, an inbox `DUPLICATE`, a worker `already_applied` or a status read could acknowledge it.
  Rules: (a) acknowledging replays run in a transaction that calls `pg_current_xact_id()` and commits
  with `synchronous_commit` at least `on`; PostgreSQL then writes a commit record and waits
  (`RecordTransactionCommit` waits when an xid was assigned, xact.c), and since that record follows the
  replayed work in the WAL, its quorum acknowledgment covers it; (b) outcome reads need a durability
  lease renewed by a quorum-waited beacon commit at each role refresh (validity counted from the start
  of the beacon transaction, like ADR-0020's `valid_until`); (c) a barrier (one beacon) before serving
  after start, reconnect, a changed `pg_postmaster_start_time()` or an observed unknown outcome; (d)
  `client_connection_check_interval = 0` for platform roles, so an abandoned backend keeps waiting
  instead of committing locally, and no cancel or terminate of waiting platform backends (runbook). The
  alternative "refuse until a quorum standby's `flush_lsn` passes the local flush LSN" needs a monitor
  connection on every process; one beacon commit proves the same thing with the runtime role.
  Consequence: during a partition the former region refuses quickly instead of hanging per request, and
  at most one pool of abandoned backends per process accumulates before the barrier stops new commits.
- **D1591 — In-region RPO inside a multi-region cell (review Should-fix 2).** Protection rule: a
  `sync_quorum` configuration is protected when k exceeds the number of in-region members of the
  `ANY k`/`FIRST k` list. Feature 010's in-region RPO 0 is kept either by a quorum that always includes the
  in-region standby (`ANY 2 (eu-west-2, eu-central-1)`: both must acknowledge; no standby may be lost
  without degrading) or by an in-region promotion that first brings the chosen standby to the highest
  flush LSN any synchronous standby reported (cascading replication or the WAL archive). The quickstart
  and harness use the first form with three clusters.
- **D1592 — Governance gate (review Should-fix 3).** ADR-0009 and the constitution's Governance require a
  diverging ADR to carry benchmark data and failure tests. The owner may accept the direction now; the
  ADR's `Accepted` status, ADR-0009's "Superseded in part" line and the constitution amendment wait for the
  region drill report and the commit-latency benchmark (T032-063). The amendment text stays in the ADR.
- **D1593 — Epochs enforced in the database, not by an operator flag (review Should-fix 4).** Triggers on
  `scheduler_shards`, `engine_leases`, `scheduler_members` and `connector_leases` refuse a granting or
  renewing write (expiry after `clock_timestamp()`) unless `current_setting('sagat.region', true)` is the
  active region, the authority is `ACTIVE` and `NEW.region_epoch` (where present) is the current epoch;
  releases and expiries pass. `region-init` sets `ALTER ROLE sagat_app SET default_transaction_read_only =
  on` (stored in `pg_db_role_setting`, physically replicated); region-aware processes connect with
  `-c sagat.region=<r> -c default_transaction_read_only=off` only while `ACTIVE` and rebuild their pool on
  a role or epoch change. An older binary sets neither, so it can neither take work nor write once the
  cell is initialized. `SAGAT_REGION_ALL_PROCESSES_UPGRADED` is withdrawn. The settings are a
  compatibility fence against old binaries, not a security boundary against code holding the runtime
  role's credentials (which can always set them).
- **D1594 — Epoch in downstream keys (review Should-fix 5; coordinator C2).** `run_events.region_epoch
  bigint NOT NULL`, added with `DEFAULT 0` (metadata-only on existing rows) and then `DEFAULT
  sagat_region_epoch()` (a `STABLE` SQL function reading `region_authority`, 0 when absent), so every
  insert path records the epoch of the authority it commits under without code at each call site. Span
  ids include it (unchanged formula at 0). 020 keys analytic records by `(tenant, run, seq)` and adds the
  epoch component to its keys, dedupe tokens and object names. After an unplanned activation a projector
  emits 020's `run_tombstone` for a run when the run's next event has a later epoch than the one before
  (stateless detection; no extra table), for earlier epochs from that sequence on; `region-emit-tombstones`
  turns the divergence report's former-only runs into whole-run tombstones. 026 uses `(epoch, seq)` in
  its stream cursor. Cost: 8 bytes and one function call per journal row (benchmark criterion, T032-046).
- **D1595 — Key inventory (coordinator C11).** Closed list in §6; fingerprints recorded by the active
  region, compared by passive processes and by the activation preflight; blocking for keys whose
  divergence is unsafe, warning for keys whose divergence fails closed.
- **D1596 — CDC under failover (coordinator C12).** In `sync_quorum`, 021's flush LSN is reported after the
  quorum-waited inbox commit (FR-032-009 makes that true); under `async`, changes inside the RPO are lost:
  the slot moved past them; slot re-creation or a backfill (021) recovers them, and the divergence report
  and `region-reingest-inbox` help when the former primary is reachable.
- **D1597 — Time classes (coordinator C13).** Every time column of this feature is class `infrastructure`
  in 028's `crates/infra-postgres/time-classification.json`; `region_history.cut_replay_at` is the former
  primary's clock and is stated so; virtual test time never advances failover state.
- **D1598 — `/v1/region` needs a token; the peer probe reads `GET /region` on the admin listener (review
  Minor 1).** The admin listener is on the internal network (as `/metrics`, `/livez`), reachable over TLS
  when peers are configured, answers only cell, region, role and epoch, and needs no token.
- **D1599 — Activation also removes `connector_leases` (review Minor 2).** Step leases, script
  build/test job leases (`lease_token_hash`, `lease_expires_at`) and secret-cleanup claims (90 s) are
  fenced by their tokens and only delay work until their TTL; activation leaves them.

## 3b. Key inventory of a cell (coordinator C11)

Fingerprint: symmetric key → first 16 bytes of HMAC-SHA-256(key, `sagat/key-fingerprint/v1/<name>`);
asymmetric → SHA-256 of the public key. Recorded per `(name, key id)`; a key absent in every region is
not deployed and not checked.

| Key (`name`) | Owner | How it reaches each region | Rotation | Divergence between regions | Preflight |
|---|---|---|---|---|---|
| `secrets_key_ring` (`SAGAT_SECRETS_KEYS`) | 009 | operator copies the ring to each region's secret store | add everywhere, then activate; `reseal-secrets` | sealed secrets unreadable; fail closed | blocking (ids + fingerprints) |
| `script_signing` (`SAGAT_SCRIPT_SIGNING_KEY`; runners pin its public key in `SAGAT_RUNNER_TRUSTED_KEYS`) | 005, 023 | operator secret per region | new key trusted by runners first | new artifacts refused by runners of the other region | blocking |
| `intent_binding` (017 32-byte provisioning binding key) | 017 | control-plane secret per region | per 017 runbook | OpenBao intent reconciliation fails; secure resubmission | blocking |
| `flow_control_digest` (022, D1168) | 022 | control plane and engine secret per region | refused while holders/waiters exist (G1163) | a business key maps to a new semaphore: concurrency limit bypass | blocking |
| `push_signing` (`SAGAT_PUSH_SIGNING_KEY`) | 019 | engine secret per region; public key at `/v1/push-signing-keys` | publish new before use | receivers pinned to a key id refuse pushes | warning |
| `fairness_digest` (019 deployment digest key) | 019 | secret per region | G1057 | fairness-key digests and grouping change | warning |
| `decision_link` (`SAGAT_DECISION_LINK_KEYS`) | 026 | control-plane file per region | current + previous | outstanding decision links refused (fail closed) | warning |
| `codec_assertion` (`SAGAT_CODEC_ASSERTION_KEYS`; JWKS `/v1/codec-jwks`) | 031 | control-plane secret per region | current + previous, JWKS cache | codec servers with a cached JWKS refuse assertions | warning |
| `template_index_pins` (`SAGAT_TEMPLATE_INDEXES` pinned keys) | 028 | configuration per region | operator | an index accepted in one region, refused in the other | warning |
| MCP scoped tokens (025) | 025 | stored as SHA-256 in `mcp_tokens` (replicated with the database); OAuth tokens verified by the IdP | — | none (no cell key) | — |
| Git credentials, binding webhook secrets (028); webhook and subscription secrets (004, ADR-0025) | 028, 004 | namespace secrets (key ring or OpenBao) | as secrets | covered by `secrets_key_ring` and OpenBao | — |
| OpenBao workload identity and data | 017, ADR-0035 | per-region workload identity; data by Raft snapshot (G1560) | OpenBao | missing credential versions fail closed | warning (reachable) |
| TLS certificates of cell endpoints and the worker API | deployment | per region, valid for the cell names | operator | clients refuse the other region | — (routing check in the drill) |

## 4. Gaps and limits

- **G1560** — OpenBao has no cross-region replication in this design; the reference procedure ships Raft
  snapshots, so credential versions written after the last snapshot fail closed in the new region until
  restored or resubmitted.
- **G1561** — In `async` mode under a partition, the former region keeps acknowledging while clients
  reach it and it cannot see the new epoch; only the operator's fencing ends it before the peer probe.
- **G1562** — External effects of lost work (async, or a former primary's locally committed waits) may
  have happened; receivers reconcile with the failover notice and the API. Not preventable.
- **G1563** — A run lost in a failover and started again by a client receives a new run id; activity
  idempotency keys derived from the run id then differ; business-key templates are recommended.
- **G1564** — Followers that do not send the epoch can miss reused sequences below the head.
- **G1565** — The commit-latency cost of `sync_quorum` is unmeasured until the benchmark runs.
- **G1566** — No reads from a standby region (feature 010 G242 unchanged).
- **G1567** — Regional brokers with mirrored offsets (MirrorMaker-style) are not supported.
- **G1568** — Object-store replication lag is the provider's; the verifier samples; delete-marker
  replication depends on provider configuration.
- **G1569** — The drill is one host with a simulated WAN; no real region pair has been measured.
- **G1570** — Clock skew beyond lease margins is not simulated (feature 010 G252 extended).
- **G1571** — No tenant across cells and no relocation between cells.
- **G1572** — Narrowed by D1593: an epoch-unaware binary using the runtime role cannot write in an
  initialized cell. A process holding the owner role, or code that sets the session settings itself, is
  not fenced by them.
- **G1573** — Without a `pg_monitor` connection, protection is `UNKNOWN` and nothing claims RPO 0.
- **G1574** — Webhook senders that do not retry on 503 lose events posted during a drain or to a passive
  region.
- **G1575** — Tenant-visible audit of an activation is written after the activation commits, by fan-out.
- **G1576** — DNS/GSLB propagation is outside the platform; the drill measures its own resolver only.
- **G1577** — After an unplanned failover the new primary is unprotected until a cross-region standby is
  attached; if the operator keeps a synchronous configuration naming the lost region, commits block.
- **G1578** — The safety of D1566 and D1590 depends on driver and server behaviour (statement timeout
  interrupting the wait, an abandoned backend not detecting its client with
  `client_connection_check_interval = 0`) to be proven by T032-023 before the design is called safe.
- **G1579** — A superuser who cancels or terminates a waiting platform backend leaves a window of at most
  one durability lease (default 3 s) in which outcome reads may show that local-only commit.
- **G1580** — The beacon adds one small quorum-waited commit per process per refresh in `sync_quorum` cells.
- **G1581** — A run the former primary advanced after the cut and the new authority never advances again
  gets no automatic tombstone; only the divergence report (former primary reachable) produces one.
- **G1582** — Spans exported by the former region for lost work cannot be retracted from a trace backend.
- **G1583** — Key fingerprints compare keys, not their custody: a key present in both regions but leaked
  in one is not detected.
- **G1584** — Under `async`, lost CDC changes need 021's slot re-creation or backfill; the platform cannot
  recover them from the customer database's slot after the slot moved.

## 5. Owner questions

- **O1560** — Accept the direction of ADR-0067? Recommended default: yes, the direction now; `Accepted`,
  ADR-0009's line and the amendment of D1583 after the drill and benchmark reports (D1592).
- **O1561** — Default mode and publication of cross-region objectives? Recommended default:
  `sync_quorum` as reference mode; RPO/RTO across regions remain benchmark criteria, not published and
  not a release gate, until measured on a real two-region installation.
- **O1562** — Placement granularity? Recommended default: tenant home cell (D1576).
