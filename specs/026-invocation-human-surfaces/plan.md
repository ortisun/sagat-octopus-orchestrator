# Implementation Plan: Invocation and human-interaction surfaces

**Branch**: `feature/026-invocation-human-surfaces` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: Proposed — not started. Revised on 2026-10-26 after the independent architecture and
security reviews (findings applied are listed in research.md under the revised decisions). Re-baselined
on 2026-11-24 against `develop` at `2c94064f` for release 0.3.0: the stale items and the decisions D026-001
to D026-017 are in research.md ("Re-baseline against `develop`"); this plan carries them in place (marked
"revised 2026-11-24") and in four new sections at the end: Run creators, Boundaries with other work,
Fleet capabilities and Build slices (2026-11-24).

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

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, NATS client, the platform validator of
design 043 (`crates/infra-schema/src/run_input/`, with its work counter and permits) for every value this
feature validates, never the `jsonschema` crate (revised 2026-11-24, Decision 026-003), `hmac`/`sha2`. New adapter dependency, only in `crates/infra-connectors`: an SMTP
client (candidate `lettre`, pinned, TLS-only, no default features) — selected and pinned in T026-004.
Slack and Teams use the existing guarded HTTP client. No new dependency in `crates/domain`.

**Storage**: PostgreSQL is the authority for links, receipts, stream heads and events, decision links,
channels and deliveries (ADR-0002); payload bytes in the existing content-addressed `payloads` table;
NATS carries only the existing `run.finished` hint.

**Testing**: Rust unit and property tests; integration and fault suites on real PostgreSQL
(Testcontainers) as `integration_invocation_human_surfaces` and `fault_invocation_human_surfaces`;
`contract_rest_invocation_human_surfaces`, `contract_grpc_invocation_human_surfaces`,
`contract_asyncapi_events_webhooks_messaging` (baseline growth); `e2e_invocation_human_surfaces` with a
real browser; six-language `sdk_conformance` scenarios (Rust joined, revised 2026-11-24); the security battery; the runtime-role sweep;
a performance harness under `tests/performance/invocation-human-surfaces/`. Local Slack, Teams and SMTP
stand-ins (the existing `tests/support/receiver.rs` pattern and an SMTP sink container pinned in the
test fixtures); real-provider smoke tests are optional and never the only evidence.

**Target Platform**: existing single-region Linux containers, Compose and Helm profiles.

**Project Type**: modular monolith with six SDKs and the Studio (revised 2026-11-24).

**Performance Goals**: measure, on a declared environment, waiter wake-up latency (hint and poll paths),
the batched poll's database cost at 2,000 waiters, stream append throughput per run and publish-to-SSE
latency, public submission refusal cost before the database. No latency is promised (NFR-026-006).

**Constraints**: nothing waits inside a transaction; no user code in the control plane (summary and
form logic use the existing bounded evaluator and the JSON Schema adapter); tenant from authentication
or the stored link only; typed errors from the closed set; telemetry without values, tokens or
addresses.

**Scale/Scope**: 52 functional requirements (51 and 52 added on 2026-11-24), 8 user stories, 2 ADRs
(0061 and 0068, accepted with amendments on 2026-11-24 under Owner decision 2026-11-24-4).

## Constitution Check

Design checks, not approval of an implementation. Re-evaluated after Phase 1 design: unchanged.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | Transaction table below; submission, append, opening, decision and fan-out each one transaction with audit/outbox; terminal runs never reopen; first commit wins (T026-047). |
| II — pure domain, inward dependencies | Pass | Pass | Domain types only (form model, stream limits, link state, destinations); SMTP/HTTP clients only in `infra-connectors`; `check-architecture.py` (T026-062). |
| III — at-least-once effects | Pass | Pass | Chat/e-mail sends at-least-once with stable delivery id and stated duplicates (G1322); no compensation is promised for a sent message. |
| IV — pinned versions, six SDKs | Pass | Pass | Forms, links, OpenAPI and streams follow the pinned version; additive IR/protocol/REST; shared conformance in six languages (T026-055..058); fleet capabilities through the instance registry so that no process of an older release reads an IR field it does not know (revised 2026-11-24). |
| V — isolated code, tenant and secrets | Justified | Pass | Anonymous principal kind (ADR-0061) with one capability on its own origin and pool; owner recheck; tokens hashed; Slack tenant only from the decision-link row; no secrets in messages; egress guard for every send (Complexity Tracking rows 1, 3, 4). |
| VI — OTel, audit, typed errors | Pass | Pass | Observability section; audit in the decision transaction; reasons in `contracts/errors.md`. |
| VII — inbox/outbox | Pass | Pass | Notifications are outbox deliveries in the causing transaction (ADR-0025); Slack interactions are admitted to the inbox (ADR-0012). |
| VIII — failure tested honestly | Pass | Pass | Failure matrix and rule-9 tasks (T026-046..050); nothing ticked without evidence. |

