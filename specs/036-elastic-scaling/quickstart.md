# Quickstart: validating elastic scaling (slice 1)

How to prove the first delivery works end to end. Nothing here is implemented yet: the test targets and commands below are the ones the tasks create, named by the repository's `<kind>_<feature>` convention, and each scenario states the requirement and success criterion it closes. Contracts: [`contracts/`](./contracts/README.md). Model: [`data-model.md`](./data-model.md).

## Prerequisites

- Rust toolchain from `rust-toolchain.toml`; Docker (or a Colima context) for Testcontainers: PostgreSQL 16, Kafka, NATS, ElasticMQ (the SQS emulator).
- `helm` for the chart contract; `kind` and the Cilium and KEDA versions pinned by `tests/e2e/scalability-ha/kubernetes/run.sh` for scenario S9.
- Guard the shared machine (AGENTS rule 19): a private target directory outside the repository, at most two heavy builds, free disk first.

```sh
export CARGO_TARGET_DIR="$HOME/.cache/sagat-targets/036-elastic-scaling"
df -h .
```

A sample configuration used by the scenarios (checked against [`contracts/scaling-config.schema.json`](./contracts/scaling-config.schema.json)):

```json
{
  "version": 1,
  "units": {
    "engine": {"minReplicas": 2, "maxReplicas": 8, "costCeiling": 8, "perReplicaConnections": 20, "connectionWeight": 2},
    "control-plane": {"perReplicaConnections": 20, "connectionWeight": 1}
  },
  "database": {"serverMaxConnections": 200, "reserveConnections": 20},
  "egress": {"allow": [{"host": "prometheus.monitoring.svc", "port": 9090, "tls": false, "allowPrivate": true}]},
  "sources": [
    {"name": "engine-cpu", "kind": "METRICS_QUERY", "unit": "cores", "range": {"min": 0, "max": 64},
     "intervalSeconds": 15, "freshnessSeconds": 45,
     "destination": "http://prometheus.monitoring.svc:9090",
     "query": "sum(rate(container_cpu_usage_seconds_total{pod=~\"engine-.*\"}[2m]))"}
  ],
  "policies": [
    {"name": "ingest-by-lag", "target": {"unit": "engine", "role": "INGEST"},
     "bindings": [{"signal": "ingest.lag.backlog", "interpretation": "DEMAND", "target": 1000}],
     "min": 2, "max": 8, "costCeiling": 8,
     "scaleOut": {"step": {"maxChange": 4}},
     "scaleIn": {"step": {"maxChange": 1, "everySeconds": 120}, "stabilizationSeconds": 300},
     "mode": "OBSERVE"}
  ]
}
```

## Scenarios

### S1 — Lag per connector (US01; FR-036-001–003, 005, 006; SC-036-001)

```sh
cargo test -p sagat-tests --test integration_elastic_scaling --locked -- lag:: --nocapture
```

For each of Kafka, NATS and SQS against a real broker: pause the consumption, publish 500 messages, read the lag (backlog 500 within the lag tolerance, age where the broker can say, sample time); resume (backlog 0, `CAUGHT_UP`). Fill the tenant's undispatched-event limit (`BLOCKED`); set a low `ratePerSecond` (`RATE_LIMITED` with a retry time); stop the broker (`unavailable`, never 0). **Expected:** the figures above, `null` for what a broker cannot state, a cross-namespace read answers as a missing connector and is audited.

### S2 — Balanced Kafka partitions (US02; FR-036-007–011; SC-036-002, SC-036-003)

```sh
cargo test -p sagat-tests --test integration_elastic_scaling --locked -- assignment:: --nocapture
cargo test -p sagat-tests --test fault_elastic_scaling --locked -- handover:: --nocapture
```

A 12-partition topic with a fixed message set: one reader holds 12; two more join (4, 4, 4 within the rebalance time); one stops cleanly and one is killed in the middle of a batch (the others absorb its partitions within the rebalance time and the lease). Then 4 partitions with 6 readers (4 hold one, 2 hold none, `PARTITION_BOUND`); add a partition while running. **Expected:** the inbox holds every message exactly once, no position moved backwards, the held counts differ by at most one.

### S3 — A policy by configuration, from observe to enforce (US04, US03; FR-036-014–018, 021; SC-036-005)

```sh
cargo test -p sagat-tests --test integration_elastic_scaling --locked -- config:: --nocapture
cargo test -p sagat-tests --test contract_elastic_scaling --locked -- config_schema --nocapture
```

Start with no policy: nothing changes. Apply the sample (observe): decisions are recorded, nothing is published as authoritative. Switch to `ENFORCE`, publish lag above target: the desired capacity rises in steps within bounds and `scaling_desired_replicas{authority="authoritative"}` follows; after the scale-in window it falls by the step. Apply an invalid file (a minimum above a maximum, an unknown signal): refused with a typed error, the last valid version stays. **Expected:** the golden vectors V1–V12 of [`contracts/evaluation.md`](./contracts/evaluation.md) hold through the loop.

