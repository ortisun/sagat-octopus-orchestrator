# Tasks — 027 internal apps

Status: Proposed — not started. No task is done; a task is checked only with the evidence it names,
recorded with the real command and result. Format: `- [ ] T027-NNN [P?] [USn?] description —
requirements — files — evidence`. `[P]` = can run in parallel with the other `[P]` tasks of its phase.

## Phase 1 — Setup

- [ ] T027-001 Record the owner's answers to O1360–O1362 and the acceptance (or change) of ADR-0062; hand the coordinator the overview scope line (G1360). — all FR — `docs/adr/ADR-0062-internal-apps-narrow-the-viewers-authority-and-confine-custom-code.md`, `specs/027-internal-apps/research.md` — evidence: dated owner decision.
- [ ] T027-002 [P] Add Cargo targets `integration_internal_apps`, `contract_rest_internal_apps`, `fault_internal_apps`, `e2e_internal_apps`, `perf_internal_apps` with empty `main.rs`. — FR-027-034 — `tests/Cargo.toml`, `tests/{integration,contract,fault,e2e,performance}/internal-apps/main.rs` — evidence: `cargo test -p sagat-tests --test integration_internal_apps --locked` compiles.
- [ ] T027-003 [P] Add the second Vite entry (`apps.html`, `src/apps-main.tsx`) and a build test that the entry contains no Studio shell. — FR-027-023, 027 — `frontend/apps/studio/vite.config.ts`, `frontend/apps/studio/apps.html` — evidence: `npm --prefix frontend run build`, Vitest.
- [ ] T027-004 Merge [contracts/apps.openapi.yaml](contracts/apps.openapi.yaml) (as `x-planned: '027'`), [app.schema.json](contracts/app.schema.json) and the `details.reason` values of [errors.md](contracts/errors.md) into the canonical contracts; regenerate `schema.d.ts`; list the new operations in the cross-tenant battery. — FR-027-002, 033, 037 — `specs/001-platform-foundation/contracts/{openapi.yaml,app.schema.json}`, `frontend/apps/studio/src/api/schema.d.ts`, `tests/contract/security-multitenancy/rest/battery.rs` — evidence: CI `contracts`, `npm run check:api`.
- [ ] T027-005 [P] Fixtures: `refund` and `refund-reconciliation` workflows (declared Query/Update, one output field per classification level, one encrypted field when 031 exists), the example app, an invalid-app corpus and malicious component bundles. — FR-027-002, 011, 018, 026 — `tests/contract/internal-apps/fixtures/` — evidence: fixtures validate against the canonical schemas.

## Phase 2 — Foundational (blocks every story)

