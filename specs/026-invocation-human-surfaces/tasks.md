# Tasks: Invocation and human-interaction surfaces

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), proposed
[ADR-0061](../../docs/adr/ADR-0061-link-bearers-hold-one-narrow-capability-and-never-decide.md).

Status: Proposed — not started; revised 2026-10-26 after the architecture and security reviews and the
coordinator decisions C1–C22. No task is ticked; a task is ticked only with the evidence it names
(a test and its real result, or a command and its output), recorded in a future `verification.md`.

**Format**: `T026-NNN [P?] [Story] Description — Files — Evidence — Traces`. `[P]`: can run in
parallel with the other `[P]` tasks of its phase (different files, no dependency). Tests are written
first and seen failing for the right reason (AGENTS.md rule 18). Suites: `tests/{contract,integration,
fault,e2e}/invocation-human-surfaces/`, targets `*_invocation_human_surfaces` in `tests/Cargo.toml`.

## Phase 1: Setup

- [ ] T026-001 Owner review of ADR-0061 and ADR-0068 and answers to O1320–O1322; record them in
  `docs/owner-decisions.md` and the ADR status — Files: `docs/adr/ADR-0061-*.md`, `docs/adr/ADR-0068-*.md`,
  `docs/owner-decisions.md` — Evidence: owner's recorded answer — Traces: FR-026-013..027, 031, 037, 042.
- [ ] T026-002 [P] Add test targets `integration_`, `fault_`, `contract_rest_`, `contract_grpc_`,
  `e2e_`, `perf_invocation_human_surfaces` and their `main.rs` — Files: `tests/Cargo.toml`,
  `tests/{integration,fault,contract,e2e}/invocation-human-surfaces/main.rs` — Evidence: `cargo test -p
  sagat-tests --test integration_invocation_human_surfaces --no-run` builds — Traces: all.
- [ ] T026-003 [P] Slack/Teams stand-in receivers and an SMTP sink container pinned by digest,
  loopback-only, self-starting (AGENTS.md rule 16) — Files: `tests/support/receiver.rs`,
  `tests/support/smtp.rs` — Evidence: fixture self-test passes — Traces: FR-026-035..038.
- [ ] T026-004 [P] Select and pin the SMTP client crate (TLS-only, no default features) in
  `crates/infra-connectors`; dependency advisory audit — Files: `crates/infra-connectors/Cargo.toml`,
  `Cargo.lock` — Evidence: `cargo deny`/audit output clean — Traces: FR-026-038.

## Phase 2: Foundational (blocks every story)

- [ ] T026-005 Merge [openapi-fragment.yaml](./contracts/openapi-fragment.yaml) and
  [errors.md](./contracts/errors.md) into the canonical OpenAPI (marked `x-implemented: '026'` only when
  built) and the remediation table; add `STREAM_BUFFER_FULL` to the SDK catalog — Files:
  `specs/001-platform-foundation/contracts/openapi.yaml`, `sdks/error-catalog.json` — Evidence: contract
  lint in CI `contracts`; `contract_rest_invocation_human_surfaces` compatibility test (no removed or
  narrowed element); `(cd frontend/apps/studio && npm run api && npm run check:api)` — Traces: FR-026-046, 050.
- [ ] T026-006 Merge [workflow-ir-fragment.schema.json](./contracts/workflow-ir-fragment.schema.json)
  into the canonical IR schema; extend the fixture corpus with old documents unchanged and new shapes;
  refuse the new fields while their capability is disabled — Files:
  `specs/001-platform-foundation/contracts/workflow.schema.json`, `crates/infra-ir/src/schema.rs`,
  `crates/infra-ir/tests/` — Evidence: schema corpus test, disabled-capability refusal test — Traces:
  FR-026-003, 013, 029, 033, 050.
- [ ] T026-007 Merge [worker-protocol.md](./contracts/worker-protocol.md) into `worker.proto` and the
  protocol document; negotiate the new minor without changing older minors — Files:
  `specs/001-platform-foundation/contracts/worker.proto`, `docs/architecture/worker-runtime-protocol.md`,
  `crates/api-grpc/src/service.rs` — Evidence: `contract_grpc_invocation_human_surfaces` (older-minor
  worker served unchanged, new rpc refused below the minor) — Traces: FR-026-014, 050.
