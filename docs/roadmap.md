# Roadmap with gates and dependencies

| Order | Feature | Demonstrable deliverable | Gate |
|---|---|---|---|
| 1 | 001 Platform Foundation | API/auth/namespace/schema, proto and local docker | Contract tests and tenant isolation |
| 2 | 002 Durable Engine | Run 2 steps and resume after crash | Fault injection outbox/lease |
| 3 | 003 Sagas | reserve→fail→compensate and reconciliation | External ambiguity simulation |
| 4 | 004 Events & Messaging | webhook→inbox→wait/signal, NATS/Kafka/SQS | Duplicate/out-of-order/DLQ |
| 6 | 005 Inline Scripts | publish/build/sandbox TS, Python, C# | Egress/resource isolation |
| 7 | 006 SDKs | five SDKs with a common contract | Compliance matrix per SDK |
| 8 | 007 Studio | editing, publish, runtime DAG + timeline | E2E author-to-run |
| 9 | 008 Observability | OTel, alerts, run operations | Diagnosis by correlation |
| 5 | 009 Security | advanced RBAC, secrets, tenant quotas | Pen-test and escape checks |
| 10 | 010 Scale & HA | sharding, fairness, HA and benchmarks | Throughput and recoverability |
| 11 | 011 GitOps & Versioning | versioning, promotion, diff, rollback | Old runs preserved |
| 12 | 012 Developer Experience | CLI, local emulator, docs and examples | End-to-end onboarding |

**Realistic milestones:** M0=contracts and journal spike; M1=durable vertical slice; M2=sagas/events; M3=scripts and first SDK; M4=Studio; M5=enterprise hardening; M6=multi-SDK/HA. Avoid estimates without team composition and benchmark.

**Dependencies:** `001 → 002 → {003,004,009} → {005,006,007} → {008,011,012} → 010`, with cross-cutting security from 001; later phases can advance in parallel once contracts stabilize.
