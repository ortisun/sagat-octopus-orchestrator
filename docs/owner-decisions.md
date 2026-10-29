# Owner decisions

Decisions that only the product owner could take, with what each one unblocks. Each entry is also
recorded where the open item lived (gap register, ADR, runbook or specification). "Not built"
means the decision is taken and the implementation is still to be done; it is not evidence.

## 2026-10-28 — elastic scaling (feature 036)

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | O1706, who applies replicas and resources when a scaling policy is enforced | Option B, chosen in `/speckit-clarify`: by default the environment's autoscaler or an external controller follows the platform's published desired capacity; in addition an **optional applier** (a separate component, off by default, with only the permissions to change the replicas and resources of the platform's own workloads) carries out enforced decisions in the supported orchestrator, for operators who want a closed loop. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-051 to FR-036-055, US12, SC-036-015 and SC-036-016) | Specified; not built |
| 2 | O1707, where outside data for scaling comes from | Option B, chosen in `/speckit-clarify`, with the owner's words: “O orquestrador deve consultar as métricas (mensagens enfileiradas, cpu, memoria, etc..). Ele dará os sinais ao keda ou outro provider fazer o scaling in ou out.” The platform queries external sources declared by configuration, evaluates, and hands the signals and the decisions to the autoscaling provider. Declaring a source is an operator action (destination allowlist, credentials by reference, bounded read-only queries); tenants publish custom signals instead. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-056 to FR-036-061, US13, SC-036-017 to SC-036-019) | Specified; not built |
| 3 | O1708, programmatic access to signals and policies and the SDKs | Option A, chosen in `/speckit-clarify`: the management interface is the documented, versioned REST contract plus the operator command line. The six official SDKs do not change in this feature (their client covers run operations only, and a new client operation must enter all six with a scenario and a golden). | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-019, Assumptions, Out of scope) | Specified; not built |
| 4 | O1703, what a tenant may do about scaling | Option B, chosen in `/speckit-clarify`: a tenant reads the lag, state and decisions of its own connectors and adjusts their in-flight parallelism and rate limit by hand within its quota; every scaling policy is operator-only; a tenant never controls replicas or resources. Consequence recorded as O1704 (not yet confirmed): a custom signal published by a tenant has no automatic effect until an operator admits it to a policy. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-023, FR-036-024, US05) | Specified; not built |
| 5 | O1709, delivery order of feature 036 | Option B, chosen in `/speckit-clarify`: the first delivery is US01, US02, US03, US04, US05, US06, US10 and US13 (consumer lag, balanced Kafka partitions, scaling on lag, policies by configuration and through the management interface, explanation, safety, pulled sources, signals and desired capacity exported to the autoscaling provider). The other roles (US07), the dedicated ingestion role (US09), vertical scaling (US08), the optional applier (US12) and the benchmark on separate machines (US11) follow. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, Context "Delivery order") | Specified; not built |

## 2026-10-26

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | Remaining implementation and release target | “Continue com a implementacao do restante agora para a release v0.3”. Complete the eight remaining feature-014 tasks for release 0.3.0 with their original acceptance criteria and required gates. | [0.3.0 implementation ledger](../specs/014-workflow-primitives/implementation-0.3.md) | Implementation in progress; no capacity or release guarantee inferred |

## 2026-10-25

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | Implemented-task integration and release | “Quero que mergeie todas as tasks já implementadas. Crie as versoes e releases necessárias”. Merge and release implemented work; no approval of pending feature-014 production ceilings, general nesting, unverified work or waived gates is inferred. | [ADR-0040](adr/ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md), [0.2.0 notes](releases/0.2.0.md) | Authorized; merge/publication require actual integrated-head gates |

The implementation agent selected one coordinated **0.2.0** under the existing pre-1.0 breaking-minor policy and synchronized release tooling. The owner did not explicitly name this version. GitHub Releases remains the distribution channel; no environment deployment or registry publication is added.