- [ ] T026-008 Merge [asyncapi-fragment.yaml](./contracts/asyncapi-fragment.yaml); regenerate
  `events.baseline.json` only after reading the diff — Files:
  `specs/001-platform-foundation/contracts/events.asyncapi.yaml`, `events.baseline.json` — Evidence:
  `contract_asyncapi_events_webhooks_messaging` passes without the bless switch — Traces: FR-026-034, 036.
- [ ] T026-009 [P] Domain identity and quotas: `Permission::LinkRead`, `LinkManage` and role grants,
  `Subject::public_link`, the five quota resources — Files: `crates/domain/src/identity.rs`,
  `crates/domain/src/quota.rs` — Evidence: role-matrix unit tests; reserved actor refused by
  `Subject::parse` — Traces: FR-026-024, 043, 047.
- [ ] T026-010 [P] Domain types: form model, stream declarations, budgets and verdicts, link state
  machine and `LinkBearer` (no `authorize`), chat/e-mail destinations and kinds, Slack app binding,
  approval notify/summary/form/input — Files: `crates/domain/src/{forms,streams,public_links}.rs`,
  `crates/domain/src/outbound.rs`, `crates/domain/src/execution/approval.rs` — Evidence: unit and
  property tests (budget arithmetic per channel, pool and run); a compile-fail test that `LinkBearer`
  cannot become a `Principal` — Traces: FR-026-001, 013..016, 019, 022, 024, 029, 033, 035, 042.
- [ ] T026-011 IR codec and publication checks for `streams`, `emits`, `notify`, `summary`, `form`,
  `x-sagat-form`, summary expressions on the existing evaluator, reserved `sagat.` channel names, and
  the feature-031 encoded-location check (`ENCODING_READ_BY_FORM`) when 031 is present — Files:
  `crates/infra-ir/src/{convert,evaluator}.rs`, `crates/infra-schema/src/classification/publication.rs` —
  Evidence: codec round trip,
  every refusal reason of errors.md, carrier classification for stream, summary and form schemas —
  Traces: FR-026-003, 013, 029, 033.
- [ ] T026-012 Migrations, storage classification, grants and refusing down migrations for links
  (embed id, live-run cap, inline presets), streams (inline `data`, channel usage), approval channels,
  notification channels (`classification_set_by`, Slack columns, the global interactive index), Slack
  app bindings and chat identities (consumed codes through a `SECURITY DEFINER` function; registry
  entries of data-model.md) — Files: `crates/infra-postgres/migrations/<next>_*.{up,down}.sql`,
  `crates/infra-postgres/storage-classification.json` — Evidence: migration up/down tests, manifest test
  in `integration_security_multitenancy` (`run_stream_events.data` classified `content`),
  `tests/integration/platform-foundation/runtime_privileges` (no direct `UPDATE` on codes) — Traces:
  FR-026-015, 021, 022, 030, 035, 042, 044, 047.
- [ ] T026-013 [P] Telemetry vocabulary: span names, metric names and closed label enums of the plan —
  Files: `crates/telemetry/src/`, `crates/application/src/telemetry.rs` — Evidence: label-set unit tests
  — Traces: FR-026-045.

**Checkpoint**: contracts merged, schema and domain ready.

## Phase 3: US01 — Start a run from a generated form (P1) 🎯 MVP

- [ ] T026-014 [P] [US1] Form-model walker and `x-sagat-form` validation per
  [form-model.md](./contracts/form-model.md) — Files: `crates/infra-schema/src/forms.rs` — Evidence:
  unit and property tests (determinism, each keyword, unsupported optional/required, bounds,
  encrypted marker, `secret` never prefilled) — Traces: FR-026-001..004, 006.
- [ ] T026-015 [US1] Form service and `getStartForm` route — Files: `crates/application/src/forms.rs`,
  `crates/api-http/src/routes/forms.rs` — Evidence: `integration_invocation_human_surfaces forms::*`
  (authorization, foreign version 404, reasons, cache bound) — Traces: FR-026-001, 002, 043, 044.
