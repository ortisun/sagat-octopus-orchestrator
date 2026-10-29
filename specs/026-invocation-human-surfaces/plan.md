# Implementation Plan: Invocation and human-interaction surfaces

**Branch**: `feature/026-invocation-human-surfaces` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: Proposed — not started. Revised on 2026-10-26 after the independent architecture and
security reviews (findings applied are listed in research.md under the revised decisions).

**Input**: [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md),
[contracts/](./contracts/), proposed
[ADR-0061](../../docs/adr/ADR-0061-link-bearers-hold-one-narrow-capability-and-never-decide.md) (links)
and [ADR-0068](../../docs/adr/ADR-0068-workflow-streams-are-a-fenced-classified-log-beside-the-journal.md)
(Workflow Streams).

## Summary

Six surfaces on top of unchanged execution semantics: (1) a server-computed form model per carrier
schema, rendered by the Studio, the public page and the SDKs; (2) `executeRun`/`getRunOutcome`, a
bounded wait that holds no database resource, woken by the existing `run.finished` hint and one
batched poll per process; (3) Workflow Streams, a fenced, budgeted per-run log with inline data
beside the journal (ADR-0068), appended through a new worker-protocol rpc and delivered over SSE with
offsets and derived attempt control events; (4) public form links served on their own origin by a
distinct, one-capability principal kind on behalf of an accountable owner, in a capped pool of their
own (ADR-0061); (5) approval
notifications, signed decision links that are pointers (never authority), a decision page and an
optional approval input form, all bound to the feature-014 decision; (6) notification channels as new
destination kinds of the ADR-0025 outbound pipeline. A per-workflow OpenAPI document specializes the
existing generic operations. Every surface is additive and enabled separately.

