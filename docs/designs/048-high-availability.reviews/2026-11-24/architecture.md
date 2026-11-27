# Architecture review of design 048 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: b6eb72653abe886dba7aca84f4de4dee76b13622
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing; it read the design and Architecture decisions 0130 and 0131, and compared them with feature 032
and Architecture decision 0067 in the working tree.

Verdict of the reviewer: close to ready, not yet; the layering is clean.

## Blockers

1. The promise "nothing acknowledged without the standby" leaves out outcome reads: a locally committed transaction
   after a cancelled wait is visible to a status read, which Architecture decision 0067 decision 6 and requirement
   032-009 (c) and (d) treat as an acknowledgment, in one region too. Fix: take the durability lease and the
   reconnect barrier into 048 in-region, or narrow the claim and register the gap.
2. The default commit mechanism contradicts 032's decided one (D1566, requirement 032-009 (e)): 048 keeps the 5 s
   server timeout during `COMMIT`, so the server cancels the wait and the commit becomes visible, then relies on reading
   a warning that the driver may not expose and on `lc_messages = 'C'`, which is superuser-only. 032 decided
   `SET LOCAL statement_timeout = 0` before `COMMIT`, a client deadline reporting `COMMIT_OUTCOME_UNKNOWN`,
   `client_connection_check_interval = 0`, and no cancelling of waiting backends. Fix: adopt D1566 as 048's rule; the
   warning check only as an extra defence.
3. Task 048-12 "mark every migration's phase" would change the checksum sqlx stores for applied migrations and break
   every deployed database. Fix: never edit applied migrations; the lint skips versions up to a recorded cut-off.

## Majors

1. The lease extension's steps are in the wrong order and race: the keeper's first round can overwrite
   `last_round_at` before the pass reads the gap; takeovers are not held back; two engines can both extend;
   `pg_control_checkpoint()` can report the old timeline until the first checkpoint; restarts in place are not
   covered. Fix: check and extend inside the keeper's round transaction with a compare-and-set on the timeline, read
   the timeline from the current WAL position, refuse reclaim while it differs, also trigger on
   `pg_postmaster_start_time()`.
2. The extension has no upper bound and undoes 032's fence (a cross-region activation and a point-in-time restore
   change the timeline too); it contradicts Architecture decision 0020 decision 2. Fix: cap at 300 s, skip when the
   region epoch changed, amend Architecture decision 0020.
3. The failover-budget arithmetic is understated (a dead engine's shards wait for the rest of their 15 s lease; probe
   periods and idle pooled connections not counted) and "the budget" means two things. Fix: recompute, one figure, size
   the report window against the 300 s gate or state the consequence.
4. Refusing connections in recovery conflicts with 032's passive processes and with the stale-read pool; readiness
   following writability contradicts Architecture decision 0082 decision 5. Fix: exempt those; keep readiness and answer
   a typed 503, or record the divergence.
5. The beacon table duplicates 032's `region_beacons`, and one row per process serializes replays after a failover; a
   transaction id without WAL does not wait for the standby, which 032 does not record. Fix: one table in 048, a row
   per pool connection, correct 032.
6. No contract changes are named (the unknown-commit reason, `SCHEMA_TOO_OLD`, `BINARY_TOO_OLD`, the staleness marker,
   the SDK retry window).
7. Telemetry is claimed but not specified; no alert before writes stop when standbys are lost.
8. The upgrade order contradicts itself (contracts never run with `--phase expand`) and the tool (`-- no-transaction`
   must be the first line).
9. Gaps in the expand rule and the lint: functions, triggers, policies and grants; `SET NOT NULL`; an invalid index
   after a retried concurrent build; linking a contract to its expand; no retry after `lock_timeout`.
10. Several promises have no failing check: the closed list of standby reads, waited replays on new paths, Tasks 048-4
    and 048-5 not in Enforced by, image digests, capability gating.
11. The two-primaries sampler cannot see an isolated old primary, and "not in recovery" does not show commits.
12. The owner-decision citations do not resolve on `develop` (2026-11-24-1 and -4 are on unmerged branches) and Task
    048-16's gate cites the scope decision instead of the owner's spend.
13. Designs 048 and 051 depend on each other and no build order is recorded.

## Nits

1. SDK retries are 4 attempts in all (3 retries, 0.7 to 1.4 s).
2. CUJ 2: a closed connection cannot read the warning.
3. The client bound when the server timeout is zero (the migration pool).
4. Stale reads: "operational dashboards" is not a closed list; the lag formula misreads an idle primary; in a 032 cell
   the read address must be in-region; reads authorized on the primary gain no availability in a failover; the bound
   should be an application type.
5. Zone spread with `DoNotSchedule` needs `nodeTaintsPolicy: Honor`.
6. Version pins: Patroni 4 quorum mode, the CloudNativePG version with the required durability; cite which standby is
   promoted.
7. "In one statement" is two tables and an audit row; the continuity row's contention.
8. The mixed-writer gate is vacuous for 0.3.0 and first runs for the release after.
9. Retrying a heartbeat for 120 s is pointless; retry the latest only.
10. Architecture decisions 0020 and 0082 do not name the new ones; the new tables need classification and privileges.

Counts: Blocker 3, Major 13, Nit 10