- [ ] T026-016 [US1] Studio form renderer and *Start run* defaulting to the form with the JSON fallback
  — Files: `frontend/apps/studio/src/features/forms/`, `frontend/apps/studio/src/features/runs/StartRun.tsx`
  — Evidence: Vitest (levels as text, write-only secret, no value in any storage or URL, one key per
  form, pointer errors) — Traces: FR-026-004, 005, 049.
- [ ] T026-017 [US1] Browser journey and accessibility — Files: `frontend/e2e/tests/start-form.spec.ts`
  — Evidence: real-browser run with axe WCAG AA checks, keyboard only — Traces: SC-026-001, SC-026-014.

## Phase 4: US02 — Call a workflow and wait (P1) 🎯 MVP

- [ ] T026-018 [US2] Invocation use cases, `RunWaiters` port, in-process registry woken by the
  `run.finished` and feature-022 `run.result_returned` hints (every replica subscribes without a queue
  group) and one batched status poll — Files: `crates/application/src/invocation.rs`,
  `crates/api-http/src/streams.rs`, `crates/infra-postgres/src/execution.rs`, `apps/control-plane/src/` —
  Evidence: unit tests of the registry bounds; integration test asserting no pool connection is held
  while waiting; two-replica test that the replica holding the waiter is woken — Traces: FR-026-007..009.
- [ ] T026-019 [US2] `executeRun` and `getRunOutcome` routes — Files:
  `crates/api-http/src/routes/invocation.rs` — Evidence: `integration_invocation_human_surfaces
  invocation::*` (200, 202 + Location, replay, 409, `WAITER_LIMIT` before admission, wait clamp) —
  Traces: FR-026-007, 008, 043.
- [ ] T026-020 [US2] Outcome content through the feature-016 result projection; `reveal` audit; closed
  failure codes; reserved `EARLY_RETURN` — Files: `crates/application/src/invocation.rs`,
  `crates/application/src/payload_disclosure.rs` — Evidence: audience × level matrix with canaries;
  `payload.pii_revealed` in the read transaction — Traces: FR-026-010, 011, SC-026-006.

## Phase 5: US03 — Workflow Streams (P1) 🎯 MVP

- [ ] T026-021 [US3] `AppendStreamEvents`: lease check, `emits`, reserved `sagat.` names, schema,
  secret scan, head allocation, deduplication, channel budget, author and reserved pools, per-run hard
  ceiling and rate; refusal never fails the step; data inline — Files: `crates/api-grpc/src/service.rs`,
  `crates/api-grpc/src/limits.rs`, `crates/application/src/streams.rs`,
  `crates/infra-postgres/src/streams.rs` — Evidence: `contract_grpc_invocation_human_surfaces streams::*`,
  integration tests for every refusal and scope of worker-protocol.md, and a test that an append changes
  no run version, journal, outbox, `telemetry_pending` or expression input (ADR-0068) — Traces:
  FR-026-014, 015, 019.
- [ ] T026-022 [US3] `streamRunEvents` SSE and page: `<epoch>.<offset>` cursors, `Last-Event-ID`,
  channel filter, `attempts` snapshot, `superseded`/`settled` control events, `reset` on an epoch
  mismatch, `end`, `reconnect`, ping, subscription counters and membership recheck — Files:
  `crates/api-http/src/routes/streams.rs`, `crates/api-http/src/streams.rs` — Evidence: integration tests
  for resume after 50 random disconnects, a second attempt while connected, a cursor of another epoch,
  `end` after the last event, 429 at the ceiling — Traces: FR-026-016, 017, SC-026-005.
- [ ] T026-023 [US3] Stream disclosure per subscriber audience and reveal audit at open and recheck —
  Files: `crates/application/src/streams.rs` — Evidence: canary matrix (viewer, operator, grant holder;
  `secret` never sent) — Traces: FR-026-018, SC-026-006.
