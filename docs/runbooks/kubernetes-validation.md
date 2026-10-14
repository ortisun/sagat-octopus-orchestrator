# Kubernetes product and KEDA validation

This runbook deploys the real Sagat control plane, execution engine, PostgreSQL schema, development
identity provider, Python SDK workers, Prometheus, and KEDA into a disposable kind cluster. It then
creates workflow runs through the authenticated API and proves that KEDA scales workers from the
engine's real queue metrics and returns them to the configured minimum after the queue drains.

The harness creates a uniquely named cluster and a task-local kubeconfig. It never selects or deletes
an existing cluster. Run it only with a Docker context that has at least four CPUs, 5.5 GiB of memory,
and about 15 GiB of free disk.

## Run

From the repository root:

```bash
tests/e2e/scalability-ha/kubernetes/run.sh
```

The default Docker context is `colima-sagat-remaining`. Override it without changing the global
Docker or Kubernetes context:

```bash
G247_DOCKER_CONTEXT=my-disposable-context \
  tests/e2e/scalability-ha/kubernetes/run.sh
```

The run builds the three product images and a Python SDK worker with bounded Docker resources. If the
four exact `g247-local` tags already represent the source under test, skip that build explicitly:

```bash
G247_SKIP_BUILD=1 tests/e2e/scalability-ha/kubernetes/run.sh
```

On success the harness deletes its cluster and temporary kubeconfig. Preserve them for diagnosis with
`G247_KEEP_CLUSTER=1`; the final output gives both names. Delete that cluster explicitly after use:

```bash
DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}' colima-sagat-remaining)" \
  kind delete cluster --name <name-from-the-run>
```

Do not use a shared or production kubeconfig. The harness installs Cilium 1.20.2 and KEDA 2.18.1 and
loads local images into its own kind node.

## Evidence produced

The test fails unless all of these checks complete:

1. the namespace enforces the restricted Pod Security profile;
2. PostgreSQL initializes separate owner and runtime roles, migrations run, and development tenant,
   administrator, workspace, and namespace data are seeded;
3. a token from the development OIDC provider authenticates the real API;
4. the Helm control-plane and execution-engine Deployments become available with NetworkPolicies
   enabled;
5. an undeclared pod cannot connect to the control-plane Service;
6. the authenticated API binds the worker identity, publishes an activity workflow, and persists 60
   workflow runs;
7. Prometheus observes nonzero `pool_ready_steps`, and KEDA increases the real Python SDK worker
   Deployment from one to at least two replicas;
8. every run reaches `SUCCEEDED`, the queue metric returns to zero, and the worker Deployment returns
   to one replica after the HPA stabilization window;
9. deleting the execution-engine pod results in an available replacement; and
10. the ScaledObject reports `Ready=True`.

The success marker includes the observed maximums:

```text
G247 Kubernetes validation passed: peak_ready=<count> peak_worker_replicas=<count> recovered_replicas=1
```

The default HPA downscale stabilization window is five minutes, so a healthy run normally takes eight
to ten minutes after image and chart dependencies are cached.

## Additional policy verification

While the disposable cluster is still retained, run the broader chart security verifier against its
explicit kubeconfig:

```bash
KUBECONFIG=/absolute/path/from-the-run/kubeconfig \
CHART_DIR=infra/helm \
  tests/security/sandbox/kubernetes-network-policy.sh
```

It creates and removes only uniquely named test namespaces. Its success marker is:

```text
Kubernetes pod security, ingress isolation and role-specific egress checks passed
```

## Common failures

- `403` from the development identity provider means a port-forwarded request did not carry the
  provider's allowed `Host: dev-idp:9100` header.
- A worker registration error saying the identity is not bound means the enqueue/binding job did not
  complete. Inspect `job/g247-enqueue` before restarting workers.
- KEDA `no such host` events for Prometheus mean its address is not namespace-qualified. It must be
  `http://prometheus.sagat-g247.svc:9090` because KEDA runs in its own namespace.
- `ScaleDownStabilized` with zero metrics is expected for up to five minutes. Do not treat a nonzero
  replica count during that interval as a failed recovery.
- API TLS handshake timeouts from `kubectl` usually mean the Docker VM's CPUs are saturated. Stop
  unrelated builds rather than changing the global Kubernetes context.
