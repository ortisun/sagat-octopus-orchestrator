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
| 14 | 014 Workflow Primitives | forEach/map, loops, approval, switch, Query/Update, Continue-As-New | In progress on `feature/workflow-primitives` |
| 15 | 015 OpenTelemetry and OpenBao | all three signals through a Collector; OpenBao as the secret and key authority | Draft specification only (was the second 013; renumbered by the owner on 2026-10-16) |
| 16 | 016 Payload Classification | per-field classification (`public`, `internal`, `pii`, `secret`) of run payloads, audited disclosure by level, `payload:reveal-pii` | Implemented and merged into 0.2.0; final integration gates passed on 2026-10-26 ([evidence](../specs/016-payload-classification/release-gates-2026-10-25.md)); includes the documented event-read break. Eight feature-014 tasks remain open under ADR-0040 |
| 17 | 017 Database Connections | UI-managed PostgreSQL connections and OpenBao secrets shared by SDK, visual and ad hoc workflows/steps ([spec](../specs/017-database-connections/spec.md)) | Implemented and merged into 0.2.0; real PostgreSQL/OpenBao faults, five-SDK/UI parity, full Compose and recovery passed ([evidence](../specs/017-database-connections/verification.md)). Production installation remains operator-managed |
| 18 | 018 Schedules and Bulk Operations | first-class schedules (overlap, backfill, catch-up, pause, trigger-now), batch operations over a run selection, reset to a point as a new run, Signal/Update-with-Start, business workflow ids ([spec](../specs/018-schedules-bulk-operations/spec.md)) | Draft: spec, plan and tasks proposed; ADR-0050 proposed |
| 19 | 019 Worker Deployments and Dispatch | worker deployment versions (pinned/auto-upgrade, ramp, drain), priority and fairness keys, heartbeat checkpoints, task-token completion, push dispatch, opt-in warm script containers, worker insights ([spec](../specs/019-worker-deployments-dispatch/spec.md)) | Draft; ADR-0051 and ADR-0052 proposed |
| 20 | 020 Visibility, Analytics and Archival | classified search attributes, a bounded list query language and saved views; journal-derived analytics projection with pluggable sinks; verified archival under a namespace retention policy ([spec](../specs/020-visibility-analytics-archival/spec.md)) | Draft; ADR-0053 and ADR-0054 proposed |
| 21 | 021 Integration Resources and Triggers | typed resources and managed operations (HTTP, GraphQL, S3, SMTP, Slack), variables, PostgreSQL CDC, custom HTTP routes, WebSocket, MQTT, Pub/Sub, Service Bus and e-mail triggers ([spec](../specs/021-integration-resources-triggers/spec.md)) | Draft; ADR-0055 proposed |
| 22 | 022 Flow Control and Step Policies | step result cache, early return, failure handlers, concurrency limits by key, standalone steps, draft test mode ([spec](../specs/022-flow-control-step-policies/spec.md)) | Draft; ADR-0056 proposed |
| 23 | 023 Additional Script Languages | Bash, PowerShell, PHP and Rust scripts in the feature-005 sandbox model ([spec](../specs/023-script-languages/spec.md)) | Draft; ADR-0057 proposed |
| 24 | 024 Data Engines and Pipelines | MySQL/MariaDB, SQL Server, BigQuery, Snowflake, DuckDB; sandboxed DuckDB/Polars transforms, data assets, lineage, freshness, write-audit-publish ([spec](../specs/024-data-engines-pipelines/spec.md)) | Draft; ADR-0058 proposed |
| 25 | 025 AI Agents and MCP | durable agent step, MCP server and client, LLM provider resources ([spec](../specs/025-ai-agents-mcp/spec.md)) | Draft; ADR-0059 and ADR-0060 proposed |
| 26 | 026 Invocation and Human Surfaces | generated and public forms, run-and-wait, per-workflow OpenAPI, Workflow Streams, approvals and notifications through channels ([spec](../specs/026-invocation-human-surfaces/spec.md)) | Draft; ADR-0061 and ADR-0068 proposed |
| 27 | 027 Internal Apps | bounded app builder over declared operations, viewer-narrowed sessions, sandboxed custom components ([spec](../specs/027-internal-apps/spec.md)) | Draft; ADR-0062 proposed; scope needs owner decision O1360 |
| 28 | 028 Developer Tooling | time-skipping test environment, SDK interceptors, VS Code extension, Studio↔Git sync, curated template hub ([spec](../specs/028-developer-tooling/spec.md)) | Draft; ADR-0063 proposed |
| 29 | 029 Custom Roles and Folders | custom roles, groups, folders with ACLs, per-operation access lists ([spec](../specs/029-custom-roles-folders/spec.md)) | Draft; ADR-0064 proposed; IdP groups depend on 013 |
| 30 | 030 Cross-namespace Services | Nexus-like service contracts and operations within one tenant ([spec](../specs/030-cross-namespace-services/spec.md)) | Draft; ADR-0065 proposed |
| 31 | 031 Client-side Encryption | SDK payload codecs with customer-held keys, codec server, opacity checks at publication ([spec](../specs/031-client-side-encryption/spec.md)) | Draft; ADR-0066 proposed |
| 32 | 032 Multi-region Failover | active-passive cells with fenced region epochs ([spec](../specs/032-multi-region-failover/spec.md)) | Draft; ADR-0067 proposed; needs a constitution amendment (owner decision O1560) |

**Realistic milestones:** M0=contracts and journal spike; M1=durable vertical slice; M2=sagas/events; M3=scripts and first SDK; M4=Studio; M5=enterprise hardening; M6=multi-SDK/HA. Avoid estimates without team composition and benchmark.

**Dependencies:** `001 → 002 → {003,004,009} → {005,006,007} → {008,011,012} → 010`, with cross-cutting security from 001; later phases can advance in parallel once contracts stabilize.

**Parity drafts 018–032 (2026-10-26):** proposed to bring Temporal and Windmill capabilities into the platform's architecture; none is approved or implemented. Suggested order, for the owner to confirm: `{018, 019, 020, 022} → {021, 026, 028, 029} → {023, 024, 025, 027, 030, 031} → 032`, because 018 consumes 020's run selector, 025 and 027 consume 022's standalone steps and 026's Streams and forms, 031 uses 028's interceptors, 029's identity-provider groups wait for 013, and 032 needs a constitution amendment. The cross-feature notes are in [SPECIFICATION-INDEX.md](../SPECIFICATION-INDEX.md).