- [ ] T026-024 [P] [US3] Retention purge, one run per transaction, data deleted with the rows, byte
  release; registry entry in feature 020's content-store registry — Files: `crates/scheduler/src/passes.rs`,
  `crates/infra-postgres/src/streams.rs` — Evidence: integration test: the tenant's content bytes before
  the first append equal those after the purge; erasure deletes at once — Traces: FR-026-021.
- [ ] T026-025 [P] [US3] Script publishing through feature 023's reserved, capability-gated
  `stream.append` host call: the supervisor appends under the attempt's lease — Files:
  `apps/sandbox-runner/src/`, `runners/` — Evidence: `integration_inline_code_sandbox_containers` scenario
  publishing under the lease; refusal after lease end; refusal when the capability is not granted —
  Traces: FR-026-020, G1332.
- [ ] T026-026 [US3] Studio *Streams* tab: list per channel, progress display, latest attempt by default,
  resume, unavailable states — Files: `frontend/apps/studio/src/features/runs/StreamsTab.tsx`,
  `frontend/e2e/tests/streams.spec.ts` — Evidence: Vitest + real-browser journey with WCAG checks —
  Traces: FR-026-049, SC-026-014.

**Checkpoint**: MVP (US01–US03) independently demonstrable.

## Phase 6: US04 — Public form links (P2)

- [ ] T026-027 [US4] Link management use cases, store and routes: create (quota `public_links` default
  0, owner holds `run:start`, public-forms origin configured, required live-run cap, embed id), edit with
  revisions, pause/resume/rotate/revoke, list without tokens — Files:
  `crates/application/src/public_links.rs`, `crates/infra-postgres/src/public_links.rs`,
  `crates/api-http/src/routes/public_links.rs` — Evidence: integration tests incl. idempotent management
  replay and every validation reason, including 031's `ENCODING_READ_BY_FORM` — Traces: FR-026-022, 043,
  047.
- [ ] T026-028 [P] [US4] Token generation, hashing, fragment URL, rotation; redaction of the token in
  every log path — Files: `crates/application/src/public_links.rs`, `crates/telemetry/src/` — Evidence:
  unit tests; log canary test with the token — Traces: FR-026-023.
- [ ] T026-029 [US4] Public-forms origin: Host-based mounting of the anonymous router (no bearer
  extractor) and the public build only on that origin, `404` on the Studio origin, start-up refusal of
  equal origins; per-process guard keyed on the peer address (forwarded headers only from configured
  trusted proxies); durable per-link and per-namespace buckets; `getPublicForm` — Files:
  `crates/api-http/src/routes/public.rs`, `crates/api-http/src/public_site.rs`,
  `crates/api-http/src/limits.rs`, `apps/control-plane/src/` — Evidence: contract tests: the same 404 for
  each unusable state; refusal before the database for malformed tokens (counted queries); spoofed
  `X-Forwarded-For` ignored without trusted proxies; Studio-origin 404; the bucket debit commits
  before the admission transaction and is not refunded by a later refusal — Traces: FR-026-025, 027.
