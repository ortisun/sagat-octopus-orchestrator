# G247 Kubernetes deployment and KEDA validation report

**Run date:** 2026-10-11 23:57 to 2026-10-12 00:07 America/Sao_Paulo
**Result:** PASS
**Requirement:** G247 / T010-038 / FR-010-004

## Environment

| Component | Version or identity |
| --- | --- |
| Host | macOS arm64, Colima Docker endpoint with 4 CPUs and 5.77 GiB memory |
| kind | v0.33.0 |
| Kubernetes | v1.37.0, one arm64 control-plane node |
| Cilium | Helm chart/app 1.20.2 |
| KEDA | Helm chart/app 2.18.1 |
| Helm | v4.3.0 |
| Sagat chart | 0.4.0, app 0.3.0 |
| Source base commit | `ad6146c49b52bc1bfea24ce5623a53d3ff50c5d7` |

The images were built from an uncommitted feature worktree. The exact tested artifacts are recorded
below because the base commit alone does not identify that dirty source snapshot.

| Image | Image ID | Built (America/Sao_Paulo) |
| --- | --- | --- |
| control plane | `sha256:fae00b6f9803293006ee2170521a37fc1bcb72af9db29bb69c8d7f038f817d3e` | 2026-10-11 23:41:31 |
| execution engine | `sha256:b3b8389aa297ea487ec5ff4df6eb5edee933c14fa510d3872d6f99abdc26c11a` | 2026-10-11 23:38:38 |
| development IdP | `sha256:cb333f06ed4c0b585c3d1ac32e632e62d9d07be085ba4667df3de194aaae751d` | 2026-10-11 23:35:55 |
| Python SDK worker | `sha256:4b84d2a8bfc6d2227acacc6509532b628a485974d73322181ceb9400042aad7a` | 2026-10-11 23:42:01 |

PostgreSQL 16 Alpine and Prometheus 3.5.0 were pinned by manifest-list digest in the fixture.

## Command

The final clean run used cached images and retained its cluster long enough to collect evidence:

```bash
G247_SKIP_BUILD=1 G247_KEEP_CLUSTER=1 \
  tests/e2e/scalability-ha/kubernetes/run.sh
```

The separate repository policy verifier then ran against that same explicit disposable kubeconfig:

```bash
KUBECONFIG=<disposable-run-kubeconfig> CHART_DIR=infra/helm \
  tests/security/sandbox/kubernetes-network-policy.sh
```

## Results

| Check | Observed result |
| --- | --- |
| Migrations and seed | migration and seed Jobs completed |
| Product readiness | control plane 1/1; execution engine 1/1 |
| Authentication | development OIDC token accepted by the control plane and engine |
| Network isolation | undeclared pod timed out connecting to the API; `G247_UNDECLARED_API_DENIED=1` |
| Real workload | 60 API-created workflow runs; `G247_ENQUEUED=60` |
| Queue signal | `pool_ready_steps` peaked at **56** |
| KEDA scale-up | Python SDK worker Deployment increased from 1 to **3** replicas |
| Completion | all 60 runs succeeded; `G247_DRAINED=60` |
| Recovery | both KEDA metrics returned to 0 and the worker Deployment returned to **1** replica |
| Engine replacement | engine pod was deleted; its Deployment returned Available with a new pod |
| ScaledObject | `Ready=True`, `Active=False` after drain |
| Chart security verifier | restricted Pod Security, ingress isolation, and role-specific egress passed |

Final harness output:

```text
G247 Kubernetes validation passed: peak_ready=56 peak_worker_replicas=3 recovered_replicas=1
```

At final capture, the KEDA HPA reported both external metrics as zero, min/max replicas `1/3`, and one
current replica. The replacement engine pod was Ready with an age of 79 seconds while the Deployment
was 7 minutes 52 seconds old, confirming that the post-delete availability check observed a new pod.

## Defects found during the run

The validation exposed one product-chart configuration gap: the typed product configuration rejects
an HTTP JWKS endpoint unless `SAGAT_ALLOW_INSECURE_JWKS_HTTP=true`, but the chart could not express
that development-only setting. The chart now has `oidc.allowInsecureJwksHttp`, renders the environment
variable when enabled, and refuses the option outside `environment: dev`.

Harness defects found and corrected before the clean pass were restricted-volume initialization for
PostgreSQL, the dev-IdP image's persistent key default, Host validation during port-forwarding, the
worker binding/readiness order, and the namespace-qualified Prometheus address used by KEDA.

## Evidence limit

This run proves the artifact IDs above. Because concurrent feature work continued after those local
images were built, release acceptance still requires rebuilding from the final committed tree and a
smoke rerun. No pass is claimed for an image that was not exercised here.

The disposable cluster had one node. Engine pod replacement was exercised, but node drain and
cross-node rescheduling were outside this run; those require a separate multi-node disruption test.
