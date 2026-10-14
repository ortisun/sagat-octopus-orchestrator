# Runbook: security and governance controls (feature 009)

What an operator needs to run the controls that keep tenants apart and secrets secret: worker
bindings, the secret store and its key ring, quotas, egress policy, and TLS on the worker API. The
decisions behind them are D42-D59 in `specs/009-security-multitenancy/research.md`; the commands below
were run against this repository, and what is **not** built is listed at the end.

## 1. What governs what

| Control | Decides | Where it is enforced | Managed by |
|---|---|---|---|
| Worker binding | Which namespaces and activities a worker identity may serve | At registration and on every heartbeat, completion, failure and secret resolution | `PUT/DELETE /namespaces/{id}/worker-bindings/{subject}` (`worker:bind`) |
| Secret | A named value a step may ask for while it holds a lease | Publishing a workflow that declares it; `ResolveSecrets` | `/namespaces/{id}/secrets...` (`secret:manage`, names with `secret:read`) |
| Secret access list | Who may reference one secret from workflows they publish | Publishing, and again at every resolution | `PUT/DELETE /namespaces/{id}/secrets/{name}/access/{subject}` |
| Quota | How much of the platform a namespace or tenant may use | Inside the transaction that creates the thing | `/quotas` and `/namespaces/{id}/quotas` (`quota:manage`; read with `quota:read`) |
| Egress policy | Where a namespace's outbound requests may go | The script supervisor reads it before every mediated host call; other outbound features use `EgressService` | `/namespaces/{id}/egress-policy` (`egress:manage`) |
| TLS / mTLS | Who may reach the worker API and that nobody reads it on the wire | The engine's listener | `SAGAT_GRPC_TLS_*` |

Only administrators hold `worker:bind`, `secret:manage`, `quota:manage` and `egress:manage`. A
developer can list the names of secrets and use the ones whose list names them; an operator or viewer
can read quotas; a worker identity can do nothing but its work.

## 2. Setting it up

### The key ring (secrets)

```sh
# One key: 32 random bytes, base64. Generate it once and keep it in your secret manager.
head -c 32 /dev/urandom | base64
# The ring: id:base64, first entry seals new values, every entry opens what it sealed.
export SAGAT_SECRETS_KEYS='k1:<base64 of the key>'
```

Set the **same** ring on the control plane (which seals) and on every execution engine (which opens).
Ids are `[a-z0-9_-]`, 1 to 32 characters. A malformed ring stops the process at start with the name of
the variable and never its value. Without a ring the secret routes do not exist (404) and workers cannot
resolve secrets (`NotAllowed`); nothing else changes.

### TLS on the worker API

```sh
export SAGAT_GRPC_TLS_CERT_FILE=/etc/sagat/tls/server.crt   # PEM, full chain
export SAGAT_GRPC_TLS_KEY_FILE=/etc/sagat/tls/server.key    # PEM
export SAGAT_GRPC_TLS_CLIENT_CA_FILE=/etc/sagat/tls/clients-ca.crt   # optional: require client certificates
```

Certificate and key are both or neither; a client CA needs both. A file that is missing or is not PEM
stops the process with the variable's name. **With a key ring, outside `SAGAT_ENV=dev`, the engine
refuses to start without TLS** unless `SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS=true` states that a mesh or
an ingress terminates it in front. mTLS authenticates the channel; the bearer token still names the
worker (gap G44). Rotate the certificate by replacing the files and restarting (a reload is not built).

### Workers need a binding

A worker identity is refused at registration (`PERMISSION_DENIED`, `x-sagat-error: NOT_BOUND`) until an
administrator binds it. Bind a fleet to everything in a namespace, or to what it should serve:

```sh
# everything in the namespace
curl -X PUT "$API/v1/namespaces/$NS/worker-bindings/payments-workers" \
  -H "authorization: Bearer $ADMIN" -H 'content-type: application/json' \
  -d '{"activities": ["*"]}'
# only the payment activities and one more
... -d '{"activities": ["payments.*", "audit.log"]}'
```

