# Implementation plan — 027 internal apps

Status: Proposed — not started.

**Branch**: `feature/027-internal-apps` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md) |
**ADR**: [ADR-0062](../../docs/adr/ADR-0062-internal-apps-narrow-the-viewers-authority-and-confine-custom-code.md) (Proposed)

## Summary

Apps are immutable, declarative documents (pages, built-in components, declared bindings) with the
feature-011 lifecycle. A person opens an app by exchanging their token for a short-lived, opaque
**app session** pinned to one version; the runtime then invokes bindings by id on a dedicated route,
and the server dispatches each to an existing use case (declared Query/Update, start, run list,
classified disclosure) **as the viewer**, intersected with the version's declaration and the app's
audience. Custom code runs only in opaque-origin sandboxed frames served from a dedicated component
sandbox origin; they hold no credential and receive at most what their author may read. Embedding
needs a separate, operator-configured apps origin, where sign-in hands back only an authorization
code that the control plane redeems; the Studio stays unframeable. Revised on 2026-10-27 after the
independent architecture and security reviews (research D1365, D1371, D1388, D1397–D1399).

## Technical context

- **Language/Version:** Rust stable (`rust-toolchain.toml`); TypeScript/React 19 for the Studio.
- **Primary dependencies:** existing Axum, SQLx, serde (adapters only), `sagat-infra-ir` canonical
  JSON and diff, `sagat-infra-schema`; Vite multi-entry build. No new third-party dependency is
  planned; a Markdown subset renderer is written in-house or pinned after review (D1393).
- **Storage:** PostgreSQL (authority); no NATS, no object store.
- **Testing:** `cargo test` with real PostgreSQL (Testcontainers), contract tests over OpenAPI and the
  app schema, the cross-tenant battery, fault tests, Vitest, Playwright journeys driven by
  `e2e_workflow_studio`-style Rust harness.
- **Target platform:** Linux containers; Compose and Helm profiles gain an apps host and a
  component sandbox host.
- **Performance goals:** none promised; NFR-027-005 measures overhead on a declared environment.
- **Constraints:** single region; no code in the control plane; viewer identity; no browser storage.
- **Scale/scope:** bounded by the ceilings below.

## Architecture and crate placement

| Layer | Path (new unless marked) | Holds |
|---|---|---|
| Domain | `crates/domain/src/apps.rs`, `crates/domain/src/apps/{document,binding,reference,session,component,audience}.rs` | Typed app document model (no serde_json), closed kinds, references/predicates, limits, publication rules, classification passing rule, embed-origin and audience value objects, session policy, action-key derivation |
| Domain (extend) | `crates/domain/src/identity.rs`, `crates/domain/src/audit.rs`, `crates/domain/src/quota.rs` | 4 permissions and role grants; audit actions and `ResourceType::App`, `AppComponent`, `AppSession`, `AppPublicationRequest`; quota resources `apps`, `app_components`, `app_sessions` |
| Application | `crates/application/src/apps.rs`, `crates/application/src/apps/{catalog,publishing,sessions,invocation,components,policy}.rs` | Use cases and ports `AppStore`, `AppSessionStore`, `AppComponentStore`, `AppDocumentCodec`; dispatch to existing services |
| Application (extend) | `crates/application/src/declared_operations.rs`, `runs.rs`, `payload_disclosure.rs` | Optional `InvocationOrigin` parameter; their stores insert the app audit row in their own transaction |
| Codec | `crates/infra-ir/src/apps/{codec,canonical,diff}.rs` | Parse closed JSON/YAML, canonical form, SHA-256 digest, diff, problem list; used by API and CLI |
| PostgreSQL | `crates/infra-postgres/src/apps.rs`, `crates/infra-postgres/src/app_sessions.rs`, migration `crates/infra-postgres/migrations/<ts>_internal_apps.{up,down}.sql` | Tables of [data-model.md](data-model.md), triggers, grants, storage classification entries |
| HTTP | `crates/api-http/src/routes/apps.rs`, `routes/app_runtime.rs`, `dto_apps.rs`, `apps_surface.rs` | Management and runtime routes; apps-host router and per-response CSP; session-token extractor |
| HTTP (extend) | `crates/api-http/src/studio.rs`, `crates/api-http/src/extract.rs`, `routes/declared_operations.rs`, `routes/execution.rs` | Studio policy gains `frame-src <sandbox origin>/v1/app-runtime/frames/` only; host selection for apps and sandbox origins; no change to `frame-ancestors 'none'`; ordinary `Idempotency-Key`/`updateKey` values starting with `app:` refused (`RESERVED_KEY_PREFIX`) |
| Scheduler | `crates/scheduler/src/app_sessions.rs` | Leased (`engine_leases`, name `app-session-purge`) bounded purge loop |
| CLI | `apps/cli/src/manifest.rs`, `apps/cli/src/apps.rs` | Manifest `apps`/`appComponents`, validate/plan/apply/promote through REST |
| Frontend | `frontend/apps/studio/src/features/apps/{list,editor,runtime,frames,admin}/`, `frontend/apps/studio/apps.html`, `src/apps-main.tsx`, `src/styles/apps.css` | Editor, shared renderer, standalone runtime entry, frame host, admin screens |
| Contracts | `specs/001-platform-foundation/contracts/{openapi.yaml,app.schema.json}`, `specs/011-gitops-versioning/contracts/manifest.schema.json` | Merged from [contracts/](contracts/) at implementation |

