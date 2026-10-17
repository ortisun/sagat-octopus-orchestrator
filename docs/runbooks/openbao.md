# OpenBao and managed database deployment

Feature 017 uses OpenBao KV v2 behind Sagat's secret API. UI and SDK users use logical
secret/connection names; only backend workload identities call OpenBao. PostgreSQL is
the first target driver. See [managed execution](database-runner.md) and
[UI/SDK usage](../guides/managed-database-executions.md).

## Production prerequisites

Use Kubernetes 1.30+ (official chart requirement), a CNI that enforces NetworkPolicy,
three worker nodes, persistent encrypted volumes, a trusted TLS issuer and your OIDC
workload token issuer. The reviewed OpenBao chart is **0.30.0**, server/agent **2.7.1**,
pinned by SHA-256 in the supplied files. The download helper verifies the chart archive.
No application image contains bootstrap credentials.

1. Fetch with `python3 infra/openbao/fetch-chart.py /private/charts/openbao-0.30.0.tgz`.
2. Create namespace `openbao` and Secret `openbao-tls` containing `tls.crt`, `tls.key`,
   `ca.crt`. SANs must include `openbao.openbao.svc` and
   `*.openbao-internal.openbao.svc`. Supply your storage class in an operator overlay.
3. Install release **openbao** into namespace **openbao**, using
   `infra/openbao/production-values.yaml`. It has three Raft voters, retained data/audit
   PVCs, TLS verification, non-root containers and finite resources. The certificate
   must also cover any operator endpoint used for bootstrap.
4. Initialize **one node only** using `bao operator init`, distributing Shamir shares
   to separate custodians, or configure an approved auto-unseal provider. Unseal all
   three nodes; wait until all are ready and `bao operator raft list-peers` shows three
   voters. Never initialize the remaining nodes as separate clusters. Keep unseal
   material outside the cluster and outside its data backup. The fixture's one-share
   ceremony is for disposable tests only.
5. Use a short-lived, privileged operator token in a private file to run:

   ```sh
   python3 infra/openbao/bootstrap-kubernetes.py \
     --address https://openbao.openbao.svc:8200 --ca /private/ca.crt \
     --token-file /private/operator-token --namespace sagat \
     --fullname sagat-octopus-orchestrator
   ```

   The script configures `secret/` as KV v2, Kubernetes auth, three scoped policies and
   matching roles. The server uses its local rotating ServiceAccount token for
   TokenReview. Only its SA has `system:auth-delegator`. Audit is **declarative in the
   server HCL**: OpenBao 2.7 disables API-created devices by default. The script verifies
   that the configured audit device exists. Do not enable the unsafe compatibility flag.
   Revoke the bootstrap root token after establishing operator access; never mount it
   into Sagat. The bootstrap script does not revoke an operator's token implicitly.
6. Render/apply `infra/openbao/render-network.py --kubernetes-cidr <API-service-IP/32>
   --kubernetes-cidr <API-endpoint-IP/32>`. Include both pre/post-DNAT endpoints and
   the real API port. This policy permits DNS, TokenReview, peer Raft traffic and the
   three Sagat backend components. It excludes database runners and ordinary workers.
   Match the documented release/namespace names or adapt the selectors explicitly.

Audit logs use HMAC redaction. Ship the audit PVC logs to the operator's protected log
store, rotate them with a tested reopen procedure, and alert on filesystem pressure:
audit failure can stop authenticated operations. TLS and server configuration upgrades
use the chart's `OnDelete` strategy: replace/unseal one node at a time, retaining quorum.
Never delete the PVCs as an upgrade procedure.

## Sagat workloads

Start from `infra/helm/examples/ha.yaml` plus `examples/database.yaml`. Replace the
example namespace UUID/slug and private CIDRs. Build Dockerfile target `database-runner`,
publish it to your registry and supply its actual immutable digest; the example refuses
to render with an empty image. There is no fabricated release image.

Provide these Secrets in the Sagat namespace:

| Secret | Keys | Recipient |
|---|---|---|
| `openbao-ca` | `ca.crt` | Backend agents and provider clients |
| `openbao-intent` | `intent-key`, exactly 32 random raw bytes | Control plane only |
| `sagat-engine-tls` | `tls.crt`, `tls.key` | Engine TLS listener |
| `sagat-engine-ca` | `ca.crt` | Database runner |
| `sagat-database-runner-identity` | `token` | Database runner |

Generate the intent key once, share it across control-plane replicas and back it up
with the metadata. It is separate from both target credentials and the local legacy
key ring. See the [recovery/rotation procedure](database-recovery.md) before replacing it.

Each backend has a distinct SA and OpenBao Agent. Only the agent mounts its projected
Kubernetes JWT (audience `openbao`, 10-minute lifetime). It writes a finite provider token
into a memory volume; the backend mount is read-only. Tokens have 30-minute TTL and a
one-hour maximum, so the agent renews and re-authenticates. Changes to agent roles/auth
mounts trigger pod replacement through a configuration checksum. With
`serviceAccount.create=false`, create the three named SAs yourself.