`payments.*` covers `payments.charge` and not `payments` or `paymentsx.charge`. Each call replaces what
the identity had in that namespace. Upgrading an existing deployment: bind its worker identities once
(gap G47) or they will be refused.

## 3. Day-two operations

### Quotas

```sh
curl "$API/v1/namespaces/$NS/quotas" -H "authorization: Bearer $ADMIN"        # limit, source, exact use
curl -X PUT  "$API/v1/namespaces/$NS/quotas/active_runs" ... -d '{"limit": 2000}'
curl -X PUT  "$API/v1/quotas/worker_sessions"            ... -d '{"limit": 200}'   # tenant-wide
curl -X DELETE "$API/v1/namespaces/$NS/quotas/active_runs" ...                  # back to the next level
```

Resources: `active_runs` and `workflows` and `secrets` (per namespace), `tenant_active_runs` and
`worker_sessions` (per tenant). The limit in force is the namespace's setting, else the tenant-wide
one, else the tenant's **ceiling**, and never more than the ceiling. A refused request is `429` with
`code: QUOTA_EXCEEDED`, not retryable. Child runs count as runs. The count is exact and taken under a
lock, so a limit is never overshot by replicas racing for the last place.

**The ceiling** is the most a tenant's administrators may set for a resource: the platform default
(**provisional**: 10 000, 50 000, 1 000, 1 000, 500 in the order above; gap G42) unless an operator has
granted that tenant more. It is shown as `ceiling` in every quota view, and a `PUT` above it is a `422`:
a tenant must not be able to remove the limit that protects the other tenants. Only the operator raises
it, from a shell that can reach the database, and no API call does:

```sh
# grant a tenant more room (a number), or take the grant away (default): audited as system:operator
DATABASE_URL=... SAGAT_CEILING_TENANT_ID=<tenant uuid> SAGAT_CEILING_RESOURCE=active_runs \
  SAGAT_CEILING_VALUE=50000 control-plane quota-ceiling
DATABASE_URL=... SAGAT_CEILING_TENANT_ID=<tenant uuid> SAGAT_CEILING_RESOURCE=active_runs \
  SAGAT_CEILING_VALUE=default control-plane quota-ceiling
```

Lowering a ceiling applies at once, also to a limit the tenant had set higher: what is enforced is the
smaller of the two. Granting a ceiling does not raise a limit the tenant set; with nothing set, the whole
ceiling is available.

### Secrets

```sh
curl -X PUT "$API/v1/namespaces/$NS/secrets/stripe-key" ... -d '{"value": "sk-live-..."}'   # create or rotate
curl "$API/v1/namespaces/$NS/secrets" ...                          # names, versions, key ids; no values
curl -X PUT "$API/v1/namespaces/$NS/secrets/stripe-key/access/dora" ...   # let dora use it
curl -X DELETE "$API/v1/namespaces/$NS/secrets/stripe-key/access/dora" ...
```

A workflow declares the names it needs on an activity: `"secrets": ["stripe-key"]`. A value is accepted
here and nowhere else and is never returned. Publishing a workflow that declares a secret needs a
publisher whose role holds `secret:use` and who is on the secret's list (administrators are not subject
to the list); a missing secret and one the publisher may not use get the same `403 SECRET_NOT_USABLE`.
The publisher is checked again each time a worker asks for the value, so **revoking someone from a
secret, demoting them or removing them from the tenant stops the workflows they published from
resolving it**; republish them as someone who has access.

**Rotating a value:** `PUT` the new value. The next resolution sees it; the last five versions are kept.

**Rotating the key ring** (a key was exposed, or on a schedule):

