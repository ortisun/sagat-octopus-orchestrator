# ADR-0067: A cell has one writable region at a time, fails over active-passive by an operator's activation, and fences everything granted before it with a region epoch

**Status:** Proposed. The owner may accept the direction so that implementation can start; the status `Accepted`, ADR-0009's "Superseded in part by ADR-0067" line and the constitution amendment below wait for the region drill report and the commit-latency benchmark, as ADR-0009's review criteria and the constitution's Governance require (feature 032, D1592, T032-063).
**Decided by:** agent proposal, pending owner acceptance (2026-10-26); revised after independent architecture review (2026-10-27)
**Refines / supersedes:** Would supersede [ADR-0009](ADR-0009-single-region-first.md) in part once accepted (the single-region constraint, for a cell that declares standby regions; in-region HA and the deferral of active-active stand). Refines [ADR-0020](ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md) (the fence also checks the region epoch). Builds on ADR-0002, ADR-0004, ADR-0012, ADR-0017, ADR-0025.
**Enforced by:** pending — the tests that must exist before acceptance of the implementation are named in [specs/032-multi-region-failover/tasks.md](../../specs/032-multi-region-failover/tasks.md): T032-020 (activation is one transaction, crash at every statement), T032-021 (epoch fence with a negative control), T032-023 and T032-058 (region loss mid-commit, cancelled synchronous wait, restart of the former primary, and acknowledgments through replays, duplicates and reads), T032-059 (an epoch-unaware binary cannot take a lease or write), T032-062 (in-region failover inside a two-region cell), T032-060 (tombstones downstream), T032-009 (no unlogged table or sequence), T032-045 (region drill fails on acknowledged loss in `sync_quorum`).

## Context

ADR-0009 chose a single region with high availability first, "avoids complex consensus/global-order
prematurely", and asked for a new ADR with benchmark data, failure tests, operational cost and
compatibility with existing runs before changing that. Feature 010 built in-region HA (shard leases with a
fence, a synchronous standby, PITR, a drill gating releases against RPO 0 / failover RTO ≤ 5 min / PITR
RTO ≤ 1 h) and recorded what it does not cover: a standby in another zone or region, the object store,
the broker, the key ring (G250, G251). The constitution's Delivery Constraints name "single region with
high availability" in the technology baseline and put "multi-region active-active" out of scope for V1.

The forces:

1. **The authority cannot be split.** Principle I: a transition, the next tasks, timers and the outbox
   commit in one transaction; a transition applied twice or lost after commit cannot be repaired. Run
   state, quotas, receipts and leases are mutable rows, not a mergeable event log. Temporal reconciles a
   split brain by merging histories with failover versions; this platform has nothing to merge with.
2. **During a partition no single database can decide.** ADR-0020 rejected an external leader because
   "the action's transaction cannot ask it"; across regions, the database on either side does not know
   what the other did. Some arbiter outside one database (an operator, an HA manager with a quorum) has
   to decide which side is the authority.
3. **PostgreSQL acknowledges commits locally when a synchronous wait is cancelled.** `syncrep.c` emits a
   WARNING and returns success; a former primary that restarts marks waiting transactions committed. A
   cross-region synchronous quorum fences the former primary only if the platform never treats such a
   commit as acknowledged, and those commits are then visible locally: a keyed replay, an inbox
   duplicate, a worker `already_applied` or a status read would acknowledge them through a read path.
   An in-region standby left out of a remote-only quorum becomes asynchronous, which silently weakens
   feature 010's in-region RPO 0.
4. **Effects are at least once.** Work redone after a failover repeats external effects unless keys or
   lookups absorb them (ADR-0004). Duplicates are expected; they must be absorbed, not prevented.
5. **Nothing is measured yet.** There is no benchmark of cross-region commit latency and no drill on a
   real region pair; a decision now is a design decision whose release depends on evidence.
6. **Old binaries do not know about epochs.** A process of an earlier version in a standby region would
   act as soon as the database is promoted; an operator's assurance cannot stop it, the database can.
