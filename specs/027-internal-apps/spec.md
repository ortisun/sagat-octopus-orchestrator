# Feature Specification: Internal apps

**Feature Branch**: `feature/027-internal-apps`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: A low-code app builder in the Studio, at parity with Windmill Apps: pages and components
(table, form, chart, text, button, tabs) whose data bindings reach declared Queries/Updates, workflow
starts and resource operations only through platform APIs; app documents versioned
draft/publish/immutable like workflows (feature 011 manifests, GitOps); app permissions; embedding
with CSP and sandboxed iframes; custom components only in sandboxed iframes; no secrets in the
browser; server-side authorization with the viewer's identity, with a decision on app-scoped
identities; classification-aware display; WCAG AA; design tokens.

## Context and delivery boundary

Teams that run processes on the platform need small internal screens — "find an order, see its
state, approve the refund, start a reconciliation" — without writing and hosting a web application.
Windmill offers this as Apps. This feature adds it under the platform's existing guarantees: an app
is a **declarative document** of pages, built-in components and **declared bindings**, published as
an **immutable version**, and opened by a person whose **own authority** is checked by the server
on every action.

What an app can do is bounded by construction:

- An app has **no backend of its own**. A binding is one of a closed set of platform operations:
  a declared Query or Update of a run (feature 014, ADR-0031), a workflow start, a run list, or a
  classified run result (feature 016, ADR-0033). Resource operations wait for features 021/022.
- An app has **no author code in the trusted page**. Built-in components are configured, not
  programmed; references and predicates form a closed language. Author JavaScript exists only inside
  a **custom component**, which runs in an opaque-origin sandboxed frame served from a dedicated
  sandbox origin, without credentials, and receives only what its author may read.
- An app **narrows, never amplifies**. Its session can invoke only the bindings of its pinned
  version; each invocation is authorized as the viewer (D1361). Delegated identities are an owner
  question (O1361), not part of this feature.

The architecture decision is proposed in
[ADR-0062](../../docs/adr/ADR-0062-internal-apps-narrow-the-viewers-authority-and-confine-custom-code.md).
Decisions, gaps and owner questions are in [research.md](research.md) (D1360–D1399 and D1645–D1646, G1360–G1375
and G1645, O1360–O1362). Nothing here is implemented.

## User Scenarios & Testing *(mandatory)*

### US01 — Build and publish an app over declared operations (Priority: P1)

An author with `app:write` creates the app `refund-desk` in a namespace, adds a page with a table of
`refund` runs, a text block, a form and a button, binds the table to a run list, the details to the
workflow's declared Query `summary`, the button to its declared Update `approve`, and the form to a
start of the workflow `refund-reconciliation`. The server validates the draft while the author
works; publishing creates an immutable version that viewers can open.

**Why this priority**: It is the product. Without a validated, immutable app version nothing else
can be trusted or tested.

**Independent Test**: Through the API and the Studio, save an invalid draft, see the server's
problems, fix them, publish, and open the active version; verify the version's digest, pinned
binding targets and audit, and that the published document cannot be changed.

**Acceptance Scenarios**:

1. **Given** a draft with a button bound to an Update the target workflow does not declare,
   **When** the author validates, **Then** the server answers every problem with a code and a JSON
   pointer, the draft stays saved, and publishing is refused with `422 VALIDATION_FAILED`.
2. **Given** a valid draft, **When** the author publishes with an idempotency key, **Then** the
   version, its pinned targets and components, the activation record, the pointer move and the audit
   commit in one transaction; repeating the request returns the same version.
3. **Given** a published version, **When** anyone tries to change it, **Then** no API changes it;
   the author edits the draft and publishes a new version.
4. **Given** a component bound to a binding that has not run, **When** the editor or the runtime
   draws it, **Then** it shows an explicit empty or not-loaded state, never sample rows (FR-007-010).

### US02 — Operate a process through an app with one's own authority (Priority: P1)

An operator opens `refund-desk`, selects a run, reads its declared summary and approves the refund.
Another person with the `viewer` role opens the same app and can see the run list but cannot read the
Query or approve.

**Why this priority**: Server-side authorization with the viewer's identity is the security
contract of the feature; it is what makes an app safe to publish to many people.

**Independent Test**: With real PostgreSQL and API, invoke each binding as admin, developer,
operator and viewer, from another namespace and another tenant, after a role revocation, with a
forged binding id, a run of another workflow, a duplicate click and a lost response.

**Acceptance Scenarios**:

