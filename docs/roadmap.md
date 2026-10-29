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
| 12 | 012 Developer Experience | CLI, local testing on the real engine (no emulator), docs and examples | End-to-end onboarding |
| 13 | 013 Federated Login | Studio sign-in through each organization's OIDC or SAML provider | Draft specification only |
| 14 | 014 Workflow Primitives | forEach/map, loops, approval, switch, Query/Update, Continue-As-New | 0.3 work in progress on `feature/014-v0.3-completion`; eight tasks remain open; [increment evidence](../specs/014-workflow-primitives/implementation-ledger.md) |
| 15 | 015 OpenTelemetry and OpenBao | all three signals through a Collector; OpenBao as the secret and key authority | Draft specification only (was the second 013; renumbered by the owner on 2026-10-16) |
| 16 | 016 Payload Classification | per-field classification (`public`, `internal`, `pii`, `secret`) of run payloads, audited disclosure by level, `payload:reveal-pii` | Implemented and merged into 0.2.0; final integration gates passed on 2026-10-26 ([evidence](../specs/016-payload-classification/release-gates-2026-10-25.md)); includes the documented event-read break. Eight feature-014 tasks remain open under ADR-0040 |
| 17 | 017 Database Connections | UI-managed PostgreSQL connections and OpenBao secrets shared by SDK, visual and ad hoc workflows/steps ([spec](../specs/017-database-connections/spec.md)) | Implemented and merged into 0.2.0; real PostgreSQL/OpenBao faults, five-SDK/UI parity, full Compose and recovery passed ([evidence](../specs/017-database-connections/verification.md)). Production installation remains operator-managed |

**Realistic milestones:** M0=contracts and journal spike; M1=durable vertical slice; M2=sagas/events; M3=scripts and first SDK; M4=Studio; M5=enterprise hardening; M6=multi-SDK/HA. Avoid estimates without team composition and benchmark.

Feature033 adds Redis Streams to the0.3 release scope: selectable internal notification transport
and an additional callback destination. PostgreSQL/gRPC semantics stay unchanged. Track its
[acceptance tasks](../specs/033-redis-streams/tasks.md) and actual validation evidence alongside the
remaining feature014 gates; its implementation does not waive those release requirements.

**Dependencies:** `001 → 002 → {003,004,009} → {005,006,007} → {008,011,012} → 010`, with cross-cutting security from 001; later phases can advance in parallel once contracts stabilize.

### Capacity increment in progress — 2026-10-29

Feature014's [dormant ChildRoot Acquire](../specs/014-workflow-primitives/capacity-acquire.md)
adds fenced possession transfer and original-key reconciliation beside Vacate/Finish. Native and
integrated acceptance is in progress; canonical tasks and the release remain open. Runnable work,
sweeper/callers, claim guards, nested admission and quota/backfill remain required before activation.
