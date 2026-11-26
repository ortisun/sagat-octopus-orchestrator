# Security review of design 048, round 2 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: 70c9f3f7464087d31e7cc0dc62966570be38f6f1
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing.

## Majors

1. `Sagat-Read-Staleness-Lsn` exposes the platform-wide WAL position, and so other tenants' write volume, to every
   tenant, and is useless to the client. Fix: no value (at most a source marker); a 404 or 403 never comes from the
   standby; the tenant-scoped routing check on the primary; a test that another tenant's run gives the same 404.
2. The lease extension's audit has no valid home: `audit_records.tenant_id` is required. Fix: a platform-level,
   append-only table like `platform_requirement_history`, insert-only for the runtime role, with counts and no tenant or
   lease identifiers; optional per-tenant records with their own counts only.
3. On Compose, etcd and Patroni's REST API are reachable from every container (the sandbox runner included) without
   authentication; a compromised component could end synchronous mode or force a switchover. Fix: an internal network
   for the database's control, the platform reaching only HAProxy's 5432; Patroni's REST authentication and etcd
   authentication from `.env`; no host port for etcd.
4. The CloudNativePG credentials are unspecified, and its `<cluster>-app` Secret is the database owner's. Fix: create
   `sagat_app` through managed roles with a password Secret; the owner's Secret mounted only by the migrate job; the HA
   example's URL uses `sagat_app` with full TLS verification; a check of the example's user.
5. The CloudNativePG operator is cluster-wide by default and can read every Secret. Fix: scope it to one namespace and
   keep the database in a namespace apart from the platform's Secrets.

## Nits

6. Write down the grants: `database_continuity` select and update; `durability_beacons` insert and update only;
   `schema_compatibility` no write for the runtime role; the handshake function as a definer with a pinned search path,
   revoked from public, executable by the runtime role only; extend the runtime-privilege audit.
7. The migration lint should require a pinned search path and a revoke from public on definer functions, and forbid
   grants in a migration without a transaction.
8. A security fix to a function, trigger or policy replaces it in place under a named waiver; compatibility gives way.
9. The waiting-backends metric needs no extra grant (say so); an architecture check refuses `pg_cancel_backend` and
   `pg_terminate_backend` outside tests.
10. The read pool: read-only transactions, the timeline in the position check, a bounded statement timeout and standby
    delay, the read URL in the chart's network policy.
11. An ingress network policy for the database pods; runner pods kept out.
12. The mixed-writer test pins N-1 by commit, not by tag.
13. The gate's 503 must not show release or migration versions to unauthenticated callers.

Counts: Blocker 0, Major 5, Nit 8
