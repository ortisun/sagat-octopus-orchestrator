# Security review of design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Reviewer: security (full round; the approval's security report is the re-read that follows the fixes)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing.

## Major

1. The confirmation connection and the one-run commands are not required to send `synchronous_commit=on` or pass the
   `after_connect` checks; a role setting of `local` lets the beacon commit without a standby, opening the gate on
   commits no standby holds. Fix: the same startup option and checks; a test with `ALTER ROLE ... SET
   synchronous_commit=local`.
2. `synchronous_standby_names` is checked only at connect, and the binary's default of
   `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` is unstated; a reload that empties it makes pooled connections acknowledge local
   commits for up to 30 minutes. Fix: default true (Compose false); re-read it every keeper round with a gate reason.
3. Pooled connections to an isolated former primary keep serving authorization reads after a revocation committed on the
   new primary. Fix: on any unknown outcome or unavailable `25006`, set `DATABASE_UNCONFIRMED` and stamp
   `identity_changed_at`; state the remaining bound; a drill check that a revocation is honoured.
4. (Suspicion.) Nothing verifies that the read address reaches only standbys of the current primary; "sequence ≥ S" does
   not prove the same history. Fix: compare the timeline and the position, or the event at S.
5. On Compose every service can reach the database port and try the replication or superuser role. Fix: `pg_hba` limits
   replication to the nodes and the superuser to the local socket; only `sagat_app` and the owner through HAProxy with
   `scram-sha-256`; every Patroni role's password from `.env`; no host port for the nodes or Patroni's REST interface.
6. The database's ingress policy names no pod labels, so it admits worker pools and `database-runner`; the runner's
   `denyCidrs` need not cover the database namespace. Fix: select by component label; require `denyCidrs` to cover the
   database's pods and Services; a policy test.
7. With both standbys gone, waiting commits take every slot, so authorization reads stop too, and unauthenticated paths
   that write fill the pool sooner. Fix: once a commit passes its deadline, refuse new write transactions and keep part
   of the pool for reads; refuse writes from unauthenticated requests meanwhile.
8. The backups to object storage say nothing about encryption, credentials or retention. Fix: server-side encryption with
   a managed key, workload identity, a private bucket, retention matching tenant deletion.

## Nit

1. The standby statement must keep the tenant predicate; `StaleReadPool` methods take a `TenantId`.
2. `Sagat-Read-Source` tells readers when the standby is behind; consider the span only.
3. `DATABASE_UNCONFIRMED` would join the pre-authentication 503's reasons; set a precedence with the gate's `Retry-After`.
4. Readiness pings storage before checking the gate, opening connections while unconfirmed.
5. The read pool cannot read the unlogged table on a standby; exempt it.
6. `DATABASE_READ_URL` from a Secret key, `verify-full`, redacted in `Debug`.
7. Say how CloudNativePG's managed role Secret reaches the platform's namespace and rotates; the chart has no migrate job.
8. etcd needs separate root and Patroni users, TLS or a stated plaintext internal network; Patroni REST's allowlist.
9. `sagat_server_identity()`: `ON CONFLICT DO NOTHING` and a select; qualify the table; `REVOKE ALL ... FROM PUBLIC`.
10. The drill's copy of the former primary holds every tenant's data; no host port; delete it after.
11. A killed pod leaves its waiting backends holding slots; say so in the budget and the runbook.

Counts: Blocker 0, Major 8, Nit 11