No adapter crate gains a client; no domain type imports Axum, SQLx or serde_json. Existing services
are called in process, never through HTTP loopback. No empty DTO mappers: the HTTP layer maps only
where the wire shape differs (binding results reuse existing wire schemas).

## Request flows

1. **Open.** `POST /namespaces/{ns}/apps/{app}/sessions`. In the Studio the body is `{mode}` and the
   Studio bearer token authenticates. Embedded, the popup's callback page `postMessage`s only
   `{code, state}` to the exact apps origin; the runtime checks `event.origin`, `event.source` (the popup
   it opened) and its single-use `state`, then posts `{mode: embedded, authorizationCode, codeVerifier,
   redirectUri, embedOrigin}` without a bearer token. The control plane redeems the code at the
   configured issuer's token endpoint (operator-configured URL only, with the bounded client and trust
   settings the control plane already uses to fetch the issuer's keys in `crates/api-http/src/auth/jwks.rs`), verifies the
   token with the existing verifier and discards it. In both cases the service resolves tenant and
   namespace, checks `app:read`, audience, app state, the mode's origin rule and, for custom
   components, that every pinned component's uploader still holds the version's frame level (D1399);
   mints 32 random bytes, stores their SHA-256 with the pinned version and expiry, and returns the
   token and the canonical document. No issuer token is ever returned to the browser.
2. **Invoke.** `POST /app-runtime/bindings/{bindingId}/invoke` with `Authorization: Bearer <app
   session token>`. The extractor hashes the token and loads the session (one indexed read, database
   clock for expiry). The service loads the pinned binding, validates arguments, checks the viewer's
   current membership and permissions, charges the namespace API bucket once, then calls the target use
   case with `InvocationOrigin::App {app, version, binding, session}` and the derived key.
3. **Frame.** Per mount, the runtime calls `POST /app-runtime/components/{componentName}/frames`
   with the app session and receives a single-use URL on the **component sandbox origin**
   (`SAGAT_APPS_SANDBOX_ORIGIN`). `GET /v1/app-runtime/frames/{frameToken}` there (an iframe navigation
   cannot carry `Authorization`) consumes the capability in one statement (`UPDATE … SET consumed_at =
   now() WHERE consumed_at IS NULL AND expires_at > now() AND` the session is live) and returns a
   generated HTML document: platform shim and component bundle inlined, CSP hashes computed over the
   exact inlined bytes. Every response of that route and host, whatever the status, carries the frame
   headers of the table below.

## Transaction boundaries

Every transaction starts with `begin_safely` (ADR-0014) and takes the tenant from the principal.

