# Correctness review of design 048 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: b6eb72653abe886dba7aca84f4de4dee76b13622
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
(the design, Architecture decisions 0130 and 0131, the code, and the pinned sqlx 0.9.0 source) and ran nothing;
"plausible" findings name the command that would settle them. The reviewer's own count line said 14 Majors; it listed
16, all kept here and counted.

## Blockers

1. The lease extension never fires in the usual case: the keeper's first round on the new primary rewrites
   `last_round_at` before the lost-lease pass measures the gap (about 0 s); `pg_control_checkpoint().timeline_id`
   changes only after the first checkpoint after promotion; a restart in place or an outage of the address changes no
   timeline. Fix: trigger on the gap, with a compare-and-set on the continuity row in the keeper's transaction; read the
   timeline from the current WAL file if it is wanted.
2. A commit whose synchronous wait is cancelled (by the server's 5 s timeout) becomes visible to status reads,
   `waitForCompletion`, the outbox and webhook dispatchers, and an isolated engine; after the failover the client has
   seen a completion that is lost and an external system received an effect the journal no longer holds. Fix: never let
   the server cancel a commit's wait (disable the timeout for the commit, folded into the last statement), bound it on
   the client and report the outcome as unknown; a reader test during "both standbys stopped".

## Majors

1. The driver cannot surface the warning: sqlx-postgres 0.9.0 turns every notice into a tracing event with the message
   only, filtered by the log level; the server, not the client bound, cancels first; a separate `SET LOCAL` adds a
   statement to every transaction.
2. The extension covers only `lease_expires_at`: attempt deadlines that fell in the outage time out on recovery
   (`crates/infra-postgres/src/maintenance.rs`), and a lease expiring just after recovery is not extended and its
   worker's retry can land after it. Fix: shift leases and attempt deadlines with expiry at or after the last round,
   with a floor of the maximum backoff plus the heartbeat interval.
3. Reports, claims and other engines act before the extension: a report before it gets `StaleAttempt` and gives up;
   claims count only unexpired leases and over-admit; two engines can both extend; one statement over every lease holds
   their row locks. Fix: paths that read expiry answer unavailable while an extension is pending; a compare-and-set;
   batches.
4. Extending shard leases delays the takeover of a dead engine's shards by the gap; the global "first transition" hides
   it. Fix: do not extend shard leases; measure per shard.
5. The 120 s report window is sized to the 60 s median, not the 300 s gate.
6. The client bound "server timeout + 2 s" breaks statements with a longer local timeout (`batch.rs`) and the migration
   pool (no timeout), and dropping a sqlx future mid-statement is the cancellation defect `pool.rs` documents. Fix: follow
   each transaction's effective timeout, none at 0; a task that owns the connection and closes it; TCP keepalive.
7. A phase line first in the file disables sqlx's `-- no-transaction`; a concurrent index build stopped by the lock
   timeout leaves an invalid index that `IF NOT EXISTS` skips.
8. Task 048-12 would change the checksums of applied migrations (`VersionMismatch`) and break every upgrade.
9. The handshake reads `_sqlx_migrations`, which the runtime role must not read (`runtime_role.rs`). Fix: a definer
   function; run the handshake tests as the runtime role.
10. Contract migrations and the capability switch are not fenced against live N-1 processes, and a binary that embeds
    its contract migrations would close its gate after an expand-only migrate. Fix: the registry's `activate`, raise the
    oldest compatible release in that transaction, `migrate` refuses a contract while an older release is live.
11. The lint cannot enforce "expand keeps N-1 working": 208 function, trigger, policy and view definitions in 41
    migrations; a new `NOT NULL` column without a default; `SET NOT NULL`; a new check or foreign key, which `NOT VALID`
    still applies to new rows.
12. The mixed-writer test applies the expand before the load, so it never shows a migration waiting behind the hot path,
    the switch, the contract or a rollback.
13. The two-primaries sampler cannot fail: not in recovery is not a commit, and an isolated node is unreachable. Fix:
    inspect the old primary's WAL past the fork after it rejoins.
14. Promotion safety is assumed, not configured: CloudNativePG needs its failover-quorum option and the primary
    isolation check; Patroni needs quorum mode with the strict setting; a drill fault with a lagging standby.
15. The staleness bound depends on clocks and can return an incomplete journal of a terminal run. Fix: compare the
    standby's replay position with the primary's current position read at authorization.
16. "Unknown" is collapsed into "unavailable": no outcome-unknown error; a claim whose commit is unknown leaves a lease
    nobody holds, which expires, burns an attempt and can become a review item.

## Nits

1. Class 57 is already unavailable; only `25006` is new; `57P04` should not be retryable.
2. The SDKs make 4 attempts in all (at most 1.4 s); they must treat a refused connection and an ingress 502 or 503 as
   unavailable.
3. One beacon row per process serializes replays; a transaction id alone may make the commit wait (contested by the
   architecture reviewer, who reads that a transaction that wrote no WAL does not wait).
4. Release bookkeeping needs a manifest made at release time (ordering of releases, "the last migration of a release",
   the version-to-release map; the 0.3.0 gate against 0.2.0 is skipped).
5. HAProxy needs `on-marked-down shutdown-sessions`; the chart example needs a disruption budget for the drain; the
   rollback runbook must forbid running N-1's migration job.

Counts: Blocker 2, Major 16, Nit 5