## Architecture and crate placement

| Layer | Additions (existing files extended where the semantics belong) |
|---|---|
| `crates/domain` | `src/forms.rs` (form model types, closed reasons, ceilings); `src/streams.rs` (channel declaration, event identity, limits, refusal verdicts); `src/public_links.rs` (link state machine, `LinkBearer` principal kind that has no `authorize(Permission)`); `src/identity.rs` (`Permission::LinkRead`, `LinkManage`; `Subject::public_link(id)` reserved actor); `src/outbound.rs` (`ChatDestination::{Slack, Teams}`, `EmailDestination`, `OutboundKind::{ApprovalRequested, ApprovalSettled}`, `max_classification`); `src/execution/approval.rs` (notify, summary, form declarations, decision input); `src/workflow.rs`/`src/definition.rs` (IR fields `streams`, `emits`, approval `notify`/`summary`/`form`); `src/quota.rs` (five resources). |
| `crates/application` | `src/forms.rs` (form service: version → carrier → model); `src/invocation.rs` (execute/outcome use cases, `RunWaiters` port); `src/streams.rs` (append and read use cases, ports); `src/public_links.rs` (management and submission use cases); `src/approval_progress.rs` (opening with summary, links and fan-out; decision with input); `src/approval_links.rs` (decision-link tokens, keys, resolution); `src/notifications.rs` (channel management, rendering inputs); `src/chat_identity.rs` (P3); `src/authz.rs` (matrix). |
| `crates/infra-schema` | form-model walker (`src/forms.rs`) over the schema positions of the design-043 pre-check (`src/run_input/vocabulary.rs`) and the classification walker; form hint validation (`inputFormHints`, `form.hints`); per-version OpenAPI generator (`src/openapi.rs`); stream event and decision input validation through `InputSchemas` (revised 2026-11-24). |
| `crates/infra-nats` | a core-NATS subscriber adapter for the `RunOutcomeHints` port (`sagat.v1.run.finished.*`, later `sagat.v1.run.result_returned.*`), no queue group; the crate publishes only today (added 2026-11-24, Decision 026-005). |
| `crates/infra-ir` | codec/validation of the new IR fields and capability gates (ADR-0032); summary expression checks with the existing evaluator. |
| `crates/infra-postgres` | migrations (`<next>_public_links`, `<next>_workflow_streams`, `<next>_approval_channels`, `<next>_notification_channels`, `<next>_slack_apps`, `<next>_chat_identity`), stores for each port, the code-consumption and purge `SECURITY DEFINER` functions, storage-classification entries. |
| `crates/infra-connectors` | `src/chat.rs` (Slack `auth.test`, `chat.postMessage`/`chat.update`, Teams Workflows webhook, through `guard.rs`); `src/email.rs` (SMTP, TLS required, egress guard); destination dispatch in `src/publish.rs`. |
| `crates/scheduler` | `src/deliveries.rs` renders and sends the new destinations through the existing claim/attempt fence and the guarded egress client (fixed-shape platform deliveries, ADR-0055 rule (b)); `src/monitor.rs` adds the alerts `PUBLIC_LINK_OWNER_UNAUTHORIZED` and `NOTIFICATION_TARGET_SKIPPED`; stream purge pass in `src/passes.rs` or the maintenance loop. |
| `crates/api-http` | `src/routes/forms.rs`, `src/routes/invocation.rs`, `src/routes/streams.rs`, `src/routes/public_links.rs` (management), `src/routes/public.rs` (anonymous: form, submission, receipt; a separate router with no bearer extractor, mounted only for requests whose `Host` is the public-forms origin), `src/public_site.rs` (serves the public build under `/f/{embedId}` on that origin with the per-link `frame-ancestors` header; `'none'` for unknown ids and unusable links; origins and CSP per ADR-0062 of feature 027, C15), `src/routes/notifications.rs` (channels and Slack app bindings), `src/routes/chat.rs` (P3: `/v1/chat/slack/apps/{bindingId}/requests`); `src/streams.rs` (separate counters for stream-event subscriptions and waiters); `src/studio.rs` (unchanged CSP; answers `/f/` and `/v1/public/*` with `404` on the Studio origin). |
| `crates/api-grpc` | `AppendStreamEvents` in `src/service.rs`, limits in `src/limits.rs`. |
| `crates/telemetry` | span and metric names below. |
| `apps/control-plane` | waiter registry wiring (the `crates/infra-nats` subscriber to `sagat.v1.run.finished.*` and feature 022's `run.result_returned` hint without a queue group on every replica; gate wiring with `.with_run_inputs` for the new routes, design 043); link key configuration (`SAGAT_LINK_KEYS`, a file of versioned 32-byte keys mounted only into the control plane, used with distinct labels for decision links and receipt tokens; an entry of feature 032's cell key inventory, C11); `SAGAT_PUBLIC_FORMS_ORIGIN` (start-up refuses it when it equals the Studio origin; it may share a host with the feature-027 apps origin); trusted proxy networks for the public rate guard. |
| `apps/sandbox-runner`, `runners/` | the supervisor serves feature 023's reserved, capability-gated `stream.append` host call (C17) by calling `AppendStreamEvents` under the attempt's lease (G1332). |
| `apps/cli` | `run execute`, `run outcome`, `run stream`, `workflow form`, `workflow openapi`, `link create/list/pause/resume/rotate/revoke` (with `--max-concurrent-runs`), `channel create/test`, `slack-app register/list`. |
| `frontend/apps/studio` | `src/features/forms/` (renderer), `src/features/runs/StartRun.tsx` (form by default, JSON fallback, keeping its business identifier and start delay fields), `src/features/runs/StreamsTab.tsx`, `src/features/approvals/DecidePage.tsx`, `src/features/public-links/`, `src/features/notifications/` (channels, Slack app bindings, identity-link confirmation; revised 2026-11-24: not `features/integrations/`, the connector page that feature 021 extends), a second build entry `src/public/` served only on the public-forms origin; all on the Astryx components after pull request 267 merges (Decision 026-015). |
| `sdks/*` | client operations, worker publishing, builder fields, conformance runners. |

