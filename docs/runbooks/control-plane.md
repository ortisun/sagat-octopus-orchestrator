# Runbook: control plane (feature 001)

Operational procedures for the `control-plane` process as it exists after feature 001: the REST API,
PostgreSQL as the only state, and an external OIDC provider. Everything here was run against the
code in this repository; what is **not** covered is listed at the end so nobody assumes it.

## 1. What runs

| Piece | Where | Notes |
|---|---|---|
| API | `control-plane serve`, default `127.0.0.1:8080` | Stateless; all state is in PostgreSQL |
| Admin listener | `SAGAT_ADMIN_ADDR`, default `127.0.0.1:9090` | `/metrics` only, **without authentication**. Binding it to a non-loopback address (needed for a scraper in another pod) logs `the metrics endpoint is unauthenticated and listens on a non-loopback address` at `WARN` at start: restrict that port with a Kubernetes `NetworkPolicy` (allow only the Prometheus pods) or a firewall rule, and never publish it through an Ingress or load balancer. `off` disables it |
| PostgreSQL 16 | `DATABASE_URL` | Source of truth. TLS is required outside `SAGAT_ENV=dev` |
| Key ring for secrets | `SAGAT_SECRETS_KEYS` (`id:base64,...`), optional | Seals the values of the secret store; the same ring must be set on every execution engine. Without it the secret routes do not exist. Operating the secret store, quotas, bindings and egress policy: `docs/runbooks/security-governance.md`. `control-plane reseal-secrets` re-seals stored versions after a key rotation (it moves what it can and then fails if some version is sealed under a key the ring no longer holds). Outside `SAGAT_ENV=dev` the process refuses to start with a ring unless `SAGAT_API_ALLOW_PLAINTEXT_SECRETS=true` states that TLS is terminated in front of this API, which receives the values in request bodies |
| Quota ceilings | `control-plane quota-ceiling` with `SAGAT_CEILING_TENANT_ID`, `SAGAT_CEILING_RESOURCE`, `SAGAT_CEILING_VALUE` (a number, or `default` to withdraw) | The most a tenant's administrators may set for a quota; only this command raises it. Audited as `system:operator`. See `docs/runbooks/security-governance.md` |
| OIDC provider | `SAGAT_OIDC_JWKS_URI` (HTTPS) | Only its public keys are fetched; no token is ever sent to it |

Two database roles exist on purpose. The **owner** role runs `migrate` and `provision-tenant`. The
API serves requests as **`sagat_app`**, which owns nothing, cannot alter or delete audit rows and
cannot change the schema. Never point the API at the owner role: outside `SAGAT_ENV=dev`, `serve`
checks the role it is connected as and **refuses to start** (exit code 2, before it listens) when
that role is a superuser, owns `audit_records` (directly or through a role it belongs to), or may
`UPDATE`, `DELETE` or `TRUNCATE` it. It also refuses when `audit_records` does not exist yet,
because the privileges cannot be verified: run `migrate` first. The check runs once at start; it
does not look at other role attributes (`CREATEROLE`, `REPLICATION`, ...). With `SAGAT_ENV=dev` it
is skipped, which is why the local Docker stack (dev) also works with the owner role.

## 2. Start, stop, upgrade

```sh
control-plane migrate            # owner role; idempotent, safe to run twice
control-plane serve              # runtime role
```

- `SAGAT_MIGRATE_ON_START=true` runs the migrations before serving. It needs the owner role, which
  `serve` refuses outside `SAGAT_ENV=dev`, so it is **only accepted in dev** (exit code 2 with a
  message naming the variable otherwise). In staging and production run `migrate` as a separate step
  with the owner role, then start `serve` with the runtime role.
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

- Both values are mandatory; there are no defaults. The administrator's subject must not start with
  `system:` (any case): that namespace is reserved for platform actors such as the audit actor
  `system:provisioning`, and the command exits with code 2 naming `SAGAT_PROVISION_ADMIN`. A token
  whose `sub` is in that namespace is never accepted, whatever the database contains.
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
| `GET /v1/ready` | PostgreSQL answers within 500 ms **and** the identity tables exist; cached for 1 s, and concurrent probes share one ping | Readiness |