1. Add a new key **in front**: `SAGAT_SECRETS_KEYS='k2:<new>,k1:<old>'`, on the control plane and on every engine. Deploy. New values are sealed under `k2`; everything still opens.
2. Re-seal what is under the old key: `control-plane reseal-secrets` (repeat-safe, it prints how many versions it moved; run it until it prints 0).
3. Check: `SELECT key_id, count(*) FROM secret_versions GROUP BY 1;` shows only `k2`.
4. Remove `k1` from the ring everywhere. A version still sealed under a key that is not in the ring cannot be opened: resolution fails loudly with a log line naming the key id.

If a key was **exposed**, rotating the key is not enough: also `PUT` a new value for every secret (the
old values are in whatever copied the old key and the database).

### Egress policy

```sh
curl -X PUT "$API/v1/namespaces/$NS/egress-policy" ... \
  -d '{"rules": ["api.example.com", "*.partner.example", "203.0.113.0/24"], "ports": [443], "allowHttp": false}'
curl -X DELETE "$API/v1/namespaces/$NS/egress-policy" ...      # everything denied again
```

Default is deny. A rule is an exact host, `*.domain` (below it, not the domain itself) or a CIDR network
(which also allows addressing it by IP literal). Whatever is written, loopback, link-local, the cloud
metadata addresses, multicast and reserved space stay unreachable, and private space needs an explicit
network rule. The script runner has no direct container network. `octopus.fetch` asks the supervisor,
which reads this policy immediately before each request; a committed policy change applies to the next
read, while a request already authorized and in flight may finish.

When the supervisor refuses a request, it first posts a closed category against the current fenced
attempt. Only a 204 after the `script.violation` audit commits is returned to the script as `denied`.
If the incident write times out, the lease is stale or storage is unavailable, the host call fails as
unavailable and sends no request. The incident contains no URL, hostname, header or payload. Repeated
destinations are answered each time but produce at most one row per category and attempt. This is the
script path (D59); feature 004 still owns future webhook signature, replay and inbound SSRF controls.

## 4. Symptoms