1. **Given** an operator in an app session, **When** they invoke the Update binding, **Then** the
   platform-written confirmation names the operation, workflow, run and namespace and lists the
   argument values to be sent, and the Update's state, receipt, journal, outbox, Update audit and app
   audit commit together.
2. **Given** a viewer without `run:update`, **When** the same binding is invoked, **Then** it is
   refused with `403 FORBIDDEN`, audited, and nothing changes; the button is shown as unavailable
   with the reason, not hidden behind sample state.
3. **Given** a lost response, **When** the runtime retries with the same action key, **Then** the
   original receipt is returned after current authorization; the same key with different arguments
   is `409 CONFLICT` (`IDEMPOTENCY_KEY_REUSED`).
4. **Given** a binding declared for the workflow `refund`, **When** the runtime passes the id of a
   run of another workflow, namespace or tenant, **Then** the answer is an opaque `404 NOT_FOUND`.
5. **Given** a membership suspended mid-session, **When** the next binding is invoked, **Then** it
   is refused; the session does not preserve revoked authority.
6. **Given** an Update racing the run's timeout, **When** both try to commit, **Then** the first
   commit wins and the app shows the typed outcome (`RUN_NOT_MUTABLE` or the receipt).

### US03 — See only what one may see (Priority: P1)

A table shows the result of finished reconciliation runs. Fields classified `public` appear for
every run reader, `internal` for holders of `payload:read-internal`, `pii` only after an explicit,
audited reveal by a holder of the namespace reveal grant, and `secret` never.

**Why this priority**: An app puts values in front of more people than the Studio's run view; the
disclosure rules must hold for every component.

**Independent Test**: Publish an app over a workflow whose output carries canaries at every level;
open it as each role with and without the reveal grant; inspect DOM, network responses, browser
storage, logs, spans and audit for the canaries.

**Acceptance Scenarios**:

1. **Given** a `runResult` binding, **When** a caller without the reveal grant opens the app,
   **Then** `pii` values are absent from the response and the DOM, and the cell shows "Withheld"
   with the reason when the server reports a withheld node.
2. **Given** a holder of the reveal grant, **When** they choose Reveal on that cell, **Then** one
   request with `reveal: [pii]` is sent, audited before the value leaves, and the value is kept in
   memory only.
3. **Given** a closed session, a namespace switch or a sign-out, **When** the runtime ends,
   **Then** every disclosed value is dropped; nothing was written to any browser storage.
4. **Given** a declared Query result, **When** shown, **Then** it is displayed as ADR-0031 declares
   it, and is treated as `pii` if passed to a custom component.

### US04 — Version, approve, roll back and promote apps, also from Git (Priority: P2)

An author's publication into a production namespace waits for the namespace's required approvals;
an operator-visible problem is fixed by rolling back to the previous version, closing open sessions
of the bad one; a pipeline promotes the same app document from staging to production with `sagat`.

**Why this priority**: Apps are used in production; they need the controls workflows already have
(ADR-0018), but an app is useful in development namespaces first.

**Independent Test**: Exercise draft conflicts, approval rules, duplicate publication, rollback with
and without `revokeSessions`, promotion with a missing target, and `sagat` apply over a manifest with
apps, against real PostgreSQL.

**Acceptance Scenarios**:

1. **Given** a namespace whose publication policy asks for one approval, **When** the author
   publishes directly, **Then** the answer is `409 CONFLICT` (`APPROVAL_REQUIRED`); a request names
   the draft revision and digest, the author cannot approve it, and editing the draft supersedes it.
2. **Given** version 4 active, **When** an authorized person activates version 3 with
   `revokeSessions: true` and the version they saw, **Then** the pointer moves, one activation row and
   the audit are written, and open sessions of version 4 are revoked in the same transaction.
3. **Given** a promotion whose bindings name a workflow absent from the destination, **When**
   promoted, **Then** it is refused with the missing names and nothing is published.
4. **Given** a manifest listing apps and components, **When** `sagat` applies it twice, **Then** the
   second run changes nothing and every rule is enforced by the server, not by the CLI.

### US05 — Decide who may open an app (Priority: P2)

An administrator restricts `refund-desk` to two subjects and the `operator` role, and manages who may
author, publish and administer apps.

**Why this priority**: Narrowing the audience reduces exposure, but viewer identity already keeps an
app from granting authority, so it can follow the P1 stories.

**Independent Test**: Change the audience and open the app as included and excluded subjects; verify
the change, the refusal and the audit records.

**Acceptance Scenarios**:

1. **Given** a non-empty audience, **When** a subject outside it opens the app, **Then** session
   creation is `403 FORBIDDEN`, audited, and the app is absent from that subject's app list.