## Technical Context

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`); the locked TypeScript, Go, Python, .NET and
Java SDK toolchains; the Studio's locked Node toolchain. No toolchain change.

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, NATS client, `jsonschema` 0.58 (through
`crates/infra-schema`), `hmac`/`sha2`. New adapter dependency, only in `crates/infra-connectors`: an SMTP
client (candidate `lettre`, pinned, TLS-only, no default features) — selected and pinned in T026-004.
Slack and Teams use the existing guarded HTTP client. No new dependency in `crates/domain`.

**Storage**: PostgreSQL is the authority for links, receipts, stream heads and events, decision links,
channels and deliveries (ADR-0002); payload bytes in the existing content-addressed `payloads` table;
NATS carries only the existing `run.finished` hint.

**Testing**: Rust unit and property tests; integration and fault suites on real PostgreSQL
(Testcontainers) as `integration_invocation_human_surfaces` and `fault_invocation_human_surfaces`;
`contract_rest_invocation_human_surfaces`, `contract_grpc_invocation_human_surfaces`,
`contract_asyncapi_events_webhooks_messaging` (baseline growth); `e2e_invocation_human_surfaces` with a
real browser; five-language `sdk_conformance` scenarios; the security battery; the runtime-role sweep;
a performance harness under `tests/performance/invocation-human-surfaces/`. Local Slack, Teams and SMTP
stand-ins (the existing `tests/support/receiver.rs` pattern and an SMTP sink container pinned in the
test fixtures); real-provider smoke tests are optional and never the only evidence.

**Target Platform**: existing single-region Linux containers, Compose and Helm profiles.

**Project Type**: modular monolith with five SDKs and the Studio.

**Performance Goals**: measure, on a declared environment, waiter wake-up latency (hint and poll paths),
the batched poll's database cost at 2,000 waiters, stream append throughput per run and publish-to-SSE
latency, public submission refusal cost before the database. No latency is promised (NFR-026-006).

**Constraints**: nothing waits inside a transaction; no user code in the control plane (summary and
form logic use the existing bounded evaluator and the JSON Schema adapter); tenant from authentication
or the stored link only; typed errors from the closed set; telemetry without values, tokens or
addresses.

**Scale/Scope**: 50 functional requirements, 8 user stories, 1 proposed ADR.

## Constitution Check

Design checks, not approval of an implementation. Re-evaluated after Phase 1 design: unchanged.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | Transaction table below; submission, append, opening, decision and fan-out each one transaction with audit/outbox; terminal runs never reopen; first commit wins (T026-047). |
| II — pure domain, inward dependencies | Pass | Pass | Domain types only (form model, stream limits, link state, destinations); SMTP/HTTP clients only in `infra-connectors`; `check-architecture.py` (T026-062). |
| III — at-least-once effects | Pass | Pass | Chat/e-mail sends at-least-once with stable delivery id and stated duplicates (G1322); no compensation is promised for a sent message. |
| IV — pinned versions, five SDKs | Pass | Pass | Forms, links, OpenAPI and streams follow the pinned version; additive IR/protocol/REST; shared conformance (T026-055..058). |
| V — isolated code, tenant and secrets | Justified | Pass | Anonymous principal kind (ADR-0061) with one capability on its own origin and pool; owner recheck; tokens hashed; Slack tenant only from the decision-link row; no secrets in messages; egress guard for every send (Complexity Tracking rows 1, 3, 4). |
| VI — OTel, audit, typed errors | Pass | Pass | Observability section; audit in the decision transaction; reasons in `contracts/errors.md`. |
| VII — inbox/outbox | Pass | Pass | Notifications are outbox deliveries in the causing transaction (ADR-0025); Slack interactions are admitted to the inbox (ADR-0012). |
| VIII — failure tested honestly | Pass | Pass | Failure matrix and rule-9 tasks (T026-046..050); nothing ticked without evidence. |

## Architecture and crate placement

| Layer | Additions (existing files extended where the semantics belong) |
|---|---|
| `crates/domain` | `src/forms.rs` (form model types, closed reasons, ceilings); `src/streams.rs` (channel declaration, event identity, limits, refusal verdicts); `src/public_links.rs` (link state machine, `LinkBearer` principal kind that has no `authorize(Permission)`); `src/identity.rs` (`Permission::LinkRead`, `LinkManage`; `Subject::public_link(id)` reserved actor); `src/outbound.rs` (`ChatDestination::{Slack, Teams}`, `EmailDestination`, `OutboundKind::{ApprovalRequested, ApprovalSettled}`, `max_classification`); `src/execution/approval.rs` (notify, summary, form declarations, decision input); `src/workflow.rs`/`src/definition.rs` (IR fields `streams`, `emits`, approval `notify`/`summary`/`form`); `src/quota.rs` (five resources). |
| `crates/application` | `src/forms.rs` (form service: version → carrier → model); `src/invocation.rs` (execute/outcome use cases, `RunWaiters` port); `src/streams.rs` (append and read use cases, ports); `src/public_links.rs` (management and submission use cases); `src/approval_progress.rs` (opening with summary, links and fan-out; decision with input); `src/approval_links.rs` (decision-link tokens, keys, resolution); `src/notifications.rs` (channel management, rendering inputs); `src/chat_identity.rs` (P3); `src/authz.rs` (matrix). |
| `crates/infra-schema` | form-model walker next to `src/classification/walk.rs`; `x-sagat-form` validation; per-version OpenAPI generator (`src/openapi.rs`). |
| `crates/infra-ir` | codec/validation of the new IR fields and capability gates (ADR-0032); summary expression checks with the existing evaluator. |
| `crates/infra-postgres` | migrations (`<next>_public_links`, `<next>_workflow_streams`, `<next>_approval_channels`, `<next>_notification_channels`, `<next>_slack_apps`, `<next>_chat_identity`), stores for each port, the code-consumption and purge `SECURITY DEFINER` functions, storage-classification entries. |
| `crates/infra-connectors` | `src/chat.rs` (Slack `auth.test`, `chat.postMessage`/`chat.update`, Teams Workflows webhook, through `guard.rs`); `src/email.rs` (SMTP, TLS required, egress guard); destination dispatch in `src/publish.rs`. |
| `crates/scheduler` | `src/deliveries.rs` renders and sends the new destinations through the existing claim/attempt fence and the guarded egress client (fixed-shape platform deliveries, ADR-0055 rule (b)); `src/monitor.rs` adds the alerts `PUBLIC_LINK_OWNER_UNAUTHORIZED` and `NOTIFICATION_TARGET_SKIPPED`; stream purge pass in `src/passes.rs` or the maintenance loop. |
| `crates/api-http` | `src/routes/forms.rs`, `src/routes/invocation.rs`, `src/routes/streams.rs`, `src/routes/public_links.rs` (management), `src/routes/public.rs` (anonymous: form, submission, receipt; a separate router with no bearer extractor, mounted only for requests whose `Host` is the public-forms origin), `src/public_site.rs` (serves the public build under `/f/{embedId}` on that origin with the per-link `frame-ancestors` header; `'none'` for unknown ids and unusable links; origins and CSP per ADR-0062 of feature 027, C15), `src/routes/notifications.rs` (channels and Slack app bindings), `src/routes/chat.rs` (P3: `/v1/chat/slack/apps/{bindingId}/requests`); `src/streams.rs` (separate counters for stream-event subscriptions and waiters); `src/studio.rs` (unchanged CSP; answers `/f/` and `/v1/public/*` with `404` on the Studio origin). |
| `crates/api-grpc` | `AppendStreamEvents` in `src/service.rs`, limits in `src/limits.rs`. |
| `crates/telemetry` | span and metric names below. |
| `apps/control-plane` | waiter registry wiring (subscriptions to `sagat.v1.run.finished.*` and feature 022's `run.result_returned` hint without a queue group on every replica); link key configuration (`SAGAT_LINK_KEYS`, a file of versioned 32-byte keys mounted only into the control plane, used with distinct labels for decision links and receipt tokens; an entry of feature 032's cell key inventory, C11); `SAGAT_PUBLIC_FORMS_ORIGIN` (start-up refuses it when it equals the Studio origin; it may share a host with the feature-027 apps origin); trusted proxy networks for the public rate guard. |
| `apps/sandbox-runner`, `runners/` | the supervisor serves feature 023's reserved, capability-gated `stream.append` host call (C17) by calling `AppendStreamEvents` under the attempt's lease (G1332). |
| `apps/cli` | `run execute`, `run outcome`, `run stream`, `workflow form`, `workflow openapi`, `link create/list/pause/resume/rotate/revoke` (with `--max-concurrent-runs`), `channel create/test`, `slack-app register/list`. |
| `frontend/apps/studio` | `src/features/forms/` (renderer), `src/features/runs/StartRun.tsx` (form by default, JSON fallback), `src/features/runs/StreamsTab.tsx`, `src/features/approvals/DecidePage.tsx`, `src/features/public-links/`, `src/features/integrations/` (channels, Slack app bindings, identity-link confirmation), a second build entry `src/public/` served only on the public-forms origin. |
| `sdks/*` | client operations, worker publishing, builder fields, conformance runners. |

## Transaction boundaries

| # | Operation | Commits together | Never in the transaction |
|---|---|---|---|
| T1 | Link management | link row, revision, management receipt, audit, `public_links` count | token plaintext (returned once after commit) |
| T2 | Public submission | before it, in its own committed decision (as ADR-0027 API rate admission): debit of the durable per-link and per-namespace buckets after an unlocked lookup of the link by token hash. Then, in the slots of ADR-0050 decision 7: slot 1 — link row `FOR UPDATE` (state, expiry, submission counter, exact live-run count of the link) and the read-only 029 access check; slot 2 — submission receipt (derived token hash); slot 4a — owner membership `FOR SHARE` (017 helpers) → 029 access fences → namespace row; slots 4b–5 as any admission; slot 6 — run rows, whose active-run admission also takes the exact `public_link_active_runs` count under the feature-009 namespace lock (run with origin `PUBLIC_LINK`, version pin, input payload, first tasks, journal, outbox, `run.started` and `public_link.submitted` audit); slot 8b — usage rows last | the process rate guard; schema validation (repeated against the locked revision inside) |
| T3 | `executeRun` admission | exactly the `startRun` transaction and its receipt | the wait (after commit, holding nothing) |
| T4 | Stream append batch | lease check under the step row `FOR SHARE` → head `FOR UPDATE` → per event: dedupe lookup, channel budget, pool and per-run ceiling, event row with inline data, channel usage and pool counters, head advance → deferred byte accounting | run row lock, journal, outbox, `payloads` |
| T5 | Approval opening (any owner kind, including 025 `AGENT_TOOL_CALL`) | existing opening (request, routes, timer, journal, audit, outbox) + summary payload + one delivery per active target + one decision link per delivery + `notification.skipped` audits; none of the notification rows for a draft test run (022 `DRAFT_TEST`) | provider sends |
| T6 | Decision (any path) | existing decision (receipt with server-derived `via`, route, timer cancel, journal, audit, outbox) + input payload + `approval.settled` deliveries + decision links closed | the link lookup that resolved the approval (read-only, before) |
| T7 | Expiry / cancellation | existing expiry or cancellation + `approval.settled` deliveries + links closed | — |
| T8 | Delivery attempt | ADR-0025: claim under rate slot, attempt row, next state, `provider_ref`, health, auto-disable audit; for a delivery carrying a `pii` field, the recheck of `classification_set_by`'s reveal grant and its audit, or the channel's pause (`PII_GRANT_REVOKED`) | the provider call itself (between claim and record, fenced by attempt number), sent by the engine through the guarded egress client with only the channel's credential (ADR-0055 rule (b), C3) |
| T9 | Fan-out of `run.finished`/alerts | existing fact transaction + deliveries for matching channels (filters evaluated in SQL) | rendering |
| T10 | Slack interaction (P3) | inbox admission keyed by binding + Slack action id (binding found by the path id, signature verified with its secret before); then dispatcher: decision-link row → tenant/namespace, check against the binding's tenant, team and channel, identity lookup inside that tenant, T6 as the linked subject + `event_deliveries` effect row | the signature check (before); any tenant taken from the payload |
| T11 | Stream purge | head lock, events (with their data), channel usage and head delete, byte release (one run per transaction) | — |
| T12 | Slack app binding registration | after an `auth.test` call outside the transaction: binding row with the returned team id, audit, `slack_app_bindings` count; an interactive channel write checks the global `(team, channel)` index | the bot token value |
| T13 | Identity link (P3) | code issue: code hash, audit. Slash command (dispatcher): consume the code through the SECURITY DEFINER function (single use, team check) + pending link + audit. Confirmation (subject's signed-in session): pending → active + audit | the code plaintext after it is shown |

Lock order: admissions follow ADR-0050 decision 7; this feature contributes the public-link row to slot
1, the owner recheck to slot 4a and the public-link pool count to slot 6's active-run admission, with no
deviation (it was listed in 018's G1605 before this alignment). Transitions on an existing run keep
"run row first": channel rows are read `FOR KEY SHARE` after the approval request row, and an append
never takes a run lock. Database clock is sampled after each guarding lock (the 014/017 post-lock rule);
instants follow feature 028's time-class catalog (C13).

## Authorization matrix

| Operation | Permission and checks | admin | developer | operator | viewer | worker / runner |
|---|---|:-:|:-:|:-:|:-:|:-:|
| Get form model, workflow OpenAPI | `definition:read` | ✓ | ✓ | ✓ | ✓ | – |
| `executeRun` | `run:start` + `run:read` (outcome) | ✓ | ✓ | ✓ | – | – |
| `getRunOutcome` | `run:read`; result per feature 016 | ✓ | ✓ | ✓ | ✓ (public fields) | – |
| Read stream events | `run:read`; data per feature 016; `pii` with grant + `reveal` | ✓ | ✓ | ✓ | ✓ (public fields) | – |
| Append stream events | current lease of a node that declares the channel | – | – | – | – | ✓ (lease) |
| List/read public links | `link:read` | ✓ | ✓ | ✓ | – | – |
| Create/edit/pause/rotate/revoke links | `link:manage` + owner must hold `run:start`; quota | ✓ | – | – | – | – |
| Submit through a link | link bearer (ADR-0061); owner recheck | anonymous | | | | |
| Read a receipt | receipt bearer; public fields only when declared | anonymous | | | | |
| Manage notification channels and Slack app bindings | `event:manage` + `secret:use` for the bindings | ✓ | ✓ | – | – | – |
| Set `maxClassification` to `internal` | `event:manage` + `payload:read-internal` | ✓ | ✓ | – | – | – |
| Set `maxClassification` to `pii` | `event:manage` + `payload:read-internal` + namespace `payload:reveal-pii` grant | grant | grant | – | – | – |
| Read channels, deliveries | `event:read` | ✓ | ✓ | ✓ | ✓ | – |
| Resolve a decision link | signed-in member of the link's tenant with `approval:read` | ✓ | ✓ | ✓ | ✓ | – |
| Decide (page, API, Slack) | `approval:decide` + FR-014-008 eligibility at commit | ✓ | ✓ | ✓ | – | – |
| Issue a link code, confirm or revoke a chat identity link | the signed-in subject itself, with `approval:decide`; confirmation names the exact Slack user and workspace | ✓ | ✓ | ✓ | – | – |
| Slack interaction or slash command | binding signature (path id) + decision-link row tenant = binding tenant; then the linked subject's own checks | Slack | | | | |

Every check is repeated on receipt replay; a foreign resource is an opaque `NOT_FOUND`; denials go
through the existing restricted denial audit. When feature 029 is enabled, visibility of the workflow
behind a public link (at each submission, for the owner) and behind a decision link (at resolution, for
the caller) is decided by the 029 evaluator (coordinator decision C10).

## Finite ceilings

Proposed defaults and hard ceilings; design bounds to verify (SC-026-012), not capacity promises.

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Form carrier schema size / fields / nesting / choices per field | 64 KiB / 128 / 4 / 100 | 64 KiB / 256 / 8 / 1,000 |
| `x-sagat-form.placeholder` | 120 chars | 120 chars |
| `executeRun` / `getRunOutcome` wait (`run_wait_ms`) | 30 s | 60 s |
| Waiters per subject / tenant / process | 20 / 200 / 2,000 | 100 / 1,000 / 10,000 |
| Batched waiter poll interval | 500 ms | ≥ 250 ms |
| Stream channels per version / `emits` per node | 8 / 4 | 32 / 8 |
| Stream channel schema | 16 KiB | 64 KiB |
| Event data (canonical) | 16 KiB | 64 KiB |
| Events per batch / batch bytes | 64 / 64 KiB | 256 / 256 KiB |
| Author channel budget (`budget` of one channel) | 5,000 events / 2 MiB | 100,000 / 32 MiB |
| Author pool (all author channels of a run) | 10,000 events / 4 MiB | 100,000 / 64 MiB |
| Reserved pool (all `sagat.` channels of a run; budgets fixed by the owning feature, e.g. 025) | 65,536 events / 32 MiB | 131,072 / 64 MiB |
| Per-run hard ceiling over both pools | — | 262,144 events / 128 MiB (and `tenant_content_bytes`) |
| Appends per attempt per second | 20 | 100 |
| Stream retention after run end (`stream_retention_seconds`) | 7 days | 30 days, never beyond the run |
| Stream subscriptions per subject / tenant / process | 20 / 200 / 2,000 | as graph streams, separate counters |
| Stream page size | 200 | 1,000 |
| Public links per namespace (`public_links`) | 0 | 1,000 |
| Link lifetime | 30 days | 365 days |
| Submissions per link (cap) | none | 1,000,000 |
| Live runs per link (`maxConcurrentRuns`, required) | 10 | 100 |
| Live public-link runs per namespace (`public_link_active_runs`, within `active_runs`) | 20 | the namespace's `active_runs` limit |
| Submissions per link per minute | 10 | 600 |
| Submissions per namespace per minute | 60 | 6,000 |
| Submission body | 64 KiB | 256 KiB (run input limit) |
| Exposed fields / presets bytes / embed origins | 64 / 4 KiB / 0 | 128 / 16 KiB / 8 |
| Receipt reads per receipt per minute | 30 | 120 |
| Submission receipt retention | 30 days | 90 days, never beyond the run |
| `notify` targets per approval / summary fields / summary bytes | 2 / 8 / 4 KiB | 4 / 16 / 16 KiB |
| Approval form schema / input | 16 KiB / 16 KiB | 64 KiB / 64 KiB |
| Notification channels per namespace | 20 | 200 |
| E-mail recipients per channel | 10 | 50 |
| Rendered message (Slack text / Teams card / e-mail) | 4 KiB / 16 KiB / 32 KiB | 8 KiB / 28 KiB / 64 KiB |
| Channel rate | 1 message/s (Slack per channel) | ADR-0025 range 1–100/s |
| Workflow filter names per channel | 8 | 32 |
| Slack app bindings per namespace | 2 | 10 |
| Chat identity links per subject / code lifetime / code entropy | 2 / 10 min / 128 bits | 8 / 10 min / ≥ 128 bits |
| Link codes issued per subject per hour / slash-command code attempts per Slack team per minute | 5 / 10 | 20 / 60 |
| Decision-link validity | min(approval timeout, 30 days) | 30 days |
| Workflow OpenAPI document | 1 MiB | 4 MiB |

Effective value = min(request, namespace, deployment). Zero is not unlimited; count zero refuses new
creation. Admission fails before any work. Existing run, payload, content and API rate quotas also apply.

## Failure and recovery matrix

| Failure | Expected state | Handling and test |
|---|---|---|
| Crash before T2/T3 commit | nothing exists | client retries with the same key; new admission (T026-047) |
| Crash after T2/T3 commit, before response | run and receipt exist | same key replays receipt/run; `executeRun` waits again |
| Control plane restarts during a wait | run unaffected | connection error; retry with key returns the same run (T026-048) |
| `run.finished` hint lost | waiter not woken by hint | batched poll observes terminal status within one interval (T026-048) |
| Early return committed (022) | no `run.finished` | batched poll observes the early result within one interval (T026-065, G1334) |
| Waiter on replica A, hint consumed by replica B | — | impossible by design: every replica subscribes without a queue group (T026-048) |
| Wait deadline and terminal commit in the same instant | either `200` or `202` | both correct; no state change (T026-047) |
| Stream batch response lost | events stored | replay with same publisher sequence returns offsets (T026-046) |
| Lease expires mid-batch | append refused | `STALE_ATTEMPT`, nothing stored; next attempt appends with new generation (T026-046) |
| Append races completion | first commit wins | later append stale (barrier test, T026-046) |
| Channel budget, a pool or the per-run ceiling reached | append refused | `QUOTA_EXCEEDED`/`STREAM_LIMIT` naming the scope; other pool unaffected; step continues (T026-021) |
| Newer attempt starts while a subscriber is connected | events of the old attempt delivered | `superseded` control event; on resume the `attempts` snapshot (T026-022) |
| Owner demoted during submission | depends on commit order | demotion first: refused, alert; submission first: admitted, next refused (T026-031) |
| Link revoked during submission | same as above | lock order link row → membership (T026-031) |
| Link at its live-run cap or namespace pool full | refused before admission | `429`; members' remaining `active_runs` capacity untouched (T026-031) |
| Public page framed by another tenant's approved origin | browser refuses | per-link `frame-ancestors` from the embed id (T026-033) |
| Channel missing or paused at approval opening | approval opens | `notification.skipped` audit + alert (T026-038) |
| Summary evaluation fails | approval opens without summary | closed failure code recorded (T026-038) |
| Decision races expiry | first commit wins | the loser is an audited no-op; links closed once (T026-047) |
| Decision with input replayed / changed | receipt / conflict | `request_hash` covers input (T026-040) |
| Decision without input retried across the upgrade | replay | fingerprint unchanged when no input (compatibility test, T026-040) |
| Provider acknowledged, record lost | delivery retried | duplicate message stated; same delivery id (T026-049) |
| Provider credential unavailable or 5xx | retry | ADR-0025 schedule → `DEAD` → redrive; auto-disable (T026-049) |
| Decision-link key rotated | old links | verified against current and previous key; older keys refuse (T026-039) |
| Slack interaction replayed or delayed | one decision | inbox dedupe; stale signature `401` (T026-045) |
| Button value naming another tenant's decision link | nothing decided | tenant from the link row must equal the binding's tenant, team and channel; audited in the binding's tenant (T026-045) |
| Second tenant registers the same interactive Slack channel | refused | global `(team, channel)` index, `CHANNEL_ALREADY_BOUND` (T026-044) |
| Stolen identity-link code | no active link | single-use consumption, team check, per-team rate, subject confirmation in the Studio (T026-044) |
| NATS down | no hints | polling paths serve waiters and streams; outbox catches up (T026-048) |
| Cell failover (feature 032) | offsets of the lost tail may be reused | cursor epoch differs → `event: reset`, reader restarts from the snapshot (T026-022) |
| Reset of a run (feature 018) | new run | stream log not copied; reopened approvals notify through a new opening (T026-068) |
| Reveal grant behind a `pii` channel revoked | channel paused | next `pii` delivery refused, channel `PAUSED` with `PII_GRANT_REVOKED`, audited (T026-049) |
| PostgreSQL unavailable | no admission | `503 UNAVAILABLE`, retryable, nothing confirmed |
| Failed compensation of a run started by a form or link | as feature 003 | unchanged; no new compensation path; a sent message is never "undone" |

## Observability

Spans (OTLP through `crates/telemetry`, attributes `sagat.tenant.id`, `sagat.namespace.id`,
`sagat.run.id`, `sagat.outcome` and the closed reason; never a token, address, value or subject):
`invocation.execute`, `invocation.wait`, `form.model`, `workflow.openapi`, `public_link.submit`,
`public_link.receipt`, `stream.append` (gRPC), `stream.subscribe`, `approval.notify` (opening fan-out),
`approval.link.resolve`, `notification.deliver` (child of the existing delivery attempt span),
`chat.interaction` (P3).

Metrics (fixed labels only):

- `run_wait_requests_total{outcome}` (`terminal`, `early_return`, `pending`, `refused`),
  `run_wait_duration_seconds{outcome}`, `run_waiters_open` (gauge), `run_waiter_wakeups_total{source}`
  (`hint`, `poll`).
- `form_model_requests_total{status}` (`available`, `unavailable`).
- `workflow_openapi_documents_total{outcome}`.
- `public_link_submissions_total{outcome}` (`accepted`, `replayed`, `refused_rate`, `refused_state`,
  `refused_validation`, `refused_quota`, `refused_capacity`, `refused_owner`),
  `public_link_receipt_reads_total{outcome}`.
- `chat_requests_total{outcome}` (`verified`, `bad_signature`, `stale`, `tenant_mismatch`,
  `unlinked`, `decided`), `chat_link_codes_total{outcome}` (`issued`, `consumed`, `expired`,
  `refused`, `confirmed`).
- `stream_appends_total{outcome}` (`accepted`, `replayed`, `stale`, `invalid`, `limit`, `rate`),
  `stream_budget_refusals_total{scope}` (`channel`, `author_pool`, `reserved_pool`, `run`),
  `stream_cursor_resets_total` (epoch mismatch),
  `stream_events_appended_total`, `stream_append_bytes_total`, `stream_subscriptions_open` (gauge),
  `stream_delivery_lag_seconds` (append to send).
- `approval_notifications_total{outcome}` (`queued`, `skipped`), `approval_link_resolutions_total{outcome}`
  (`active`, `settled`, `expired`, `invalid`).
- `notification_messages_total{destination,outcome}` (`slack`, `teams`, `email` × the ADR-0025 attempt
  outcomes).

Audit actions (in the transaction of the decision): `public_link.created`, `.updated`, `.paused`,
`.resumed`, `.rotated`, `.revoked`, `public_link.submitted`, `public_link.refused` (closed reason,
collapsed per link and reason for 300 s); `notification_channel.created`, `.updated`, `.deleted`,
`.classification_raised`, `.paused_pii_grant_revoked`, `notification.pii_delivered` (per delivery
carrying a `pii` field), `notification.skipped`; `slack_app.registered`, `.reverified`, `.invalid`;
`approval.decided` gains the server-derived `via`; `payload.pii_revealed` for outcome, stream and
decision-page reveals (feature 016 rules); `chat_identity.code_issued`, `.pending`, `.confirmed`,
`.unlinked`, `chat.refused` (closed reason, collapsed per binding for 300 s); existing quota, secret
and run audits unchanged.

Alerts (engine monitor, ADR-0017): `PUBLIC_LINK_OWNER_UNAUTHORIZED`, `NOTIFICATION_TARGET_SKIPPED`;
existing dead-items and outbox alerts cover channel deliveries. Runbook: `docs/runbooks/invocation.md`
(new) and additions to `events.md`, `workflow-primitives.md` and `payload-classification.md`.

## Migration and compatibility plan

1. Contracts first (T026-005..008): OpenAPI paths and schemas (additive), IR fields (additive in v1,
   ADR-0032), worker proto minor (additive), AsyncAPI outbound kinds (grow only, baseline regenerated
   after review), error reasons (no new `Error.code`).
2. Migrations are additive; `CHECK` constraints are widened, never narrowed; down migrations refuse
   while new-kind subscriptions, links, stream events or decision links exist.
3. Capability gates: publication refuses `streams`, `emits`, `notify`, `summary`, `form` and
   `x-sagat-form` until the corresponding capability is enabled on the server; an older server cannot
   publish them; an older server is not a safe reader of runs that used them (ADR-0032 decision 8), so
   rollback disables new publication and starts while keeping readers.
4. Runs in flight are unaffected: no existing approval gains notifications retroactively; existing
   subscriptions keep their kinds; the graph stream and `startRun` are unchanged; a decision request
   without `input` keeps today's `request_hash` byte for byte, so a decision retried across the upgrade
   replays (compatibility test in T026-040).
5. SDKs: new operations behind a server-version check (`ROUTE_NOT_FOUND` means an older server).
6. Rollout order: forms → execute/outcome → streams → channels → approval channels → public links →
   OpenAPI → Slack in-chat (P3). Public links ship disabled (quota 0) and need the public-forms origin
   configured; feature 016's carrier list (FR-016-002) is amended in the same delivery (T026-067).

## Complexity Tracking

| Violation or added complexity | Why needed | Simpler alternative rejected because |
|---|---|---|
| A second principal kind (anonymous link bearer) beside `Principal` | Anonymous intake must not become a subject, a role or a permission holder (constitution V) | A restricted service-account subject could be granted more and confused with people (ADR-0061 option 3) |
| A stream log beside the journal (ADR-0068) | High-rate non-state events must not grow the journal or the graph stream | Journal events, JetStream and blobs each break an invariant (ADR-0068 options 2–5) |
| A second build entry served on a second origin | The public page must carry no shell, sign-in or authenticated code, and per-link framing needs its own response path | Reusing the Studio origin allows cross-tenant framing and same-origin scripting of the Studio (ADR-0061 option 5) |
| A global unique index across tenants for interactive Slack channels | One Slack channel must not decide for two tenants | Tenant-scoped uniqueness leaves the cross-tenant ambiguity of ADR-0061 option 6 |

## Delivery sequence

1. Phase 1–2 (Setup, Foundational): contracts, permissions, quota resources, migrations, domain types,
   capability gates.
2. US01 forms (P1) → US02 execute/outcome (P1) → US03 streams (P1): MVP.
3. US06 channels (P2) → US05 approval channels (P2) → US04 public links (P2) → US07 OpenAPI (P2).
4. US08 Slack in-chat decisions (P3).
5. Polish and gates: fault suites, security battery, SDK conformance, Studio journeys, runbooks,
   benchmark, contract merge.

## Verification plan

Every checked task names its test and result (AGENTS.md rules 12 and 17). Required evidence: the
rule-9 set — a crash at every commit point of T1–T11, duplicate submissions/appends/decisions/
interactions, the decision-versus-expiry race and the wait-deadline race, lost `run.finished` hints,
restart during waits and streams, tenant isolation over every new operation (battery), quota and
ceiling exhaustion at and past each limit, failed compensation unchanged for runs started through the
new surfaces; canary values at each level through outcomes, streams, receipts, summaries, messages,
logs and spans; five-SDK conformance; real-browser journeys with WCAG checks; the benchmark report with
environment and failures. A skipped or failing test is reported as such; no number is invented.