7. **Derived records are keyed by journal sequence.** After a lossy failover sequences are reused by
   other events; sinks, span ids and stream cursors keyed without an epoch keep the lost timeline.

## Decision

1. **A cell is the unit of replication and failover.** A cell is one PostgreSQL authority with its object
   store and its processes, deployed in two or three regions. Exactly one region of a cell MUST be
   writable at any time. A deployment MAY have several cells; cells share no data. All namespaces of a
   tenant MUST be in one cell.
2. **Replication is physical.** Standby regions MUST hold a physical streaming replica of the whole
   cluster and a provider-replicated bucket. Logical replication of a subset MUST NOT be used for
   failover.
3. **The platform MUST NOT elect, forward or merge.** Promotion is decided by the operator or the
   operator's HA manager. Standby regions MUST refuse work with a typed reason and MUST NOT forward,
   proxy or redirect requests. Divergent writes of a former primary MUST NOT be merged into the
   authority; only inbound events MAY be re-admitted through the normal inbox admission.
4. **The region epoch is the authority's record of who may act.** The cell's database MUST hold one
   authority record (active region, epoch, state, declared mode) and an append-only history. The epoch
   MUST increase by exactly one, by compare-and-set, only through an operator's activation on a database
   that is not in recovery. A process MUST act only when the authority of the database it is connected
   to names its region active; it MUST connect only to its own region's database and bucket. The
   database MUST enforce this for epoch-unaware binaries: once a cell is initialized the runtime role is
   read-only by default and only processes that identify their region (`sagat.region`) and run in the
   active region override it, and triggers on the lease tables refuse a take or renewal whose region or
   epoch is not the authority's.
5. **Activation fences what came before, in one transaction.** Activation MUST, in the transaction that
   advances the epoch, raise the generation and expire every shard lease, remove singleton leases and
   replica memberships of earlier epochs, and write the history row with the replication cut point. The
   ADR-0020 fence MUST also compare the epoch, in the row read it already performs. Worker sessions and
   leases need no new fence: the existing token, generation and expiry checks refuse what the new
   authority does not hold.
6. **No acknowledgment without durability in the declared mode, on any path.** In `sync_quorum` mode the
   synchronous quorum MUST NOT be satisfiable without a standby outside the active region (k greater
   than the number of in-region members), and feature 010's in-region RPO MUST be kept either by a
   quorum that always includes the in-region standby or by an in-region promotion that first verifies
   the highest synchronous flush LSN. No operation MUST be acknowledged on the strength of a COMMIT that
   returned after a cancelled or terminated replication wait; a commit that misses its deadline is an
   unknown outcome. Answers that acknowledge earlier work without writing (replays from receipts,
   inbox and connector duplicates, worker `already_applied`) MUST follow a quorum-waited commit of their
   own transaction; outcome reads MUST be served only under a durability lease renewed by a quorum-waited
   beacon commit; nothing MUST be served after a start, reconnect or database restart until a beacon
   commit completes. In `async` mode the platform MUST state that acknowledged work can be lost and MUST
   measure it.
7. **An unplanned activation needs a written attestation** that the former primary is fenced and its
   other standbys detached; a planned one needs evidence that the promoted database replayed the former
   primary's final LSN. A process that sees a higher epoch of its cell at a peer MUST stop and MUST NOT
   activate itself.
8. **RPO and RTO across regions are benchmark criteria** measured by a region drill on a stated
   environment, until the owner decides to publish objectives. The single-region objectives of feature
   010 (owner decision 8) are unchanged where decision 6's in-region rule holds; where it does not, the
   in-region RPO is reported as not guaranteed.
9. **Every record derived from the journal carries the epoch of the event.** Journal events MUST record
   the epoch they were committed under; span ids, follower cursors, stream cursors and analytic keys MUST
   include it (unchanged at epoch 0); after an unplanned activation projectors and sinks MUST continue from
   their replicated cursors and receive tombstones for the lost ranges.

## Considered options

1. **Active-passive per cell, operator activation, epoch fence (chosen).** Keeps one authority per cell,
   reuses the lease/fence model, the inbox/outbox and idempotency keys for duplicates, and adds no
   consensus. Costs a governance amendment, cross-region commit latency in `sync_quorum`, and an
   operator in the loop for failover.