## Transaction boundaries

| # | Operation | Commits together | Never in the transaction |
|---|---|---|---|
| T1 | Link management | link row, revision, management receipt, audit, `public_links` count | token plaintext (returned once after commit) |
| T2 | Public submission | before it, in its own committed decision (as ADR-0027 API rate admission): debit of the durable per-link and per-namespace buckets after an unlocked lookup of the link by token hash; then the run input gate of design 043 on the merged input of the unlocked revision (revised 2026-11-24, Decision 026-017; the transaction refuses another revision and the use case repeats once). Then, in the slots of ADR-0050 decision 7: slot 1 — link row `FOR UPDATE` (state, expiry, submission counter, exact live-run count of the link) and the read-only 029 access check; slot 2 — submission receipt (derived token hash); slot 4a — owner membership `FOR SHARE` (017 helpers) → 029 access fences → namespace row; slots 4b–5 as any admission; slot 6 — run rows, whose active-run admission also reads `public_link_active_runs` from `public_link_active_counts` under the feature-009 namespace lock (run with `public_link_id` set, revised 2026-11-24, Decision 026-004; version pin, input payload, first tasks, journal, outbox, `run.started` and `public_link.submitted` audit); slot 8b — usage rows last | the process rate guard; schema validation (repeated against the locked revision inside) |
| T3 | `executeRun` admission | exactly the `startRun` transaction and its receipt | the wait (after commit, holding nothing) |
| T4 | Stream append batch (revised 2026-11-24, Decision 026-006) | attempt row `FOR UPDATE` (refused when closed) → lease read without a lock, database clock after it → head `FOR UPDATE` → per event: dedupe lookup, channel budget, pool and per-run ceiling, event row with inline data, channel usage and pool counters, head advance → with `final`, close the attempt row → deferred byte accounting of `workflow_stream` | any lock on `runs`, `steps` or `run_events`; the tenant usage row before commit; journal, outbox, `payloads`; the schema validation (before the transaction, under a design-043 permit) |
| T4b | Attempt close by a feed read or the maintenance pass (added 2026-11-24) | `UPDATE run_stream_attempts SET closed_at … WHERE closed_at IS NULL` for an attempt whose committed step state is settled or superseded; it waits for an append holding the row | anything of `steps` or `runs` |
| T5 | Approval opening (any owner kind, including 025 `AGENT_TOOL_CALL`) | existing opening (request, routes, timer, journal, audit, outbox) + summary payload + one delivery per active target + one decision link per delivery + `notification.skipped` audits; none of the notification rows for a draft test run (022 `DRAFT_TEST`) | provider sends |
| T6 | Decision (any path) | existing decision (receipt with server-derived `via`, route, timer cancel, journal, audit, outbox) + input payload + `approval.settled` deliveries + decision links closed | the link lookup that resolved the approval (read-only, before) |
| T7 | Expiry / cancellation | existing expiry or cancellation + `approval.settled` deliveries + links closed | — |
| T8 | Delivery attempt | ADR-0025: claim under rate slot, attempt row, next state, `provider_ref`, health, auto-disable audit; for a delivery carrying a `pii` field, the recheck of `classification_set_by`'s reveal grant and its audit, or the channel's pause (`PII_GRANT_REVOKED`) | the provider call itself (between claim and record, fenced by attempt number), sent by the engine through the guarded egress client with only the channel's credential (ADR-0055 rule (b), C3) |
| T9 | Fan-out of `run.finished`/alerts | existing fact transaction + deliveries for matching channels (filters evaluated in SQL) | rendering |
| T10 | Slack interaction (P3) | inbox admission keyed by binding + Slack action id (binding found by the path id, signature verified with its secret before); then dispatcher: decision-link row → tenant/namespace, check against the binding's tenant, team and channel, identity lookup inside that tenant, T6 as the linked subject + `event_deliveries` effect row | the signature check (before); any tenant taken from the payload |
| T11 | Stream purge (revised 2026-11-24, Decision 026-008) | stamp `terminal_seen_at` on heads of terminal runs; then per run: head lock, events (with their data), attempt rows, channel usage and head delete, byte release | the run's terminal transaction (nothing is written there) |
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
| Form hint `placeholder` (`inputFormHints`, `form.hints`) | 120 chars | 120 chars |
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
| Per-run hard ceiling over both pools | — | 262,144 events / 128 MiB (and `tenant_stream_bytes`, revised 2026-11-24) |
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
   while new-kind subscriptions, links, stream events or decision links exist. Revised 2026-11-24:
   timestamps after the newest on `develop` at merge (`20270119000000` today); every CHECK added `NOT
   VALID` and validated by a later migration (design 043's rule and its static test); the
   `destination_kind` CHECK keeps `REDIS` (feature 033).
3. Capability gates: publication refuses `streams`, `emits`, `notify`, `summary`, `form` and
   `inputFormHints` until the corresponding instance-registry capability is active (revised 2026-11-24,
   section Fleet capabilities below); an older server cannot
   publish them; an older server is not a safe reader of runs that used them (ADR-0032 decision 8), so
   rollback disables new publication and starts while keeping readers.
4. Runs in flight are unaffected: no existing approval gains notifications retroactively; existing
   subscriptions keep their kinds; the graph stream and `startRun` are unchanged; a decision request
   without `input` keeps today's `request_hash` byte for byte, so a decision retried across the upgrade
   replays (compatibility test in T026-040).
5. SDKs: new operations behind a server-version check (`ROUTE_NOT_FOUND` means an older server).
   Six SDKs, Rust included (revised 2026-11-24). The worker-protocol minor is the next free one of the
   register in `sdks/SPEC.md` when this contract merges (1.7 is feature 041's).
5b. Existing operations (added 2026-11-24, Decision 026-014): `decideWorkflowApproval` is marked
   `x-extended: '026'`; the pinning tests of features 016 (`tests/contract/payload-classification/openapi.rs`)
   and 035 (`tests/contract/iceberg-writes/openapi.rs`) gain an exclusion for that mark with the reviewed
   one-operation diff in their comment, never a re-blessed baseline.
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
logs and spans; six-SDK conformance (revised 2026-11-24); real-browser journeys with WCAG checks; the
benchmark report with environment and failures; the hot-path work budget of design 038
(`tests/integration/claim-transition-efficiency/work_budget.rs`) unchanged by every slice (added
2026-11-24: no statement of this feature runs in a lease, a heartbeat or a completion). A skipped or failing
test is reported as such; no number is invented.

## Run creators (2026-11-24)

Every path of this feature that creates a run passes the run input gate of design 043 and, once feature
022's throughput slice is merged, its permit port. The rows, their gate calls and what each caller gets on
each refusal are in research.md, Decision 026-017. In short: a Studio form is `startRun` (design 043 row 1);
`executeRun` is a second entry of row 1, checked before any waiter is registered; a public-link submission
is a new creator (`Creator::PublicLink`, metric label `public_link`) whose refusals keep `field` and `rule`
only inside an exposed pointer; a synchronous custom route stays row 9 (the dispatcher). No notification
channel starts a run. The coordinator adds these rows to design 043's Technical Plan 1 and to feature 022's
"Run creators" table before the code of slices 1 and 7 (documentation first, Owner decision 2026-11-20-8);
each row gets the test that fails when the creator bypasses the gate (design 043 Requirement 043-016) or the
permit port (Task 022-064).

## Boundaries with other work (2026-11-24)

- **Feature 021 (custom HTTP routes).** 021 keeps its asynchronous `202` and the reserved route field
  `response`, refusing `mode: wait` at creation until this feature's capability `synchronous-routes-v1` is
  active. What 026 needs from 021, and nothing more: the reserved field, that refusal, and
  `event_deliveries.run_id` written by the dispatcher in its delivery transaction (it already is, for every
  trigger). Order: 021's routes merge in mode `accept`; 026 builds its waiter (slice 1) independently; the
  route wait (Requirement 026-012, Decision 026-010) is one joint task after both (Task 026-065, slice 11),
  with 021's contract test and 026's waiter test. Channel credentials never reuse 021 resources (ADR-0055
  rule 11b).
- **Design 043 (run input validation).** Every validated value goes through its validator, every new
  creator through its gate (above); its subset is not widened by this feature (hints moved beside the
  schema, Decision 026-001).
- **Design 047 (OAuth resources and curated hub), being written.** Notification channels keep their own
  credentials as secret bindings (ADR-0055 rule 11b) in release 0.3.0; this feature does not depend on 047.
  Weighed for later: the Slack app binding's bot token MAY come from a 047 OAuth connection once 047 offers
  Slack's token rotation, because an installed Slack app is an OAuth grant and rotation is the one thing a
  stored token lacks; the binding keeps its signing secret and its `auth.test`-verified team id, which are
  not OAuth concepts, and the tenant check of Architecture decision 0061 decision 7 does not change. A Teams
  Workflows webhook URL and an SMTP relay credential are not OAuth credentials and stay channel secrets. The
  coordinator should record that boundary in design 047.
- **Feature 041 (step logs).** Separate store and permission (Decision 026-011); its minor 1.7, its hot-path
  rule and its storage-class pattern are followed (Decisions 026-006 and 026-007).
- **Design 044 (workflows written as code).** Requirement 026-051 and Decision 026-016.

## Fleet capabilities (2026-11-24)

Instance-registry capabilities (Architecture decision 0082, `crates/application/src/instances.rs`), each
activated by the operator once every live process of the listed kinds lists it (Decision 026-013):

| Capability | Kinds | Gates | Built in |
|---|---|---|---|
| `form-hints-v1` | control plane, engine | publication of `inputFormHints` and `form.hints` | slice 0a (declared), slice 2 (checked) |
| `workflow-streams-v1` | control plane, engine | publication of `streams` and `emits`; the append rpc | slice 0a, slice 3 |
| `notification-channels-v1` | control plane, engine | creation of `SLACK`, `TEAMS`, `EMAIL` channels | slice 0a, slice 5 |
| `approval-channels-v1` | control plane, engine | publication of `notify`, `summary`, `form` | slice 0a, slice 5 |
| `public-links-v1` | control plane | creation of public links; the anonymous routes | slice 0a, slice 7 |
| `synchronous-routes-v1` | control plane | feature 021's `response.mode: wait` | slice 0a, slice 11 |

## Build slices (2026-11-24)

Two builders, lane A and lane B, run in parallel. Within one window the two lanes own disjoint crates,
apps, SDK directories and Studio paths, so their branches merge without conflicts; ownership of a crate may
pass from one lane to the other between windows. A builder whose slice needs a crate of the other lane in
the same window codes against the port declared in slice 0a and runs its integration tests after the other
slice merges. Each builder keeps its own `CARGO_TARGET_DIR` outside the repository (two heavy builds at most,
AGENTS.md rule 19). Migrations are written in slice 0a only and allocated at merge after the newest on
`develop`. The test crate `sagat-tests` is shared by file: slice 0b declares every module of
`tests/{integration,fault,contract,e2e}/invocation-human-surfaces/main.rs`, and each later slice owns the
files named in its row.

| Window | Slice | Lane | Tasks | Owns (crates, apps, SDKs, Studio, tests) | Migrations and contract files | Depends on |
|---|---|---|---|---|---|---|
| 0 | 0a Foundation of the server | A | T026-009, 010, 011, 012, 013, 070, 071 (migration and ADR-0027 amendment text), 074 (migration) | `crates/domain`, `crates/infra-ir` (codec, publication checks, fixture corpus), `crates/telemetry`, `crates/application/src/instances.rs` and the new port traits only (`invocation.rs`, `streams.rs`, `public_links.rs`, `notifications.rs`, `approval_links.rs`, `chat_identity.rs` as declarations), `crates/infra-postgres/migrations/`, `crates/infra-postgres/storage-classification.json` | every migration of `data-model.md` (`NOT VALID` then validate) | design 043 built; ADR-0061 and ADR-0068 accepted (done 2026-11-24) |
| 0 | 0b Contracts and fixtures | B | T026-002, 003, 004, 005, 006 (the schema file), 007 (the proto file and the protocol document), 008, 076 | `specs/001-platform-foundation/contracts/` (`openapi.yaml`, `workflow.schema.json`, `worker.proto`, `events.asyncapi.yaml`, `events.baseline.json`), `docs/architecture/worker-runtime-protocol.md`, `sdks/error-catalog.json`, `sdks/SPEC.md` (sections and the minor register row), the proto copies under `sdks/*/proto`, `tests/Cargo.toml` and the `main.rs` skeletons, `tests/support/{receiver,smtp}.rs`, `crates/infra-connectors/Cargo.toml` and `Cargo.lock` (the SMTP crate), the two pinning tests of Decision 026-014, `frontend/apps/studio/src/api/` (generated types) | the four contract files; the next free worker minor | none |
| 1 | 1 Invocation server | A | T026-018, 019, 020, 069 (the `executeRun` row) | `crates/application` (`invocation.rs`, `payload_disclosure.rs`, `run_input.rs` for the creator label), `crates/infra-postgres/src` (batched poll in `execution.rs`), `crates/api-http` (`routes/invocation.rs`, `streams.rs` counters), `apps/control-plane`; tests `invocation.rs` | — | slice 0a, 0b; design 043 Technical Plan 1 rows written by the coordinator |
| 1 | 2 Schema-side builders and hints | B | T026-014, 042, 072, 075 (walker and hint validation) | `crates/infra-schema` (`forms.rs`, `openapi.rs`, validation of hints), `crates/infra-nats` (the subscriber adapter) | — | slice 0a, 0b |
| 2 | 3 Forms routes and Workflow Streams server | A | T026-015, 043, 021, 022, 023, 073, 067 (the stream carriers of feature 016) | `crates/application` (`forms.rs`, `streams.rs`), `crates/infra-postgres/src` (`streams.rs`), `crates/api-http` (`routes/forms.rs`, `routes/streams.rs`, `routes/versioning.rs`), `crates/api-grpc` (`service.rs`, `limits.rs`), `apps/control-plane` (subscriber wiring); tests `forms.rs`, `openapi.rs`, `streams.rs` | — | slices 1 and 2 |
| 2 | 4 Stream passes, SDK clients and workers, CLI | B | T026-024, 073 (maintenance close), 055 and 056 (execute, outcome, form, OpenAPI, stream consume and publish with the final batch), 059 (invocation and stream commands) | `crates/scheduler` (`passes.rs` or the maintenance loop), `sdks/typescript`, `sdks/go`, `sdks/python`, `sdks/dotnet`, `sdks/java`, `sdks/rust`, `apps/cli`; tests `purge.rs` | — | slice 2; its integration tests run after slice 3 merges |
| 3 | 5 Channels and approval channels server | A | T026-034, 037, 038, 039, 040, 077 (gated) | `crates/application` (`notifications.rs`, `approval_progress.rs`, `approval_links.rs`), `crates/infra-postgres/src` (`outbound.rs`, `approval_progress.rs`), `crates/api-http` (`routes/notifications.rs`, `routes/operations.rs`, `routes/primitives.rs`), `crates/infra-ir` (summary checks), `crates/infra-schema` (`classification/publication.rs`), `apps/control-plane` (`SAGAT_LINK_KEYS`); tests `channels.rs`, `approvals.rs` | — | slice 3; Task 026-077 also Owner question 1081 |
| 3 | 6 Delivery adapters and the first Studio surfaces | B | T026-035, 036 (rendering and fan-out in the scheduler), 031 (the alert in the monitor), 049, 016, 017, 026 | `crates/infra-connectors` (`chat.rs`, `email.rs`, `publish.rs`), `crates/infra-egress` if the guard needs a new destination kind, `crates/scheduler` (`deliveries.rs`, `monitor.rs`), `frontend/apps/studio/src/features/{forms,runs}/`, `frontend/e2e/tests/{start-form,streams}.spec.ts`; tests `deliveries.rs` (fault) | — | slices 3 and 4; pull request 267 (Astryx) merged |
| 4 | 7 Public links server | A | T026-027, 028, 029, 030, 031 (the races), 032, 033 (`public_site.rs`), 069 (the public-link row), 074 (counters in use) | `crates/application` (`public_links.rs`), `crates/infra-postgres/src` (`public_links.rs`, `quotas.rs`), `crates/api-http` (`routes/public_links.rs`, `routes/public.rs`, `public_site.rs`, `limits.rs`, `studio.rs`), `apps/control-plane` (public-forms origin, trusted proxies); tests `links.rs` | — | slice 5; design 043 row for public links written |
| 4 | 8 People-facing Studio, SDK approvals and builders, scripts | B | T026-041, 033 (Studio), 036 (Studio channel page), 055 and 056 (decisions with input, builders), 025 | `frontend/apps/studio/src/{features/approvals,features/public-links,features/notifications,public}/`, `frontend/e2e/tests/{approval-decide,public-link}.spec.ts`, `sdks/*`, `apps/sandbox-runner`, `runners/` | — | slices 5 and 6; feature 023's `stream.append` host call for Task 026-025 |
| 5 | 9 Slack in-chat server and the server gates | A | T026-044, 045 (route), 046, 047, 048, 050, 051, 053, 054, 058 (server) | `crates/application` (`chat_identity.rs`), `crates/infra-postgres/src` (`chat_identity.rs`), `crates/api-http` (`routes/chat.rs`), `tests/{integration,fault,contract}/invocation-human-surfaces/` (the files of these tasks), `tests/contract/security-multitenancy/` | — | slices 7 and 8 |
| 5 | 10 Slack adapter and dispatcher, conformance, runbooks, benchmark | B | T026-044 (`auth.test` adapter), 045 (dispatcher), 057, 060, 061, 063, 052 | `crates/infra-connectors` (`chat.rs`), `crates/scheduler` (`events.rs`), `tests/contract/sdk-conformance/`, `docs/runbooks/`, `docs/architecture/` (the documents of Task 026-060 and the threat model), `tests/performance/` | — | slices 7 and 8 |
| 6 | 11 Joint hooks and closing | coordinator | T026-065 (with features 021 and 022), 078 (with design 044), 068, 064, 062, 066 | as each task names | — | both lanes done; features 021 and 022 merged for their hooks; design 044 built for Task 026-078 |

What each window delivers on its own: window 1, `executeRun` and `getRunOutcome` with the poll path (the
hint path joins in window 2); window 2, the MVP of User stories 026-01 to 026-03 on the server and in the
SDKs; window 3, channels and channel approvals; window 4, public links and every Studio surface; window 5,
Slack in-chat decisions and the release gates. A window's slices merge one at a time into `develop`, each
with its own pull request and CI, lane A first when both touch `research.md`.
