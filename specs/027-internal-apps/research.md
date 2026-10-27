# Feature 027 — Baseline, sources, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, on the worktree `sagat-octopus-parity-specs` (branch
`feature/018-032-parity-specs`, cut from `origin/develop` at `70f09435`, release 0.2.0). Nothing in
this feature is implemented. Every repository path below was opened or listed during the
investigation; behaviour is described as it exists at that revision.

## Repository evidence

- **Scope statement.** `docs/architecture/overview.md` ("Explicit scope left for later releases")
  lists a "Generic low-code style application designer". The constitution's Delivery Constraints do
  not exclude an app builder, but the overview does. This feature proposes a *bounded* builder and
  asks the owner to accept it (O1360); the overview text is the coordinator's to change.
- **Studio serving and CSP.** `crates/api-http/src/studio.rs` serves the Studio under `/studio/`,
  same origin as the API (ADR-0016), with `default-src 'none'`, `script-src 'self'`,
  `style-src 'self' 'unsafe-inline'`, `connect-src 'self' <issuer>`, `frame-ancestors 'none'` and no
  `frame-src`: the Studio can neither be framed nor frame anything today. The token is in memory only.
- **Declared operations.** `POST /runs/{runId}/queries/{operationName}` and
  `POST /runs/{runId}/updates/{operationName}` (and the chain-addressed variants) are implemented
  (`x-implemented: '014'` in `specs/001-platform-foundation/contracts/openapi.yaml`;
  `crates/api-http/src/routes/declared_operations.rs`; use cases in
  `crates/application/src/declared_operations.rs`). The IR declares them under `state.queries` /
  `state.updates` with `disclosure: declared-business-result-v1`
  (`specs/001-platform-foundation/contracts/workflow.schema.json`, `$defs/query`, `$defs/update`).
  `DeclaredUpdateRequest.updateKey` is the caller's idempotency key (1–128 printable ASCII).
- **Starting and listing runs.** `startRun` (`POST /runs`, `Idempotency-Key`, `StartRunRequest`
  with `workflowId`, optional `versionId`) and `listRuns` (filters `workflowId`, `status`, `origin`)
  exist; `crates/application/src/runs.rs` holds `RunService::start`.
- **Classified disclosure.** `POST /runs/{runId}/payload-disclosures` (`x-implemented: '016'`,
  `crates/api-http/src/routes/payload_disclosure.rs`) discloses input/result/journal values under the
  pinned `x-sagat-classification` carrier, with `reveal: [pii]` and a namespace reveal grant for
  `pii` (ADR-0033, FR-016-006..009). Query results are **not** classification carriers
  (FR-016-002): ADR-0031 governs them.
- **Permissions and roles.** `crates/domain/src/identity.rs` has 48 permissions and seven built-in
  roles (`admin`, `developer`, `operator`, `viewer`, `worker`, `runner`, `database-runner`).
  `viewer` holds `run:read` but neither `run:query`, `run:update` nor `run:start`; `operator` and
  `developer` hold all three. No app permission exists.
- **Publication machinery.** ADR-0018 / feature 011: drafts with CAS, immutable versions with a
  digest, append-only `workflow_activations`, publication policy per namespace (0–3 approvals) with
  `publication_requests`/`publication_decisions`, promotion of the same document, the `sagat` CLI as a
  client of the public API (`apps/cli/src/manifest.rs`), and
  `specs/011-gitops-versioning/contracts/manifest.schema.json` (root `additionalProperties: false`,
  only `workflows`).
- **Audit and quotas.** `audit_records` (migration `20261008000000_identity`) is append-only
  (`actor`, `action`, `resource_type`, `resource_id`, `reason`, `trace_id`; names in
  `crates/domain/src/audit.rs` such as `run.queried`). `crates/domain/src/quota.rs` holds count and
  byte/rate resources (ADR-0027); `crates/infra-postgres/storage-classification.json` covers every
  variable-size column.