2. **Given** an audience change, **When** it commits, **Then** existing sessions of excluded
   subjects are refused on their next invocation.
3. **Given** another tenant's app name, **When** any route is called, **Then** the answer is
   `404 NOT_FOUND`, indistinguishable from a missing app.

### US06 — Embed an app in an approved internal portal (Priority: P2)

An administrator allows `https://portal.example.internal` to embed `refund-desk`. Staff sign in once
through a popup to the platform's identity provider and use the app inside the portal.

**Why this priority**: Embedding is a common need, but it introduces a new trust boundary and needs
an operator-configured apps origin.

**Independent Test**: In a real browser with a deployment that has an apps origin and an allowlist,
frame the app from an allowed and a refused origin, sign in through the popup, verify that the host
page cannot read the session, and that the Studio still refuses to be framed.

**Acceptance Scenarios**:

1. **Given** no apps origin configured, **When** an administrator opens the embedding settings,
   **Then** they are shown as unavailable with that reason, and no route sends a permissive
   `frame-ancestors`.
2. **Given** an allowed host, **When** it frames the app, **Then** the runtime page carries
   `frame-ancestors` with exactly the effective origins; a refused host is blocked by the browser.
3. **Given** an embedded app, **When** the viewer signs in, **Then** a user-initiated popup performs
   the authorization-code flow with PKCE and hands back only the code, by `postMessage` to the exact
   apps origin; the control plane redeems the code and opens the app session, no issuer token reaches
   apps-origin script, and no credential is accepted from the host page.
4. **Given** an origin not in the deployment allowlist, **When** a namespace administrator adds it,
   **Then** it is refused with `422 VALIDATION_FAILED` (`EMBED_ORIGIN_NOT_ALLOWED`).

### US07 — Extend an app with a sandboxed custom component (Priority: P3)

In a namespace where an administrator enabled custom components, a developer uploads a timeline
component bundle with declared properties and events and places it on a page.

**Why this priority**: Built-in components cover the common screens; custom code is the riskiest
part and must not block the rest.

**Independent Test**: Upload malicious bundles (fetch, storage, parent DOM access, top navigation,
self-navigation, message flood, oversized message, `</script>` sequence) and verify in a real browser
that each is refused or contained, and that the session token never reaches the frame.

**Acceptance Scenarios**:

1. **Given** custom components disabled in the namespace, **When** a bundle is uploaded, **Then** it
   is refused with `403 FORBIDDEN` (`COMPONENTS_DISABLED`); a version referencing a component fails
   validation with the problem `COMPONENTS_DISABLED`. Without a component sandbox origin the policy
   cannot be enabled (`422`, `COMPONENT_SANDBOX_UNAVAILABLE`).
2. **Given** a frame, **When** its code calls `fetch`, reads storage or accesses `window.parent`,
   **Then** the browser refuses it; the frame has an opaque origin and no credential.
3. **Given** a component property bound to a `pii` value without `allowPii`, **When** published,
   **Then** validation refuses it; a `secret` value can never be bound.
4. **Given** a frame that navigates itself or floods messages, **When** detected, **Then** the host
   tears the frame down and shows the component as failed; no further data is sent to it.
5. **Given** a component event mapped to a write binding, **When** emitted, **Then** the host shows
   its own confirmation; the component cannot confirm on the viewer's behalf.
6. **Given** a component uploaded by a developer without `payload:read-internal`, **When** a version
   passes it a value that can be `internal` (for example a `runResult` field), **Then** publication
   fails with `COMPONENT_AUTHOR_LEVEL`; the same holds for the app-version publisher.
7. **Given** a component that writes a run id into its state, **When** that state feeds the target
   of a `runResult` binding whose result reaches the component, or any argument of a write binding,
   **Then** publication fails with `FRAME_TAINTED_FLOW`.

### Edge Cases

- A listed run continues as new: run-addressed Query/Update keep their address and never forward
  (FR-014-013); a chain-addressed binding resolves the current head.
- The target workflow's active version changes an argument schema: invocation answers `422`
  (`INVALID_ARGUMENTS`), the app's validation reports `BINDING_CONTRACT_CHANGED`; runs of older
  versions answer per their pin. A deleted target answers `NOT_FOUND`; the page never breaks.
- Two authors save one draft revision: the second gets `409 CONFLICT` and the conflict view.
- The session expires during a write: admission uses the database clock; an admitted write completes;
  the next call is `401` (`APP_SESSION_EXPIRED`) and the runtime asks to reopen.
