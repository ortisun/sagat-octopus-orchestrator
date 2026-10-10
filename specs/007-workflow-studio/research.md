# Research & Decisions: Visual Studio, Monaco and runtime graph

## Main decision

React Flow for the DAG; Monaco in the panel; versioned server-side state (optimistic concurrency) and runtime graph projected by event cursor; virtualization of large DAGs.

## Rationale

Offer intuitive authoring and operation of complex processes with a faithful depiction of executions. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## What already exists (read on 2026-10-10, before deciding)

- **REST**: `createWorkflowDraft`, `getWorkflow`, `updateWorkflowDraft` (compare-and-swap on `draftRevision`, `409` when
  stale; the draft is checked only for being storable JSON of at most 1 MiB), `publishWorkflow` (validates the draft
  against the schema, the graph rules and the expression subset; `422` with one message), `listWorkflowVersions`,
  `getWorkflowVersion`, `listRuns`, `getRun`, `listRunJournalEvents` (cursor, **no payloads**: a `payloadRef` digest
  only), `getRuntimeGraph` (the definition's graph overlaid with the state of every step, `projectionSeq` = the run's
  version), `operateRun` (pause, resume, cancel, retry step, retry compensation, skip compensation, resolve step;
  `RERUN_AS_NEW` and `MARK_MANUAL_REVIEW` are refused with `422`, G58), the scripts API of feature 005 (draft `PATCH`,
  test runs, versions), `GET /auth/context`.
- **Not there**: a dry-run validation with a list of problems; a diff of two documents; a way to read only what changed
  in a run's graph; any push channel (no SSE, no WebSocket); serving of static files; CORS; a browser login (the API
  verifies OIDC/JWT bearer tokens, there is no issuer in the repository and no dev token issuer, G10).
- **The IR** (`workflow.schema.json`) has the node types `activity`, `timer`, `waitEvent`, `condition`, `parallel`,
  `child`, `saga`, `script`, `foreach`, `humanApproval`, `end`; triggers are a list on the document, not nodes; a join is
  the `next` of a `parallel`. `foreach` and `humanApproval` are accepted by the schema and refused at publish (no engine
  yet). A definition may have 10 000 nodes (`MAX_NODES`); the compensation and reconciliation nodes the engine derives
  are steps of the run too.

## Decisions (feature 007: D138 onwards)

