# Runbooks

How to operate each part of the platform: what to configure, what the alerts mean, how a failure shows and how to
recover. A runbook describes what is built; when it and the code disagree, one of them is fixed. To try the platform
first, see the [quickstarts](../quickstarts/README.md).

## The core platform

| Runbook | What it covers |
|---|---|
| [control-plane.md](control-plane.md) | The control plane: starting it, the first tenant, health, metrics, the audit trail, backup, keys (feature 001) |
| [execution-engine.md](execution-engine.md) | The execution engine and the scheduler (feature 002) |
| [sagas.md](sagas.md) | Sagas, compensations and reconciliation (features 003 and 037) |
| [events.md](events.md) | Events, webhooks and messaging, and the Redis Streams transport (features 004 and 033) |
| [scripts.md](scripts.md) | Scripts and the sandbox runner (feature 005) |
| [workflow-primitives.md](workflow-primitives.md) | Workflow primitives: operating and validating the current build (feature 014) |
| [api-rate-admission.md](api-rate-admission.md) | How each namespace's API calls are admitted against a durable rate limit |

## Schedules and run operations

| Runbook | What it covers |
|---|---|
| [schedules.md](schedules.md) | Schedules (feature 018) |
| [batch-operations.md](batch-operations.md) | Batch operations over a selection of runs (feature 018) |
| [business-ids.md](business-ids.md) | Business identifiers, start delay and the with-start calls (feature 018) |
| [run-reset.md](run-reset.md) | Resetting a run to a point (feature 018) |
| [instance-registry.md](instance-registry.md) | The instance-capability registry (feature 018, shared with features 029 and 031) |
| [run-input-validation.md](run-input-validation.md) | Checking every run input against its version's schema (design 043) |
| [flow-control.md](flow-control.md) | Concurrency keys and rate limits of runs and steps: activation, keys, waits, resets and the production gates (feature 022) |

## Data, secrets and databases

| Runbook | What it covers |
|---|---|
| [payload-classification.md](payload-classification.md) | Per-field payload classification and audited disclosure (feature 016) |
| [database-connections.md](database-connections.md) | Database connections and their OpenBao credentials (feature 017) |
| [database-runner.md](database-runner.md) | The managed PostgreSQL runner, and the Iceberg connection probe (features 017 and 035) |
| [openbao.md](openbao.md) | Deploying OpenBao and the managed database (feature 017) |
| [database-recovery.md](database-recovery.md) | Recovering the managed database and OpenBao |

## Studio, SDKs and developers

| Runbook | What it covers |
|---|---|
| [studio.md](studio.md) | The Studio (feature 007) |
| [sdks.md](sdks.md) | The SDKs (feature 006) |
| [sdk-packaging.md](sdk-packaging.md) | Building the six SDK packages and checking a clean install |
| [sdk-observability.md](sdk-observability.md) | The OpenTelemetry signals the SDKs emit, and what the application owns |
| [developer-experience.md](developer-experience.md) | The `sagat` command, the development identity provider, `sagat doctor`, and what is not built (feature 012) |
| [gitops.md](gitops.md) | GitOps: versions, approvals, promotion and rollback (feature 011) |

## Observability, security and scale

| Runbook | What it covers |
|---|---|
| [observability.md](observability.md) | Traces, metrics, the telemetry projection, operating a run and reading the audit trail (feature 008) |
| [alerts.md](alerts.md) | How alerts work, and what to do about each one (feature 008) |
| [security-governance.md](security-governance.md) | Security and governance controls (feature 009) |
| [scaling.md](scaling.md) | Scaling and high availability of the engine (feature 010) |
| [disaster-recovery.md](disaster-recovery.md) | Backup, point-in-time recovery, failover and the drill (feature 010) |
| [kubernetes-validation.md](kubernetes-validation.md) | Deploying into a disposable cluster and proving that KEDA scales the workers |

## Releases

| Runbook | What it covers |
|---|---|
| [release.md](release.md) | Cutting a release on GitHub Releases, the only distribution channel |