- [ ] T027-006 Domain model of the app document: pages, grid layout, closed component kinds, bindings, references, predicates (depth ≤ 4), declared parameters (`public`/`internal` only), limits of the plan; unit and property tests on bounds. — FR-027-002, 003, 032 — `crates/domain/src/apps.rs`, `crates/domain/src/apps/` — evidence: `cargo test -p sagat-domain apps`, `python3 scripts/check-architecture.py`.
- [ ] T027-007 [P] Permissions `app:read|write|publish|manage` with role grants of D1386; audit actions and resource types; quota resources `apps`, `app_components`, `app_sessions`; extend the permission matrix test. — FR-027-021, 031, 032 — `crates/domain/src/{identity.rs,audit.rs,quota.rs}` — evidence: identity matrix test.
- [ ] T027-008 Codec: parse JSON/YAML, refuse unknown fields, canonical form, `sha256:` digest, closed problem list with JSON pointers. — FR-027-002..004 — `crates/infra-ir/src/apps/{codec,canonical}.rs` — evidence: fixture corpus tests; the same crate used by `apps/cli`.
- [ ] T027-009 [P] Codec diff and review items (`NEW_WRITE_BINDING`, `NEW_CUSTOM_COMPONENT`, `NEW_ALLOW_PII`, `UPDATE_WITHOUT_EXPECTED_REVISION`). — FR-027-009 — `crates/infra-ir/src/apps/diff.rs` — evidence: unit tests.
- [ ] T027-010 Migration `<ts>_internal_apps` with every table of [data-model.md](data-model.md): tenant-composite FKs, "version ≤ 1,000 per app", append-only and immutability ALWAYS triggers, partial unique request index, grants to `sagat_app`, down that refuses while rows exist; storage classification entries. — FR-027-006, 032, 034, 035 — `crates/infra-postgres/migrations/`, `crates/infra-postgres/storage-classification.json` — evidence: migration up/down test, `runtime_privileges`, classification manifest test in `integration_security_multitenancy`.
- [ ] T027-011 Application ports `AppStore`, `AppSessionStore`, `AppComponentStore`, `AppDocumentCodec` and the PostgreSQL adapters using `begin_safely`. — FR-027-001, 034 — `crates/application/src/apps.rs`, `crates/infra-postgres/src/{apps.rs,app_sessions.rs}` — evidence: integration tests with real PostgreSQL, cross-tenant FK refusal.
- [ ] T027-012 Add optional `InvocationOrigin` to the declared Query/Update, start and payload-disclosure use cases; their stores insert `app.binding_invoked` in their own transaction; existing callers pass `None`. — FR-027-031 — `crates/application/src/{declared_operations.rs,runs.rs,payload_disclosure.rs}`, matching `crates/infra-postgres/src/` stores — evidence: existing `integration_workflow_primitives`, `integration_payload_classification` and durable-execution suites pass unchanged; new atomicity test.
- [ ] T027-013 [P] HTTP skeleton: `routes/apps.rs`, `routes/app_runtime.rs`, `dto_apps.rs`, error mapping to the reasons of errors.md. — FR-027-033 — `crates/api-http/src/` — evidence: `contract_rest_internal_apps` validates responses against OpenAPI.
- [ ] T027-014 [P] Spans and metrics with closed labels; a test that no label carries an identifier. — FR-027-030 — `crates/telemetry/src/metrics.rs`, `crates/application/src/apps/` — evidence: telemetry unit test.

## Phase 3 — US01 Build and publish (P1) — MVP

- [ ] T027-015 [US1] Catalog and drafts: create (quota), list filtered by audience/authoring, detail, tombstone, draft CAS, supersede on save. — FR-027-001, 005, 035 — `crates/application/src/apps/catalog.rs` — evidence: integration tests (conflict, quota zero refuses, tombstone refusals).
- [ ] T027-016 [US1] Validation with target resolution: workflow and operation exist, mapping vs `argsSchema`/`inputSchema`, accessibility rules, classification passing, encoded-field checks per 031's registry (`ENCODING_READ_BY_FORM`, `ENCODING_NOT_PRODUCIBLE`, `ENCODING_READ_BY_EXPRESSION`), `operation` kind refused. — FR-027-004, 011, 020, 028 — `crates/application/src/apps/publishing.rs` — evidence: negative corpus, each problem code once.
- [ ] T027-017 [US1] Publication transaction: version, targets, components, activation row, pointer, idempotency, audit. — FR-027-006 — `crates/infra-postgres/src/apps.rs` — evidence: integration (replay returns the same version; one audit), fault per statement in T027-054.
- [ ] T027-018 [P] [US1] Shared renderer for `text` (Markdown subset, no HTML), `table` (virtualized, truncation notice), `chart` (title + data table), `button`, `tabs`, `form` (026 form model), using tokens only; no-sample-data states. — FR-027-003, 016, 028, 029 — `frontend/apps/studio/src/features/apps/runtime/`, `src/styles/apps.css` — evidence: Vitest, `tokens.test.ts` extended to `apps.css`.
- [ ] T027-019 [US1] Studio Apps list and editor: palette, grid canvas operable by keyboard, outline alternative, inspector, bindings panel, server validation panel, publish dialog, leave guard, conflict dialog reuse. — FR-027-027, 028 — `frontend/apps/studio/src/features/apps/{list,editor}/`, `src/app/{routes.tsx,Shell.tsx}` — evidence: Vitest, axe in component tests.
- [ ] T027-020 [US1] Browser journey author → validate → publish → open, timed for SC-027-001. — FR-027-001..006, 027 — `frontend/e2e/tests/internal-apps-author.spec.ts` — evidence: Playwright run with duration recorded.