- **D138 — The Studio is a separate frontend workspace** (`frontend/`, npm workspaces; `apps/studio`): React 19,
  TypeScript `strict`, Vite, React Flow (`@xyflow/react`) for the canvas, Monaco bundled locally (no CDN: the CSP allows
  only the platform's own origin and the editor must work offline). Dependencies are pinned by `package-lock.json`, each
  at least two weeks old when chosen, installed with `npm ci --ignore-scripts`. No UI kit with its own theming: plain CSS
  with design tokens, light, dark and forced-colors themes; accessible primitives from a small headless library only
  where focus management is hard (dialogs, tabs).
- **D139 — The server owns validation.** A new `POST /workflows/{id}/validate` runs the same codec as publish on a
  document given in the body (or the stored draft) and answers **200** with a list of problems (`severity`, `code`,
  `message`, `nodeId`, `path`) and the warnings of publish, valid or not. A draft is still saved when it is invalid (the
  author is mid-edit); **publishing is what is gated**, in the server (`422`) and in the UI (the button explains why it
  is disabled). Advisory lints the server does not know (a node with no timeout, a compensation nobody maps) are done in
  the client, are labelled advisory, and never block.
- **D140 — Diff and breaking-change classification live in `sagat-infra-ir`**, over two IR documents, and are served by
  `GET /workflows/{id}/diff?from=&to=` (a version number or `draft`; default: active version to draft). The domain does
  not change. **Breaking** (a caller or an emitter of the contract can fail): a trigger removed or changed; an
  `inputSchema` property removed, its type changed or a `required` entry added; an `outputSchema` property removed or
  its type changed; a `waitEvent` whose event type was removed or renamed (publishers keep sending the old one).
  **Attention** (not a contract break, but somebody must act): an activity name added or changed (a worker must serve it
  and be bound to it), a secret name added, a script digest changed. Everything else is a plain change.
- **D141 — Live updates are journal-cursor deltas.** `GET /runs/{id}/graph/changes?since=<seq>` returns the run status, the new
  `projectionSeq`, the journal events after `seq` (light: seq, type, step, attempt, time) and the **full objects of the
  nodes those events name or create** (a node changes only through events that name it or that activate it: a routing, a fork,
  a saga entered, a compensation plan, a lookup carry the steps they create, `RunEvent::changed_nodes`; the equivalence of the
  deltas with the whole graph is a test, over a chain and over a saga whose undos wait for each other, and an invariant of the
  domain's chaos test), not the whole graph; `reset: true` with the full graph
  when the cursor is ahead of the run or more than 500 events behind. `GET /runs/{id}/stream` is the same payload as
  `text/event-stream` (`id:` = the projection seq, `Last-Event-ID` and `?after=` resume) and the polling fallback is the
  delta endpoint, so a missed signal between two snapshots is impossible: every transition is a journal event with a
  sequence. The browser reads the stream with `fetch` (an `EventSource` cannot send the `Authorization` header).
- **D142 — A stream is bounded.** At most `N` per tenant and `M` per subject (`429` beyond), polled on the server every
  500 ms and backing off to 2 s when nothing moved for 30 s, a comment line every 15 s, closed at the token's expiry and
  after 15 minutes at the latest (`event: reconnect`), and closed when the run is finished and the last delta is sent.
  Polling the database per stream is the honest first version; a push from the engine (`LISTEN/NOTIFY` or NATS) is
  feature 010's (G143).
- **D143 — The control plane serves the Studio** under `/studio/` from `SAGAT_STUDIO_DIR` (off when unset), same origin as
  the API, so no CORS is added. Headers: a CSP of `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; base-uri 'none';
  form-action 'self'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `Cross-Origin-Opener-Policy: same-origin`; hashed assets are `immutable`, `index.html` is `no-store`. A Studio on
  another origin is an ingress decision, not a feature of the API.
- **D144 — The browser holds its token in memory only.** Sign-in is a bearer token the user pastes (development and
  automation) or, when the deployment configures an issuer in `/studio/config.json`, the OIDC authorization-code flow with
  PKCE; the token is never written to `localStorage`, `sessionStorage`, a cookie or a URL, and it is dropped on reload.
- **D145 — The node registry maps the IR, not the product words.** *Trigger* and *Webhook* are entries of `triggers[]`
  drawn as start cards (an event/webhook/message/cron/manual trigger); *Step* is `activity` or `script`; *Wait* is
  `waitEvent`; *Timer* is `timer`; *Join* is the `next` of a `parallel` (drawn as a join handle, not a node); *Saga*,
  *Condition*, *Parallel*, *Child* and *End* are themselves. `foreach` and `humanApproval` are drawn, flagged "not run by
  this engine yet", and cannot be published (the server says so).
- **D146 — Two people editing the same draft** is a `409` on `draftRevision`. The Studio keeps the author's copy, shows
  the server's, offers *reload theirs*, *keep mine and save over* (after a diff of the two) and *save mine as a copy*.
  Nothing is overwritten without a diff on screen.
- **D147 — Large graphs degrade by thresholds, not by freezing.** Up to 1 000 nodes the canvas draws every card and is
  interactive (no per-node effects; the benchmark measured 59–60 frames per second panning and zooming 1 000 drawn cards);
  above 1 000 it draws simplified cards and only the ones in view (`onlyRenderVisibleElements`); above 10 000 (the
  definition limit, with derived nodes more) it opens in the list view and offers the canvas on demand. Lists of
  10 000+ rows (runs, journal, table view) are windowed (29 rows in the page at most, of 9 991).
- **D148 — The alternative view is first class.** Every graph has a table/list view with the same data and actions (a
  list of nodes with status, attempts, duration, failure, and the actions available), reachable by keyboard, and the
  default for assistive technology users who ask for it (`prefers-reduced-motion` and a visible toggle). WCAG 2.2 AA is
  a target checked by automated axe-core runs and a keyboard walkthrough in the journeys; a screen-reader test by a person
  is not part of this feature (G146).
- **D149 — Operational actions explain, then ask twice when it matters.** The confirmation says what the server will do
  (which step, what stays in the journal, what a compensation or a skip means for the saga) in the words of the
  operation's description, requires the reason the API requires (8 to 2 000 characters), and for the decisions that need
  `saga:override` asks for the step id typed back. The server is the authority: the buttons shown follow `/auth/context`
  and the node's state, and a `403`/`409` is shown as what it is.
- **D150 — Evidence**: Rust contract, integration and fault tests for the new endpoints; Vitest for the logic and the
  components; Playwright journeys (US01..US04 and the whole of SC-007-001) played by a harness that boots the real
  platform and a TypeScript SDK worker, in the same way as the SDK conformance suite; axe-core in the journeys; a browser
  benchmark of a 1 000-node canvas and a 10 000-row list.
- **D151 — The document carries no layout.** Positions are computed (dagre; a linear layered layout above 1 500 nodes) and
  the positions a user dragged stay in the browser per workflow (G148). The document is what the platform stores, hashes and
  compares; a move of a card is not a new version, and a diff shows changes of meaning only.
- **D152 — Links are never typed.** A link between nodes is made on the canvas or chosen from a list of the nodes that exist, through
  edits that refuse a cycle, a link to itself and a link the kind of node does not have; renaming a node follows every reference.
  The document cannot be left pointing at a node that is not there by the Studio's own doing.
- **D153 — The CSP allows inline styles and no inline script.** `style-src 'self' 'unsafe-inline'`: the code editor and the canvas
  set styles from script, and CSS cannot run code; `script-src 'self'` has no `unsafe-inline`, no `unsafe-eval`, no remote origin.

## Gaps (feature 007: G142 onwards)

- **G142 — Graph validation reports one graph error at a time.** The domain stops at the first rule broken (a cycle, an
  unreachable node); schema problems are all listed. The Studio shows the first and revalidates after each fix.
- **G143 — Streams poll the database** (D142): every open stream reads the run's own row at each tick (every 0.5 s while the
  run moves, every 2 s once it has been still for half a minute, each with up to a quarter more so that streams do not wake in
  step) and reads the graph, a snapshot of every step of the run, only when the row's version moved; a quiet stream costs what
  `GET /runs/{id}` costs. Bounded per subject (20), per tenant (200) and in the process (2 000), and every 30 s each stream asks
  again whether its subject is still a member with the right to read. Before the independent review each tick read the whole
  graph (120 scans of the steps table in four seconds for one quiet stream). A push from the engine (`LISTEN/NOTIFY` or NATS)
  belongs to feature 010 with the rest of the scale work.
- **G144 — No payload is shown.** The journal and the graph carry no input, output or error text (a `payloadRef` digest
  and a `failureCode` only), so "redacted payload" holds by construction and the Studio cannot show a step's data. Showing
  it needs a redaction policy per payload class (feature 009's classification is not built, G37) and a read API.
- **G145 — "Retry run as new" and "mark manual review" are not available** (`422` from the API, G58): the Studio shows
  them disabled with the reason, it does not fake them.
- **G146 — Accessibility is checked by tools and a keyboard walkthrough**, not by people using a screen reader; WCAG 2.2
  AA is a target, not a claim.
- **G147 — Human approval and foreach nodes cannot be published** (no engine, 002/003): the canvas draws them so a
  document that has them can be read, and says so.
- **G148 — Manual positions are per browser.** A card a user moves is remembered in that browser only (not shared, not in the
  document); another person, or the same one elsewhere, sees the computed layout.
- **G149 — The browser sign-in with an identity provider (authorization code with PKCE) is built from the deployment's public
  settings and tested against a stand-in provider, played by the journeys (`tests/oidc.spec.ts`: discovery, the redirect, the
  verifier checked against the challenge, a wrong state, a refusal); it has not been run against a real one.** The pasted token
  is the supported development path (D144).
- **G150 — The code editor offers syntax colouring and the language service the browser has** (TypeScript and JSON only); for
  the other languages the platform checks the source when it builds it, and the page says so.
- **G151 — The "reference browser" of NFR-007-PERF is not defined by the specification, and the benchmark measures one.** The
  report (`tests/performance/reports/007-workflow-studio.md`) says which: headless Chromium on the machine it names. A low-end
  machine, Firefox and Safari are not measured, and the floors of the report ("usable": 30 frames per second, a few seconds to
  open) are this feature's reading of "usable", chosen before the measured rounds.
- **G152 — Two low advisories remain in the Studio's production dependencies.** `monaco-editor` 0.57.0 bundles DOMPurify
  `<=3.4.15` (GHSA-6688-9rhm-gjv2, an `IN_PLACE` sanitising case); npm's suggested fix is to go back to 0.56.0, which loses the
  editor features the script page uses. The Studio does not hand the editor markup from outside (it shows the author's own
  source and the platform's diagnostics as text), and the page's policy forbids inline script, so the advisory is accepted and
  the CI job (`npm audit --audit-level=moderate`) lets lows through until Monaco ships a fixed bundle. The tooling advisories
  that were open when the frontend was built (vitest's `tinypool`, `source-map-js`) are fixed by vitest 4.1.11 and a
  lockfile update.
- **G153 — The `operationId` of an operation is derived, not stored.** It is a version-8 UUID made from the run and the
  `Idempotency-Key`, so an operation sent again because its answer was lost is answered with the same id (before, every answer
  carried a new random one, which made a replay look like a second operation). It names the request: there is no table of
  operations to look it up in, and the record of what was decided is the journal event (`step.retry_requested`, `saga.step_skipped`…).
- **G154 — The run does not say whether a stop was asked for.** `status` shows `CANCELLING` for a run asked to stop, but a run
  that compensates or waits for a person shows `COMPENSATING` or `MANUAL_REVIEW` whether or not it was, and the platform refuses
  *pause* and *cancel* of a run that was (`409`). The Studio disables them only for `CANCELLING` and leaves the rest to the
  platform, whose refusal it shows as it is. Exposing the stop (`stopReason`) in the run is a contract change for a later feature.
- **G155 — The breaking-change classifier checks a set of kinds of change, not every change to a contract** (D140): a trigger gone
  or changed, an input property gone, retyped or newly required, an output property gone or retyped, an awaited event type gone.
  It does not judge an output property that stops being required, a type gained where there was none, a narrower `enum`, a new
  `pattern` or limit, `additionalProperties: false`, the items of an array or a `oneOf`; the publish dialog says which kinds it
  checked instead of saying there were no breaking changes.
- **G156 — A trace is shown as an id to copy, not a link** (FR-007-005 "trace links"): the Studio does not know where a deployment
  keeps its traces. Opening one in a tracing tool needs a configurable URL template (`SAGAT_STUDIO_TRACE_URL`), not built.
- **G157 — The code of a script is edited on the Scripts page; a script node pins a build.** The node's inspector does not open the
  editor (US02-AC01 says "edits code in a node"): what the user story needs, editing the code and having a workflow use the build,
  is done in two places, with the picker between them (`ScriptPicker`); a link from the node to the editor is not built.
- **G158 — The files of the Studio have no request metrics or trace context**: their router is merged after the API's protections
  (D143), on purpose (no credentials, no tenant), and is bounded by its own limits (64 files in flight, a minute each, 30 s of
  stall), but the platform's `http_requests` series does not count them.