2. **Active-active with conflict resolution (Temporal multi-cluster style).** Lower RTO and writes in
   both regions. Lost: it needs mergeable histories, per-namespace failover versions and conflict rules
   for every mutable row (quotas, receipts, leases, approvals), and it accepts a split brain by design;
   the constitution keeps it out of scope.
3. **Per-tenant logical replication with per-namespace failover.** Finer placement. Lost: DDL is not
   replicated (every migration becomes a two-phase operation), triggers and `SECURITY DEFINER`
   functions fire again on apply, sequences of operations across tenants are not atomic, and the
   conflict handling would be the platform's.
4. **A witness or consensus service run by the platform to elect the active region automatically.**
   Lower RTO without an operator. Lost: it is the second authority ADR-0020 rejected, its decision
   cannot be checked inside the action's transaction, and it duplicates what PostgreSQL HA managers
   already do; the operator may still automate promotion with such a manager and call the activation.
5. **Status quo: restore from backups into another region.** No new mechanism. Lost: RTO of a restore
   from remote storage, RPO of the archive, object store and broker not covered (G250, G251).

## Consequences

- **Good.** Regional loss becomes a failover instead of a restore; acknowledged work is not lost in
  `sync_quorum` with protection; every grant of the former epoch is fenced; duplicates are absorbed by
  the existing contracts; each change has a history row and tenant-visible audit.
- **Bad.** `sync_quorum` adds a cross-region round trip to every commit (unmeasured, G1565), a small
  beacon commit per process per refresh and a commit per replay, and it halts writes when no remote
  standby acknowledges, unless the operator degrades to `async` explicitly. Journal rows grow by 8 bytes
  and a function call. In
  `async` a partition lets the former region acknowledge work the authority will not have (G1561), and
  external effects of lost work may have happened (G1562). An operator is needed for every failover.
- **Harder.** Every process needs region configuration and region-local endpoints; every future feature
  that stores a cursor followed by clients must carry the epoch; schema changes must not introduce
  unlogged tables or sequences.
- **Runs in flight.** Pinned versions and journals replicate; runs continue from the cut in the new
  region; the journal of a run may lose its tail in `async` mode, and followers reset.
- **Rollback.** The migration is additive; with no authority row nothing changes. A cell that was
  initialized cannot silently return to single-region mode (the history is kept; the down migration
  refuses).
- **Operational cost.** A second (or third) region of database, bucket, broker and processes at standby
  capacity, replication monitoring with a `pg_monitor` role, quarterly region drills.

## Constitution amendment required (stated, not applied)

Delivery Constraints, technology baseline: replace "single region with high availability" with "one
active region per cell with high availability; an optional cross-region standby region with
active-passive failover as defined by ADR-0067". Keep "Out of scope for V1: multi-region active-active,
…" unchanged. Version 1.0.0 → 1.1.0 (MINOR). Sync Impact Report: ADR-0009 status line ("Superseded in
part by ADR-0067"), `docs/architecture/overview.md`, `docs/architecture/capacity-and-topology.md`,
`docs/runbooks/disaster-recovery.md`, and an annotation of the "V1 single-region HA" assumption lines of
specs 001–012. This amendment and this ADR are owner decisions (feature 032, O1560). The amendment pull
request is opened only after the drill report and the commit-latency benchmark exist; until then this
text stays here and the constitution is unchanged.

## Evidence

None yet; this is a proposal. Design and sources: [spec](../../specs/032-multi-region-failover/spec.md),
[research](../../specs/032-multi-region-failover/research.md) (D1560–D1599, G1560–G1584, PostgreSQL
synchronous replication documentation and source, Temporal and Windmill failover semantics),
[plan](../../specs/032-multi-region-failover/plan.md). Acceptance of the implementation requires the
failure tests listed under **Enforced by**, the region drill report
(`tests/performance/reports/multi-region-failover.md`) with its environment and failures, and the
commit-latency benchmark of both modes.