- [ ] T026-030 [US4] `LinkBearer` submission admission (T2) in the merged lock order of ADR-0050
  (decision 7: link row in slot 1, receipt in 2, owner membership and namespace in 4a, pool count
  with slot 6's active-run admission, usage rows last; buckets debited in their own decision before):
  presets merge,
  exposed-field check (`FORM_INPUT_INVALID`), owner and 029 recheck, live-run cap and namespace pool
  counts, derived receipt token, `system:public-link:<id>` actor, origin `PUBLIC_LINK` — Files:
  `crates/application/src/public_links.rs`, `crates/infra-postgres/src/public_links.rs` — Evidence:
  integration tests (one run per key, replay returns the same receipt token, conflict, foreign fields
  refused); unit test that the bearer authorizes no `Permission` — Traces: FR-026-024, 025, 026, ADR-0061.
- [ ] T026-031 [US4] Races and abuse: owner demotion and link revocation versus submission (both commit
  orders, barriers not sleeps), cap and pool races (live-run cap of 2 under 10 concurrent submissions;
  members still start runs when the public pool is full), alert `PUBLIC_LINK_OWNER_UNAUTHORIZED`,
  refusal audit collapsing, no address in audit/logs/spans — Files:
  `tests/fault/invocation-human-surfaces/links.rs`, `crates/scheduler/src/monitor.rs` — Evidence: fault
  suite results — Traces: FR-026-025, 028, SC-026-007.
- [ ] T026-032 [P] [US4] Receipt endpoint and `public-fields-v1` disclosure, including feature 025's
  model-output floor for model-written fields — Files: `crates/api-http/src/routes/public.rs`,
  `crates/application/src/public_links.rs` — Evidence: canary tests: only `public` result fields, a
  model-written field annotated `public` withheld when its transcript was `pii`, no run id, no failure
  code — Traces: FR-026-026, SC-026-006.
- [ ] T026-033 [US4] Public page entry under `/f/{embedId}` on the public-forms origin with per-link
  `frame-ancestors` (`'none'` for unknown or unusable links; origins and CSP per ADR-0062), and Studio
  link management — Files: `frontend/apps/studio/src/public/`,
  `frontend/apps/studio/src/features/public-links/`, `crates/api-http/src/public_site.rs`,
  `frontend/e2e/tests/public-link.spec.ts` — Evidence: real-browser journey without an account; framing
  refused from another origin and from an origin approved only for another tenant's link; header test on
  every status code; WCAG checks — Traces: FR-026-027, 049.

## Phase 7: US06 — Notification channels (P2)

- [ ] T026-034 [US6] Channel management as outbound subscription variants: settings, secret bindings,
  Slack app binding reference, `maxClassification` (default `public`; `internal` needs
  `payload:read-internal`; `pii` the reveal grant and `classification_set_by`), `runFilter`, quota,
  health, `NOTIFICATION_DESTINATION_DENIED` — Files: `crates/application/src/notifications.rs`,
  `crates/infra-postgres/src/outbound.rs`, `crates/api-http/src/routes/notifications.rs` — Evidence:
  integration tests incl. recipients hidden from `event:read`-only callers — Traces: FR-026-035, 036, 039.
- [ ] T026-035 [P] [US6] Slack, Teams and SMTP adapters through the egress guard with escaping, bounds
  and closed outcome mapping — Files: `crates/infra-connectors/src/{chat,email}.rs`,
  `crates/infra-connectors/src/publish.rs` — Evidence: adapter tests against the stand-ins (header
  injection, mrkdwn/HTML escaping, provider text never logged, TLS required) — Traces: FR-026-037, 038.
- [ ] T026-036 [US6] Delivery rendering at send time, `provider_ref`, fan-out of `run.finished` and alerts
  with SQL filters in the fact transaction, *Send test message*; Studio channel page — Files:
  `crates/scheduler/src/deliveries.rs`, `crates/infra-postgres/src/{execution,operations}.rs`,
  `frontend/apps/studio/src/features/integrations/` — Evidence: integration tests (one delivery per
  channel and event, test message via pipeline); Vitest — Traces: FR-026-036, 037, 039, 049.

## Phase 8: US05 — Approval through channels (P2)

- [ ] T026-037 [US5] Approval declarations end to end: builders' document, publication checks, summary
  expression bounds, summary level ≥ source level (`SUMMARY_LEVEL_BELOW_SOURCE`, 025's floor for
  model-written sources), form carrier — Files: `crates/infra-ir/src/convert.rs`,
  `crates/domain/src/execution/approval.rs`, `crates/infra-schema/src/classification/publication.rs` —
  Evidence: publication tests for each reason — Traces: FR-026-029, 033.
- [ ] T026-038 [US5] Opening transaction (T5): summary evaluation and storage, one delivery and one
  decision link per active target, skipped targets audited and alerted, evaluation failure recorded —
  Files: `crates/application/src/approval_progress.rs`, `crates/infra-postgres/src/approval_progress.rs`
  — Evidence: integration tests incl. missing/paused channel and failing summary — Traces: FR-026-030.
- [ ] T026-039 [US5] Decision links: token and MAC, `SAGAT_LINK_KEYS` with rotation, validity in
  infrastructure time plus `PENDING`, `resolveApprovalLink` (cheap refusal, tenant check, 029 visibility
  when enabled, audience projection) — Files: `crates/application/src/approval_links.rs`,
  `crates/api-http/src/routes/operations.rs`, `apps/control-plane/src/` — Evidence: unit tests (forgery,
  truncation, rotated key, removed key, validity never compared with the approval deadline);
  integration tests (foreign tenant 404) — Traces: FR-026-031, 032.
- [ ] T026-040 [US5] Decision with input (T6) and settlement deliveries (T6/T7): validation
  (`DECISION_INPUT_INVALID`), fingerprint with input, server-derived `via`, links closed,
  `approval.settled` edit or follow-up, every owner kind including 025 `AGENT_TOOL_CALL`, no rows for
  draft test runs — Files: `crates/application/src/approval_progress.rs`,
  `crates/infra-postgres/src/approval_progress.rs`, `crates/api-http/src/routes/primitives.rs` —
  Evidence: integration tests (replay, changed input 409, missing required input, later step reads
  `steps.<node>.output.input`); compatibility test that a request without input hashes byte-identically
  to the pre-feature fingerprint and an in-flight retry across the upgrade replays — Traces: FR-026-030,
  033, 034.
- [ ] T026-041 [US5] Studio decision page: sign-in with the token kept in `sessionStorage` only across
  the redirect, fragment cleared, audience summary, form, per-page decision key, link token sent (never a
  client `via`), forwarded-link notice — Files: `frontend/apps/studio/src/features/approvals/DecidePage.tsx`,
  `frontend/e2e/tests/approval-decide.spec.ts` — Evidence: real-browser journey: ineligible, initiator,
  cross-tenant, eligible with form; storage inspection after return; WCAG checks — Traces: FR-026-032,
  049, SC-026-009.

## Phase 9: US07 — Per-workflow OpenAPI (P2)

- [ ] T026-042 [P] [US7] Deterministic generator specializing the generic operations — Files:
  `crates/infra-schema/src/openapi.rs` — Evidence: byte-identical output test; OpenAPI 3.1 lint in a
  contract test — Traces: FR-026-040.
- [ ] T026-043 [US7] `getWorkflowOpenApi` route with `ETag`/`304`, and the agreement corpus (generated
  valid and invalid requests against the server) — Files: `crates/api-http/src/routes/versioning.rs`,
  `tests/contract/invocation-human-surfaces/openapi.rs` — Evidence: 100 % agreement on the corpus —
  Traces: FR-026-041, SC-026-011.

## Phase 10: US08 — Decide inside Slack (P3)

- [ ] T026-044 [US8] Slack app bindings and identity linking: `auth.test` registration and
  re-verification, global interactive `(team, channel)` index, single-use ≥ 128-bit codes consumed
  through the `SECURITY DEFINER` function, team check, per-team rate limit, pending link confirmed only
  by the subject in the Studio, revocation on membership removal — Files:
  `crates/application/src/chat_identity.rs`, `crates/infra-postgres/src/chat_identity.rs`,
  `crates/infra-connectors/src/chat.rs`, `frontend/apps/studio/src/features/integrations/` — Evidence:
  integration tests (second tenant binding the same interactive channel refused, code reuse, code from
  another workspace, stolen code without confirmation, expiry, membership removal) — Traces: FR-026-042.
- [ ] T026-045 [US8] Slack requests at `/v1/chat/slack/apps/{bindingId}/requests`: signature with the
  binding's secret before the database, inbox admission, tenant only from the decision-link row and
  matched to the binding's tenant, team and channel, identity lookup inside that tenant, dispatcher
  decision, ephemeral replies via the pipeline — Files: `crates/api-http/src/routes/chat.rs`,
  `crates/scheduler/src/events.rs` — Evidence: tests for bad and stale signatures and unknown bindings
  (uniform 401), replay (one decision), unlinked/pending and ineligible (no decision), a button value
  naming another tenant's link (refused, audited in the binding's tenant only) — Traces: FR-026-042,
  SC-026-015.