## Phase 4 — US02 Operate with one's own authority (P1)

- [ ] T027-021 [US2] Sessions: open (`studio`, `preview`), 32-byte token, SHA-256 at rest, expiry = min(TTL, token expiry), end, quota, audit. — FR-027-014, 017 — `crates/application/src/apps/sessions.rs` — evidence: integration tests including database-clock expiry.
- [ ] T027-022 [US2] Session extractor: app session tokens accepted only on `/app-runtime/*`, refused elsewhere; JWTs refused on runtime routes; embedded session creation accepts an authorization code instead of a bearer token. — FR-027-014 — `crates/api-http/src/routes/app_runtime.rs`, `crates/api-http/src/extract.rs` — evidence: contract tests both directions.
- [ ] T027-023 [US2] Invocation dispatch for `query`, `update`, `start`, `listRuns`: pinned binding, argument validation, declared-workflow constraint, viewer authorization, derived keys `app:<app-id>:<binding-id>:<action-key>` (D1365), per-session rate, namespace bucket once, session row read `FOR SHARE` in slot 1 of ADR-0050's merged lock order (D1646); reserve the `app:` prefix on every route that takes a caller-chosen key (`RESERVED_KEY_PREFIX`) and count pre-existing prefixed records in the migration (G1373). — FR-027-011..013, 015 — `crates/application/src/apps/invocation.rs`, `crates/api-http/src/extract.rs`, `crates/api-http/src/routes/{execution.rs,declared_operations.rs}` — evidence: integration tests per kind; REST contract test that `startRun`, `updateRunState`, `updateExecutionChain` and every other key-taking route refuse an `app:` key.
- [ ] T027-024 [US2] Authorization matrix (4 roles × 5 kinds × own/other namespace/other tenant × 3 modes) and battery entries. — FR-027-013, 021, 034 — `tests/integration/internal-apps/authorization.rs` — evidence: SC-027-002 table generated by the test.
- [ ] T027-025 [US2] Faults: duplicate and lost write response, same key different arguments, timeout vs Update race, revocation between read and write (`FOR SHARE`, held lock), control-plane restart mid-session, lost broker hint for an app-started run, app-started saga with failed compensation. — FR-027-015, 031 — `tests/fault/internal-apps/` — evidence: `fault_internal_apps` with `verify_run`/`assert_consistent`.
- [ ] T027-026 [P] [US2] Runtime UI: platform-written confirmation with target run id and the argument values to be sent (≤ 32 fields, 200 characters each, `secret` form fields masked), unavailable states with reasons, in-memory action keys, no automatic re-key, bounded polling without overlap, `Retry-After`. — FR-027-015, 019, 027 — `frontend/apps/studio/src/features/apps/runtime/` — evidence: Vitest with fake timers; snapshot of the confirmation content.
- [ ] T027-027 [US2] Draft preview sessions: labelled everywhere, writes confirmed. — FR-027-017 — application + `features/apps/editor/` — evidence: integration + Vitest.
- [ ] T027-028 [US2] Browser journey operate/deny (quickstart B1–B4). — FR-027-013, 015 — `frontend/e2e/tests/internal-apps-operate.spec.ts` — evidence: Playwright run.

## Phase 5 — US03 See only what one may see (P1)

