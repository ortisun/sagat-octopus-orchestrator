<p align="center">
  <img src="docs/hero.png" alt="Sagat Octopus workflow orchestrator" width="600">
</p>

# Sagat Octopus Orchestrator

A self-hosted platform for durable workflows, sagas and event-driven automation. Build workflows in
Studio or with TypeScript, Go, Python, C# and Java SDKs; run activities in your workers and inline
scripts in isolated containers. Inspect execution history, retries and compensations in the same UI.

PostgreSQL is the source of truth. NATS JetStream transports notifications; Kafka, NATS and SQS
connectors integrate external systems. External effects are **at-least-once**: use idempotency keys
and reconciliation for operations such as payments.

**License:** [MIT](LICENSE). **Implementation and validation status:**
[requirement traceability](docs/traceability.md). This README documents the source checkout; it does
not assume that application images, CLI binaries or SDK packages have been published to a registry.
Studio labels and project documentation are in English.

## Contents

- [Install and start with Docker](#install-and-start-with-docker)
- [Access ports](#access-ports)
- [Configure the platform](#configure-the-platform)
- [Run your first workflow](#run-your-first-workflow)
- [Configure integrations](#configure-integrations)
- [Write and run scripts](#write-and-run-scripts)
- [Develop without application containers](#develop-without-application-containers)
- [Deploy to Kubernetes](#deploy-to-kubernetes)
- [Validate an installation](#validate-an-installation)
- [Troubleshooting](#troubleshooting)
- [Repository and operational scripts](#repository-and-operational-scripts)
- [Architecture, limits and contributing](#architecture-limits-and-contributing)

## Install and start with Docker

### Prerequisites

| What you want to run | Required tools |
|---|---|
| Local platform and Studio | Git, Docker Engine/Desktop with Compose v2 and BuildKit |
| `sagat` CLI or native backend | Rust via rustup; the repository pins the version in [rust-toolchain.toml](rust-toolchain.toml) |
| Frontend development | Node.js 20.19+ and npm; use `npm ci` with the committed lockfile |
| SDK examples | The selected language's toolchain; see [SDK quickstarts](docs/quickstarts/README.md#6-serve-it-with-an-sdk) |
| Kubernetes | kubectl, Helm, a compatible cluster, a NetworkPolicy-capable CNI and prepared dependencies |

Docker must have enough memory and disk for Rust builds and database/broker images. The disposable
Kubernetes test requires at least **4 CPUs, 5.5 GiB RAM and about 15 GiB free disk**. These are test
harness prerequisites, not production sizing recommendations.

Run the following commands from the repository root, in a POSIX shell:

```sh
git clone https://github.com/ortisun/sagat-octopus-orchestrator.git
cd sagat-octopus-orchestrator
cp infra/docker/.env.example infra/docker/.env
chmod 600 infra/docker/.env
```

Edit `infra/docker/.env` before starting. Replace `POSTGRES_PASSWORD` and `SAGAT_APP_PASSWORD` with
different values. Hex passwords from `openssl rand -hex 24` avoid connection-URL escaping issues.
Do not commit `.env`, token files or keys. Do not overwrite an existing local `.env` when updating.

```sh
docker info
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env config --quiet
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
```

This builds and starts PostgreSQL, NATS, the REST API/Studio, the execution engine and the development
identity provider. The `migrate` service applies the schema and exits successfully. The seed creates
**tenant `dev`**, **administrator `dev-admin`**, **workspace `dev`** and **namespace `payments`**.
Migrations and the development seed are safe to rerun.

The base stack does **not** start a sandbox runner, an SDK worker, Kafka, SQS or observability tools.
Those are configured separately below. The development identity provider issues tokens without a
user login; keep this stack on loopback and use your own OIDC provider for shared environments.

### Install the CLI and sign in

```sh
cargo install --path apps/cli --locked
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
mkdir -p target/local
sagat dev token --out target/local/admin.token
sagat --token-file target/local/admin.token doctor
```

Open **http://127.0.0.1:18080/studio/** and paste the token file's contents into **Access token**.
The token lasts one hour; run `sagat dev token --out target/local/admin.token` again to renew it.
Studio holds the token in memory, so a page refresh can require signing in again.

In Studio, **Namespaces** lists and creates namespaces; the sidebar selector changes the active
namespace. **Workflows**, **Runs**, **Scripts**, **Integrations** and **Operations** operate in that context.
**Operations** shows alerts and worker sessions; authorized operators can drain or resume a worker identity with an audited reason.
On an unfinished run, authorized users can **Send signal** and explicitly confirm its JSON payload.

### Inspect, stop and restart

```sh
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env ps -a
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env logs --tail=100 control-plane execution-engine
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env stop
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env start
```

Use `up -d --build` after source changes, or `up -d` after environment changes, so containers are
recreated as needed. `restart` alone does not apply changed environment variables. `down` removes
containers and preserves named volumes. **`down -v` deletes the database, broker data and development
identity signing key**; it is not a troubleshooting step for an environment you need to keep.

## Access ports

Defaults from [Compose](infra/docker/compose.yaml), its [observability overlay](infra/docker/compose.observability.yaml)
and [Helm values](infra/helm/values.yaml):

| Service | Local host address/port | Container port | Configuration |
|---|---|---|---|
| Studio | `http://127.0.0.1:18080/studio/` | 8080 | `SAGAT_API_PORT` |
| REST API | `http://127.0.0.1:18080/v1` | 8080 | `SAGAT_API_PORT` |
| Worker gRPC | `http://127.0.0.1:19000` | 9000 | `SAGAT_GRPC_PORT` |
| Development IdP | `http://127.0.0.1:18090` | 9100 | `SAGAT_IDP_PORT` |
| PostgreSQL | `127.0.0.1:15432` | 5432 | `SAGAT_PG_PORT` |
| Internal NATS | `127.0.0.1:14222` | 4222 | `SAGAT_NATS_PORT` |
| NATS monitoring | `http://127.0.0.1:18222/healthz` | 8222 | `SAGAT_NATS_MONITOR_PORT` |
| Grafana, optional | `http://127.0.0.1:13000` | 3000 | `SAGAT_GRAFANA_PORT` |
| Prometheus, optional | `http://127.0.0.1:19091` | 9090 | `SAGAT_PROMETHEUS_PORT` |
| Jaeger, optional | `http://127.0.0.1:16686` | 16686 | `SAGAT_JAEGER_PORT` |
| OTLP collector, optional | Not published on the host by the overlay | 4318 | Internal `http://otel-collector:4318` |
| Process metrics | Not published on the host by Compose | API 9090; engine 9091; runner 9092 | `SAGAT_ADMIN_ADDR` per process |

Compose publishes application ports on `127.0.0.1`. Inside its network, use service names such as
`postgres:5432`, `control-plane:8080` and `execution-engine:9000`, not host ports or `localhost`.
Helm exposes ClusterIP services on 8080 and 9000 by default; external URLs depend on your ingress,
TLS and port-forward configuration.

## Configure the platform

The local source of configuration is [infra/docker/.env.example](infra/docker/.env.example).
Native binaries read environment variables; Kubernetes uses [infra/helm/values.yaml](infra/helm/values.yaml)
and existing Kubernetes Secrets. Compose passes the variables declared in its YAML: adding an
arbitrary variable to `.env` does not automatically inject it into a container.

| Concern | Configuration | What to check |
|---|---|---|
| Database | `DATABASE_URL` | Migrations use the owner; API/engine use the least-privilege `sagat_app` role |
| Identity | `SAGAT_OIDC_ISSUER`, `SAGAT_OIDC_AUDIENCE`, `SAGAT_OIDC_JWKS_URI`, `SAGAT_OIDC_TENANT_CLAIM` | API and engine must trust the same provider and tenant claim |
| Development mode | `SAGAT_ENV=dev` | Required by `seed-dev` and `dev-idp`; never use the development issuer in production |
| Secret encryption | `SAGAT_SECRETS_KEYS` | Same persistent key ring on API and engine; format `key-id:base64` |
| Script artifacts | `SAGAT_SCRIPT_SIGNING_KEY` | Persistent Ed25519 signing key on every control-plane replica |
| Trace links | `SAGAT_STUDIO_TRACE_URL` | Public URL template, e.g. `http://127.0.0.1:16686/trace/{traceId}`; HTTPS required outside loopback |
| Studio files | `SAGAT_STUDIO_DIR` | `/srv/studio` in the image; an absolute path to `frontend/apps/studio/dist` natively |
| Internal broker | `SAGAT_NATS_URL`, `SAGAT_NATS_REQUIRE_TLS`, `SAGAT_NATS_CREDENTIALS_FILE` | Native/deployment configuration; Compose uses its local `nats` service |
| Telemetry | `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`, `SAGAT_LOG_FORMAT` | Point each process at the collector it can reach |
| Sandbox | `SAGAT_RUNNER_*`, container engine access | Separate runner identity, token, namespace bindings and language images |

### Enable secret storage and script authoring

After building the images, generate keys:

```sh
openssl rand -base64 32
docker run --rm sagat-octopus-orchestrator/control-plane:dev script-key
```

Put the first output in `.env` as `SAGAT_SECRETS_KEYS=local:<base64-value>` and the second as
`SAGAT_SCRIPT_SIGNING_KEY=<signing-key>`. Then recreate the application containers:

```sh
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d control-plane execution-engine
```

Without these settings the corresponding secret/script routes return **404**. Keep the keys across
restarts and replicas: losing the encryption key makes stored secrets unreadable. Signing-key
replacement needs a deliberate migration; it does not automatically re-sign old artifacts.
Outside development, protect REST and gRPC with TLS. The `*_ALLOW_PLAINTEXT_SECRETS` flags describe
an explicitly protected TLS-termination setup; they do not enable encryption.

### Add observability

Set `GRAFANA_ADMIN_PASSWORD` in `.env`, then use both Compose files:

```sh
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.observability.yaml \
  --env-file infra/docker/.env up -d --build
```

Sign in to Grafana as `admin` with the configured password; the overlay defaults to `admin` if none
is supplied. The provisioned dashboard uses Prometheus; traces go through the collector to Jaeger.
Use the same file pair for subsequent `ps`, `logs`, `up` and `down` operations on this stack.
See [observability](docs/runbooks/observability.md) and [alerts](docs/runbooks/alerts.md).

## Run your first workflow

The CLI examples below use a token file. All paths are relative to the checkout root.

```sh
sagat new hello-world target/local/hello-world
sagat --token-file target/local/admin.token workflow validate target/local/hello-world/workflow.json
sagat --token-file target/local/admin.token workflow publish target/local/hello-world/workflow.json
```

In a **second terminal**, from the same checkout, start the development worker and leave it running:

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
sagat --token-file target/local/admin.token worker dev \
  --mocks target/local/hello-world/mocks.yaml \
  --workflow target/local/hello-world/workflow.json --bind
```

Back in the first terminal:

```sh
sagat --token-file target/local/admin.token workflow run hello-world --input '{"name":"Ada"}' --wait
```

Expected result: **SUCCEEDED**, with two completed activities and a persisted journal. Inspect it in
**Runs**, or with `sagat --token-file target/local/admin.token workflow inspect <run-id>`.
The development worker uses canned answers but the actual engine and worker protocol. `--bind`
replaces that identity's activity bindings in the namespace; use separate worker identities for
independent applications. After token expiry, renew it and restart this CLI worker.

Next, try the [payment saga](docs/quickstarts/saga.md) to observe compensation, then replace the mock
worker with a [TypeScript](docs/quickstarts/typescript.md), [Go](docs/quickstarts/go.md),
[Python](docs/quickstarts/python.md), [C#](docs/quickstarts/dotnet.md) or [Java](docs/quickstarts/java.md) worker.

## Configure integrations

Open **Integrations** in Studio for the selected namespace:

1. An administrator opens **Network access**, preserves existing rules and authorizes the broker's
   hosts/networks and ports. This policy applies to all outbound connections in the namespace.
2. Choose **New integration**, a unique name and **Kafka**, **NATS JetStream** or **Amazon SQS**.
3. Fill in the type-specific settings and authentication. Save initially **Paused**.
4. Reopen **Edit** and choose **Active** when ready to consume messages. Configure a workflow
   `message` trigger using the integration name to start runs from those messages.

| Type | Connection settings | Supported credentials |
|---|---|---|
| Kafka | Servers, topic, initial offset for partitions without a checkpoint | SASL PLAIN, SCRAM-SHA-256 or SCRAM-SHA-512 |
| NATS JetStream | Servers, stream, durable consumer, optional subject filter | Token, username/password or complete `.creds` contents |
| SQS | Queue URL and AWS region | Access key ID, secret access key, optional session token |

You can also reference an existing namespace secret. New credentials require `secret:manage`;
connector editing requires `event:manage`; network policy editing requires `egress:manage`.
Credentials are encrypted on the server and are not shown again. Use `tls://host:port` when entering
broker credentials; never embed them in URLs. Private addresses require an explicit CIDR rule, and
server network restrictions still apply.

Secret creation and connector creation are separate writes. If the second fails, Studio retains the
saved secret reference for retry; it does not delete an unused secret automatically. **Active is a
configuration state, not proof of connectivity.** Check runtime errors and message receipt. After
credential rotation, pause and resume the connector so its reader reloads the credentials.

This menu manages **inbound connectors**. Outbound broker subscriptions use the separate API
explained in [the events runbook](docs/runbooks/events.md). The platform's **internal NATS** is a
deployment setting, separate from customer integrations. Custom broker CA/client-certificate upload
is not part of the current form or adapter contract.

## Write and run scripts

### Script structure

Keep the workflow definition, script source and activity mocks separate:

```text
my-workflow/
├── workflow.json     # Declarative graph; script nodes pin an artifact digest
├── enrich.ts         # Source that the sandbox builds and executes
└── mocks.yaml        # Development answers for ordinary worker activities
```

A TypeScript script accepts JSON-compatible input and returns JSON-compatible output:

```ts
export default function enrich(input: { orderId: string; items: number }) {
  return {
    orderId: input.orderId,
    items: input.items,
    express: input.items <= 3,
  };
}
```

The [inline-script template](templates/inline-script) contains this complete example. The control
plane builds immutable, signed artifacts; workflow nodes reference `artifactDigest`, not a mutable
source filename. A published workflow version is immutable.

| Language | Runtime/build contract |
|---|---|
| TypeScript / Python | Platform-provided function/shim contract; no arbitrary package installation |
| C# | Isolated Roslyn build and separate runtime; reviewed offline dependency catalog |
| Go | One `package main` source defining `Run(input any, client *octopus.Client) (any, error)`; native target required |
| Java | One public `Script.java` implementing `sagat.inline.InlineScript`; Java 17 class artifacts |

See [script operations](docs/runbooks/scripts.md), [C# runner](runners/dotnet/README.md),
[Go runner](runners/go/README.md) and [Java runner](runners/java/README.md) for exact interfaces and
supported imports. CLI `script test/publish` accepts TypeScript and Python; use Studio/the script
API for the other language workflows. Five SDK languages do not imply arbitrary inline dependencies.

### Start the sandbox runner

The signing key alone enables authoring, not execution. To execute scripts:

1. Create a membership with role **`runner`** in the tenant and issue its token. Use the development
   IdP only locally; see [runner identity setup](docs/runbooks/scripts.md#2-setting-it-up).
2. As an administrator, bind that subject through
   `PUT /v1/namespaces/{namespace-id}/worker-bindings/{runner-subject}` with
   `{"activities":["script.*"]}`.
3. Save its token to `infra/docker/runner.token`; this must be a file, not a directory.
4. Set `SAGAT_RUNNER_NAMESPACES=payments=<namespace-id>` and `SAGAT_DOCKER_GID` in `.env`.
   Use the socket's group ID on the **Docker daemon host**, not an unrelated host socket.
5. Pull/build the language images on the same Docker engine used by the runner. For TypeScript and Python:

```sh
docker pull node:24-alpine
docker pull python:3.13-alpine
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env \
  --profile sandbox up -d --build sandbox-runner
```

The runner starts containers through the Docker socket. Give it a dedicated host/VM; do not grant
that socket to the API or scheduler. It must have no database credentials or private signing key.
For Go, configure a homogeneous native target (`linux-arm64` or `linux-amd64`) and its builder/runtime
images. Image names, probes, resource ceilings and token rotation are in the [script runbook](docs/runbooks/scripts.md).

### Test, publish and use a script

With a healthy runner:

```sh
sagat --token-file target/local/admin.token script test templates/inline-script/enrich.ts \
  --input '{"orderId":"o-1","items":2}'
sagat --token-file target/local/admin.token script publish templates/inline-script/enrich.ts
```

Expected test result: `express: true`. Publishing prints an `artifactDigest`. Copy that digest into
the script node of your workflow before validating and publishing the workflow. The template's
all-zero digest is deliberately a placeholder and will be rejected. Ordinary activity nodes still
need a worker; script nodes need the runner. Script execution has no direct network or unrestricted
secret access; mediated capabilities are governed by namespace policy and the script ceiling.

## Develop without application containers

You can run PostgreSQL/NATS in Docker and compile the application locally. This is a separate startup
mode; do not bind native processes to ports already occupied by application containers.

```sh
rustup show
cargo build --locked -p sagat-control-plane -p sagat-execution-engine -p sagat-sandbox-runner -p sagat-dev-idp -p sagat-cli
npm --prefix frontend ci
npm --prefix frontend run build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d postgres nats
```

Run `target/debug/control-plane help`, `target/debug/execution-engine help` and
`target/debug/dev-idp help` for the full variable reference. Configure native services with host
addresses: PostgreSQL `127.0.0.1:15432`, NATS `127.0.0.1:14222`, REST `127.0.0.1:18080`,
gRPC `127.0.0.1:19000`, and development JWKS at your native IdP's bound address. The token issuer
claim must exactly match `SAGAT_OIDC_ISSUER`, including any trailing slash.

Start in this order: owner `control-plane migrate` → owner `control-plane seed-dev` in development →
IdP → `control-plane serve` and `execution-engine serve` with the runtime database role → worker/runner.
Set `SAGAT_STUDIO_DIR` to the absolute build directory. Full settings are in the
[control-plane](docs/runbooks/control-plane.md), [engine](docs/runbooks/execution-engine.md) and
[Studio](docs/runbooks/studio.md) runbooks.

For frontend hot reload with an API already running:

```sh
SAGAT_API_URL=http://127.0.0.1:18080 npm --prefix frontend run dev
```

Vite uses port 5173 and proxies `/v1` to that API. A frontend development server does not start the
backend, database, worker or runner.

## Deploy to Kubernetes

### What the chart installs

[infra/helm](infra/helm) installs the control plane/Studio, execution engine and optional SDK worker
pools, with services, disruption budgets and NetworkPolicies. PostgreSQL, NATS, OIDC, certificates,
Prometheus, KEDA and the sandbox runner's container-engine infrastructure are **not installed by the
chart**. Database migrations are also an explicit operator step.

Use [values.yaml](infra/helm/values.yaml) as the field reference and
[examples/ha.yaml](infra/helm/examples/ha.yaml) as a starting configuration, not a ready-to-use
production file. The chart declares Kubernetes **1.27+**. If enabling KEDA, install its CRDs/operator
and provide the reachable Prometheus address first; HPA CPU scaling requires metrics-server.

### Prepare images, dependencies and secrets

Build and push images to a registry accessible to the cluster. Replace the example registry/tag:

```sh
export SAGAT_IMAGE_PREFIX=registry.example.com/team
export SAGAT_IMAGE_TAG=your-reviewed-version
docker build -f infra/docker/Dockerfile --target control-plane \
  -t "$SAGAT_IMAGE_PREFIX/sagat-octopus-orchestrator/control-plane:$SAGAT_IMAGE_TAG" .
docker build -f infra/docker/Dockerfile --target execution-engine \
  -t "$SAGAT_IMAGE_PREFIX/sagat-octopus-orchestrator/execution-engine:$SAGAT_IMAGE_TAG" .
docker push "$SAGAT_IMAGE_PREFIX/sagat-octopus-orchestrator/control-plane:$SAGAT_IMAGE_TAG"
docker push "$SAGAT_IMAGE_PREFIX/sagat-octopus-orchestrator/execution-engine:$SAGAT_IMAGE_TAG"
```

Before installing, prepare the target namespace and the referenced Secrets using your secret manager
or controlled manifests. Do not put real secrets in a committed values file. For example, after
preparing files containing only the appropriate database URLs:

```sh
kubectl create namespace sagat
kubectl -n sagat create secret generic sagat-db \
  --from-file=DATABASE_URL=/secure/path/runtime-database-url
kubectl -n sagat create secret generic sagat-db-owner \
  --from-file=DATABASE_URL=/secure/path/owner-database-url
```

Use `imagePullSecrets` through `image.pullSecrets` for private registries. PostgreSQL must already
have the owner/runtime roles and runtime grants; the development initialization script in
[infra/docker/postgres-init](infra/docker/postgres-init) illustrates that separation.

| Secret reference | Required key(s) | Purpose |
|---|---|---|
| `database.existingSecret` | `DATABASE_URL` | Runtime `sagat_app` database URL; provision the role and grants separately |
| Migration Job secret, operator-managed | `DATABASE_URL` | Database owner URL, used only for migrations |
| `secrets.existingSecret`, if enabled | `SAGAT_SECRETS_KEYS` | Shared API/engine encryption key ring |
| `nats.credentialsSecret`, if used | `nats.creds` | Internal NATS authentication |
| `engine.grpc.tlsSecret`, if native TLS | `tls.crt`, `tls.key` | Worker API TLS |
| `controlPlane.nativeTls.tlsSecret` and `.clientCaSecret`, if native TLS | `tls.crt`, `tls.key`; `ca.crt` | Public API TLS and verified webhook client identity |

The HA example names an ingress TLS-termination setup, an engine TLS Secret and a sample worker
image/token. Supply those dependencies or adapt/remove the corresponding settings. Configure
`oidc.*`, database URLs, image registry/tag, worker activity bindings and all network peers for your
actual installation. NetworkPolicies fail closed: DNS, PostgreSQL, OIDC and configured NATS peers
must be explicit. Global allow-all peers are refused. An application-level egress rule does not
replace Kubernetes network authorization.

### Migrate, render and install

Create an operator-owned, one-shot Job using the **same control-plane image tag** as the release,
with `args: ["migrate"]` and `DATABASE_URL` from the **owner** Secret. Wait for successful completion
before installing or upgrading engine replicas. The Job needs database egress and credentials, but
not a dev seed or runner token. Provision production tenant/membership records using the
[identity/governance procedures](docs/runbooks/security-governance.md); `seed-dev` is development-only.

A migration Job manifest can start with the following. Replace the image/tag, save it outside
source-controlled secrets and adapt resource/network settings to your cluster:

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: sagat-migrate-your-version
  namespace: sagat
spec:
  backoffLimit: 1
  template:
    spec:
      restartPolicy: Never
      securityContext:
        runAsNonRoot: true
        runAsUser: 10001
        seccompProfile: {type: RuntimeDefault}
      containers:
        - name: migrate
          image: registry.example.com/team/sagat-octopus-orchestrator/control-plane:your-reviewed-version
          args: ["migrate"]
          env:
            - {name: SAGAT_ENV, value: prod}
            - name: DATABASE_URL
              valueFrom:
                secretKeyRef: {name: sagat-db-owner, key: DATABASE_URL}
          securityContext:
            allowPrivilegeEscalation: false
            capabilities: {drop: [ALL]}
```

```sh
kubectl apply -f /secure/path/sagat-migrate.yaml
kubectl -n sagat wait --for=condition=complete job/sagat-migrate-your-version --timeout=10m
kubectl -n sagat logs job/sagat-migrate-your-version
```

Use a new Job name for each upgrade; Jobs do not rerun just because the image tag changed in a local
file. If migration fails, inspect the error before proceeding. Do not print database credentials.

After preparing the environment-specific values file and Kubernetes namespace:

```sh
# Run against the intended context; inspect it before making changes.
kubectl config current-context
helm lint infra/helm -f /secure/path/sagat-values.yaml
helm template sagat infra/helm --namespace sagat \
  -f /secure/path/sagat-values.yaml > /tmp/sagat-rendered.yaml
kubectl apply --dry-run=server -f /tmp/sagat-rendered.yaml
helm upgrade --install sagat infra/helm --namespace sagat \
  -f /secure/path/sagat-values.yaml --wait --timeout 10m
```

Set `image.registry` to your registry prefix **with a trailing slash** (for example,
`registry.example.com/team/`) and `image.tag` to the reviewed tag in that values file.
The chart's default resource prefix is **`sagat-octopus-orchestrator`**, not the Helm release name
`sagat`; `fullnameOverride` changes it. With the default prefix:

```sh
kubectl -n sagat get deployments,pods,services,jobs
kubectl -n sagat rollout status deployment/sagat-octopus-orchestrator-control-plane
kubectl -n sagat rollout status deployment/sagat-octopus-orchestrator-engine
kubectl -n sagat logs deployment/sagat-octopus-orchestrator-engine --tail=100
kubectl -n sagat port-forward service/sagat-octopus-orchestrator-control-plane 18080:8080
```

With ingress-terminated TLS and a plaintext in-cluster API, the last command exposes Studio locally
at `http://127.0.0.1:18080/studio/`. With native API TLS, use HTTPS and a trusted certificate/server
name instead; do not disable certificate verification to make the check pass. A real OIDC token is
required: this chart does not install the local development identity provider.

Script support additionally needs a signing-key injection and an isolated runner deployment. The
current chart does not provide an end-to-end sandbox installation; follow the [runner runbook](docs/runbooks/scripts.md)
and keep its Docker socket off ordinary application pods. Upgrade sequencing, database connection
budgets and scaling are in [scaling.md](docs/runbooks/scaling.md); backups and restore are in
[disaster-recovery.md](docs/runbooks/disaster-recovery.md).

### Reproduce the disposable cluster validation

For a development verification that provisions its own kind/Cilium/KEDA environment:

```sh
G247_DOCKER_CONTEXT=your-disposable-docker-context \
  tests/e2e/scalability-ha/kubernetes/run.sh
```

This creates a uniquely named cluster, runs real workflows and worker autoscaling/recovery checks,
and deletes its own cluster on success. It uses a task-local kubeconfig. It is a validation harness,
not a production installer. Prerequisites, retained-cluster diagnosis and evidence are in
[kubernetes-validation.md](docs/runbooks/kubernetes-validation.md).

## Validate an installation

### Basic health

```sh
curl -fsS http://127.0.0.1:18080/v1/health
curl -fsS http://127.0.0.1:18080/v1/ready
curl -fsS http://127.0.0.1:18222/healthz
sagat --token-file target/local/admin.token doctor
```

`/health` proves the API process answers; `/ready` also checks storage. Neither proves a worker can
complete an activity. Run the hello-world above and check **SUCCEEDED**, then the payment saga for
compensation, and a script test if sandbox execution is enabled. Inspect the journal and trace ID
when diagnosing failures; health endpoints alone are not an end-to-end test.

### Source and contract checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/check-architecture.py
npm --prefix frontend ci
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend run check:api
npm --prefix frontend test
npm --prefix frontend run build
```

The Rust integration suite uses real Docker services. Opt-in/ignored suites are not evidence of a
pass unless explicitly executed. For the real API/engine/browser journey, install Chromium from
`frontend/` with `npx playwright install chromium`, then run from the checkout root:

```sh
cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

The default browser harness requires the sandbox. `SAGAT_STUDIO_NO_SANDBOX=1` is an explicitly partial
mode that excludes the script journey; it does not validate script execution.

The isolated Compose smoke test builds a separate project and removes **its own** volumes:

```sh
scripts/smoke-compose.sh
```

It still needs available host ports. Stop a local stack first or assign unused ports through its
`ENV_FILE`/exported settings. See [test strategy](docs/test-strategy.md),
[SDK conformance](docs/runbooks/sdks.md) and the per-feature `quickstart.md` files for security,
crash/recovery, broker, performance and disaster-recovery suites.

## Troubleshooting

Start with `sagat doctor`, then the affected service's logs. Record the error **code**, **trace ID**,
run ID and timestamp; redact tokens, credentials and payloads before sharing diagnostics.

| Symptom | Check | Resolution |
|---|---|---|
| Docker cannot connect | `docker context ls`, `docker info`, `DOCKER_HOST` | Start/select the intended engine. On Colima, use `docker --context <name> …`; native runners and Testcontainers also need the matching socket in `DOCKER_HOST` |
| Bind mounts are empty on Colima/remote Docker | Repository sharing on the daemon's VM/host | Share the checkout before starting Compose. Mount paths resolve on the daemon host, not necessarily your terminal's filesystem |
| `sagat_app` missing or password authentication fails | Postgres initialization logs, runtime role and existing volume | Init scripts run only on a new database volume. Repair the role/grants or rotate the existing password deliberately; changing `.env` does not rewrite database users. Preserve existing data |
| Address already in use | Port table and running containers/native processes | Change the matching `SAGAT_*_PORT` and recreate the container, or stop the conflicting process |
| API healthy but not ready | `/v1/ready`, migration and database logs | Complete migrations; correct database connectivity and runtime permissions |
| `401` or Studio returns to sign-in | Token expiry, issuer, audience, tenant claim, JWKS access | Renew local tokens; align OIDC configuration. Refreshing Studio discards its in-memory session |
| `403` or missing creation action | Role/permissions and selected namespace | Use an appropriately authorized identity; check namespace membership and policy instead of bypassing authorization |
| Runs remain READY/RUNNING | Engine logs, worker process, activity binding | Start the correct worker, register its binding and confirm gRPC connectivity. A published workflow does not create a worker |
| Scripts return `404` | `SAGAT_SCRIPT_SIGNING_KEY` and restarted/recreated API | Configure the persistent signing key; rebuild/recreate as necessary |
| Secrets return `404` | `SAGAT_SECRETS_KEYS` on API/engine | Configure the shared key ring and correct TLS settings |
| Script stays QUEUED/BUILDING or `SCRIPT_SANDBOX_UNAVAILABLE` | Runner token, socket permissions, language images and bindings | Start the sandbox profile with a real token file and accessible container engine; inspect runner logs |
| `SCRIPT_UNTRUSTED_ARTIFACT` | Digest/signature and trusted public key | Investigate integrity/key mismatch; do not disable signature checks |
| Integration cannot be saved or is DENIED | Namespace network policy and broker address | Explicitly authorize host/CIDR and port; use TLS for credentials. Kubernetes egress must also permit the connection |
| Active integration receives nothing | Broker reachability, credentials, topic/stream/consumer and runtime health | Active is not a connection test. Inspect the broker and connector; pause/resume after rotating credentials |
| `429` / `QUOTA_EXCEEDED` | Typed error, quota resource and queued work | Resolve stalled workers/runners or adjust the authorized quota; avoid blind retries |
| `STORAGE_QUOTA_EXCEEDED` or truncated script logs | Content/log capacity and retention settings | Review retained data and quota/retention policy; do not remove execution history blindly |
| Studio displays an old screen or cannot load a chunk | Rebuilt assets, served directory and cached open tab | Finish the build, then reload and sign in. Containerized Studio needs a rebuilt image; native Studio serves its configured `dist` directory |
| Helm refuses to render | Required Secrets/OIDC values and explicit network peers | Complete environment-specific values; do not turn off policy just to silence validation |
| Kubernetes `CrashLoopBackOff` / `ImagePullBackOff` | `kubectl describe pod`, previous logs, image tag, Secrets | Fix the reported dependency or configuration; check migrations before restarting everything |
| KEDA does not scale | ScaledObject conditions, Prometheus URL and queue metrics | Use a namespace-qualified Prometheus address. Downscale stabilization can take five minutes |

Useful Kubernetes diagnostics:

```sh
kubectl -n sagat get events --sort-by=.lastTimestamp
kubectl -n sagat describe deployment/sagat-octopus-orchestrator-engine
kubectl -n sagat logs deployment/sagat-octopus-orchestrator-engine --previous --tail=100
```

Further procedures: [control plane](docs/runbooks/control-plane.md),
[execution engine](docs/runbooks/execution-engine.md), [scripts](docs/runbooks/scripts.md),
[security/governance](docs/runbooks/security-governance.md), [events](docs/runbooks/events.md),
[Studio](docs/runbooks/studio.md), [alerts](docs/runbooks/alerts.md).

## Repository and operational scripts

```text
apps/                         Rust binaries: control-plane, execution-engine, sandbox-runner, dev-idp, CLI
crates/                       Domain, application services, adapters, scheduling and telemetry
frontend/apps/studio/          React/TypeScript application
frontend/e2e/                 Real-browser Playwright journeys
sdks/                         TypeScript, Go, Python, C# and Java worker/client SDKs
runners/                      Trusted language shims, builders and sandbox runtime images
templates/                    Starter workflows, script source and development mocks
examples/                     SDK examples and workflow documents
tests/                        Unit-support, integration, contract, E2E, security, fault and performance suites
infra/docker/                 Development Compose, Dockerfile, PostgreSQL initialization
infra/helm/                   Kubernetes chart and deployment values
infra/observability/           Collector, Prometheus, Grafana and alert configuration
infra/dr/                     Disaster-recovery drill and journal verification
scripts/                      Repository validation and SDK packaging tools
specs/001..012/                Feature specifications, plans, tasks, contracts and evidence
docs/                         Architecture, ADRs, references, quickstarts and operational runbooks
.specify/memory/constitution.md Architecture and delivery principles
```

| Script | Purpose and side effects |
|---|---|
| `scripts/check-architecture.py` | Checks crate dependency/layer rules; uses Cargo metadata |
| `scripts/smoke-compose.sh` | Builds and tests an isolated Compose project; removes only its own project/volumes |
| `scripts/package-sdks.sh` | Builds SDK distribution artifacts into `target/sdk-packages` by default; requires all packaging toolchains and an empty output directory; does not publish packages |
| `tests/e2e/scalability-ha/kubernetes/run.sh` | Creates a disposable kind cluster and checks product/KEDA behavior |
| `tests/security/sandbox/kubernetes-network-policy.sh` | Verifies policy/admission using an explicit target kubeconfig and temporary test namespaces |
| `tests/performance/scalability-ha/run.sh` | Opt-in load harness; inspect requirements before reserving resources |
| `infra/dr/drill.sh` | Opt-in PostgreSQL backup/restore/failover drill |

For SDK packaging and release prerequisites, see [sdk-packaging.md](docs/runbooks/sdk-packaging.md).
For API routes, workflow fields and CLI syntax, use [API reference](docs/reference/api.md),
[workflow document reference](docs/reference/workflow-document.md) and [CLI reference](docs/reference/cli.md).

## Architecture, limits and contributing

State transitions, next-task scheduling and outbox writes obey documented transactional boundaries.
Workers use leases and fencing; paused/waiting workflows do not occupy a worker thread. Sagas ask
external providers to compensate effects; they cannot promise that every effect is reversible.
The journal is execution evidence; telemetry is diagnostic evidence derived from it.

Scripts run outside the control plane/scheduler under quotas and isolation. The runner's container
engine is a privileged boundary and belongs on dedicated infrastructure. Published versions do not
mutate in place, and running workflows are not automatically migrated. Throughput numbers are
measured workloads, not universal capacity guarantees.

Current limitations and evidence belong in [traceability](docs/traceability.md), each feature's
`research.md`/`quickstart.md`, and [performance reports](tests/performance/reports). In particular,
a passing disposable cluster test does not establish production capacity or disaster recovery for
your deployment. Validate backups, broker access, OIDC, worker behavior and network policy in your
own environment before rollout.

Start with [architecture](docs/architecture/overview.md),
[execution semantics](docs/architecture/execution-semantics.md), [ADRs](docs/adr) and
[AGENTS.md](AGENTS.md). Contributions use **Gitflow** and **Conventional Commits**: work on feature or
bugfix branches from `develop`, never commit directly to `main`/`develop`, and include real validation
evidence. Read the [constitution](.specify/memory/constitution.md) and the active feature's spec, plan
and tasks before implementation. Spec Kit resolves the active feature through `.specify/feature.json`
or `SPECIFY_FEATURE_DIRECTORY`; new features continue numbering from 013.
