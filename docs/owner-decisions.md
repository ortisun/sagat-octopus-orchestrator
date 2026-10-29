# Owner decisions

Decisions that only the product owner could take, with what each one unblocks. Each entry is also
recorded where the open item lived (gap register, ADR, runbook or specification). "Not built"
means the decision is taken and the implementation is still to be done; it is not evidence.

## 2026-10-27

Answers to the scope questions of the feature 035 specification (Apache Iceberg write steps): items 1–3 were asked before it was written, items 4–8 at `/speckit-clarify`, items 9–11 when the owner extended the scope to step-output destinations. "Not built" means the decision is taken and the implementation is still to be done; it is not evidence.

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | 035 write modes | **Append, overwrite and upsert** (merge by key) in the first delivery; the recommendation was append + overwrite with upsert later. | `specs/035-iceberg-write-steps/research.md` (D035-001) | Decided; not built |
| 2 | 035 catalogs and storage | **Broad set**: REST, AWS Glue, Hive Metastore and JDBC catalogs; S3-compatible, Google Cloud Storage and ADLS Gen2 storage; the recommendation was REST + S3-compatible first. | `specs/035-iceberg-write-steps/research.md` (D035-002) | Decided; not built |
| 3 | 035 destination model | A **new connection kind of the feature-017 catalog** (revisions, OpenBao secrets, probe, managed executor, network policy, ad hoc admission); no dependency on the PR #53 drafts. | `specs/035-iceberg-write-steps/research.md` (D035-003) | Decided; not built |
| 4 | 035 batch size | At most **2 GiB per step** in at most 32 record blobs of 64 MiB, streamed into files. | `specs/035-iceberg-write-steps/research.md` (D035-013) | Decided; not built |
| 5 | 035 deletes in upsert | An upsert may declare a **delete marker** field; marked keys are removed in the same commit. | `specs/035-iceberg-write-steps/research.md` (D035-014) | Decided; not built |
| 6 | 035 unknown record fields | **Refused by default**; a step may declare that they are ignored (dropped, counted, never written); the table schema is never changed. | `specs/035-iceberg-write-steps/research.md` (D035-015) | Decided; not built |
| 7 | 035 snapshot summary identifiers | Effect key, run id, step id, **workflow name and version**; workflow names become visible to readers of the table's metadata. | `specs/035-iceberg-write-steps/research.md` (D035-016, O035-001) | Decided; not built |
| 8 | 035 table creation | **No**: tables exist beforehand; a missing table fails the step. | `specs/035-iceberg-write-steps/research.md` (D035-017) | Decided; not built |
| 9 | 035 step-output destinations | “Quero que o output dos steps também possam ser salvas no iceberg, postgres, etc...” (example: a finished payment flowing down to the lake). Outputs go to destinations **chosen per destination as synchronous** (inside the run) **or asynchronous** (batched export). | `specs/035-iceberg-write-steps/research.md` (D035-018) | Decided; not built |
| 10 | 035 destination kinds | **Iceberg, customer PostgreSQL, object storage files, ClickHouse and broker streams, all five in both modes** (recommendation for synchronous: Iceberg and PostgreSQL only). | `specs/035-iceberg-write-steps/research.md` (D035-019) | Decided; not built |
| 11 | 035/020 placement | Asynchronous export is **020's declared-record export**, which gains an `ICEBERG` sink reusing 035's connection; 035 specifies the Iceberg write step and the synchronous destinations. | `specs/035-iceberg-write-steps/research.md` (D035-020), 020 `research.md` | Decided; 020 is still a draft in PR #53 |

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