- [ ] T027-029 [US3] `runResult` through the classified disclosure with explicit `reveal: [pii]`; withheld/omitted rendering; `secret` never; "Encrypted" placeholder for envelopes (031 registry read position, never decoded or sent to a frame); `private, no-store`. — FR-027-018, 020 — `crates/application/src/apps/invocation.rs`, `features/apps/runtime/` — evidence: integration with reveal grant on/off.
- [ ] T027-030 [US3] Canary suite: values at four levels, arguments, session tokens and frame capabilities searched in DOM snapshots, all browser storages, URLs, logs, spans and audit. — FR-027-018, 019, 030, 031 — `tests/e2e/internal-apps/canaries.rs`, `frontend/e2e/tests/internal-apps-disclosure.spec.ts` — evidence: SC-027-003, zero hits.
- [ ] T027-031 [P] [US3] Lint rule forbidding storage APIs, cookies and service workers in `features/apps`; clearing on session/namespace change and sign-out. — FR-027-019 — `frontend/apps/studio/eslint.config.js` — evidence: lint fails on a seeded violation.

## Phase 6 — US04 Version, approve, roll back, promote, GitOps (P2)

- [ ] T027-032 [US4] App publication requests and decisions with the ADR-0018 §4 rules. — FR-027-007 — `crates/application/src/apps/publishing.rs`, `crates/infra-postgres/src/apps.rs` — evidence: integration (author cannot decide, supersede, recount).
- [ ] T027-033 [US4] Activation and rollback with `expectedVersion` and `revokeSessions` in one transaction. — FR-027-008 — same files — evidence: integration + fault.
- [ ] T027-034 [US4] Promotion of the same canonical document with `MISSING_DEPENDENCY`. — FR-027-010 — same files — evidence: integration across two namespaces/environments; with T027-032, T027-033 and T027-036, SC-027-005.
- [ ] T027-035 [P] [US4] Diff route and Studio versions/diff/request/rollback screens. — FR-027-009, 027 — `features/apps/editor/` — evidence: Vitest + browser journey.
- [ ] T027-036 [US4] Merge [manifest-apps.schema.json](contracts/manifest-apps.schema.json); `sagat` validate/diff/publish/promote for `apps` and `appComponents`; older-CLI refusal documented; Studio ↔ Git sync of app manifests is not part of this task (a later dependency on 028, C20). — FR-027-010, 037 — `specs/011-gitops-versioning/contracts/manifest.schema.json`, `apps/cli/src/{manifest.rs,apps.rs}` — evidence: `apps/cli/tests/manifest.rs`, idempotent re-apply.

## Phase 7 — US05 Audience and permissions (P2)

- [ ] T027-037 [US5] Audience API, effect on the next invocation, list filtering, Studio screen. — FR-027-021, 022 — `crates/application/src/apps/policy.rs`, `features/apps/admin/` — evidence: SC-027-006 tests.
- [ ] T027-038 [US5] Folder adoption (FR-027-036) behind the 029 capability: app visibility by the 029 evaluator; pending until 029 exists. — FR-027-036 — `crates/application/src/apps/` — evidence: contract test marked pending with reason until 029 lands.

## Phase 8 — US06 Embedding (P2)