## Phase 11: Polish and gates (rule 9 and Definition of Done)

- [ ] T026-046 Fault: stream appends — lost response replay, lease expiry mid-batch, append versus
  completion with a held lock, duplicate batches — Files: `tests/fault/invocation-human-surfaces/streams.rs`
  — Evidence: suite results; offsets gap-free — Traces: SC-026-004, NFR-026-003.
- [ ] T026-047 Fault: crash at each commit point of T1–T11 (injected failure per statement), the
  wait-deadline/terminal race and the decision/expiry race — Files:
  `tests/fault/invocation-human-surfaces/crashes.rs`, `races.rs` — Evidence: suite results with
  `verify_run`/`assert_consistent` — Traces: SC-026-008, SC-026-009.
- [ ] T026-048 Fault: restart of control plane and engine during waits, streams and decision pages; lost
  `run.finished` hint; NATS unavailable — Files: `tests/fault/invocation-human-surfaces/restart.rs` —
  Evidence: suite results; zero duplicate runs — Traces: SC-026-002, NFR-026-003.
- [ ] T026-049 Fault: new destinations — acknowledgement lost, credential unavailable, `DEAD` at the
  limit, redrive, auto-disable, egress denial, reveal grant revoked behind a `pii` channel (paused before
  the next `pii` delivery) — Files: `tests/fault/invocation-human-surfaces/deliveries.rs` — Evidence: suite
  results — Traces: FR-026-039, SC-026-010.
