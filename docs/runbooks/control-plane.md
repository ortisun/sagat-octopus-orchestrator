# Runbook: control plane (feature 001)

Operational procedures for the `control-plane` process as it exists after feature 001: the REST API,
PostgreSQL as the only state, and an external OIDC provider. Everything here was run against the
code in this repository; what is **not** covered is listed at the end so nobody assumes it.

## 1. What runs

| Piece | Where | Notes |
|---|---|---|
| API | `control-plane serve`, default `127.0.0.1:8080` | Stateless; all state is in PostgreSQL |
| Admin listener | `SAGAT_ADMIN_ADDR`, default `127.0.0.1:9090` | `/metrics` only. Keep it off public networks; `off` disables it |
| PostgreSQL 16 | `DATABASE_URL` | Source of truth. TLS is required outside `SAGAT_ENV=dev` |
| OIDC provider | `SAGAT_OIDC_JWKS_URI` (HTTPS) | Only its public keys are fetched; no token is ever sent to it |

Two database roles exist on purpose. The **owner** role runs `migrate` and `provision-tenant`. The
API serves requests as **`sagat_app`**, which owns nothing, cannot alter or delete audit rows and
cannot change the schema. Never point the API at the owner role.

## 2. Start, stop, upgrade

```sh
control-plane migrate            # owner role; idempotent, safe to run twice
control-plane serve              # runtime role
```

- `SAGAT_MIGRATE_ON_START=true` runs the migrations before serving; use it only with the owner role
  and a single instance.
- **Upgrade:** run `migrate` first, then roll the new binary. Migrations are additive in this
  feature, so the previous binary keeps working against the new schema.
- **Stop:** `SIGTERM` stops accepting connections and drains for `SAGAT_SHUTDOWN_GRACE_SECS`
  (default 30). Set the orchestrator's termination grace period higher than that.
- **Rollback of a migration:** each migration has a `.down.sql`. Take a backup first (section 6);
  down migrations drop data.

## 3. Create the first tenant

`seed-dev` works only when `SAGAT_ENV=dev`. In any other environment:

```sh
SAGAT_PROVISION_TENANT=acme SAGAT_PROVISION_ADMIN=alice@example.com control-plane provision-tenant
```

- Both values are mandatory; there are no defaults.
- It exits non-zero if the slug already exists and changes nothing in that case.
- It writes `tenant.provisioned` and `membership.granted` audit records (actor
  `system:provisioning`) in the same transaction as the tenant.
- The administrator signs in with a token whose `sub` equals `SAGAT_PROVISION_ADMIN` and whose
  tenant claim (`SAGAT_OIDC_TENANT_CLAIM`, default `tenant`) equals the slug.
- The administrator's subject is not written to the process log.

## 4. Health and diagnosis

| Probe | Meaning | Use for |
|---|---|---|
| `GET /v1/health` | The process is up. Touches nothing else | Liveness; a database outage must not restart pods |
| `GET /v1/ready` | PostgreSQL answers **and** the identity tables exist; cached for 1 s | Readiness |

Every response carries `X-Trace-Id`, and every error body repeats it as `traceId`. Search the JSON
logs for that value to follow one request.

### Symptom table

| Symptom | Likely cause | What to do |
|---|---|---|
| `/v1/ready` is `503`, `/v1/health` is `200` | PostgreSQL unreachable, or schema not migrated | Check the database; run `migrate`. Do not restart the API for this |
| All requests `401` with `UNAUTHENTICATED` | Wrong `SAGAT_OIDC_ISSUER`/`AUDIENCE`, expired or too-old token (`iat` older than the maximum lifetime, 1 h) or the provider rotated keys | Compare the token's `iss`/`aud`/`kid`; check `jwks_refresh_total{outcome}` |
| `401` only for tokens with a new `kid` | JWKS not refreshed yet | Refresh is rate-limited and stale keys keep serving; wait one refresh interval, check `outcome="error"` |
| Authenticated user gets `404` for a tenant's resource | Resource belongs to another tenant, or does not exist (deliberately the same answer) | Look for `access.denied.cross_tenant` audit rows in the caller's tenant |
| `429 RATE_LIMITED` | The tenant exceeded `SAGAT_RATE_LIMIT_RPS`/`BURST` (`scope="tenant"`), or one token subject exceeded `SAGAT_SUBJECT_RATE_LIMIT_RPS`/`BURST` (`scope="subject"`, default 20 req/s, burst 40, charged before any database lookup) | The response carries `Retry-After` and `retryable: true`; clients should honour it. A burst of `scope="subject"` from an unknown or non-member subject is a probe or a misconfigured client, not a reason to raise limits. Check `http_rate_limited_total{scope}` before raising anything |
| `503` `UNAVAILABLE` on API routes, `/v1/ready` fine | Load shedding: more than `SAGAT_MAX_IN_FLIGHT` requests at once (the response carries `Retry-After`) | `http_load_shed_total`, `db_pool_connections{state="in_use"}`; scale out or raise the pool |
| `408` | The request exceeded `SAGAT_REQUEST_TIMEOUT_SECS` | Slow database or client; check `http_request_duration_seconds` |
| Process refuses to start | Missing/invalid setting (exit code 2, names the variable), or `DATABASE_URL` without TLS outside dev | Fix the named variable; the message never includes the URL's password |