- Slow API under auto-refresh: requests never overlap per binding; `429` backs off with `Retry-After`.
- A binding returns more rows than the ceiling: bounded response, virtualized table, truncation shown.
- A tombstoned component stays in versions that pin it and cannot be added to new ones.
- A run id arrives in a declared URL parameter: validated and resolved only under the session's
  namespace and the binding's workflow, and shown in any write confirmation.
- Popups are blocked: the embedded runtime offers a Sign in button (a user gesture).
- An encoded field (feature 031): shown as "Encrypted", never decoded (031 registry codes only).
- The control plane restarts mid-session: sessions are rows; an in-flight action retries with its key.

## Requirements *(mandatory)*

### Functional Requirements

**Documents and lifecycle**

- **FR-027-001 — App catalog:** Namespace-scoped create/list/detail/tombstone of apps with a
  slug name unique in the namespace, a title and a description. The list returns only apps the caller
  may open or author. Creation counts against the `apps` quota.
- **FR-027-002 — App document:** A closed, versioned document (`apiVersion:
  sagat-octopus-orchestrator.io/v1`, `kind: App`) of pages with a 12-column grid layout, the
  component kinds `text`, `table`, `form`, `chart`, `button`, `tabs` and `custom`, declared bindings,
  declared URL parameters (each with a schema and a classification of `public` or `internal`) and
  pinned component versions. Unknown fields are rejected; every list, string and depth is bounded.
  A document cannot name a secret, a credential, a connection or a network address.
- **FR-027-003 — No author code in the trusted page:** Values come only from the closed reference
  language (component state, binding results, declared URL parameters, session context) and literals;
  conditions only from the closed predicates of D1366. The document cannot carry HTML, CSS, scripts,
  script URLs or expressions evaluated in the browser. Text renders as plain text or the closed
  Markdown subset of D1393, never raw HTML.
- **FR-027-004 — Server validation:** `validate` runs the publication codec on a draft and answers
  every problem (`code`, JSON pointer, safe message) and warnings, storing nothing. A draft is saved
  even when invalid; publication is what is gated (ADR-0016). The CLI uses the same codec.
- **FR-027-005 — Drafts:** One draft per app with a revision; saves use optimistic concurrency and
  a stale revision is `409 CONFLICT` with the current revision.
- **FR-027-006 — Immutable versions:** Publishing stores the canonical document and its SHA-256
  digest, the resolved binding targets, the pinned component versions, an activation record, the
  pointer move, any request state change and the audit in one transaction. A version is never
  updated; the same idempotency key returns the committed version.
- **FR-027-007 — Approvals:** In a namespace whose publication policy requires approvals, a direct
  publish is `409 CONFLICT` (`APPROVAL_REQUIRED`); an app publication request names the draft revision
  and digest and follows ADR-0018 §4 (other approvers than the requester and the last editor, one
  decision each, one rejection ends it, edit supersedes, one open request per app, approvals recounted
  against the policy in force at publish).
- **FR-027-008 — Activation and rollback:** Moving the pointer to an existing version requires a
  reason and optionally the version the caller saw (`409` `ACTIVE_VERSION_CHANGED` on mismatch),
  appends to an append-only history, and with `revokeSessions: true` revokes open sessions of other
  versions in the same transaction. Nothing is rewritten.
- **FR-027-009 — Diff:** Compare two versions or a version and the draft, listing added, removed
  and changed pages, components and bindings, and flagging new write bindings, new custom components
  and new `allowPii` property bindings as review items.
- **FR-027-010 — GitOps:** The 011 manifest gains optional `apps` and `appComponents` entries.
  `sagat` validates them with the server's codec, shows the plan and applies through the public API;
  re-application changes nothing. Promotion publishes the same canonical document byte for byte into
  another namespace of the same or a later environment, resolving targets and components by name in
  the destination and refusing what is missing.

**Bindings and authorization**

- **FR-027-011 — Closed binding kinds:** `query` (declared Query of a run or chain), `update`
  (declared Update of a run or chain), `start` (start a published workflow, active or pinned
  version), `listRuns` (run metadata of one declared workflow, bounded page, existing filters) and
  `runResult` (classified input or result of a run of the declared workflow). The kind `operation` is
  reserved and refused with `422` (`BINDING_CAPABILITY_UNAVAILABLE`) until features 021/022 provide a
  standalone operation API.
- **FR-027-012 — Server dispatch only:** The runtime invokes a binding by its id with argument
  values; the server resolves the binding from the session's pinned version, validates the arguments
  against the declared mapping and the target's schema, and calls the existing use case. A binding id
  absent from the pinned version is `404 NOT_FOUND`. A run-targeted binding accepts only a run of its
  declared workflow in the session's namespace; anything else is an opaque `404`.
