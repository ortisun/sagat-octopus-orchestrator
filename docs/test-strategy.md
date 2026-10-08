# Testing and certification strategy

## Pyramid

- Unit: journal event reduction, DAG planner, backoff policies, IR validation, authz, serialization.
- Integration: real Postgres via Testcontainers, real NATS JetStream, S3-compatible object store, HTTP stub and MinIO when feasible.
- Contract: REST OpenAPI, gRPC Protobuf, AsyncAPI and `SDK compliance suite` per language.
- Property-based/model-based: state and ordering invariants; forbid terminal→running without creating a new instance.
- Fault injection: interrupt workers at claim/execute/ack, cut off the database/broker, duplicate messages, delay timers, diverging clocks, S3 outage.
- Security: cross tenant, secret exfiltration, code sandbox breakout, webhook replay, SSRF, RBAC, quotas.
- E2E: author→publish→trigger→run→saga compensation→visualize; inline code TypeScript and C#; human approval.
- Load/stress: partitioning, active steps, 10k/100k/1M waits, pools/quotas, high fanout, huge DAG and backpressure.

## Critical matrix

| Scenario | Expected invariant |
|---|---|
| Crash after transaction before NATS | Task reappears via outbox |
| NATS redelivery | Single valid lease and single step commit |
| Crash after side effect | Idempotency/reconciliation prevents duplicate effect when the provider cooperates |
| Simultaneous timeout and signal | One atomic resolution, the other is a no-op/audited |
| Compensation also fails | Retry, `COMPENSATION_FAILED` state, human action |
| Version change during Run | Run stays on the pinned version |
| Executor runs malicious script | Isolation, limits and network enforced |
| Tenant A tries to query tenant B | 403/404 without metadata leakage |

## Reproducible benchmarks

Always profile: CPU/cores, RAM, DB size, index, HA config, number of shards/workers, tenancy, payload, operation mix, replay mode and window. Publish p50/p95/p99, sustainable throughput, failures and regression vs baseline.

**Target scenarios to prove**: 10k, 100k and 1M runs waiting for an event/timer without dedicated workers; 100, 1000, 5000 transitions/s on properly sized hardware; shutdown/restart of 30% of the workers over 20 minutes. Numbers are experimental hypotheses/targets and may not be reached in V1.