| Operation | Commits together | Notes |
|---|---|---|
| Create app | `apps` row, empty draft revision 0, quota charge, `app.created` audit | Quota lock before insert |
| Save draft | Draft document and revision (CAS), content delta, `app.draft_updated` audit | Invalid drafts are saved |
| Publish | `app_versions`, `app_version_targets`, `app_version_components`, `app_activations` row, `apps.active_version_id`, request → `PUBLISHED` if any, idempotency record, `app.published` audit | App row `FOR UPDATE` first; targets resolved and checked inside the transaction; the frame level of the version (D1399) is checked against the current rights of the publisher and of every pinned component uploader, read under the existing membership and reveal-grant share locks |
| Request / decide / withdraw | Request or decision row, state change, audit | Supersede on draft save happens in the save transaction |
| Activate / rollback | `app_activations` row, pointer, `revoked_at` on other versions' sessions if asked, `app.activated` audit (+ `app.session_revoked` count reason) | `expectedVersion` checked under the app lock |
| Promote | As publish, in the destination namespace, plus provenance | Source version is read-only |
| Audience / embedding / policy change | Rows, audit | Readers see the change on the next invocation |
| Component upload | `app_components` (first), `app_component_versions`, quota and content charge, audit | Bytes checked before the transaction |
| Session create | `app_sessions` row, `app.session_opened` audit (embedded: after the code was redeemed outside the transaction) | Live sessions (per namespace and per subject) are counted exactly under the namespace quota lock, so expiry needs no write; denial: `app.session_denied` best effort, never authorizes |
| Session end / revoke | `revoked_at`, audit | Unconsumed frame capabilities die with the session |
| Frame capability mint / consume | Mint: `app_session_frames` row (hash, component version, `expires_at` = min(now + 60 s, session expiry)); consume: one conditional `UPDATE` | A second load finds `consumed_at` set → `404` with frame headers |
| Invoke `update` | Existing Update transaction (state, receipt, journal, audit, outbox) **plus** `app.binding_invoked` | Receipt replay precedes mutability checks but follows current authorization (ADR-0031 rule 9) |
| Invoke `start` | Existing start transaction (run, initial tasks, journal, outbox, idempotency, audit) **plus** `app.binding_invoked` | Same key + same body → same run |
| Invoke `query`, `runResult` | Existing read + audit transaction **plus** collapsed `app.binding_invoked`; bytes leave after commit | ADR-0028/0031 read-then-audit rule |
| Invoke `listRuns` | Read only; collapsed `app.binding_invoked` audit in a short transaction | No payload |
| Purge | Delete sessions expired more than 24 h ago, bounded batch, one tenant per transaction | Lease-owned loop |

**Lock order.** App row → draft → publication request → (destination) workflow rows read with
`KEY SHARE` for target resolution → quota/usage rows last (ADR-0027). Invocation takes no app lock:
the version is immutable. The app session row sits in **slot 1 (authority)** of the merged admission
lock order whose home is ADR-0050 (coordinator decision C4: 029 access check → memberships →
namespace row → 026 public-link row / 027 app session): a write reads it `FOR SHARE` there, before the
request receipt and before any chain or run row; everything after it follows the underlying use case
unchanged (including chain-before-run, FR-014-013) with usage rows last. Session revocation by activation takes the app lock, then updates
sessions by `(tenant_id, app_id)` index; invocations read sessions without locks, and a write
re-reads its session row `FOR SHARE` inside the underlying transaction (it conflicts with the
revoking `UPDATE`), so a revocation that commits first refuses the write and one that commits later
applies to the next invocation. The runtime role holds `UPDATE` on `app_sessions`, which `FOR SHARE`
requires.

## Authorization

| Permission | admin | developer | operator | viewer | worker / runner / database-runner |
|---|---|---|---|---|---|
| `app:read` (list, open published) | yes | yes | yes | yes | no |
| `app:write` (create, drafts, preview, validate) | yes | yes | no | no | no |
| `app:publish` (publish, request, activate, promote, upload components) | yes | yes | no | no | no |
| `app:manage` (audience, embedding, namespace app policy, delete) | yes | no | no | no | no |

| Action | Requires |
|---|---|
| Open a published app | `app:read`; audience includes subject or role; app not tombstoned |
| Invoke `query` | session + `run:read` + `run:query` |
| Invoke `update` | session + `run:update` |
| Invoke `start` | session + `run:start` |
| Invoke `listRuns` | session + `run:read` |
| Invoke `runResult` | session + `run:read`; `payload:read-internal` for `internal`; reveal grant and explicit reveal for `pii` |
| Decide an app publication request | `publication:approve`; not the requester or last editor |