- [ ] T026-050 Quota and ceiling exhaustion at and one past every limit of the plan — Files:
  `tests/integration/invocation-human-surfaces/ceilings.rs` — Evidence: typed refusal per row — Traces:
  FR-026-047, SC-026-012.
- [ ] T026-051 Security battery over every new operation (anonymous included), cross-tenant opacity,
  token/secret/value canaries in responses, logs, spans and audit; independent `security-reviewer` pass
  — Files: `tests/contract/security-multitenancy/rest/battery.rs`, `tests/support/canary.rs` — Evidence:
  battery result; review notes — Traces: NFR-026-001, 002, SC-026-013.
- [ ] T026-052 Update the threat model for the anonymous and chat trust boundaries — Files:
  `docs/architecture/security-and-sandbox.md` — Evidence: reviewed diff — Traces: FR-026-024, 042.
- [ ] T026-053 Runtime-role sweep over the new suites — Evidence: `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo
  test -p sagat-tests --test integration_invocation_human_surfaces` result — Traces: FR-026-044.
- [ ] T026-054 Telemetry tests: every span, metric and audit action present with closed labels, no
  values — Files: `tests/integration/invocation-human-surfaces/telemetry.rs` — Evidence: suite result —
  Traces: FR-026-045, SC-026-018.
- [ ] T026-055 [P] `sdks/SPEC.md` additions and TypeScript and Go clients, worker publishing and builders
  — Files: `sdks/SPEC.md`, `sdks/typescript/`, `sdks/go/` — Evidence: per-SDK unit tests and lints of
  `sdks/AGENTS.md` — Traces: FR-026-048.
- [ ] T026-056 [P] Python, C# and Java clients, worker publishing and builders — Files: `sdks/python/`,
  `sdks/dotnet/`, `sdks/java/` — Evidence: per-SDK unit tests and lints — Traces: FR-026-048.
- [ ] T026-057 Conformance scenarios of [sdk-conformance.md](./contracts/sdk-conformance.md) with
  reviewed goldens — Files: `tests/contract/sdk-conformance/scenarios/`, `expected/` — Evidence:
  `SAGAT_SDK_SCENARIO=<name> cargo test -p sagat-tests --test sdk_conformance -- --ignored` for each,
  five languages, rerun without bless — Traces: SC-026-003.
- [ ] T026-058 Compatibility: previous-release SDK clients and workers, the previous Studio build and the
  old fixture corpus against the new server; down-migration refusals — Evidence: `SAGAT_SDK_SOURCE_ROOT`
  run and migration tests — Traces: FR-026-050, SC-026-019.