| Symptom | Likely cause | What to do |
|---|---|---|
| `429` `QUOTA_EXCEEDED` on starting a run, creating a workflow, registering a worker or storing a secret | The namespace or tenant is at its limit | `GET /namespaces/{id}/quotas` shows limit, source and exact use; end runs, delete what is unused, or raise the limit |
| A step failed with `CHILD_QUOTA_EXCEEDED` | A parent step wanted to start a child run while the namespace or the tenant was at its quota of active runs. The step fails instead of waiting: a waiting child would sit at the head of the scheduler's oldest-first queue and keep other tenants' children from starting (G32). It is not `CHILD_LIMIT_EXCEEDED`, which is the size of one tree | Free or raise the quota; a workflow can route on this code (its error edge) and start again later |
| `422` on a quota `PUT` saying the value is above the ceiling | The administrator asked for more than the platform grants this tenant | The view shows the `ceiling`. If the tenant should have more, the operator grants it (section 3, Quotas); otherwise it is working as designed |
| New worker connections are closed at once, or `grpc_connections_refused_total{reason="per_peer"}` rises | One address (an IPv6 peer counts as its /64) already holds `SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER` connections. Workers behind one NAT or load balancer all look like that address | Raise the cap to the size of the fleet behind it (one connection per worker process is enough), or to `SAGAT_GRPC_MAX_CONNECTIONS` to turn it off |
| A worker's first call after a quiet period fails once with `UNAVAILABLE`, or every 30 minutes | The engine closes a connection that is silent for `SAGAT_GRPC_IDLE_TIMEOUT_SECS` (300) and asks every connection to finish and reconnect after `SAGAT_GRPC_MAX_CONNECTION_AGE_SECS` (1800); a new connection that sends nothing is closed after `SAGAT_GRPC_FIRST_BYTE_TIMEOUT_SECS` (10) and a stalled TLS handshake after `SAGAT_GRPC_TLS_HANDSHAKE_TIMEOUT_SECS` (10). `grpc_connections_closed_total{reason}` counts the first two | Expected; the client reconnects and retries (the SDKs do). Raise the timeouts only if a link is slower than they assume |
| `403` `NOT_BOUND` at registration | The worker identity has no binding covering an activity it advertises | Bind it (section 2); the audit action `worker.denied` names the activity |
| A worker that was working gets `NOT_BOUND` on a heartbeat or report | Its binding was removed or narrowed; its sessions are closed and its attempts released | Expected after a deliberate change; bind it again and let it re-register |
| `403` `SECRET_NOT_USABLE` when publishing | The secret does not exist, or the publisher's role lacks `secret:use`, or they are not on its list | Check the name, then `GET .../secrets/{name}/access`; the answer is the same for missing and forbidden on purpose |
| A worker gets `SECRET_NOT_USABLE` from `ResolveSecrets` | The secret was deleted, or the **publisher of that version** lost access, was demoted or left | Restore access, or republish the workflow as someone who has it; audit action `secret.denied` has the node and attempt |
| A step failed with `SECRET_IN_OUTPUT` | The activity returned a secret it was given (in its output, or as the code it failed with, and in any version of the secret that is still stored); the output was discarded | Fix the activity; rotate that secret if the value was written anywhere the platform does not control |
| `ResolveSecrets` answers `INVALID_ARGUMENT` "secrets are not configured" | The engine has no `SAGAT_SECRETS_KEYS` | Give it the same ring as the control plane |
| Resolution fails with a log line "a stored secret cannot be opened" | A version is sealed under a key that is not in this process's ring | Restore the key to the ring, or `PUT` the secret again |
| Engine exits 2 naming `SAGAT_SECRETS_KEYS` | A ring and a plaintext worker API outside dev | Configure TLS, or state that it is terminated in front |
| Control plane exits 2 naming `SAGAT_API_ALLOW_PLAINTEXT_SECRETS` | A ring is configured, so the API receives secret values in request bodies, and it listens in plaintext outside dev | Terminate TLS in front of it and set this to `true` to say so |
| `control-plane reseal-secrets` finishes with "N secret versions could not be opened" | Some versions are sealed under a key the ring no longer holds. All the others were moved first and the number is in the message | Restore that key to the ring (keep it until the command reports none), then run it again; the command is safe to repeat |
| A developer lists fewer secrets than the namespace holds | By design: anyone but an administrator sees only the secrets whose access list names them, so that names cannot be probed | An administrator grants access (`PUT .../secrets/{name}/access/{subject}`) |
| Engine exits 2 naming a `SAGAT_GRPC_TLS_*` variable | Missing pair, unreadable or non-PEM file | Fix the named variable |
| TLS clients fail with a certificate error | The client does not trust the CA, the name differs from the certificate, or (mTLS) it presents no certificate signed by the client CA | Check the chain and the SAN; `grpc_connections_total` stops rising for them |
| Cross-tenant `404`s from one caller | Someone is probing identifiers of another tenant | `SELECT * FROM audit_records WHERE action = 'access.denied.cross_tenant'` (one row per caller and resource per window) |
| A script gets `denied` from `octopus.fetch` | Its artifact disables egress, or the current namespace policy refuses the scheme, host, port or resolved address | Query `script.violation` for that run and attempt. Its reason is one of the seven closed categories; change the namespace policy only if the destination is intended |
| A script host call fails as unavailable and `script_incidents_total` does not rise | The supervisor could not read current policy or could not commit the incident before acknowledging it | Check control-plane reachability, the runner's current lease/binding and PostgreSQL. The outbound request was not made; retry through the workflow policy after the control plane recovers |

## 5. Incidents