## 2026-10-26 — nested capacity (lane `feature/014-capacity-protocol`)

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | O701, reduction of `iteration_concurrency` | After a reduction (including to 0) already admitted work is **not paused**: it runs to completion and only new admissions are blocked, as the released contract and runbook say. A tenant may therefore exceed a reduced limit while its admitted work lasts. | [capacity-eligibility.md](../specs/014-workflow-primitives/capacity-eligibility.md), [zero policy](../specs/014-workflow-primitives/zero-iteration-policy.md), `research.md` D704/D705 | Explicit zero in the 0.3 working branch; nested group runtime remains pending. O702–O704 are consequences, open to correction |

## 2026-10-16

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | G42, quota values | The current defaults (10,000 active runs per namespace and 50,000 per tenant; 1,000 workflows; 1,000 worker sessions; 500 secrets; 10 GiB content and 1 GiB logs; 100 API requests/s with burst 200; 1,000 events/s with burst 2,000; 30-day events; 365-day scripts) are the **single default tier**. Commercial tiers wait for a billing model; operators keep adjusting per tenant under the ceiling. | `crates/domain/src/quota.rs`, ADR-0027, 009 `research.md` | Done (no behaviour change) |
| 2 | G42/G80, payload classification | **Per field, four levels** (`public`, `internal`, `pii`, `secret`); an unmarked field is `pii`; `pii` needs a new audited permission; `secret` is never shown. | [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md), 004 and 009 `research.md` | Not built (needs its own feature) |
| 3 | SC-008-001 | **N = 10**: a diagnosis passes with at most 9 counted interactions. | 008 `spec.md`, `docs/validation/human-acceptance.md` | Done; the human sessions are still to run |
| 4 | G21, run result | Reaching `end` evaluates `outputExpr`, stores the result (at most 256 KiB, charged to the content quota) in the terminal transaction and gives it to the parent as `steps.<child>.output`; disclosure follows ADR-0033. | 002 `research.md`, workflow schema | Done (`bugfix/run-result`); API disclosure waits for ADR-0033 |
| 5 | G141, late heartbeat | Option (b): the server answers a heartbeat for an attempt the same worker already completed with a distinct non-error status; SDKs unchanged. | 006 `research.md`, `sdks/SPEC.md` §6.4 | Done (`bugfix/late-heartbeat`) |
| 6 | G7, token revocation | **Accepted risk**: tokens live at most 1 hour, roles are read on every request, and suspending a membership is the incident response. | 001 `research.md` | Closed |
| 7 | G9, denied requests | **Every 403 is audited**, bounded by the existing per-subject rate limit. | 001 `research.md` | Done (`access.denied.permission`; `denials::*` in `integration_security_multitenancy`) |
| 8 | D241, RPO/RTO | Reference objectives for the Helm topology with a synchronous standby: **RPO 0** for acknowledged work, **failover RTO ≤ 5 min**, **PITR restore RTO ≤ 1 h**; the drill is to gate a release. An asynchronous standby states its own RPO. | 010 `research.md`, `docs/runbooks/disaster-recovery.md`, `docs/traceability.md` | Objectives published and enforced by the drill, which gates the `release` workflow; not measured on a real cluster |
| 9 | G48, secret delivery | A secret gets an optional list of **worker pools** allowed to receive it; a lease is not handed to another worker and the refusal is audited. | 009 `research.md`, `docs/runbooks/security-governance.md` | Done (see G48 for what a pool is) |
| 10 | G162/G163, Studio | The provisional `S` stays until the owner picks one of the three geometric logos (G162 open). **Scripts follows Workflows** in the navigation (G163). | 007 `research.md`, `frontend/apps/studio/src/app/Shell.tsx` | G163 done; G162 open |
| 11 | T007-049/T007-051 | The owner reviews the visual corrections of 2026-10-12 before accepting them. | — | Waiting for the owner |
| 12 | G123/G203/G216, publication | **GitHub Releases only**: SDK packages and `sagat` binaries with checksums and signatures; no npm, PyPI, NuGet or Maven for now; Go from the GitHub module path. | 006, 011, 012 `research.md`, `docs/runbooks/release.md` | Workflow built (`bugfix/github-releases`); first tagged release still to run |
| 13 | Two drafts numbered 013 | `013-federated-login` keeps 013; `013-otel-openbao` becomes **015** (`FR-015`, `SC-015`) and its draft ADR is renumbered (ADR-0022 is taken; next free after ADR-0033). Its constitution edits are redone on the ratified constitution. | `docs/roadmap.md` | Decided; the drafts live uncommitted in their own worktrees and are renumbered when they are committed |