- [ ] T027-039 [US6] Apps and sandbox host surfaces: exact `Host` selection, apps-host allowlist (runtime page, callback, `openAppSession`, `endAppSession`, `invokeAppBinding`, `mintAppComponentFrame`; `ROUTE_NOT_FOUND` otherwise), sandbox host serving only `getAppComponentFrame`, start-up refusal of every origin equality forbidden by ADR-0062 decision 8 (Studio, apps, sandbox, `SAGAT_PUBLIC_FORMS_ORIGIN`), codec origins (`SAGAT_STUDIO_CODEC_ORIGINS`) only in the Studio policy, CSP builder with effective `frame-ancestors`; Studio CSP gains only `frame-src <sandbox origin>/v1/app-runtime/frames/`. — FR-027-023, 025 — `crates/api-http/src/{apps_surface.rs,studio.rs}` — evidence: header contract tests for every surface at every status code (200, 4xx, 404, 429, 5xx, 503), including routing misses.
- [ ] T027-040 [US6] Embedding settings, namespace app policy, deployment configuration (`SAGAT_APPS_ORIGIN`, `SAGAT_APPS_EMBED_ALLOWLIST`, `SAGAT_APPS_SANDBOX_ORIGIN`), `EMBED_ORIGIN_NOT_ALLOWED`, `EMBEDDING_UNAVAILABLE`, `COMPONENT_SANDBOX_UNAVAILABLE`. — FR-027-023, 024 — `crates/application/src/apps/policy.rs`, `apps/control-plane` config — evidence: integration.
- [ ] T027-041 [US6] Embedded sign-in: user-initiated popup, authorization code + PKCE; the callback page posts only `{code, state}` to the exact apps origin; the runtime accepts it only from the opened popup with its single-use `state`; session creation redeems the code server-side (configured issuer only, bounded like the existing JWKS fetch) and discards the token; callback COOP. — FR-027-014, 023 — `frontend/apps/studio/src/apps-main.tsx`, `src/auth/`, `crates/application/src/apps/sessions.rs` — evidence: Vitest (wrong origin, wrong source, replayed `state` ignored); journey with `dev-idp` asserting no issuer token in page memory, storage or network responses to the page.
- [ ] T027-042 [P] [US6] Compose overlay and Helm values for the apps host, the component sandbox host and ingress routing (G1372). — FR-027-023, 025 — `infra/docker/`, `infra/helm/` — evidence: rendered chart test; compose smoke.
- [ ] T027-043 [US6] Browser embedding journey: allowed host works, refused host and Studio unframeable, host cannot read the session. — FR-027-023 — `frontend/e2e/tests/internal-apps-embedding.spec.ts` — evidence: SC-027-007.

## Phase 9 — US07 Sandboxed custom components (P3)

- [ ] T027-044 [US7] Component upload: policy gate (needs a sandbox origin), size, UTF-8, forbidden sequences matched ASCII case-insensitively (`</SCRIPT`, `</Style`, `<!--`), digest dedupe, `uploaded_by` recorded, restrict deletion, quota. — FR-027-024 — `crates/application/src/apps/components.rs` — evidence: integration with the malicious corpus, including mixed-case sequences.
- [ ] T027-045 [US7] Frame capabilities and route: `mintAppComponentFrame` per mount (60 s, single use, ≤ 256 per session, never past the session), consumption in one conditional `UPDATE`, generated document, CSP hashes over exact bytes, sandbox headers on every response, route-template logging. — FR-027-025 — `crates/api-http/src/routes/app_runtime.rs`, `crates/infra-postgres/src/app_sessions.rs` — evidence: contract test recomputing hashes; second load `404`; expired and ended-session capabilities `404`; log canary.
- [ ] T027-046 [US7] Host frame manager implementing [component-frame.md](contracts/component-frame.md): source check, schema validation, size/rate, `ready` timeout, teardown on reload, write events through host confirmation. — FR-027-026 — `frontend/apps/studio/src/features/apps/frames/` — evidence: Vitest.
- [ ] T027-047 [US7] Frame data rules: classification passing (`allowPii`, `secret` never, Query/Update results as `pii`); data-flow computation of each component's `frame_level`; publisher and uploader rights checked at publication (`COMPONENT_AUTHOR_LEVEL`) and uploaders re-checked at session open; frame-taint analysis refusing tainted targets/arguments of bindings whose results reach a frame and of write bindings (`FRAME_TAINTED_FLOW`). — FR-027-026 — `crates/domain/src/apps/component.rs`, `crates/application/src/apps/publishing.rs`, `features/apps/frames/` — evidence: unit fixtures for each flow (state → `runResult` target → frame; state → update argument; event payload → start input), integration with a developer lacking `payload:read-internal` and with a revoked uploader.
- [ ] T027-048 [US7] Browser security suite with malicious bundles (fetch, storage, parent DOM, top/self navigation, flood, oversize, reuse of a spent frame URL) and the B5 scenario (a component steering a `runResult` read while an administrator views the app is refused at publication). — FR-027-024..026 — `frontend/e2e/tests/internal-apps-components.spec.ts` — evidence: SC-027-008.
- [ ] T027-049 [US7] Merge the frame contract and G1361 into the security architecture. — FR-027-025, 026 — `docs/architecture/security-and-sandbox.md` — evidence: review.