- **A secret value leaked.** `PUT` a new value now. `SELECT created_at, actor, reason FROM audit_records WHERE action = 'secret.resolved' AND resource_id LIKE '<namespace id>/%'` shows who was handed which secret and version, by run and node; revoke the identities involved (`DELETE .../worker-bindings/{subject}` closes their sessions and releases their leases at once).
- **The key ring leaked.** Rotate the ring (section 3), then replace every secret value.
- **A worker identity is compromised.** Remove its bindings in every namespace; its sessions are closed and the attempts it holds are reclaimed by the scheduler, and its late reports are refused as stale. Revoke the identity at the OIDC provider as well.
- **A sandbox attempted forbidden egress.** Preserve the `script.violation` rows for its run, step, attempt and generation, then inspect the script artifact and namespace policy. The row contains a category only; use the execution diagnostics if the destination is needed for investigation, applying the deployment's data-retention rules.
- **A tenant's data may have been reached by another tenant.** This is a critical finding by definition (spec SC-009-001). Preserve `audit_records` and the logs, and run the cross-tenant battery against the deployed version (`cargo test -p sagat-tests --test contract_rest_security_multitenancy`).
- **A developer left.** Their published workflows stop resolving secrets at the next attempt (D46). Republish what is still needed as someone with access.

## 6. Observability

Audit actions (immutable, redacted, never a value): `quota.changed`, `secret.stored`, `secret.deleted`,
`secret.access_granted`, `secret.access_revoked`, `secret.resolved`, `secret.denied`,
`egress.policy_changed`, `egress.denied`, `script.violation`, `worker.bound`, `worker.unbound`, `worker.denied`,
`access.denied.cross_tenant`.

Metrics (labels are closed sets; no tenant, subject, run, namespace or secret is ever a label):

| Metric | Meaning |
|---|---|
| `security_denials_total{action}` | Every denial that is audited as `access.denied.cross_tenant`, `worker.denied` or `egress.denied`, counted per attempt **before** the audit write. The audit table keeps one row per caller and resource per window and may drop events under flood; this counter does not, so it is the one to alert on |
| `script_incidents_total{category}` | Sandbox supervisor incident reports accepted after the fenced audit transaction committed; categories are a closed set and carry no destination or tenant label |
| `quota_exceeded_total{limit}` | Requests refused by a limit, by which limit (`active runs in the namespace`, `worker sessions of the tenant`, ...); the same refusals appear as `429` in `http_requests_total` |
| `secrets_handed_out_total` | Secret values given to workers through `ResolveSecrets`, counted after the resolution and its audit record committed |
| `secret_resolution_denied_total` | `ResolveSecrets` refused because a declared secret is no longer usable (deleted, or its publisher lost access) |
| `grpc_connections_refused_total{reason}` | Connections turned away without taking a place from anybody else (`per_peer`) |
| `grpc_connections_closed_total{reason}` | Connections the engine closed because the peer was silent: `silent` (new, sent nothing) or `idle` (established) |
| `secret_output_detected_total` | Outputs found to contain a secret's value. A detection, counted when found: if the commit that fails the step is lost, the retry counts it again |
| `grpc_shed_total{class}`, `grpc_in_flight`, `grpc_connections_total`, `grpc_requests_total{method="ResolveSecrets",code}` | Worker API admission and traffic (feature 002) |

Suggested alerts (the rules themselves belong to feature 008): any increase of
`security_denials_total{action="access.denied.cross_tenant"}` from one deployment is worth a look, a
sustained rate is an incident; any increase of `secret_output_detected_total` means an activity is
returning a credential; `quota_exceeded_total` rising for one limit on a quiet system means a limit
is too low or a client is looping.

Feature 009 reuses the published Feature 010 chart and adds role-scoped egress policies to it.
The control plane gets DNS, PostgreSQL and OIDC; the engine adds NATS only when configured; workers
get DNS plus the control-plane API and engine gRPC. Each worker pool starts with denied ingress and
may add validated, pool-local rules. Empty peers, global CIDRs, missing ports and empty ingress
sources fail Helm rendering. Reproduce the G45 verification on an explicit disposable Cilium cluster:

```sh
KUBECONFIG=/path/to/disposable-kubeconfig \
  CHART_DIR=infra/helm tests/security/sandbox/kubernetes-network-policy.sh
```