## 2026-10-17

Answers to the four product questions of the feature 016 specification (per-field payload classification, [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md)). "Not built" means the decision is taken and the implementation is still to be done; it is not evidence.

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | 016 P1, how `payload:reveal-pii` is granted | A **per-namespace grant list** managed by administrators (worker-binding and secret-access-list precedent); the six roles stay six and no role holds the permission; grants and revocations are audited. | `specs/016-payload-classification/research.md` (D275), ADR-0033 addendum | Decided; not built |
| 2 | 016 P2, event inbox and dead-letter values | **Restrict now**: `getInboxEvent` `data` and connector dead-letter `body` and `headers` stop being returned to plain `event:read` holders; they need the same reveal grant, everyone else gets structure only; field classification of events waits for event-type schemas. A breaking change for readers of those fields. | `specs/016-payload-classification/` (US08, FR-016-026 to FR-016-029, D302 to D305, G272, G292), ADR-0033 addendum | Decided; not built |
| 3 | 016 P3, readings of ADR-0033 | The **restrictive** reading: an undeclared field is at least `pii` even under a `public` parent; a schema with no annotation is `pii` throughout; a payload with no schema at all is structure only for everyone. | `specs/016-payload-classification/research.md` (D265, D269) | Decided; not built |
| 4 | 016 P4, self-grant | An administrator **may grant reveal to themselves**, audited. | `specs/016-payload-classification/research.md` (D306) | Decided; not built |
| 5 | Self-contained infrastructure | **Everything the platform needs to run or to be tested lives in this repository.** Local and CI infrastructure is declared in `infra/docker/compose.yaml` (and `compose.observability.yaml`), with pinned images, loopback-only ports and development credentials; nothing may depend on a container, volume or service that belongs to another project or has to be started by hand. When feature 015 adopts OpenBao it is added as a service of that compose file (dev mode, pinned image, healthcheck, initialised by a one-shot job), not borrowed from another stack. Integration tests start their own disposable containers. | `AGENTS.md` (infrastructure), 015 draft | Decided; the stack today needs no OpenBao (secrets use the sealed application vault in PostgreSQL) |

## 2026-10-27 — Official Rust SDK

The owner reported the missing Rust SDK during the active0.3 implementation. Add an official Rust
SDK with the current SDK client/worker/builder semantics and shared conformance, tracked by
[feature034](../specs/034-rust-sdk/spec.md) and [ADR-0070](adr/ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md).
The active0.3 context is the release target; existing Redis033 and remaining014 work stays in scope.

## 2026-10-27 — Redis Streams for release0.3

The owner requested Redis Streams in communications/callbacks now in v0.3 and selected:
“Alternativa ao NATS e transporte adicional para callbacks, selecionável por configuração”.
Feature033 implements optional internal Redis outbox notifications and Redis callback destinations,
retaining PostgreSQL authority, worker gRPC, NATS compatibility and existing HTTP callbacks.
[Specification](../specs/033-redis-streams/spec.md); [ADR-0069](adr/ADR-0069-redis-streams-share-outbox-and-delivery-authority.md).
Implementation/tests remain required; no production deployment, automatic release or inbound Redis
connector is inferred. This explicitly approves dependencies on the existing004/009/017 services.