Foreign or missing resources are an opaque `404`; within the tenant a refusal is `403`, audited
(owner decision of 2026-10-16 on G9). Replays re-authorize. The Studio hides nothing silently: a
missing permission shows the control unavailable with the reason.

## Security headers

| Surface | Policy |
|---|---|
| Studio pages | Existing policy plus `frame-src <sandbox origin>/v1/app-runtime/frames/` (027) and, from feature 031, the `SAGAT_STUDIO_CODEC_ORIGINS` in `connect-src` (deployment-wide); `frame-ancestors 'none'` kept; no author or anonymous content served (ADR-0062 decision 8) |
| Apps runtime page (apps host) | `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src <sandbox origin>/v1/app-runtime/frames/; base-uri 'none'; form-action 'none'; frame-ancestors <effective origins or 'none'>`; the sign-in callback route has COOP `unsafe-none` (the popup needs its opener, G1364) and sends the code only to the exact apps origin; `no-referrer`; `nosniff` |
| Component frame (sandbox origin; every response of the host at every status, including `404` and `5xx`) | `sandbox allow-scripts; default-src 'none'; script-src 'sha256-<shim>' 'sha256-<bundle>'; style-src 'sha256-<style>'; img-src data:; connect-src 'none'; font-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors <studio origin> <apps origin>`; `Cache-Control: private, no-store`; `Referrer-Policy: no-referrer`; restrictive `Permissions-Policy` |
| Runtime API responses | `Cache-Control: no-store` (`private, no-store` for disclosures) |

