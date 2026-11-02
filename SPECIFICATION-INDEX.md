# Executive index — Sagat Octopus Orchestrator Spec Kit

## What is built and what remains

Features 001–012 have implementations and automated evidence: a Rust durable-workflow engine and journal, Saga orchestration, events/webhooks plus inbound Kafka/NATS/SQS, sandboxed TypeScript/Python/C# scripts, SDKs in TypeScript/Go/Python/C#/Java, React Flow/Monaco Studio, multi-tenant governance, OTLP/HTTP JSON observability, GitOps/DX, scheduler sharding/fairness, Helm, fault/load suites and a DR drill.

The requirement-level authority is `docs/traceability.md`. Material gaps include `foreach`/`humanApproval`, blob-backed payloads, Go/Java script runtimes and arbitrary dependency catalogs, outbound customer broker producers, storage/log retention quotas, KMS-backed secret keys, registry/package releases, journal partitioning, queue quotas, 5 000 transitions/s, human studies and a full product/KEDA cluster rollout. MIT was selected on 2026-10-11; package metadata and publication verification remain current work.

## How to navigate

- **Vision:** `README.md`, `docs/architecture/overview.md`.
- **Principles:** `.specify/memory/constitution.md`.
- **Critical semantics:** `docs/architecture/execution-semantics.md`, `worker-runtime-protocol.md`, `sagas.md`, `storage-and-events.md`.
- **Visual and SDK experience:** `docs/architecture/studio-ux.md`, `workflow-ir-and-sdk-modes.md`, `sdk-contract.md`.
- **Security and operations:** `docs/architecture/security-and-sandbox.md`, `observability-slo.md`, `capacity-and-topology.md`.
- **Per-feature plan:** `specs/{001..012,014,016..032}-{slug}/spec.md`, `plan.md`, `tasks.md`, `research.md`, `data-model.md`, `quickstart.md`, `checklists/requirements.md`.
- **Contracts:** `specs/001-platform-foundation/contracts/`.
- **Tasks:** `specs/*/tasks.md` have IDs, paths and tests to direct agents.
- **Example:** `examples/payment-saga.workflow.json`, `examples/sdk-parity.md`.

## Delivered dependency sequence

1. 001 Foundation: tenant/auth/API/proto/migrations.
2. 002 Durable Engine: transactions, leases, outbox and recovery.
3. 009 Security baseline: isolation, tokens, RBAC, secrets, quotas, bindings and egress.
4. 003 Sagas and 004 Events.
5. 005 Inline Scripts and 006 SDKs.
6. 007 Studio.
7. 008 Observability, 011 GitOps and 012 DX.
8. 010 Scale/HA, followed by completion work that integrated C# sandboxing, Studio/DX and Helm network isolation.

## Pending product and validation decisions

- [017 Database Connections](specs/017-database-connections/spec.md): PostgreSQL connections configured in the UI, OpenBao secrets in the first delivery, and shared SDK/visual/ad hoc execution. User approved implementation on 2026-10-16; [ADR-0035](docs/adr/ADR-0035-database-connections-and-managed-execution.md) is accepted and the feature plan/tasks track implementation. Product completion remains pending runtime evidence.

- Choose registry ownership, signing and release policy for the five SDK packages and the `sagat` binary.
- Choose production identity/KMS/object-storage providers and the retention/classification tiers.
- Define a distributed performance testbed that separates workers, API and PostgreSQL; the checked-in reports are single-machine evidence and do not prove 5 000 transitions/s.
- Run a full product/KEDA deployment, node-drain exercise and real dependency-protocol checks; current cluster evidence covers NetworkPolicy canaries and restricted Pod admission only.
- Conduct the newcomer timing and human accessibility studies that automation cannot replace.
- Approve financial and compensation policies per application domain.

## Proposed parity features 018–032 (drafts, 2026-10-26/27)

Fifteen draft features bring Temporal and Windmill capabilities into the platform's architecture. Each has a
spec, plan, research, data model, contract fragments, quickstart and tasks; none is approved or implemented,
and every ADR they propose is `Proposed`. They were written in parallel, reviewed independently (architecture,
security and cross-feature consistency) and revised. Suggested order and gates: [docs/roadmap.md](docs/roadmap.md).

