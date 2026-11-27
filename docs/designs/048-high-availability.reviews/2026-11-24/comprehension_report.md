# Comprehension review of design 048 (2026-11-24, stage 6 of egm-review)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: b6eb72653abe886dba7aca84f4de4dee76b13622
Reviewer: egm-comprehension
Note: the reviewer's output, kept whole; the reviewer read files only and ran nothing.

## What this proposes

The design makes PostgreSQL high availability within one region part of the product, in five parts:

- **Reference topologies.** There are two, both tested: CloudNativePG on Kubernetes and Patroni with etcd and HAProxy on Compose. Each runs one primary and two standbys across three zones, with quorum-one synchronous replication, and refuses to commit when no standby is reachable. The HA manager moves the platform's single address to the new primary.
- **Platform changes.** Connections get client-side time limits. The platform refuses connections to read-only nodes and treats them as unavailable. It never acknowledges a commit whose synchronous wait was cancelled, and replayed answers first commit a write that waits for a standby. Leases are extended by the outage gap. SDK report retries last 120 s.
- **Standby reads.** Standbys serve only a closed list of reads that can tolerate staleness, through an optional second address, with a 5 s staleness limit.
- **Rolling upgrades.** Migrations are marked expand or contract and checked by a lint. A compatibility table and a startup/heartbeat handshake close a binary's serving gate on an incompatible schema. New write behaviour waits for a fleet census. A test runs N-1 and N writers together.
- **Evidence.** Measured drills on Kubernetes and Compose gate each release: no acknowledged work lost, no two primaries, and at most 300 s to the first transition.

## How the existing system works, per this document

Every environment runs a single PostgreSQL. Failover is a manual runbook: fence the old primary, promote a standby, repoint the platform. The current drill never repoints the platform and assumes a constant 15 s shard takeover. The platform has one `DATABASE_URL` and treats SQLSTATE `25006` as an internal error. It has no client-side query limit and no maximum connection lifetime. It acknowledges commits whose standby wait was cancelled. Its SDKs give up after about 5 s of retries. Worker leases are 30 s and expire during an outage, and since feature 037 an expired activity can need a person. Upgrades stop every writer, although older runbooks contradict this. No binary checks the schema version it runs on, and migrations take locks with no limit on how long they wait.

## Findings

1. Major (lines 268-271, 500 and 237). The owner gate on Task 048-16 is cited inconsistently: the cost paragraph names Owner decision 2026-11-24-4, the task line names OD-2026-11-24-1; no Open Question cites 2026-11-24-4.
2. Major (Requirements 048-030, 048-014, 048-015, 048-042; the Risks row). "The failover budget" has two numbers (60 s median reported, 300 s gate); 048-014 sizes the SDK window against 60 s, 048-015 promises no repeat "within the failover budget" without saying which, so a 200 s failover passes the gate while reports stop retrying at 120 s.
3. Major (CUJ 2, Requirements 048-010 and 048-012, Open Question 4). What cancels the synchronous wait and what the client sees is unclear: a closed connection cannot deliver the warning; "unavailable" against "unknown" is not defined.
4. Major (Tasks 048-6 and 048-11 against Open Question 4). Task 048-6 builds the warning mechanism before Task 048-11 chooses the mechanism.
5. Major (Requirement 048-012, "The replay rule in practice"). Why a newer waited commit proves an older one is on a standby (WAL is received in order) is not stated.
6. Major (Requirement 048-013, "The lease extension in practice"). Scope (only promotions, not restarts or partitions), who writes the row (singular, three engines), the clock reasoning (the gap carries the offset), and whether the keeper's update is an extra statement.
7. Major (throughout). Core project terms undefined or unlinked: keeper round, lost-lease pass, shard lease, serving gate, instance registry, census, capability, integrity oracle, chain shape, review item, `reconcileWith`, transition, token, generation, fence.
8. Major (the upgrade order). Contract migrations' application, which migration writes the compatibility row and what it records, "treated as of their release", and how the lint links a contract to its expand.
9. Nit (line 150). Requirement 048-030 sits under "Rolling upgrades".
10. Nit (line 401). How polling `pg_is_in_recovery()` observes a commit on two timelines.
11. Nit. When the writability check runs.
12. Nit. How a response says it may be stale.
13. Nit. Why zone loss and isolation are excluded from the 60 s median.
14. Nit. Secondary unlinked references (features, gaps, design 038's budget, the checkers, `begin_safely`).
15. Nit. "the standbys" implies both; the diagram labels one arrow "sync".
16. Nit. No task references Requirement 048-030 or Non-functional requirements 048-002 and 048-003; Requirement 048-042 is referenced only by Task 048-17; Task 048-1 says "write" ADRs that exist.

Counts: Blocker 0, Major 8, Nit 8