## 5. Metrics

Scrape `http://<admin-addr>/metrics` (Prometheus text format). Labels are route **templates**,
status codes, fixed reason names and an HTTP method taken from a closed set (`GET`, `POST`, `PUT`,
`PATCH`, `DELETE`, `HEAD`, `OPTIONS`, and `OTHER` for any other token a client sends). Identifiers,
tenants, subjects and client-chosen text are never labels. As a backstop, a series that is not
updated for 10 minutes is dropped from the page, and the process releases such series every minute
even when nothing scrapes it (the recorder only expires series while rendering).

| Metric | Type | Useful for |
|---|---|---|
| `http_requests_total{method,route,status}` | counter | Error ratio, traffic per route |
| `http_request_duration_seconds{route}` | histogram | Latency percentiles |
| `auth_failures_total{reason}` | counter | Spikes of a single reason (e.g. unknown key) |
| `jwks_refresh_total{outcome}` | counter | `error` or `no_usable_keys` means the provider or network is failing |
| `audit_denials_total{outcome}` | counter | `dropped` or `failed` means cross-tenant attempts were not recorded |
| `http_rate_limited_total{scope}` | counter | Throttling, by `tenant` or `subject` budget |
| `http_load_shed_total` | counter | Overload |
| `db_pool_connections{state}` | gauge | Pool saturation (`in_use` near `SAGAT_DB_MAX_CONNECTIONS`) |

Starting points for alerts (suggestions, **not** shipped or tuned; pick thresholds from your own
baseline): `/v1/ready` failing for more than a minute; 5xx ratio above your SLO; any sustained
`jwks_refresh_total{outcome="error"}`; any `audit_denials_total{outcome="failed"}`; `in_use` at
the pool maximum.

## 6. Audit trail

The audit table is append-only: `UPDATE`, `DELETE` and `TRUNCATE` are rejected by triggers that stay
active even under replication settings, and `sagat_app` has no such privilege. Read it with the
owner role or `sagat_app`:

```sql
SELECT created_at, actor, action, resource_type, resource_id, trace_id
FROM audit_records
WHERE tenant_id = (SELECT id FROM tenants WHERE slug = 'acme')
ORDER BY created_at DESC
LIMIT 100;
```

- Cross-tenant attempts are recorded in the **actor's** tenant, deduplicated per actor/resource in a
  5-minute window, so a flood does not grow the table without bound.
- `403` (permission denied inside one's own tenant) is **not** audited yet (research G9).
- Records contain `trace_id`; the trace id may be chosen by the caller (G15), so treat it as a
  correlation hint, not as proof of origin.

## 7. Backup and restore

State is only PostgreSQL. Use your standard physical backups plus point-in-time recovery. A logical
dump works for small installations:

```sh
pg_dump --format=custom --file=sagat-$(date +%F).dump "$OWNER_DATABASE_URL"
pg_restore --clean --if-exists --dbname="$OWNER_DATABASE_URL" sagat-YYYY-MM-DD.dump
```

After a restore, run `control-plane migrate` (idempotent) and confirm `/v1/ready` before sending
traffic. The `sagat_app` role is a cluster-level object and is **not** part of a database dump. Create
it on the target server *before* restoring or migrating (the Docker image does it through
`infra/docker/postgres-init/01-runtime-role.sh`). The migration that grants its privileges silently
skips the grants when the role does not exist yet, so if you created the role after migrating,
re-apply the `GRANT` statements from `20261008000200_runtime_role_grants.up.sql` by hand and check
with `\dp` that `sagat_app` has no `UPDATE`/`DELETE`. **Restore drills have not been run in this
repository**; do one before relying on this section.

## 8. Credentials and keys

- **Database password:** change it in the secret store, then restart the API. `DATABASE_URL` is never
  logged (credentials are redacted in every message).
- **OIDC key rotation:** nothing to do. The key cache refreshes on an unknown `kid`; the old keys keep
  verifying tokens until they expire. An empty key set from the provider never replaces a good one.
- **Revoking access:** delete the membership row. Roles are read from the database on every request,
  so it takes effect on the next request. A token that is still valid keeps authenticating
  (identity) but loses every permission. There is no token revocation list yet (G7).

## 9. Not covered (read before relying on this)

- **High availability.** One instance is what was tested. Rate limits are per process, so several
  instances multiply them; a shared limiter and HA procedures belong to feature 010.
- **OTLP span export** is not implemented (feature 008). Logs, `X-Trace-Id` and the metrics above are.
- **Alert rules and dashboards** are not shipped.
- **Restore** and **failover** have not been rehearsed.
- A development identity provider does not exist (G10): local use needs your own provider.