The verifier rejects unsafe values before creating its test namespaces, applies the rendered policies and service
accounts, validates PodSpecs derived from every workload template against restricted Pod Security
admission, and uses controlled TCP listeners for PostgreSQL, NATS, OIDC, control-plane and engine
ports. It proves declared TCP paths work, undeclared Kubernetes API and cross-role paths fail,
identical labels in another namespace do not grant ingress, and one worker pool cannot inherit
another pool's overrides. The integrated chart passed this matrix on Cilium. This is a network and
admission proof: it does not install the product Deployments or KEDA objects and does not exercise
real PostgreSQL, NATS or OIDC protocols. `contract_scalability_ha` separately verifies the complete
render and the exact role, port, peer, optional-NATS, pool-override and disabled-policy contracts.

### Local verification with kind

Run from the repository root with Docker, kind, Helm, kubectl and jq installed. This recipe uses
Kubernetes 1.37.0 and Cilium 1.20.2. It follows the [Cilium kind installation guide](https://docs.cilium.io/en/stable/installation/kind/):
the default CNI is disabled and Cilium uses Kubernetes IPAM. The node becomes ready after Cilium is
installed. A unique cluster name and a separate kubeconfig keep the current cluster context intact.
Select a working Docker engine first; with Colima, set `DOCKER_CONTEXT` to its named context.

```sh
security_dir=$(mktemp -d)
security_cluster="sagat-security-$(date +%s)-$$"
export KUBECONFIG="$security_dir/kubeconfig"

kind create cluster --name "$security_cluster" \
  --config tests/security/sandbox/kind.yaml --kubeconfig "$KUBECONFIG"
helm install cilium oci://quay.io/cilium/charts/cilium \
  --version 1.20.2 --namespace kube-system \
  --set ipam.mode=kubernetes --set operator.replicas=1 \
  --set image.pullPolicy=IfNotPresent --wait --timeout 5m
kubectl wait --for=condition=Ready nodes --all --timeout=120s
CHART_DIR=infra/helm tests/security/sandbox/kubernetes-network-policy.sh
```

The verifier removes its own namespaces on success or failure. After inspecting the result, remove
only the cluster created above and its temporary kubeconfig:

```sh
kind delete cluster --name "$security_cluster"
rm -rf -- "$security_dir"
unset KUBECONFIG
```

For an existing disposable kind cluster, export its kubeconfig to a separate file with
`kind export kubeconfig --name <cluster> --kubeconfig <file>`, then run the verifier with
`KUBECONFIG=<file>`. The verifier does not create, reconfigure or delete that cluster.

## 7. Not covered

- Webhook signature, replay and inbound SSRF controls are documented with feature 004.
- Product-approved default quota values per tier (G42); provisional bounds are engineering defaults.
- A KMS or HSM behind the key ring (G43); binding a client certificate to a token subject (G44).


### Activity concurrency policy changes

Activity limits apply to unexpired `LEASED`/`RUNNING` tasks in one tenant and namespace.
Admission holds shared tenant and namespace policy locks, then the exact activity lock, through the
lease transaction. Multi-activity claims acquire activities by activity name and node id, so two
workflows with opposite node orders cannot deadlock each other. Distinct activities retain concurrent
admission. Namespace defaults exclusively fence that namespace; tenant defaults and operator ceilings
exclusively fence the tenant. An edit waits for earlier admissions to finish before acknowledging.
Existing leases remain valid after a lower limit; subsequent admissions use the new effective limit.
Auditing shares each policy-write transaction. `quota_exceeded_total{limit="activity_concurrency"}`
records refusals without activity or tenant metric labels.

Reproduce lock ordering and policy races with
`cargo test -p sagat-tests --test integration_durable_execution activity_locking` against Docker.
The ABBA regression gates two concurrent lease writes after admission to exercise the original
opposite-activity ordering deterministically.

## Durable byte, rate and retention quotas (ADR-0027)

`tenant_content_bytes` charges stored content and outstanding blob reservations;
`tenant_log_bytes` charges retained diagnostic bytes. The schema classification in
`crates/infra-postgres/storage-classification.json` enumerates every variable-size column.
PostgreSQL triggers and transaction-local deltas commit counters with the owning mutation.
Content reuse is charged once; deletes release charged bytes. Lowering a ceiling does not erase
existing data and still permits shrinking mutations. Blob admission reserves the declared size
(or configured maximum) before PUT; finalization converts it to measured bytes. Unknown late-PUT
reservations remain charged until safe cleanup, including unresolved legacy uploads conservatively
reserved at 5 GiB each.

Logs retain the largest permitted UTF-8 prefix and explicitly mark omitted diagnostics. Build/test
completion and event/connector progress can therefore finish at zero log capacity. A refused new
build artifact or test output becomes non-retryable `STORAGE_QUOTA_EXCEEDED` under the original
job fence. Read `failureCode` even when the diagnostic string is empty. The metric
`quota_log_truncated_total` records clipping without tenant or payload labels.

Namespace API rates use a shared PostgreSQL token bucket (two-second burst), following authorization
and canonical namespace resolution. See [the route inventory](api-rate-admission.md). Quota policy
administration remains available at zero API quota. Event namespace/source buckets commit with
inbox admission and connector progress: duplicates do not debit tokens, and refusal does not advance
a broker checkpoint or acknowledge its delivery. Event retention checks include the replay window
`2 * clock skew + 60 seconds`. Script forward leases pin retention before execution, including
script compensation/reconciliation obligations; lowering the current ceiling never rewrites existing
expiry or revokes an admitted obligation. A separate, unbound execution report uses current policy.

Until a poll transaction commits and exposes tasks, its claims are new admission. If an
oversized-input preflight failure would allocate charged on-error data, full storage rolls back
all tentative claims and returns quota backpressure; existing READY work remains durable and may
wait for capacity. No exposed lease token or external completion is lost. Already-exposed worker
reports and scheduler expiries use the protected terminal/stop recovery paths instead. Event/cron
starts and event wake-ups likewise retain their durable inbox/recovery record when new admission
is refused; no broker checkpoint or acknowledgement advances for a rejected transaction.

### Upgrade and rollback

This change requires a coordinated writer upgrade; mixed old/new writers are not supported while
byte enforcement is enabled. Old processes do not acquire the new policy/usage locks, stamp script
retention pins, or implement quota-safe completion. Stop admission, drain or stop all control-plane,
engine and runner writers, apply migrations with the owner role, deploy the matching binaries, then
resume. Read replicas alone do not make an old writer safe. Preserve database backups before this
migration and test the sequence on a restored copy.

Before migration `20261025000000`, check historical script trace identifiers using counts only:

```sql
SELECT count(*) AS invalid_script_trace_ids FROM script_executions
WHERE trace_id IS NOT NULL AND
      (trace_id !~ '^[0-9a-f]{32}$' OR trace_id = repeat('0',32));
```

The validated constraint refuses an upgrade containing invalid legacy identifiers. Investigate and
repair those records under the operator's existing evidence-retention procedure before retrying;
the migration does not delete, print, or silently grandfather arbitrary historical content.
New reports accept only nonzero, 32-character lowercase hexadecimal trace IDs.

Migration `20261024000000` backfills counters and initially leaves content/log enforcement disabled
in the owner-only `quota_accounting_control` row so consumer integration can be verified. Final
migration `20261026000000` enables both guards after the verified consumer changes; a fresh database
with all migrations has enforcement on. Its rollback disables the guards without deleting usage or
reservations. Both directions refuse a missing control row. Do not enable it with older binaries. API/event rates and retention admission are application
policies and require the matching binaries independently of these byte switches. Rollback refuses
to discard outstanding reservations or non-default new policies; `20261025000000` additionally
refuses rollback while truncation/failure-code evidence exists. Restore or forward-fix according to
the deployment's recovery plan rather than deleting evidence to bypass these guards.