### S4 — The same policy through the management interface (US05; FR-036-019–025; SC-036-005, SC-036-013)

```sh
cargo test -p sagat-tests --test contract_elastic_scaling --locked -- operator_api tenant_api --nocapture
cargo test -p sagat-tests --test security_elastic_scaling --locked -- cross_tenant operator_audience --nocapture
```

By hand once the operator listener is on (`SAGAT_OPERATOR_ADDR`):

```sh
sagat scaling policies create -f policy.json            # starts in OBSERVE
sagat scaling policies evaluate ingest-by-lag           # dry evaluation, nothing applied
sagat scaling overrides create --policy ingest-by-lag --kind MAX --value 3 --expires-in 1h --reason "maintenance"
sagat scaling decisions list --policy ingest-by-lag --limit 5
sagat scaling desired
```

**Expected:** identical decisions for identical samples whichever way the policy was declared; a configuration-owned policy refuses an edit (`OWNED_BY_CONFIG`) and accepts an override that expires by itself; a tenant token is refused by the operator listener; a tenant sets parallelism and rate within quota and a policy request from a tenant is refused.

### S5 — Explain and reproduce a decision (US06; FR-036-026–029; SC-036-006)

Provoke a scale-out clamped by the partition count and by the database budget; read the record (`sagat scaling decisions get <id>`); submit its `inputs` to `POST /scaling/policies/{name}/evaluate`. **Expected:** the record lists each signal with value, age and use, the raw demand and each limit in order; the dry evaluation returns the same decision; the metrics scrape contains no tenant, namespace, connector, run or activity label.

### S6 — Safety (US10; FR-036-043–048; SC-036-007, SC-036-008)

```sh
cargo test -p sagat-domain scaling -- --nocapture                      # properties and vectors
cargo test -p sagat-tests --test fault_elastic_scaling --locked -- evaluator:: --nocapture
```

Cut the telemetry (stale signals): every unit holds and a condition is raised. Kill the evaluator between collecting and recording; run two evaluators with a held lock. Flood a shared signal from one tenant. Freeze globally. **Expected:** no partial decision, exactly one decision in force, ceilings never passed, the freeze effective by the next evaluation and audited, the roles keep their capacity when the evaluator is down.

### S7 — Pulled sources (US13; FR-036-056–060; SC-036-017, SC-036-018)

```sh
cargo test -p sagat-tests --test integration_elastic_scaling --locked -- sources:: --nocapture
cargo test -p sagat-tests --test security_elastic_scaling --locked -- sources:: --nocapture
```

Declare a `METRICS_QUERY` source against a test metrics server that answers a CPU figure, and a `BROKER_QUEUE` source against a test queue; use them in policies; test one with `POST /scaling/sources/{name}/test`. Then break them: unreachable, slow, `NaN`, out of range, a redirect, an oversized answer. **Expected:** `unavailable` and a hold in each case, never 0; a tenant, an off-allowlist destination, a credential over plain text or a query that does not reduce to one number is refused; no credential in any record, log, metric or answer.

### S8 — Chart rendering (FR-036-030, 061)

```sh
helm lint infra/helm -f infra/helm/examples/ha.yaml
cargo test -p sagat-tests --test contract_scalability_ha --locked      # extended, not loosened
```

Render `engine.autoscaling.mode: platform` with a `scaling` block: one KEDA trigger on `max(scaling_desired_replicas{unit="engine",authority="authoritative"})`, the ConfigMap, the environment variable and the egress peers; a source whose destination is not allowed, `platform` together with another mode, or more than 64 replicas is refused at render. **Expected:** every autoscaler query uses a metric that the code emits.

### S9 — The provider follows (SC-036-019)

```sh
G247_DOCKER_CONTEXT=<ctx> tests/e2e/scalability-ha/kubernetes/run.sh      # extended with an engine policy
```

In a kind cluster with Cilium and KEDA: apply a policy in enforce, publish lag, watch the engine scale out and, after the window, in; stop the evaluator and watch the replicas hold; make a signal unavailable (`NaN`) and watch what the provider does. **Expected:** reported as it happens, including anything the provider does that the plan did not assume (`G1703`).

### S10 — The lag control loop, measured (SC-036-004)

```sh
cargo test -p sagat-tests --test perf_scalability_ha --release --locked -- --ignored lag_control_loop
```

Time for the lag to fall under target for Kafka, NATS and SQS, with the replica count reached, appended to `tests/performance/reports/scalability-ha.md` with the environment. On one machine in slice 1; the separate-machine report is slice 3. **Expected:** a number with its environment, or a statement that a level was not reached and what limited it.

## What "done" looks like for slice 1

Every task of `tasks.md` ticked with the named test and its output; the scenarios above green on the merged tree (rerun it yourself, a subagent's report is a claim); `python3 scripts/check-architecture.py && python3 scripts/check-agent-config.py`; `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the runtime-role sweep with `SAGAT_TEST_AS_RUNTIME_ROLE=1`; Redocly lint of both OpenAPI documents; the runbooks and alerts of NFR-036-OPS written.