- **Frontend and tests.** `frontend/apps/studio` (React 19, Vite 7) is the only app of the workspace;
  tokens live in `src/styles/base.css` (checked by `tokens.test.ts`); journeys in `frontend/e2e/tests/`;
  `frontend/AGENTS.md` forbids rendering withheld values or putting tokens in the DOM, logs or URLs.
  Test targets are `<kind>_<feature>` without numbers (`tests/AGENTS.md`); the cross-tenant battery is
  `tests/contract/security-multitenancy/rest/battery.rs`.

## External semantics consulted (2026-10-26)

- Windmill low-code apps ([app editor](https://www.windmill.dev/docs/apps/app_editor)): drag-and-drop
  components, "runnables" (scripts/flows) run on click, on refresh or on input change; frontend
  scripts manipulate client state; the editor is now marked legacy in favour of full-code apps.
- Windmill app policy ([`Policy` type, JSR](https://jsr.io/@windmill/windmill/doc/~/Policy);
  [changelog](https://windmill.dev/changelog/user-resources-in-apps)): an app has an
  `execution_mode` of `viewer`, `publisher`, `guest` or `anonymous`; creation defaults to
  `publisher`, so runnables execute "on behalf of publishers"; a policy enumerates the triggerable
  runnables; sandbox isolation from the viewer's session is an opt-in alpha. We take the enumerated
  policy idea and reject the publisher default (D1361).
- Temporal has no app builder; its Web UI is an operator console. Nothing to align beyond
  feature 014's Query/Update semantics.
- Browser isolation: [CSP `sandbox`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/sandbox)
  without `allow-same-origin` gives an opaque (`null`) origin with no storage or cookies;
  [`frame-ancestors`](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/frame-ancestors)
  is honoured only as an HTTP header and does not fall back to `default-src`.
- WebRTC is outside `connect-src`; the CSP `webrtc` directive is in the CSP3 draft but was found
  unshipped in browsers ([W3C issue 92](https://github.com/w3c/webappsec-csp/issues/92),
  [WebKit 255651](https://bugs.webkit.org/show_bug.cgi?id=255651); summarized from a web search, not
  re-tested here). A sandboxed frame can also navigate itself. Both ground G1361.

## Decisions

- **D1360 — Bounded builder.** Closed built-in component kinds (`text`, `table`, `form`, `chart`,
  `button`, `tabs`) plus `custom`; no app-defined backend code, no frontend expressions, no author
  HTML/CSS/JavaScript outside a sandboxed custom component. Data transformations belong in the
  workflow's declared Query/Update expressions.
- **D1361 — Viewer identity only.** Every binding executes as the viewer. Effective authority =
  viewer's current permissions ∩ the app's declared bindings ∩ the app audience. An app never
  amplifies authority; no publisher/delegated mode in this feature (O1361).
- **D1362 — App sessions are opaque, server-side.** 256-bit random tokens, SHA-256 hash stored in
  PostgreSQL, revocable at once, pinned to one app version; no new signing key. A JWT verifier
  rejects them naturally; runtime routes accept nothing else.
- **D1363 — Server dispatch.** The runtime calls one invocation route per declared binding with
  argument values; the server resolves the binding from the pinned version and calls the existing use
  case in-process. The runtime does not call the general public API.
- **D1364 — Closed binding kinds.** `query`, `update`, `start`, `listRuns`, `runResult`. `operation`
  (021/022 resource operations) is reserved in the schema and refused until a standalone-step API
  exists (G1363).
- **D1365 — Action keys (revised after review, arch S2).** The runtime draws a UUID per user action
  and reuses it on retry; the server derives the underlying key
  `app:<app-id, 32 hex>:<binding-id>:<action-key, 32 hex>`. Binding ids are limited to 48 characters
  without `:`, so the key is at most 118 characters (inside `updateKey`'s 128 and `Idempotency-Key`'s
  160). The prefix `app:` is **reserved**: `startRun`, the declared Update routes and every other route
  taking a caller-chosen key refuse a key starting with `app:` (`422`, `RESERVED_KEY_PREFIX`), so an
  ordinary client can never collide with or replay an app action, and two bindings never share a key.
- **D1366 — Closed reference and predicate language.** Dotted references into component state,
  binding results, declared URL parameters and the session context; predicates `eq`, `neq`, `in`,
  `notEmpty`, `and`, `or`, `not` (depth ≤ 4). Predicates control presentation only, never access.
- **D1367 — Lifecycle mirrors ADR-0018.** Draft CAS, immutable digest versions, append-only
  activations, approvals counted against the namespace's existing publication policy, through
  separate `app_publication_requests` tables rather than altering 011's workflow-keyed tables.
- **D1368 — Sessions pin a version.** Activation may revoke open sessions of other versions in its
  transaction (`revokeSessions`).
- **D1369 — Separate apps origin for embedding.** Embedding exists only when the operator configures
  `SAGAT_APPS_ORIGIN`, a host distinct from the Studio's. The Studio keeps `frame-ancestors 'none'`.
  The apps host serves only the runtime page, its sign-in callback, session creation and runtime
  routes; never component frames (D1397). Feature 026's public form pages may share this host under
  their own paths, never the Studio origin.
- **D1370 — Embed origins narrow.** Effective = app list ∩ namespace policy ∩ deployment allowlist
  (`SAGAT_APPS_EMBED_ALLOWLIST`); exact `https` origins, no wildcard, no path.
- **D1371 — Embedded sign-in (revised after review, sec S8).** A user-initiated top-level popup runs
  the authorization-code flow with PKCE against the configured issuer. The callback page sends only
  the **authorization code** and `state` to its opener with `postMessage` targeted at the exact apps
  origin; the runtime accepts it only if `event.origin` is the apps origin, `event.source` is the popup
  it opened and `state` matches its single-use value. The runtime posts code, PKCE verifier and
  redirect URI to session creation; the control plane redeems the code at the issuer's token endpoint,
  verifies the token as it verifies any bearer token, opens the app session and discards the token. No
  issuer token ever exists in apps-origin script. The host page never supplies a credential.
- **D1372 — Components are static bundles.** Uploaded, immutable, digest-addressed JavaScript (and
  optional CSS) with closed props/events schemas; the server checks size, UTF-8 and the absence of
  `</script`/`</style`/`<!--` sequences, matched ASCII case-insensitively (minor review finding), and
  never executes or parses the code. Off by default per namespace (O1362).
- **D1373 — Component frames.** `<iframe sandbox="allow-scripts">` plus the CSP `sandbox
  allow-scripts` header, hash-pinned inline scripts (no dependence on how `'self'` matches an opaque
  origin), `connect-src 'none'`, `form-action 'none'`, no token. Served only from the component
  sandbox origin (D1397) through a single-use frame capability in the path (D1398): a justified
  exception to "no capability in a URL" (plan, Complexity Tracking).
- **D1374 — What a component may receive.** Values effectively `public` or `internal` pass; `pii`
  only where the published version declares `allowPii` on that property binding and the viewer was
  disclosed the value; `secret` never. Query/Update results count as `pii` for this rule. The viewer's
  level alone is not enough: D1399 adds the author-side rule.
- **D1375 — Frame protocol.** `postMessage` checked by `event.source`, JSON validated against the
  pinned schemas, 64 KiB and 20 messages/s per frame by default; an unexpected second load tears the
  frame down. A component event can set state or request a read binding; a write still needs the
  host's confirmation, and frame-originated values are restricted by D1399.
- **D1376 — One renderer, two entries.** The renderer lives in the Studio package; a second Vite
  entry (`apps.html`) builds the standalone runtime. No new npm package.
- **D1377 — One codec.** App document parsing, canonical form, digest and diff in
  `crates/infra-ir/src/apps/`, shared by the API and `sagat`; the browser shows server problems.
- **D1378 — GitOps.** The manifest gains optional `apps` and `appComponents`; promotion copies the
  canonical document byte for byte and resolves binding targets by name in the destination. This is
  the 011 CLI path (`sagat` applies a manifest). Studio ↔ Git sync of app manifests is a later
  dependency on feature 028, whose first increment excludes app documents (coordinator decision C20).
- **D1379 — Accessibility at publish.** Accessible names, table captions, chart titles and data
  table alternatives are required; layouts reflow to one column at 320 px in document order.
- **D1380 — Audit in the deciding transaction.** Existing use cases gain an optional
  `InvocationOrigin`; their store inserts an `app.binding_invoked` record in the same transaction as
  the underlying decision. Reads collapse per actor/session/binding for 300 s (ADR-0031 rule 7).
- **D1381 — Quotas.** New count resources `apps`, `app_components`, `app_sessions`; structural
  limits are platform constants an operator can only lower.
- **D1382 — No SDK surface.** Apps are authored in the Studio or as manifest files; the five SDKs gain
  nothing. The SDK conformance suite is rerun as a regression only.
- **D1383 — No new message.** No outbox record, AsyncAPI message, worker-protocol minor or workflow
  IR change; underlying operations keep their own outbox writes.
- **D1384 — One form model.** A `form` component renders the server-derived form model of feature 026
  (FR-026-001..004: supported subset, classification by text and shape, `secret` write-only, no URL
  prefill), computed from a start target's `inputSchema` or an Update's `argsSchema`. One Rust
  implementation exists; if 027 is built first it implements that model where 026's plan places it,
  under 026's requirements.
- **D1385 — Audience entries.** Subjects and built-in roles, narrowing only; groups arrive with 029.
- **D1386 — Permissions.** `app:read` (admin, developer, operator, viewer), `app:write` and
  `app:publish` (admin, developer), `app:manage` (admin).
- **D1387 — Draft preview.** Holders of `app:write` may open a preview session on a draft revision;
  it runs as the author, is labelled, and confirms every write.
- **D1388 — Platform-written confirmation (revised after review, sec S9).** A write's confirmation
  names the operation kind, the target workflow, operation, run and namespace from the pinned version,
  besides the author's label, **and lists the argument values that will be sent** (paths and values,
  bounded to 32 fields and 200 characters each; `secret` form fields masked). The values shown are
  already in the viewer's page, so nothing is disclosed beyond the viewer's level.
- **D1389 — Run-targeted bindings name a workflow.** The server refuses a run of another workflow
  or namespace as an opaque `NOT_FOUND`.
- **D1390 — Navigation.** "Apps" follows Scripts in the namespace destinations, pending the owner's
  design review (G1369).
- **D1391 — Polling only.** `interval` refresh ≥ 30 s by default (floor 10 s), at most four per page;
  no SSE for bindings in this delivery.
- **D1392 — Retention.** Versions retained while the app exists (hard ceiling 1,000 per app);
  deletion is a tombstone; expired sessions purged 24 h after expiry by a leased loop.
- **D1393 — Text.** Plain text with a closed Markdown subset (emphasis, lists, headings, `https`
  links); raw HTML is never rendered.
- **D1394 — Encoded fields (031).** App positions are rows of 031's encoded-field registry
  (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`, which forbids feature-specific
  encrypted-field codes): display bindings are read positions shown as "Encrypted", never decoded and
  never passed to a frame; a `form` over an encoded target is `ENCODING_READ_BY_FORM`; a `start`
  mapping into an encoded location is `ENCODING_NOT_PRODUCIBLE` at publication (`ENCODING_REQUIRED` at
  runtime); a reference or predicate reading inside an encoded value is `ENCODING_READ_BY_EXPRESSION`. The runtime does not call a codec server in
  this delivery. Replaces this feature's earlier `ENCRYPTED_FIELD_UNSUPPORTED`.
- **D1395 — URL parameters never write.** Declared URL parameters may feed read bindings and run
  targets, never write arguments or form fields (as FR-026-004 forbids URL prefill); a write's
  confirmation shows the target run id, so a crafted link cannot silently redirect a write.
- **D1396 — Folder adoption (029).** Visibility of apps (list, detail, open, editor) is decided by
  the 029 evaluator when 029 is enabled (coordinator decision C10). An app is then a folder item kind:
  `viewer` confers `app:read`, `editor` confers `app:write` and `app:publish`, `owner` adds ACL
  administration; `app:manage` stays namespace-level, like 029's sensitive permissions. A binding's
  underlying permission is evaluated on the **target workflow** item, never on the app's folder.
- **D1397 — Component sandbox origin (review arch S1, coordinator decision 1).** Component frames are
  served only from a dedicated **component sandbox origin** (`SAGAT_APPS_SANDBOX_ORIGIN`), distinct from
  the Studio, apps and public-form origins and serving nothing but the frame route; every other path
  there is `404` with the same sandbox headers. Every response of the frame route, whatever its status
  (200, 404, 4xx, 5xx, 503), carries the sandbox CSP. Without a sandbox origin, custom components are
  unavailable and the namespace policy cannot enable them (`COMPONENT_SANDBOX_UNAVAILABLE`). A response
  that lost its headers would then run author code on an origin that holds no token and no trusted page.
- **D1398 — Single-use frame capabilities (review arch Minor).** The runtime mints a frame capability
  per mount through an app-session call; it is valid for 60 s until first use, consumed by the first
  load, and never outlives its session. A second load is refused, which the host already treats as an
  unexpected reload (teardown). A path recorded by an ingress access log is therefore already spent.
- **D1399 — Author-side data rule for frames (review sec B5 and S9, coordinator decision 2).** At
  publication the codec computes, by data flow over references, the highest level any value passed to
  a frame can have (`public` literals; declared URL parameter level; `listRuns` metadata and `session.*`
  as `internal`; `runResult` the highest level of the target carrier; Query/Update results `pii`;
  component state inherits its source). The **app-version publisher and every uploader of a pinned
  component version** must then hold the rights for that level in the namespace (`run:read`;
  `payload:read-internal` for `internal`; the reveal grant for `pii`), else publication fails with
  `COMPONENT_AUTHOR_LEVEL`. Session creation re-checks the uploaders; a component whose uploader lost
  the rights renders unavailable. Values originating in a frame (its `setState` keys and event
  payloads) are **frame-tainted**: publication refuses a tainted reference in the target or arguments
  of any binding whose result can reach a frame, and in the target or arguments of any write binding
  (`FRAME_TAINTED_FLOW`).

- **D1645 — ADR-0062 is the home of the origin and CSP refinement (coordinator decision C15).**
  ADR-0062 decision 8 lists every browser origin (Studio, apps, component sandbox, 026 public forms
  `SAGAT_PUBLIC_FORMS_ORIGIN`) with what each may serve, and the two Studio policy additions (the
  sandbox `frame-src` and 031's operator-approved codec origins `SAGAT_STUDIO_CODEC_ORIGINS` in
  `connect-src`, deployment-wide). Features 026 and 031 cite it. C15's summary lets the apps origin
  serve sandboxed frames; this feature keeps them on the dedicated sandbox origin, as round-1 decision 1
  allowed ("apps origin or a dedicated sandbox origin") and D1397 chose.
- **D1646 — The app session row is in slot 1 of the merged lock order (coordinator decision C4).**
  ADR-0050 (feature 018) is the home of the merged admission lock order; slot 1 is authority: 029
  access check → memberships → namespace row → 026 public-link row / 027 app session. A write
  invocation reads its session row `FOR SHARE` there, before the request receipt and before any chain or
  run row; activation's revocation locks the app row and then session rows and never a run, so no cycle
  exists. The merged order is a coordinator proposal for 018 to write into ADR-0050; until then this
  feature states its slot here and in the plan.

## Gaps

- **G1360** — `docs/architecture/overview.md` marks a generic low-code designer as later scope;
  needs the owner's answer to O1360 and a coordinator edit.
- **G1361** — Exfiltration by a custom component: WebRTC, self-navigation and DNS prefetch escape
  CSP. The sandbox protects credentials, the host page and other data; it is **not** a
  confidentiality boundary for values handed to the component.
- **G1362** — Viewer identity plus coarse built-in roles: a `viewer` sees read-only metadata, and
  a write needs `operator`/`developer`, granting the same rights outside the app (ADR-0031 grants are
  namespace-wide). Narrower audiences need 029 or a decision on O1361.
- **G1363** — `operation` bindings wait for 022 standalone steps (`POST /namespaces/{namespaceId}/step-executions`,
  FR-022-023, draft) and 021 for non-database resources. Until then, wrap the operation in a published
  workflow.
- **G1364** — Embedded sign-in depends on the issuer accepting the apps-origin redirect URI for the
  platform's client, the control plane reaching the issuer's token endpoint, and the host page not using
  `Cross-Origin-Opener-Policy: same-origin` (the popup needs its opener). Verified only with the
  development identity provider until tested against a real issuer.
- **G1365** — The platform cannot verify the accessibility of a custom component's content.
- **G1366** — A reload loses the in-memory action key; a later retry is a new action. Updates without
  `expectedStateRevision` and starts can then repeat; publication warns.
- **G1367** — Clickjacking from an allowed host remains possible; trusting the host is the
  administrator's decision.
- **G1368** — App versions and activation history grow until a retention policy exists (as G193/G194
  of feature 011).
- **G1369** — The design snapshot has no editor or runtime screens for apps; the owner must review.
- **G1370** — The existing `style-src 'unsafe-inline'` is inherited by the apps page; not widened.
- **G1371** — No measurement exists; latency and capacity figures are criteria to verify.
- **G1372** — The Helm chart and Compose profile have no apps or component sandbox host; ingress
  work is a task.
- **G1373** — Reserving the `app:` key prefix (D1365) refuses new ordinary keys with that prefix. A
  client already using it breaks on upgrade; the migration counts existing receipts and idempotency
  records with the prefix and the release notes say so. Existing receipts stay replayable.
- **G1374** — Ingress access logs can record frame paths; the capability is single-use and short-lived
  (D1398), and the runbook tells operators to keep such logs out of long retention.
- **G1645** — Decision C15 lets the public-forms origin share a host with the apps origin; with
  the same scheme and port they are one browser origin, so a defect in an anonymous public page could
  script an app-runtime window it opens (which holds an app session in memory). ADR-0062 says a
  deployment SHOULD use distinct hosts; sharing is a residual risk the operator accepts.
- **G1375** — Author-side checks (D1399) bound what a component author could obtain to what that
  author may already read; they cannot stop an authorized author from exfiltrating what they may read
  (G1361 remains).

## Owner questions

- **O1360 — Accept a bounded app builder into scope?** The overview defers a generic designer.
  *Recommended default:* accept the bounded builder specified here; keep a generic designer
  (own backend, own database, arbitrary code) out of scope and update the overview to say so.
- **O1361 — Allow a delegated (publisher or app-principal) execution mode?** It would let people
  without the underlying permission act through an app (Windmill's default). *Recommended default:*
  no in this feature; viewer identity only; revisit after 029, through a separate ADR.
- **O1362 — Ship custom components in the first release?** *Recommended default:* yes, disabled per
  namespace until an administrator enables it, `pii` only by explicit `allowPii`, and G1361 stated in
  the Studio when the policy is enabled.