When `/v1/ready` is `503` the response never says why; the cause is a `WARN` line in the JSON log
(`readiness probe failed`, preceded by `storage unavailable` with the detail, for example "the
database schema has not been migrated" or a pool/connection error).

With native TLS, the container healthcheck uses HTTPS whenever `SAGAT_HTTP_TLS_CERT` is set. Set
`SAGAT_HEALTHCHECK_TLS_SERVER_NAME` to the DNS name on the server certificate (default
`localhost`) and, for a private CA, mount its server trust roots at
`SAGAT_HEALTHCHECK_TLS_CA_FILE`. This trust bundle is deliberately separate from
`SAGAT_HTTP_TLS_CLIENT_CA`: a CA trusted to identify webhook clients must not automatically become
a CA trusted to identify this server. The probe resolves that name only to
`SAGAT_HEALTHCHECK_ADDR`, verifies the certificate and hostname, ignores proxy settings and
redirects, and fails within three seconds. The CA file is limited to 256 KiB.

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
| `429` right after authenticating, with `http_tenant_concurrency_rejected_total` rising | One tenant already has `SAGAT_MAX_IN_FLIGHT_PER_TENANT` requests in flight (default a quarter of `SAGAT_MAX_IN_FLIGHT`, at least 1): slow or stalled uploads, or slow handlers. The other tenants are unaffected by design | Find the tenant's clients holding requests open; the stalled ones end by themselves after `SAGAT_BODY_IDLE_TIMEOUT_SECS`. Raise the cap only if the tenant's real concurrency needs it |
| `408` | The request exceeded `SAGAT_REQUEST_TIMEOUT_SECS`, or its body stopped arriving for `SAGAT_BODY_IDLE_TIMEOUT_SECS` (default 5 s between pieces of the body) | Slow database or client; check `http_request_duration_seconds`. A client that trickles one byte at a time is bounded only by the request timeout |
| Process refuses to start | Missing/invalid setting (exit code 2, names the variable), or `DATABASE_URL` without TLS outside dev | Fix the named variable; the message never includes the URL's password. For the standalone healthcheck, TLS settings require `SAGAT_HTTP_TLS_CERT`, a DNS-only server name and an exact IP socket in `SAGAT_HEALTHCHECK_ADDR` |
| Process exits with code 2 and `database role '<name>' is too privileged to serve requests: ...` | `DATABASE_URL` of `serve` points at the owner role, a superuser, or a role that may rewrite `audit_records` (the message lists each reason) | Use the runtime role (`sagat_app`) in the `serve` environment; keep the owner role for `migrate` and `provision-tenant` only. Do not grant the runtime role more to make this go away |
| Process exits with code 2 and `the audit trail does not exist yet ... run control-plane migrate` | `serve` started before the migrations (outside dev the privileges cannot be verified without the table) | Run `migrate` with the owner role first; start the API afterwards (let the orchestrator restart it) |

### Limits and the settings behind them

All of these are **per process**: with several instances each applies its own limit (research G13).
Defaults are safe; `control-plane --help` lists every setting.

| Protection | Setting (default) | Over the limit |
|---|---|---|
| Request budget per verified token subject, charged before any database lookup | `SAGAT_SUBJECT_RATE_LIMIT_RPS` (20) and `_BURST` (40) | `429 RATE_LIMITED` with `Retry-After` |
| Request budget per tenant, charged after authentication | `SAGAT_RATE_LIMIT_RPS` (100) and `_BURST` (200) | `429 RATE_LIMITED` with `Retry-After` |
| Both budgets accept a rate of at least 0.001 and a burst of at least 1 | - | The process refuses to start (exit code 2, names the variable) |
| Requests of one tenant in flight | `SAGAT_MAX_IN_FLIGHT_PER_TENANT` (a quarter of `SAGAT_MAX_IN_FLIGHT`, at least 1) | `429` at once, `http_tenant_concurrency_rejected_total` |
| Requests in flight, all tenants (probes exempt) | `SAGAT_MAX_IN_FLIGHT` (512) | `503` at once with `Retry-After` of 3 to 8 s, `http_load_shed_total` |
| Wait for the next piece of a request body | `SAGAT_BODY_IDLE_TIMEOUT_SECS` (5) | `408` |
| Total time of one request | `SAGAT_REQUEST_TIMEOUT_SECS` (15) | `408` |
| Request body size | `SAGAT_MAX_BODY_BYTES` (65536) | `413` |
| Time to send a request head, idle keep-alive | `SAGAT_HEADER_READ_TIMEOUT_SECS` (10) | Connection closed |
| Simultaneous connections | `SAGAT_MAX_CONNECTIONS` (1024) | New connections wait in the kernel backlog |

The limiter tables are bounded (100 000 keys each). If a table is ever full, keys that are not yet
tracked share one overflow bucket, so they are throttled together; this is silent today (no metric).

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
| `http_tenant_concurrency_rejected_total` | counter | A tenant hit its in-flight cap (stalled or slow requests) |
| `http_load_shed_total` | counter | Overload |
| `db_pool_connections{state}` | gauge | Pool saturation (`in_use` near `SAGAT_DB_MAX_CONNECTIONS`) |

Prometheus rules and a Grafana dashboard for the whole platform ship in `infra/observability/`
(feature 008; see [`observability.md`](observability.md)). Their thresholds are starting points
taken from the design and not from a production baseline: tune them. Conditions that concern one
namespace (an outbox that does not drain, steps nobody serves) are alerts the engine raises
itself, read with `GET /namespaces/{id}/alerts` ([`alerts.md`](alerts.md)).

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

- **Through the API (feature 008):** `GET /audit` (Admin) returns the tenant's trail, newest first,
  narrowed by action, resource, actor and time. Reading it is itself recorded (`audit.read`) and a
  credential-shaped string in a reason is replaced on the way out. See
  [`observability.md`](observability.md) section 6.
- Cross-tenant attempts are recorded in the **actor's** tenant, deduplicated per actor/resource in a
  5-minute window, so a flood does not grow the table without bound. **`GET /audit` does not return
  them**: they exist only when the identifier belongs to another tenant, so listing them would tell
  the reader whether an identifier exists elsewhere. Read them in the table.
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
- **OTLP span export** exists (feature 008) with the limits in [`observability.md`](observability.md): HTTP with JSON only, one projector at a time.
- **Alert rules and dashboards** ship in `infra/observability/` but are not tuned to a production baseline, and alerts leave only by webhook (no pager or email integration, G175).
- **Restore** and **failover** have not been rehearsed.
- The development identity provider (G10) is `apps/dev-idp`, closed by feature 012 ([`developer-experience.md`](developer-experience.md) §2): it signs a token for anyone who asks and runs only with `SAGAT_ENV=dev`; a shared environment uses its own provider.
