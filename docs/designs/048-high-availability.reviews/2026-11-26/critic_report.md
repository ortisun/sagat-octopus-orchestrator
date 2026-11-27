# egm-design (critic) on design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and took PostgreSQL, CloudNativePG, Patroni and kernel behaviour from its knowledge.

## Major

1. The identity read fails on any server in recovery: reading an unlogged table during recovery raises 0A000, so a standby
   is refused with the wrong error and feature 032's passive process can never connect. Fix: a function that returns a
   null token in recovery; classify that refusal as unavailable.
2. A connection opened after the change to a former primary still reachable shows the old identity and is accepted;
   "newest observed" has no order and can flip back. Fix: tag each connection with its identity, drop those that differ
   from the confirmed one, refuse identities seen superseded.
3. `execute_safely`'s rule turns a function's own refusal (54000, P0001, 55P03, 57014) into an unknown outcome retried for
   310 s. Fix: an ERROR answer to `COMMIT` or to an autocommit call is rolled back (after the commit record PostgreSQL
   panics; a cancelled wait is a WARNING); unknown only for no answer, FATAL or I/O.
4. Reads from a superseded primary are bounded only by pool turnover (conntrack flows outlive the Service move;
   CloudNativePG's smart shutdown keeps sessions for 180 s), authorization and secret reads included. Fix: on an unknown
   outcome set `DATABASE_UNCONFIRMED` and drain the pools; bound the shutdown; measure the window.
5. The design diverges from feature 032, which builds on it (FR-032-009(b)/(d) and D1590(a): replays that wait through
   `pg_current_xact_id()`, a barrier on reconnect or unknown outcome). Fix: Task 048-1 lists the amendments to 032, or
   048 adopts the barrier.
6. CUJ 2 understates "both standbys gone": registry beats wait too, so readiness fails fleet-wide after 30 s and reads
   stop; on recovery every waiting commit lands at once. Fix: state the blast radius, classify a beat's unknown outcome,
   assert it in the drill.
7. The lease sizing contradicts user story 048-01: detection alone (Patroni's 30 s, Kubernetes' node grace of about 40 s)
   exceeds what a 30 s lease survives; the report window is useless after the lease is reaped; the lost-lease pass races
   the retried reports. Fix: pin detection settings, set the lease in the HA examples, hold the lost-lease pass after a
   confirmation.
8. The production reference ships unverified (the Kubernetes drill is unscheduled). Fix: mark it built, not offered for
   production, until the drill passes; name the operator release and the feature's maturity.

## Nit

1. Class 40 includes 40003; list the codes as ADR-0078 does.
2. `sagat_server_identity()` is executable by `sagat_app` only, yet owner-run commands confirm through it.
3. Concurrent confirmations queue behind the first token insert; insert the token in its own transaction first.
4. Read-only status and `synchronous_standby_names` are checked only at connect; recheck on the registry beat.
5. The architecture check scans Rust only; migrations could set `synchronous_commit`.
6. A diverged standby can pass "highest sequence ≥ S"; compare the event at S or the timeline.
7. With `restart_after_crash` off, the backend-killed fault never tests the marker; assert the start time stayed.
8. The former primary's copy must be taken before the HA manager rewinds it; fence it first.
9. Lease expiry moves to another host's clock at promotion; require clock sync and report skew.
10. The preferred mode can lose acknowledged work with nothing counting it.
11. The alternatives omit a barrier on the standby's flush position and a timeline-based identity.
12. `commit_safely` is defined for `PoolConnection` only; the confirmation's connection, `migrate`'s and `RELEASE
    SAVEPOINT` need a rule.

Counts: Blocker 0, Major 8, Nit 12