| Feature | Scope | Proposed ADRs |
|---|---|---|
| [018](specs/018-schedules-bulk-operations/spec.md) | schedules, batch operations, reset to a point, Signal/Update-with-Start, business ids | ADR-0050 |
| [019](specs/019-worker-deployments-dispatch/spec.md) | worker deployment versions, priority and fairness keys, checkpoints, task tokens, push dispatch, warm script containers | ADR-0051, ADR-0052 |
| [020](specs/020-visibility-analytics-archival/spec.md) | search attributes and list queries, journal-derived analytics sinks, archival and retention | ADR-0053, ADR-0054 |
| [021](specs/021-integration-resources-triggers/spec.md) | resources, managed operations, variables, new trigger kinds | ADR-0055 |
| [022](specs/022-flow-control-step-policies/spec.md) | step cache, early return, failure handlers, concurrency keys, standalone steps, draft test mode | ADR-0056 |
| [023](specs/023-script-languages/spec.md) | Bash, PowerShell, PHP, Rust scripts; `python-data` runtime mechanism | ADR-0057 |
| [024](specs/024-data-engines-pipelines/spec.md) | MySQL/MariaDB, SQL Server, BigQuery, Snowflake, DuckDB; data assets and pipelines | ADR-0058 |
| [025](specs/025-ai-agents-mcp/spec.md) | durable agent step, MCP server and client | ADR-0059, ADR-0060 |
| [026](specs/026-invocation-human-surfaces/spec.md) | forms and public links, run-and-wait, per-workflow OpenAPI, Workflow Streams, channel approvals and notifications | ADR-0061, ADR-0068 |
| [027](specs/027-internal-apps/spec.md) | bounded internal app builder | ADR-0062 |
| [028](specs/028-developer-tooling/spec.md) | time-skipping tests, interceptors, VS Code, Git sync, curated template hub | ADR-0063 |
| [029](specs/029-custom-roles-folders/spec.md) | custom roles, groups, folders, operation lists | ADR-0064 |
| [030](specs/030-cross-namespace-services/spec.md) | cross-namespace service operations within a tenant | ADR-0065 |
| [031](specs/031-client-side-encryption/spec.md) | client-side payload encryption and codec server | ADR-0066 |
| [032](specs/032-multi-region-failover/spec.md) | active-passive multi-region cells | ADR-0067 |

**Owner decisions that gate the drafts** (each recorded as an `O…` question in the feature's `research.md`):
the amendment of constitution principle I for retention (020, ADR-0054) and of the Delivery Constraints for
multi-region (032, ADR-0067); the scope line of `docs/architecture/overview.md` that defers a low-code designer
(027, O1360); one rule for `pii` leaving the platform (020, O1081, applied by 020/025/026/031); the placement
rule for customer-credentialed calls (021, ADR-0055); and acceptance of every proposed ADR.

**Cross-feature registries and their owners.** Each concern has one owner; the others cite it.

| Concern | Owner |
|---|---|
| Reset rule of every journal event type | 018 (`contracts/journal-and-events.md`) |
| Merged admission-transaction lock order | 018 (ADR-0050 decision 7) |
| Instance-capability registry and fleet activations (`schedules-v1`, `access-narrowing-v1`, `payload-encoding-v1`) | 018 (FR-018-052, ADR-0050 decision 9) |
| Where customer-credentialed calls run | 021 (ADR-0055) |
| Content stores and their purge/erasure rules; `pii` egress rule | 020 |
| Encoded-field positions, carriers and write paths | 031 |
| Time-class catalog (workflow vs infrastructure time) | 028 (ADR-0063) |
| Studio origins and CSP (refines ADR-0016) | 027 (ADR-0062) |
| Keys a cell holds | 032 |
| Script host calls (`checkpoint`, `stream.append`) | 023 |
| The only payload-transforming SDK stage | 031 (028 interceptors only observe and decorate) |

**Amendments to accepted work, stated as proposals:** FR-016-002's closed carrier list and classification
rules version 2 (one shared amendment, opened by the first of these features to be implemented, for every
carrier the drafts add); 014's approval owner kinds (025 owns, 026 cites); ADR-0009, ADR-0013, ADR-0016,
ADR-0023, ADR-0027, ADR-0033, ADR-0035 and ADR-0036 where each draft's ADR says "supersedes in part" or
"refines"; constitution principle I and the Delivery Constraints.

**Provisional at implementation time:** worker-protocol minor numbers (1.6 is current; drafts that write "1.7"
take the next free minor in merge order), migration timestamps, and the permission catalog order. Numbers
reserved for these drafts: ADR-0050..ADR-0068; decisions, gaps and owner questions D/G/O 1000–1599, with
overflow 1600–1674.

## Commitment terminology

`MUST` = design requirement. `V1` = first full implementation of the specified scope. `P2/later` = explicit roadmap. A checked task names implementation evidence, but it does not erase an explicit partial/open requirement in `docs/traceability.md` or a gap in the feature's `research.md`.
