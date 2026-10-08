# Topologies, capacity, DR and benchmark

## Minimum dev topology

- PostgreSQL 1 instance, NATS JetStream single-node, engine 1 replica, API 1 replica, TS/Python worker 1 pool, local S3-compatible object storage, Studio 1 replica. Development only.

## V1 production HA topology

- PostgreSQL HA/PITR in an available zone, separate poolers for writes and reads, read replicas for the read model if useful.
- NATS JetStream cluster sized appropriately, durable consumers and DLQ per capability and environment.
- Stateless API 2+ replicas, Execution Engine 2+ replicas with shard leases, Worker Supervisor 2+ replicas with pools and HPA/KEDA if approved.
- Build Workers isolated from execution, with no access to the control plane's database network.
- OTel Collector, Prometheus/Datadog/Grafana/Tempo/Jaeger depending on the operator's stack; store high-cardinality data in traces/logs.
- Object storage with lifecycle policy and KMS, retention compatible with active Runs and compliance requirements.

## Sizing model (avoid unvalidated numbers)

Static platform memory approximately `base_per_replica + active_worker_memory + query_caches`; each wait does NOT require a thread. Disk `runs + steps + journal_entries + indexes + hot payloads`, and retention has more impact than the number of suspended runs. Transition cost = PostgreSQL commits + outbox + dedupe; fanout increases write amplification.

## Benchmark scenarios

| Case | Waiting runs | Active transitions/s | Workload | Goal |
|---|---:|---:|---|---|
| BL01 | 10,000 | 100 | 2-steps + wait | measure baseline |
| BL02 | 100,000 | 1,000 | event+timer+retry mix | measure p95, DB/CPU |
| BL03 | 1,000,000 | 1,000 | mostly dormant | prove cost per wait |
| BL04 | 100,000 | 5,000 | 60s burst and normalization | stress sustainable ceiling |
| BL05 | 100,000 | 1,000 | 30% worker loss | recovery/backlog |
| BL06 | 50,000 | 500 | noisy tenant 80% traffic | fairness p99 |

Numbers are load targets, **not proven throughput goals**. Each report must declare hardware, quotas, memory, disks, PG config, HA, RTT, workers, payload sizes, p50/p95/p99, recovery time and integrity results.

## DR

- Automatic backup and PostgreSQL PITR; snapshot/version of scripts and artifacts in object storage; export of manifest and tenant configuration without secrets.
- Quarterly restore test in an isolated environment, with replay execution and hash verification.
- RPO/RTO can only be published with replication and restore drills executed; no default numeric SLA in the package.

## Scaling triggers

1. Queue lag per pool/capability.
2. DB writes p95 and hot shard contention.
3. Event matcher delays / wait resume latency.
4. Outbox delivery age and consumer lag.
5. Sandbox startup, build queue and artifact cache hit rate.

The decision to extract a microservice must demonstrate scaling, latency or fault domain isolation with benchmark data and a consistency plan.