- **FR-027-013 — Viewer authorization:** Every invocation, including an idempotent replay, requires
  a valid unrevoked session, the viewer's current membership, `app:read`, inclusion in the audience,
  and the permissions the underlying operation already requires (`query`: `run:read` and
  `run:query`; `update`: `run:update`; `start`: `run:start`; `listRuns`: `run:read`; `runResult`:
  `run:read` and the classification audience). Effective authority is their intersection; an app
  never grants a permission. Authorization fails closed.
- **FR-027-014 — App sessions:** Opening an app exchanges the caller's bearer token for an app
  session: an opaque token stored only as a hash, bound to tenant, namespace, app, version (or draft
  revision for a preview), subject and mode (`studio`, `embedded`, `preview`), expiring at the
  earlier of the configured TTL and the exchanged token's expiry. In `embedded` mode the caller
  presents an authorization code, its PKCE verifier and redirect URI instead of a bearer token; the
  control plane redeems it at the configured issuer, verifies the token like any bearer token and
  discards it (D1371). App runtime routes accept only app session tokens; no other route accepts them.
  Sessions can be ended by the viewer and revoked by activation, audience change or administrator
  action.
- **FR-027-015 — Writes:** Every `update` and `start` invocation carries a client-drawn action key
  reused on retry; the server derives the underlying key `app:<app-id>:<binding-id>:<action-key>`
  (D1365). The prefix `app:` is reserved: every route that takes a caller-chosen idempotency or update
  key refuses one starting with `app:` (`422`, `RESERVED_KEY_PREFIX`). The runtime shows a
  confirmation written by the platform from the pinned version (operation kind, workflow, operation
  name, namespace, target run id, and the argument values to be sent, bounded and with `secret` form
  fields masked) beside the author's label, and never retries with a new key. Write arguments and
  targets never come from URL parameters (D1395) or from a frame (FR-027-026). Publication warns when
  an `update` binding has no `expectedStateRevision` reference.
- **FR-027-016 — Forms:** A `form` component renders the server-derived form model of feature 026
  (FR-026-001..004) for the `inputSchema` of a `start` target or the `argsSchema` of an `update`
  target: supported subset, classification shown by text and shape, `secret` fields write-only and
  never prefilled, unsupported required fields making the form unavailable with the reason, values
  cleared on submission, navigation and session change.
- **FR-027-017 — Draft preview:** A holder of `app:write` may open a `preview` session on the current
  draft revision; it is labelled as a preview everywhere, runs as that author, and confirms every
  write.

**Disclosure and browser data**

- **FR-027-018 — Classification-aware display:** Components render only values the server returned.
  `runResult` uses the pinned classification (FR-016-006..009): withheld nodes render as "Withheld"
  with the reason, omitted nodes are absent, `pii` is returned only for an explicit reveal request by
  a reveal-grant holder and audited before it leaves, `secret` never. `query`/`update` results are
  shown as declared under ADR-0031. `listRuns` returns run metadata, never payloads.
- **FR-027-019 — Nothing persists in the browser:** Runtime responses carry `Cache-Control: no-store`
  (`private, no-store` where disclosure is involved). The runtime keeps values and the session token
  in memory only — no `localStorage`, `sessionStorage`, IndexedDB, Cache API, service worker or cookie
  — and drops them on session end, namespace change, sign-out and reload. No value, token or argument
  goes into a URL (except the frame capability of FR-027-025), a log or telemetry.