- [ ] T026-059 [P] CLI commands of the plan — Files: `apps/cli/src/` — Evidence: CLI tests in
  `tests/e2e/invocation-human-surfaces/cli.rs` — Traces: FR-026-007, 008, 017, 022, 040.
- [ ] T026-060 [P] Runbooks and architecture documents — Files: `docs/runbooks/invocation.md` (new:
  waits, streams, public links, channels, decision-link keys, erasure boundary G1330),
  `docs/runbooks/{events,workflow-primitives,payload-classification,studio}.md`,
  `docs/architecture/{studio-ux,observability-slo,storage-and-events}.md` — Evidence: reviewed diff;
  `contract_developer_experience` passes — Traces: NFR-026-002.
- [ ] T026-061 Benchmark harness and report (wake-up latency by path, batched poll cost at 2,000
  waiters, append throughput, publish-to-SSE latency, refusal cost) — Files:
  `tests/performance/invocation-human-surfaces/`, `tests/performance/reports/invocation-human-surfaces.md`
  — Evidence: report with environment and failures — Traces: NFR-026-006, SC-026-016.
- [ ] T026-062 Architecture and workspace gates — Evidence: `cargo fmt --check`, `cargo clippy
  --workspace --all-targets --locked -- -D warnings`, `python3 scripts/check-architecture.py`,
  `python3 scripts/check-agent-config.py` outputs — Traces: constitution II.
- [ ] T026-063 [P] Studio gates: design tokens only, generated client parity, a11y suite — Evidence:
  `npm --prefix frontend run test`, `npm run check:api`, `frontend/e2e/tests/a11y.spec.ts` — Traces:
  FR-026-049, SC-026-014.
- [ ] T026-064 Reproduce [quickstart.md](./quickstart.md) happy and failure scenarios on a clean stack
  and record the output — Evidence: recorded transcript — Traces: Definition of Done.
- [ ] T026-065 Dependency hooks: contract tests that `EARLY_RETURN` and route waits are never produced
  without features 022/021, that a waiter wakes on 022's `run.result_returned` hint and, with the hint
  dropped, at the next poll, and that a route returns a result only with `public-fields-v1`; update
  those drafts' dependency sections when they are integrated —
  Files: `tests/contract/invocation-human-surfaces/hooks.rs` — Evidence: test result — Traces:
  FR-026-011, 012, SC-026-017.
- [ ] T026-066 Final verification record and traceability rows — Files:
  `specs/026-invocation-human-surfaces/verification.md`, `docs/traceability.md` — Evidence: every
  command and result, failures and skips stated — Traces: all.
- [ ] T026-067 Amendment to feature 016 (coordinator decision C6): add the three carriers of the spec's
  "Amendment to feature 016" (stream channel schemas, approval summary schema, approval form schema) to
  the single consolidated amendment of FR-016-002 and classification rules version 2 kept in
  `SPECIFICATION-INDEX.md`, opened by the first parity feature implemented — Files:
  `specs/016-payload-classification/spec.md` (through that amendment),
  `crates/infra-schema/src/classification/`
  — Evidence: reviewed amendment; classification walker tests for the three carriers — Traces:
  FR-026-013, 029, 033.
- [ ] T026-068 Cross-feature registries (coordinator decisions C1, C7, C8): declare this feature's rules
  in feature 018's reset-rule registry (stream events not copied; decision input `COPY`; notification
  side rows rematerialized through a new opening), feature 031's encoded-field registry and feature 020's
  content-store registry — Files: `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`,
  the 031 and 020 registries (through their owners) — Evidence: a reset fixture showing an empty stream
  log and a fresh notification for a reopened approval — Traces: FR-026-006, 021, 030.

## Dependencies and order

Phase 1 → Phase 2 → {US01, US02, US03} (independent; MVP) → US06 → US05 (needs channels) → US04 (needs
US01's form model) → US07 (needs US02 operations) → US08 (needs US05). Phase 11 tasks follow the stories
they test; T026-067 and T026-068 run with the first story that needs them (streams and approvals);
T026-062 and T026-066 are last. Parallel within a phase only where marked `[P]`.