A host is selected only when `Host` equals the configured origin's host exactly; forwarded headers
are not trusted. The **apps host** serves the runtime page, its sign-in callback and the API routes
`openAppSession`, `endAppSession`, `invokeAppBinding` and `mintAppComponentFrame`; every other API
route there is `404` (`ROUTE_NOT_FOUND`). The **component sandbox host** serves only
`getAppComponentFrame`; anything else is `404` with the frame headers. The origin table of ADR-0062 decision 8 governs all
hosts: the sandbox origin differs from every other origin; feature 026's `SAGAT_PUBLIC_FORMS_ORIGIN`
differs from the Studio and sandbox origins and may share the apps host (G1645); start-up refuses a
configuration that breaks the table. Codec origins never enter the apps, sandbox or public-forms
policies.
Without a sandbox origin, custom components are unavailable (`COMPONENT_SANDBOX_UNAVAILABLE`).

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| App document canonical bytes | 256 KiB | 1 MiB |
| Pages per app | 16 | 64 |
| Components per page / per app | 64 / 256 | 256 / 1,024 |
| Bindings per app | 64 | 256 |
| Container nesting (tabs) | 4 | 8 |
| Reference length / path depth / predicate depth | 256 chars / 16 / 4 | 256 / 16 / 4 |
| Declared URL parameters | 8 | 32 |
| Apps per namespace (`apps` quota, includes tombstones' names) | 100 | 1,000 |
| Versions per app | 1,000 | 1,000 |
| Custom components per namespace (`app_components` quota) | 50 | 500 |
| Versions per component | 100 | 1,000 |
| Bundle bytes (script + style) | 256 KiB | 1 MiB |
| Custom components per app version | 8 | 16 |
| Props/events schema bytes per component | 16 KiB | 64 KiB |
| Frame message bytes / rate | 64 KiB / 20 per s | 256 KiB / 50 per s |
| Session TTL | 15 min | 60 min, never past the exchanged token's expiry |
| Sessions per subject per namespace / active per namespace (`app_sessions`) | 8 / 1,000 | 32 / 10,000 |
| Invocation request body | 64 KiB | 256 KiB |
| Invocations per session | 10 per s, burst 20 | 50 per s |
| Frame capability lifetime before first use | 60 s | 60 s, never past the session |
| Frame capabilities minted per session | 64 | 256 |
| Auto-refresh interval / auto-refreshing bindings per page | 30 s / 4 | ≥ 10 s / 16 |
| `listRuns` page | 50 | 100 (narrower than the API's 200) |
| Rows rendered from one binding / chart points | 1,000 / 1,000 | 10,000 / 5,000 |
| Audience entries per app | 64 | 256 |
| Embed origins per app / per namespace policy | 4 / 16 | 16 / 64 |

Effective values are min(request, namespace setting, deployment ceiling); zero is not unlimited and
refuses new admission; count quotas reuse feature 009's operator ceilings. Results keep the
underlying limits (Query result 256 KiB, disclosure 256 KiB). Document, draft, bundle, schema,
audience and origin bytes are `content` in `storage-classification.json`; token hashes and
session metadata are `protected_system`.

## Failure and recovery

| Failure | Behaviour | Test |
|---|---|---|
| Crash before/after publish commit | Before: nothing; after: replay with the key returns the version | fault, injected per statement |
| Lost publish/activate response | Idempotency/`expectedVersion` recover without a second row | integration |
| Duplicate click / lost write response | Same action key → same receipt or run | integration + browser |
| Same key, other arguments | `409` `IDEMPOTENCY_KEY_REUSED` | integration |
| Reload during uncertain write | Key lost (G1366); publication warned; user locates the run or state | browser |
| Timeout vs Update race (apps have no signal binding; this is the rule-9 timeout/signal race for app writes) | First commit wins; loser audited no-op (existing) | fault |
| Lost broker hint after an app start | Run proceeds after reconciliation | fault (`tests/fault/internal-apps`) |
| App-started saga whose compensation fails | Status `COMPENSATION_FAILED` shown; no app action can override it | e2e |
| Control plane restart | Sessions are rows; runtime continues; retries keep keys | fault |
| PostgreSQL unavailable | `503` `UNAVAILABLE`; runtime shows retry state, no cached values | integration |
| Audit insert fails | Write rolls back; read returns no value | fault |
| Session revoked between read and write | `FOR SHARE` session re-check refuses the write | fault with held lock |
| Frame navigates itself / floods | Host tears it down; marked failed; the spent capability refuses the reload | browser security |
| A frame response loses its headers (defect) | Author code runs on the sandbox origin, which holds no token and no trusted page | header contract test on every status code of the sandbox host |
| Component uploader loses `payload:read-internal` or the reveal grant | Next session renders the component unavailable; next publication fails `COMPONENT_AUTHOR_LEVEL` | integration |
| Apps origin misconfigured | Embedding unavailable; Studio unaffected | contract |

## Observability

- **Spans:** `app.validate`, `app.publish`, `app.activate`, `app.session.open`, `app.binding.invoke`,
  `app.frame.serve`, attributes `sagat.tenant.id`, `sagat.namespace.id`, `sagat.app.id`,
  `sagat.app.version`, `sagat.app.binding`, `sagat.app.binding_kind`, `sagat.outcome`; the
  underlying operation's span is a child of `app.binding.invoke`.
- **Metrics** (no identifiers in labels): `app_publications_total{outcome}`,
  `app_sessions_opened_total{mode,outcome}`, `app_binding_invocations_total{kind,outcome}`,
  `app_binding_invocation_duration_seconds{kind,outcome}`, `app_frame_requests_total{outcome}`,
  `app_sessions_purged_total`, `app_embedded_code_redemptions_total{outcome}`. `kind` ∈ {query, update, start, list_runs, run_result}; `mode` ∈
  {studio, embedded, preview}; `outcome` ∈ {ok, rejected, denied, not_found, invalid, rate_limited,
  unavailable, error}.
- **Logs:** route templates only (frame paths are logged as `/app-runtime/frames/{frameToken}`); no
  arguments, results, tokens, authorization codes or document content. Ingress access logs outside the
  platform may record frame paths; the capability is spent on first use (D1398, G1374) and the runbook
  keeps such logs out of long retention.
- **Audit actions:** `app.created`, `app.draft_updated`, `app.published`, `app.activated`,
  `app.deleted`, `app.publication_requested|approved|rejected|withdrawn|superseded|denied`,
  `app.access_changed`, `app.embedding_changed`, `app.policy_changed`, `app.component_uploaded`,
  `app.session_opened|denied|revoked`, `app.binding_invoked`, `app.binding_denied`; fixed reason codes
  such as `update:applied`, `start:accepted`, `query:read`.

## Migration and compatibility

- One additive migration with a tested `down` that refuses while app rows exist (no silent deletion);
  grants to `sagat_app` per table in [data-model.md](data-model.md).
- OpenAPI: new paths tagged `Apps`, `x-planned` until built, then `x-implemented: '027'`; the
  `appSession` bearer scheme; additive `details.reason` values. The cross-tenant battery lists them.
- New contract `app.schema.json`; manifest schema gains optional `apps`/`appComponents`.
- Existing services gain an optional parameter; existing callers pass `None`.
- Reserving the `app:` key prefix (D1365) is a behaviour change for ordinary callers: the migration
  counts existing idempotency records and Update receipts whose key starts with `app:` and reports the
  number (no row is changed; exact replays stay valid); the release notes state the refusal (G1373).
- `docs/architecture/overview.md` scope line is updated only after O1360 (G1360).
- Rollback: disable the `apps` capability (no session creation, no publication); keep tables,
  versions and audit; the Studio shows Apps unavailable.

## Constitution Check

| Principle | Status | Design evidence |
|---|---|---|
| I. State and retry explicit and durable | Pass | Publication/activation/revocation one transaction; writes reuse existing transactions plus one audit row; first commit wins; no terminal reopen |
| II. Ports and adapters | Pass | Domain model without serde_json; codec in `infra-ir`; SQL in `infra-postgres`; no client in domain/application |
| III. At-least-once, sagas compensate | Pass | No new effect; derived idempotency keys; no exactly-once claim; G1366 stated |
| IV. Versioned and pinned | Pass | Immutable app versions, sessions pinned, additive contracts, five-SDK regression |
| V. Code, tenant, secrets isolated | Justified | No code in control plane; author code only in browser opaque-origin frames on a dedicated sandbox origin, receiving at most what its author may read (D1399), not a confidentiality boundary for passed data (G1361, G1375, Complexity Tracking) |
| VI. Observability and audit | Pass | Spans, closed metrics, audit in the deciding transaction |
| VII. Inbox/outbox | Pass | No new message; underlying outbox unchanged |
| VIII. Failure tested honestly | Pass | Rule-9 matrix above; nothing marked done without evidence |

### Complexity Tracking

| Choice | Why needed | Simpler alternative rejected |
|---|---|---|
| Separate apps origin for embedding | Keeps the Studio unframeable and limits what a framed page can reach | Per-route `frame-ancestors` on the Studio origin: one origin for embedded and privileged pages |
| Frame capability in a URL path (single-use, 60 s, session-bound) | Iframe navigation cannot carry `Authorization`; avoids cookies and existence oracles; a logged path is already spent | `srcdoc` (inherits the parent's CSP, would need per-version page CSP); cookies (third-party blocking, CSRF) |
| Dedicated component sandbox origin | A response that loses its sandbox header must not run author code beside a token (review arch S1) | Frames on the Studio or apps origin: one missing header runs author code same-origin with a session |
| Server-side redemption of the embedded authorization code | No issuer token in apps-origin script (review sec S8) | Code exchange in the browser: leaves a general API token in the embedded page |
| Server-side sessions | Instant revocation, version pin, no new signing key | Signed JWT app tokens: new key management, no instant revocation |
| Browser-run author code at all | Parity with custom components | No custom components: blocks O1362; kept off by default |

## Delivery sequence

1. Contracts, ADR acceptance, domain model, codec, migration and ports (T027-001..014).
2. Persistence, catalog, drafts, validation, publication, activation (US01, US04 core).
3. Sessions and invocation dispatch for the five kinds (US02, US03).
4. Studio editor and runtime; accessibility gates (US01–US03).
5. Approvals, promotion, CLI manifest (US04); audience (US05).
6. Apps origin, embedding, popup sign-in (US06).
7. Components and frames (US07), behind the namespace policy.
8. Fault, security, performance, runbook, contract merge and gates.

## Verification plan

Cargo targets (no feature numbers): `integration_internal_apps`, `contract_rest_internal_apps`,
`fault_internal_apps`, `e2e_internal_apps`, `perf_internal_apps`, under
`tests/{integration,contract,fault,e2e,performance}/internal-apps/`; browser journeys
`frontend/e2e/tests/internal-apps*.spec.ts`; regression of `sdk_conformance`,
`contract_rest_security_multitenancy`, `contract_rest_workflow_studio` and the Studio unit suite.
Every check records its real result in the feature's tasks; pending or skipped checks are stated.