The runner has an **OIDC Sagat identity**, independent of the OpenBao identities. As the
existing identity provisioning procedure requires, an operator creates its tenant
membership as `database-runner` using the owner role; the administrator then binds that
subject via `PUT /v1/namespaces/{id}/worker-bindings/{subject}` with
`{"activities":["sagat.database.postgresql"]}`. Supply a short-lived JWT for this exact
subject/tenant and arrange atomic refresh of the mounted Secret through your issuer's
workload identity integration. The runner rereads it for every RPC. A Kubernetes JWT
is not automatically interchangeable with your OIDC token. One runner pool serves one
tenant, at most 32 configured namespaces and at most 32 sessions per identity.

The StatefulSet's ordinal name is the stable worker identity suffix. Application and
namespace CIDR/port scopes intersect. Set `deployment.denyCidrs` to **all platform state
endpoints**; never put those addresses in target egress. TargetEgress adds only explicitly
approved customer destinations to DNS and the engine gRPC endpoint. No OpenBao token,
platform PostgreSQL password, Kubernetes JWT or local secret key is mounted in the runner.
Metrics port 9092 is limited to monitoring peers. `/metrics` is liveness only; inspect
registration/poll counters and durable run progress for operational readiness.

Cleanup runs every five minutes by default, one batch at a time, through its own SA and
restricted token. It has no credential-read or metadata-delete right. The application
command has a 60-second deadline and the Job a 180-second deadline including authentication.
A failed batch retries on the next schedule; stale claims expire. Suspend this CronJob
before a coordinated backup/restore. Account for its two extra PostgreSQL connections.

## Local demonstration

Keep the normal development stack in `infra/docker/compose.yaml`. Seed the `dev` tenant
and obtain its `payments` namespace UUID using the existing developer quickstart. Then:

```sh
python3 infra/openbao/prepare-development.py --directory "$PWD/.local/database" \
  --namespace-id <payments-namespace-uuid>
```

The new directory is private and ignored by Git/Docker build context. It contains a
seven-day fixture CA/leaf certificates, restricted target role, sample `demo_orders`,
connection credential, intent key and runner policy. It refuses to overwrite an existing
environment. Set `SAGAT_DATABASE_DEV_DIR` and `SAGAT_DATABASE_POSTGRES_IMAGE` from its
`compose.env` in your shell, or pass that file as an additional Compose `--env-file`.
Use **both** `compose.yaml` and `compose.database.yaml` for every overlay command.

1. Start `openbao`, `target-postgres` and `dev-idp`; the OpenBao healthcheck remains
   unhealthy until initialization/unseal. Run `python3 infra/openbao/bootstrap-development.py
   --directory "$PWD/.local/database"`. This local-only script stores one-share unseal
   material privately and issues separate **one-hour scoped tokens**, never root tokens
   for applications. Rerun it to unseal after restart and refresh expiring fixture tokens.
2. Recreate the control plane and engine with the overlay. Register the subject
   `database-runner` with role `database-runner` and the binding described above, then
   start the `database` profile. Engine gRPC and the target use verified TLS. Docker
   networks separate the runner from the platform database and OpenBao.
3. In Studio → Connections create `orders-db`: host `target-postgres`, port `5432`,
   database `postgres`, TLS verify-full with the generated `tls/ca.crt`. Create an OpenBao
   secret using the private `credential.json`. Save, activate and run **Test connection**.
4. Execute `SELECT id, status FROM demo_orders WHERE id = $1`, binding integer `1`.
   Follow the retained run and explicitly open its authorized result. Reuse `orders-db`
   in a visual workflow or one of the five SDK builders.
5. Pause the connection: admission must fail. Reactivate it, seal/stop OpenBao: credential
   resolution must fail without a local fallback. Restore/unseal it and start a new run.
   No failure scenario should display a password, token or raw provider diagnostic.

The fixture CA, local IdP and one-share unseal file are development material. Production
uses the workload-auth profile above. Existing databases are registered; this feature
does not provision customer schemas or generate production database accounts.

## Reproducible checks

- Build Dockerfile targets `control-plane`, `execution-engine`, `dev-idp` and
  `database-runner` with their Compose image tags, then run
  `python3 tests/e2e/database-connections/compose.py` for the actual container happy/outage demo.

- `cargo test -p sagat-tests --test contract_scalability_ha --locked` checks rendered
  identities, token isolation, resource limits, rollout and fail-closed network/TLS inputs.
- `tests/e2e/database-connections/kubernetes.sh` creates/deletes its own three-node kind
  cluster with pinned Calico, OpenBao and verified TLS. It exercises projected JWT login,
  renewal/re-authentication, wrong-audience/SA refusal and actual allowed/denied sockets.
  Probe containers replace application binaries **only in that network/auth fixture**.
- The separate `integration_database_runner` and Studio database browser suites exercise
  actual SQL, credential capabilities, durable admission/results and happy/failure UI paths.
  See [verification evidence](../../specs/017-database-connections/verification.md).

Sources: [OpenBao HA TLS](https://openbao.org/docs/platform/k8s/helm/examples/ha-tls/),
[Kubernetes auth](https://openbao.org/docs/auth/kubernetes/),
[Agent auto-auth](https://openbao.org/docs/agent-and-proxy/autoauth/),
[declarative audit](https://openbao.org/docs/configuration/audit/),
[2.7 release notes](https://openbao.org/community/release-notes/2-7-0/).