## Phase 10 — Polish and gates

- [ ] T027-050 Leased purge loop for sessions (`engine_leases`, `app-session-purge`), bounded batches. — FR-027-035 — `crates/scheduler/src/app_sessions.rs` — evidence: integration with two engines.
- [ ] T027-051 Quotas and ceilings suite: every row of the plan's table at its limit, zero refused. — FR-027-032 — `tests/integration/internal-apps/limits.rs` — evidence: SC-027-011.
- [ ] T027-052 Accessibility: axe in light/dark/forced colours on editor and runtime, reflow at 320 px, keyboard-only authoring of the reference app. — FR-027-028, 029 — `frontend/e2e/tests/internal-apps-a11y.spec.ts` — evidence: SC-027-009, SC-027-010.
- [ ] T027-053 [P] Observability: spans and audit for every accepted/refused invocation, closed metric labels, redacted logs. — FR-027-030, 031 — `tests/integration/internal-apps/telemetry.rs` — evidence: NFR-027-004.
- [ ] T027-054 Crash at each publication, activation, revocation and write commit point. — FR-027-006, 008, 031 — `tests/fault/internal-apps/commit_points.rs` — evidence: SC-027-004.
- [ ] T027-055 [P] Benchmark of invocation overhead, session creation and render time on a declared environment. — NFR-027-005 — `tests/performance/internal-apps/`, `tests/performance/reports/internal-apps.md` — evidence: report with environment and failures; no target claimed.
- [ ] T027-056 Runbook: apps and component sandbox origins and ingress, embedding allowlist, issuer registration of the apps callback, session revocation, purge, ingress access logs (frame paths are spent capabilities; keep them out of long retention), incident response for a malicious component (disable policy, revoke sessions). — FR-027-023, 025, 035 — `docs/runbooks/internal-apps.md` — evidence: review.
- [ ] T027-057 Documentation: Studio design addendum of the apps screens for owner review (G1369), `docs/architecture/studio-ux.md`; hand traceability and index rows to the coordinator. — FR-027-027 — `specs/007-workflow-studio/design/README.md`, `docs/architecture/studio-ux.md` — evidence: owner review recorded.
- [ ] T027-058 Regression: five-SDK conformance, contract baselines (`events.baseline.json`, `worker.baseline.json`) unchanged, Studio suites. — FR-027-037 — `tests/contract/sdk-conformance/` — evidence: `cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored`, SC-027-012.
- [ ] T027-059 Independent security and correctness reviews of the diff against this spec; findings fixed or recorded. — NFR-027-001, 002 — review records — evidence: reviewer reports.
- [ ] T027-060 Gates: `cargo fmt --all -- --check`, clippy `-D warnings`, `python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py`, contract checks, `npm` gates of `frontend/AGENTS.md`. — all — evidence: commands and outputs recorded.
- [ ] T027-061 Automate the [quickstart](quickstart.md) A–D and record each step's real result, including skips. — FR-027-010, 035 — `tests/e2e/internal-apps/quickstart.rs` — evidence: SC-027-013.

## Dependencies and order

- Phase 1 → Phase 2 → US01 → US02 → US03 (the P1 MVP: an app over declared operations, authorized
  as the viewer, classification-aware). US04 and US05 follow US01; US06 needs US02; US07 needs US02
  and the namespace policy of T027-040.
- External: forms need feature 026's form model (T027-018); the `operation` kind stays refused until
  021/022; T027-038 waits for 029.
- `[P]` examples: T027-007/T027-009/T027-013/T027-014 in Phase 2; T027-018 beside T027-015..017;
  T027-026 beside T027-023.

## Implementation strategy

Deliver the MVP (US01–US03) behind the `apps` capability, then US04/US05, then embedding, then
components. Each increment merges its contracts, tests and documentation together; nothing is
advertised before its evidence exists.