- **FR-027-020 — Encoded fields (feature 031):** App positions follow feature 031's encoded-field
  registry (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`) and use only its
  codes; this feature defines no encrypted-field code of its own. Display bindings are read positions:
  an envelope returned by the server is shown as "Encrypted", never decoded (no codec server in this
  feature) and never passed to a custom component. A `form` component over a target whose schema
  declares encoding is refused at publication with `ENCODING_READ_BY_FORM`, as 026's derived form is;
  a `start` binding whose input mapping reaches an encoded location is a write path an app cannot fill
  and is refused at publication with `ENCODING_NOT_PRODUCIBLE` (`startRun` refuses plaintext there at runtime with `ENCODING_REQUIRED`);
  a reference or predicate of the app language that reads inside an encoded value is `ENCODING_READ_BY_EXPRESSION`.
  Update arguments are not a carrier and cannot be encoded.

**Permissions, audience and embedding**

- **FR-027-021 — Permissions:** Add `app:read`, `app:write`, `app:publish` and `app:manage` to the
  catalog with the grants of D1386. Activation requires `app:publish`; audience, embedding, namespace
  app policy and deletion require `app:manage`. Approval of a request requires
  `publication:approve`.
- **FR-027-022 — Audience:** An app may have an audience of up to 256 entries (subjects or built-in
  roles). Empty means every holder of `app:read`; non-empty narrows it. Changes are audited and take
  effect on the next invocation.
- **FR-027-023 — Embedding:** Embedding is available only when the deployment configures an apps
  origin distinct from the Studio's; the Studio keeps `frame-ancestors 'none'`. The apps origin serves
  only the runtime page, its sign-in callback, session creation and runtime routes (never component
  frames); feature 026's public pages may share its host, never the Studio's. A runtime page carries
  `frame-ancestors` with the effective origins: app list ∩ namespace policy ∩ deployment allowlist,
  under the origin table of ADR-0062 decision 8,
  exact `https` origins only. Embedded sign-in is a user-initiated top-level popup using the
  authorization-code flow with PKCE; the callback hands back only the code, by `postMessage` to the
  exact apps origin, accepted only from the popup the runtime opened with a matching single-use
  `state`. The host page cannot supply a credential.

**Custom components**

- **FR-027-024 — Component artifacts:** In a namespace whose app policy enables custom components
  (off by default, and only in a deployment with a component sandbox origin), a holder of
  `app:publish` uploads immutable, digest-addressed component versions: a JavaScript bundle, an
  optional stylesheet, and closed bounded JSON Schemas for properties and events. The server checks
  size, UTF-8 and forbidden sequences (`</script`, `</style`, `<!--`, matched ASCII
  case-insensitively) and never executes or parses the code. A component version pinned by a
  published app version cannot be removed.
- **FR-027-025 — Component isolation:** A custom component renders only in an
  `<iframe sandbox="allow-scripts">` whose document is served with the CSP `sandbox allow-scripts`,
  hash-pinned inline scripts and styles, `connect-src 'none'`, `form-action 'none'`, no plugins,
  popups, downloads or top navigation, and `frame-ancestors` limited to the Studio and apps origins.
  Frames are served only from the dedicated component sandbox origin, which serves nothing else and
  is never the Studio, apps or public-page origin; every response of the frame route, whatever its
  status, carries the sandbox headers. A frame is loaded through a single-use frame capability minted
  per mount with the app session, valid 60 s until first use and never beyond its session; it never
  receives a session token, a bearer token or platform API access.
- **FR-027-026 — Frame protocol and data rule:** Host and frame exchange only the messages of the
  frame contract, checked by window source, validated against the pinned schemas and bounded in size
  and rate. Values effectively `public` or `internal` may be passed; `pii` only to a property binding
  marked `allowPii` in the published version (Query/Update results count as `pii`); `secret` never.
  Publication also requires the app-version publisher and every uploader of a pinned component
  version to hold, in the namespace, the rights for the highest level the version can pass to any
  frame (`COMPONENT_AUTHOR_LEVEL`); session creation re-checks the uploaders and renders a component
  unavailable when one lost them. Frame-originated values (state set and payloads sent by a frame)
  must not reach the target or arguments of a binding whose result can flow to a frame, nor of any
  write binding (`FRAME_TAINTED_FLOW`, D1399). An unexpected reload or navigation of the frame tears
  it down. A component event can set its own state or request a read binding; a write needs the
  host's confirmation.

**Studio, accessibility and design**

- **FR-027-027 — Studio:** An "Apps" destination per namespace with the list, the editor (palette,
  canvas, keyboard-operable layout, outline alternative, inspector, bindings panel, server validation
  panel, versions and diff, publish and request dialogs), the runtime view, audience, embedding,
  component and namespace-policy screens. Controls the caller cannot use are shown unavailable with the
  reason.
- **FR-027-028 — Accessibility:** Built-in components and the editor meet WCAG 2.2 AA as FR-007-008
  does: keyboard operation, visible focus, forced colours, reflow to 320 px in document order, and a
  data table alternative for every chart. Publication refuses buttons, fields and tabs without an
  accessible name, tables without a caption and charts without a title. A custom component frame has
  an accessible title; its content's accessibility is the component author's (G1365).
- **FR-027-029 — Design tokens only:** Built-in components use only the Studio's tokens for colour,
  type, radius and spacing in light, dark and forced-colour themes; authors choose semantic variants,
  never colours or CSS. Elements without data are omitted or shown unavailable, never with sample
  values (FR-007-010). Custom components receive a snapshot of token values in their init message.

**Cross-cutting**

- **FR-027-030 — Observability:** OTel spans for validation, publication, activation, session
  creation, invocation and frame serving; metrics with closed labels only (no tenant, app, binding,
  run or subject); redacted logs without arguments, results, tokens, authorization codes or frame
  paths.
- **FR-027-031 — Audit:** Audit app creation, draft update, publication, requests and decisions,
  activation, deletion, audience, embedding, policy and component changes, session open/denial/
  revocation, binding invocation and denial. A write's `app.binding_invoked` record commits in the
  same transaction as the underlying decision; a read's record commits before the value leaves and
  collapses per actor/session/binding for 300 seconds. No argument, value or token enters an audit.
- **FR-027-032 — Quotas and ceilings:** Enforce the finite ceilings of the plan (document bytes,
  pages, components, bindings, depth, apps, versions, components, bundle bytes, sessions, frame
  capabilities, message size and rate, refresh intervals, rows, audience and origins). Effective = min(request, namespace,
  deployment); zero is not unlimited; admission fails before work. Invocations charge the namespace API
  rate bucket once; documents, bundles and lists are retained content (ADR-0027).
- **FR-027-033 — Typed errors:** Reuse the closed `Error.code` set with the `details.reason`
  refinements of [contracts/errors.md](contracts/errors.md); no new code; no payload, argument or
  token in a message.
- **FR-027-034 — Tenant boundary:** Tenant and namespace on every row, query, session and audit,
  composite tenant foreign keys, and every new route in the cross-tenant battery.
- **FR-027-035 — Retention:** App deletion is a tombstone that keeps versions, history and pinned
  components; versions per app are bounded; expired sessions are purged by a leased, bounded loop.
- **FR-027-036 — Folders (when feature 029 is delivered):** Visibility of apps (list, detail, open,
  editor) is decided by the 029 evaluator when 029 is enabled. Apps adopt 029's folder contract as an
  item kind: `viewer` confers `app:read`, `editor` confers `app:write` and `app:publish`, `owner` adds
  ACL administration; `app:manage` is never conferred by a folder level. A binding's underlying
  permission is evaluated on the target workflow item by 029's evaluator, never on the app's folder.
  Until 029 exists, authorization is namespace-wide as specified above.
- **FR-027-037 — Compatibility:** REST and manifest changes are additive; existing workflows, runs,
  routes, SDKs, worker protocol, AsyncAPI and the workflow IR are unchanged. The server advertises the
  `apps` capability; an older `sagat` refuses a manifest with `apps` rather than ignoring it.

### Non-functional requirements

- **NFR-027-001 (security):** Zero tested cross-tenant/namespace access, privilege amplification
  through an app or a component author, token or canary leakage to a frame, the DOM, browser storage,
  URLs, logs, spans or audit, and zero issuer tokens in apps-origin script; any such finding blocks
  release.
- **NFR-027-002 (durability):** Publication, activation, session revocation and every write keep
  their single-transaction boundaries; a crash leaves the previous state; duplicates never create a
  second version, receipt or run.
- **NFR-027-003 (compatibility):** All existing contract baselines and suites pass unchanged; the
  five-SDK conformance suite passes as a regression.
- **NFR-027-004 (observability):** Every accepted or refused invocation has a correlated span and an
  audit record; export failures never decide a commit.
- **NFR-027-005 (performance):** Invocation overhead over the underlying operation, session creation
  and runtime render time are measured p50/p95/p99 on a declared environment; no figure is promised.
- **NFR-027-006 (accessibility):** axe-core finds no serious or critical violation in the editor or
  runtime in light, dark and forced colours.

### Key Entities

- **App** (namespace-scoped, draft, active pointer, tombstone); **app draft** and immutable **app
  version** (canonical document, digest, pinned targets and components).
- **Binding** and **binding target**: a declared, closed operation and what it resolved to at publish.
- **Activation** history and **app publication request** with decisions (ADR-0018 rules).
- **Audience entry**, **embed origin**, **namespace app policy**.
- **Component** and immutable **component version** (bundle, schemas).
- **App session** and **frame capability**: the narrowing credential and per-component frame access.

## Success Criteria *(mandatory)*

All criteria are required future evidence, not completed tests.

| Criterion | Measurable outcome | Requirements |
|---|---|---|
| SC-027-001 | An author creates, validates, publishes and opens the reference app in a real browser in under 15 minutes with no direct API call; every published version is unchangeable through every route | FR-027-001..006, 016, 027 |
| SC-027-002 | 100% of binding invocations in the role × binding matrix (4 roles × 5 kinds × own/other namespace/other tenant, in studio, embedded and preview sessions) match the expected allow/deny, with zero amplification | FR-027-011..014, 017, 021, 034 |
| SC-027-003 | Canaries at all four levels and in arguments, tokens and frame capabilities are found in zero DOM snapshots, storages, URLs, logs, spans or audit records | FR-027-018..020, 025, 026, 030, 031 |
| SC-027-004 | Fault injection at each publication, activation, revocation and write commit point leaves either the previous state or the complete new state, with zero duplicate versions, receipts or runs under duplicate and lost-response replays; an ordinary caller's key with the `app:` prefix is refused on every key-taking route | FR-027-006..008, 015, 031 |
| SC-027-005 | Approval, rollback with session revocation, promotion refusal and idempotent `sagat` re-application pass in an automated suite | FR-027-007..010 |
| SC-027-006 | Audience exclusion and change take effect on the next invocation in 100% of tested cases; with 029 present, a restricted folder hides the app and the target workflow's folder decides the binding | FR-027-022, 036 |
| SC-027-007 | In a real browser, an allowed host frames the app, a refused host and the Studio cannot be framed, the host page cannot read the session, and no issuer token appears in apps-origin script | FR-027-014, 023 |
| SC-027-008 | Every malicious bundle in the security suite is refused at upload or contained in the frame, with no credential reaching it; every frame-route response at every status carries the sandbox headers; every author-level and frame-taint fixture is refused at publication | FR-027-024..026 |
| SC-027-009 | axe-core reports zero serious/critical violations in editor and runtime in three themes; reflow at 320 px and keyboard-only authoring of the reference app pass | FR-027-028, 029 |
| SC-027-010 | No stylesheet of the app editor or runtime holds a literal colour; no component draws sample data in any state | FR-027-029 |
| SC-027-011 | Every quota and ceiling in the plan refuses before work at its limit, and zero is refused, never unlimited | FR-027-032 |
| SC-027-012 | All existing contract, SDK conformance and Studio suites pass unchanged after the additive contracts merge | FR-027-033, 037 |
| SC-027-013 | The quickstart's happy and failure demos run from a clean checkout and report any skipped step | FR-027-010, 035 |

## Assumptions

- Feature 014's run- and chain-addressed Query/Update, feature 016's classified disclosure and
  feature 011's publication machinery are available as released in 0.2.0.
- Apps run as the viewer; built-in roles stay as they are (G1362).
- The issuer used by the Studio is also used for embedded sign-in.
- Apps are internal: no anonymous or public app in this feature (026 owns the anonymous principal).
- Polling is acceptable for app bindings in this delivery (D1391).

## Dependencies

- **Existing:** 001 identity and contracts, 002 runs, 007 Studio and design direction, 008
  telemetry, 009 quotas and security, 011 versioning, 014 Query/Update, 016 classification, 017 as the
  precedent for catalogs and executors.
- **New features (018–032, drafts):** 026 (the form model FR-026-001..004 used by `form`, D1384 —
  required for forms; run-and-wait for start bindings, optional); 021 and 022 (the `operation` binding
  kind through 022's standalone admission FR-022-023, G1363); 020 (search-attribute filters in
  `listRuns`, optional); 028 (Studio ↔ Git sync of app manifests is a later dependency: 028's first
  increment excludes app documents, coordinator decision C20; the 011 `sagat` manifest path of
  FR-027-010 does not depend on it); 029 (groups and folders, FR-027-036, optional); 031 (encrypted
  fields, FR-027-020; its codec origins enter only the Studio policy, ADR-0062 decision 8); 018 (the
  merged admission lock order in ADR-0050, where the app session row is slot 1).

## Out of Scope

- A generic application designer: app-owned backends, databases, server code, arbitrary frontend
  code outside a custom component, or data sources outside the platform.
- Delegated, publisher or anonymous execution (O1361; 026 for anonymous principals).
- Resource operations before 021/022; workflow streams and live push to apps (026).
- A component marketplace or sharing components across tenants.
- A codec-server decryption of encrypted fields in apps (031).

## Owner Questions

Details in [research.md](research.md#owner-questions).

- **O1360 — Accept a bounded app builder into scope?** `docs/architecture/overview.md` defers a
  generic designer. *Recommended:* accept the bounded builder of this spec; keep a generic designer out.
- **O1361 — Allow a delegated (publisher or app-principal) execution mode?** *Recommended:* no;
  viewer identity only; revisit after feature 029 with a separate ADR.
- **O1362 — Ship custom components in the first release?** *Recommended:* yes, disabled per namespace
  until an administrator enables them, `pii` only by explicit `allowPii`, G1361 shown when enabled.

See the [requirements checklist](checklists/requirements.md), the [plan](plan.md) and the
[tasks](tasks.md).
