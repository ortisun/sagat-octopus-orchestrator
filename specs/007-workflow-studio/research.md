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
- **D154 — The visual direction is a Claude Design project; the repository keeps a dated record of it** (`design/README.md`, with the
  reference image `design/runs-prototype.jpg`). The project is mutable and private to its owner, so the Studio is built from what
  the repository records, and a change in the project reaches the Studio by updating that record first (the same change as the
  code that follows it, AGENTS.md rule 10). The record says what was measured and what is not known (G160); a value is not
  written down that was only guessed.
- **D155 — The accessibility floor wins over the direction.** FR-007-008 holds in every theme: where a colour of the prototype fails
  its threshold (text 4.5 : 1, the boundary of a control and a focus ring 3 : 1) the Studio uses a lighter or stronger value, and
  the deviation is listed in `design/README.md` with its ratio. Decorative lines and card borders keep the prototype's weight.
  Rejected: following the prototype to the letter, which would relax a constraint the journeys test (a product decision, not an
  implementation detail).
- **D156 — The labels stay in English; the prototype's Portuguese is mapped in `design/README.md`.** The platform's errors, the
  documentation and the existing journeys are English; a translation is an internationalisation feature, not a restyling.
- **D157 — Light and dark are both themes of the Studio, derived from the same tokens, and the default is the system's colour
  scheme.** Decided by the owner on 2026-10-10. Dark is the theme the prototype is drawn in, so its values are exact and it is
  reviewed first; the light theme is not in the design and is derived from the same hues and the same contrast floor (G160, G161),
  for the owner to review. Nothing forces a theme on a person who set their system to the other. The `data-theme` attribute stays the
  explicit override that the stylesheet and the code editor already read (the editor follows it and the system's scheme); the
  original journeys did not exercise both themes; T007-044 now covers light, dark and forced colours. A control in the interface to choose a theme is not part of
  this decision and is not built.
- **D158 — The Runs screen is a composition, not a new route.** On a wide viewport the list and the detail pane sit side by side;
  the URL of a run stays `/runs/:id`, so a link, a reload, the back button and the journeys keep their meaning; where the two
  stop reflowing (320 px, WCAG 1.4.10) it is the two pages it is today. The *Table* tab, the accessible alternative to the graph,
  stays beside the prototype's tabs.
- **D159 — The fonts (Geist, Geist Mono) are bundled, not loaded from Google Fonts.** The prototype links them from a remote
  stylesheet; the Studio's policy is `default-src 'none'` with `font-src 'self'` and `style-src 'self' 'unsafe-inline'`, and
  D138 forbids a CDN so that the editor works offline. The faces are open (SIL OFL), so they are served from the platform's own
  origin as files of the build; the packages that carry them follow D138 (pinned by `package-lock.json`, at least two weeks old,
  `npm ci --ignore-scripts`) and are added only with the licence read. The system stacks of `base.css` stay as the fallback
  (G166).

## Gaps (feature 007: G142 onwards)

- **G142 — Graph validation reports one graph error at a time.** The domain stops at the first rule broken (a cycle, an
  unreachable node); schema problems are all listed. The Studio shows the first and revalidates after each fix.
- **G143 — Streams poll the database** (D142): every open stream reads the run's own row at each tick (every 0.5 s while the
  run moves, every 2 s once it has been still for half a minute, each with up to a quarter more so that streams do not wake in
  step) and reads the graph, a snapshot of every step of the run, only when the row's version moved; a quiet stream costs what
  `GET /runs/{id}` costs. Bounded per subject (20), per tenant (200) and in the process (`SAGAT_STREAMS_MAX`: at most half of the connections the server accepts, because an open stream holds its connection), and every 30 s each stream asks
  again whether its subject is still a member with the right to read. Before the independent review each tick read the whole
  graph (120 scans of the steps table in four seconds for one quiet stream). A push from the engine (`LISTEN/NOTIFY` or NATS)
  belongs to feature 010 with the rest of the scale work.
- **G144 — Structural payload inspection is implemented; value disclosure remains out of scope.** The journal and graph
  still carry only a `payloadRef` digest and a `failureCode`. An Admin or Operator may explicitly inspect the run input or
  an immutable journal sequence through ADR-0028's authorized, audited `structure-only-v1` endpoint. The server removes
  every object key and scalar, bounds the tree, and returns only verified catalog metadata for external content. Customer
  classification and any policy that could disclose values remain pending under G42/G39.
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
  report (`tests/performance/reports/workflow-studio.md`) says which: headless Chromium on the machine it names. A low-end
  machine, Firefox and Safari are not measured, and the floors of the report ("usable": 30 frames per second, a few seconds to
  open) are this feature's reading of "usable", chosen before the measured rounds.
- **G152 — Resolved: Monaco uses patched DOMPurify 3.4.16.** `monaco-editor` 0.57.0 declares DOMPurify 3.4.15, so the frontend
  root pins that transitive dependency to 3.4.16, which fixes GHSA-p98j-92pf-mc4p and GHSA-6688-9rhm-gjv2 without downgrading
  the editor. `npm explain dompurify` proves the override and `npm audit --audit-level=low` reports no vulnerabilities; the
  Studio's 1,105 tests, lint, typecheck and production build pass. The tooling advisories that were open when the frontend was
  built (vitest's `tinypool`, `source-map-js`) remain fixed by vitest 4.1.11 and the lockfile.
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
- **G156 — Resolved: trace links use deployment runtime configuration** (FR-007-005, T007-055).
  `SAGAT_STUDIO_TRACE_URL` supplies one `{traceId}` placeholder through the existing public,
  no-store Studio config. Startup and serving reject relative, credentialed and non-HTTPS URLs
  (apart from loopback HTTP); the browser independently requires a nonzero 32-hex id and applies
  the same URL boundary. No config omits the link while preserving copy, and rendering never
  contacts the tracing tool.
- **G157 — The code of a script is edited on the Scripts page; a script node pins a build.** The node's inspector does not open the
  editor (US02-AC01 says "edits code in a node"): what the user story needs, editing the code and having a workflow use the build,
  is done in two places, with the picker between them (`ScriptPicker`); a link from the node to the editor is not built.
- **G158 — The files of the Studio have no request metrics or trace context**: their router is merged after the API's protections
  (D143), on purpose (no credentials, no tenant), and is bounded by its own limits (64 files in flight, a request waits two seconds
  for a place, a minute for a file, 30 s of stall), but the platform's `http_requests` series does not count them. **A trade-off:**
  the files are public, so 64 clients that each read an asset slowly but steadily can hold every place for up to a minute and make
  the Studio's files answer `503` meanwhile (the API is not affected: it has its own limits); the connection cap and the minute
  bound it, and the answer says when to come back.
- **G159 — The Studio sends no telemetry from the browser.** There is no OpenTelemetry web SDK and no collection of client-side
  errors: what a person sees fail is shown with the trace id the platform gave the request (`ErrorState`, SC-007-004), and the
  platform's own spans, request metrics and the stream gauges and counters (`run_streams_open`, `run_streams_refused_total`,
  `run_streams_tick_errors_total`) are what an operator has. A page that fails before it talks to the platform leaves no record.
- **G160 — The design is exact for the dark theme of the Runs screen only.** The owner's export of 2026-10-10 gave the source of
  `sagat-octopus.dc.html`, and `design/README.md` is written from it: colours in `oklch`, the faces, the type scale, radii, sizes,
  the graph. (A first version had been written from a capture of the running prototype; one of its numbers was wrong and is
  corrected in the record.) **Still not in the design:** the light theme, focus rings, disabled states, motion, loading and error
  states, and how it behaves below 1180 px (the prototype is fixed at a minimum width of 1180 px). Those are derived by the
  Studio from the same tokens and are the owner's to review (G161).
- **G161 — Only the Runs screen is designed.** The workflow editor (canvas, palette, inspector, validation, publish, diff), the
  Scripts pages, the sign-in and the empty, loading and error states have no reference. The owner asked for all screens at once;
  they are derived from the same tokens and language **without a design**, and each is a proposal for the owner to review, not an
  implementation of one (T007-043).
- **G162 — The logo is undecided.** The project holds three geometric options and a provisional "S" mark. The shell uses the text
  brand and a plain "S" monogram as a provisional treatment; this does not select one of the geometric logos. Owner approval remains open.
- **G163 — The shell of the prototype lists what the platform does not have.** Schedules, Workers, Task queues, Secrets and
  Settings are other features' pages (no link to a page that does not exist); the counts beside the navigation and the filters
  need a count the API does not give (it pages by cursor); the sidebar's active workers and queue backlog have no read API. **Scripts
  is a page of the Studio that the prototype's navigation does not list**: it keeps an item, and where it sits is the owner's call.
- **G164 — A run has no queue, no worker and no trigger kind.** `Run` carries `startedBy`, not the way it was started; the strip of
  facts shows what exists (*Started*, *Duration*, *Started by*, and what the Studio already shows) and omits the rest. Adding them is
  a contract change for the features that own the run.
- **G165 — The prototype's tweaks (`graphLayout`, `liveUpdates`, `defaultTab`) are not settings of the Studio.** The layout is computed
  (D151), updates are followed live with the polling fallback (FR-007-006), and the first tab follows the viewer's preference for motion
  and the size of the graph. Whether any of them should be a setting is the owner's question; none is built as a setting.
- **G166 — Closed: fonts bundled and fallback verified (2026-10-11).** Geist and Geist Mono are pinned to `@fontsource/*` version `5.3.0`
  (published 2026-07-19, more than two weeks before selection), loaded from the application's origin and distributed with the SIL
  Open Font License 1.1 in `frontend/apps/studio/public/licenses/geist-OFL.txt`. Geist weights 400/500/600 and Mono 400/500 are
  imported locally; the system stacks remain. The unit gate checks the imports, versions and licence. The final Chromium suite blocked font files, asserted Geist
  did not load, and passed 320px reflow with the system fallback; T007-046 is complete.
- **G167 — The design has a *Send signal* button, the platform has the operation (`signalRun`), the Studio has no such action.**
  A signal has a name, a payload and consequences (it wakes every `waitEvent` node that expects it; one that nobody waits for is a
  `409`), so it is a feature of the Studio with its own confirmation, permission and tests, not a restyle. Not built here; the
  owner decides whether it is part of 007.
- **G168 — The prototype's *Timeline* (a bar per step over elapsed time) has no counterpart.** The Studio's *Timeline* tab is the
  journal, which is what the prototype calls *Event history*. A bar view is a new view over the step times of the graph (and, for a
  run of thousands of steps, over the virtualised list of D147); it is not built by this restyle.

## Visual-direction implementation — 2026-10-11

The restyle implements D154–D159 without changing the REST contract or execution semantics: shared tokens, the sidebar/top bar,
wide Runs list and detail, compact graph cards and curved edges, and derived workflow/Script/sign-in styles. The master list is
outside the run-specific keyed component, so sibling-run navigation preserves filters, pagination and scroll while resetting the
detail's selection and operation dialogs. Terminal runs without `finishedAt` show an unknown duration. Omitted edges retain a
dotted pattern but use the accessible control-border colour at full opacity; the decorative divider colour was too faint.

The real-browser harness now has an explicit `SAGAT_STUDIO_NO_SANDBOX=1` development mode that excludes US02 and starts no sandbox
runner. Default/CI execution still uses real containers. This is a limited validation path, not an implementation of a sandbox
guarantee. Both restyled browser benchmarks passed in three cold rounds per scenario with the existing floors unchanged. Current results
and the passing final browser checks are in `quickstart.md`; owner review is approved and US02 passed with the real sandbox in the Studio CI job linked there.


Owner review recorded on 2026-10-11: Marcelo explicitly approved the visual proposal ("Aprovo a proposta visual")
after opening the local Studio. This approval covers Runs, the light theme, editor, Scripts and sign-in;
automated CI evidence for this branch is recorded separately.

G167 resolved by T007-056 (2026-10-12): the authorized Studio confirmation uses the existing
signal endpoint; eight component tests and a real browser/API/engine waiting-run journey pass.
The earlier entry describes the restyle's scope, not the current action set.


## UX review 2026-10-14 (T007-057)

A review of the built Studio in Chromium against the running control plane (15 screens, seven widths from 320 to 1920 px, light
and dark; page overflow, clipped or overlapping text, occluded text, touch-target size, scroll regions) compared with the Temporal Web
UI and the Windmill flow editor. Findings are labelled UXR-nn here instead of taking D/G numbers, which other branches are claiming.

| | Finding | Status |
|---|---|---|
| UXR-01 | On a phone the navigation and top bar stayed pinned: 262 of 640 px. | Fixed (T007-057): one scrolling document below 45rem. |
| UXR-02 | The run canvas was 286 px tall at 768 px and began 700-1050 px down; the selected step opened below it. | Fixed: canvas height is a length (`clamp(22rem, 68vh, 46rem)`); the step is a sticky side panel where the *page area* (not the window: the runs list takes some) is 62rem or wider. Below that it still opens under the graph and scrolls into view, as D158 had it. |
| UXR-03 | The overview map covered cards, was light in the dark theme; the credit overlapped a failure label. | Fixed: map on request, themed; credit at the bottom centre. React Flow's credit is kept. |
| UXR-04 | Journal table: nine columns, eight widths. | Fixed, with a test that counts columns against widths. |
| UXR-05 | Filters lost on reload/back and not shareable; no way to open a run by id; no way to start a run. | Fixed for status and workflow (address), Workflows search (`?q=`, over the workflows loaded so far, and it says so), open by id, *Start run…*. Time-range and correlation-id filters need API support: not built. |
| UXR-06 | Three date formats and no UTC on hover outside Runs. | Fixed: one formatter (`formatLocal`), `Moment` with the UTC tooltip. A relative/UTC switch is not built. |
| UXR-07 | Five disabled actions each repeating "The run is finished." | Fixed: said once when it is the same for all, each button still described by it (`aria-describedby`). |
| UXR-08 | Editor: toolbar orphaned *Publish…*, palette sentences pushed the canvas down at 1024 px, *Add trigger* wrapped, description in monospace. | Fixed. The editor is still not a full-height application layout. |
| UXR-09 | The runs list was a table of six columns at 768 px (748 px of sideways scroll, status off screen). | Fixed: cards below 74rem. On a phone the Workflows list hides the draft revision and the last update (they are there from 45rem). |

Not done, and why (each is a feature, not a defect, and needs its own spec delta and tests):

- **A start-run form generated from the workflow's input schema.** *Start run…* exists (T007-057) with a typed JSON object, but the
  input is not validated against `inputSchema` in the browser (the platform answers 422 and the Studio shows it) and has no
  fields, defaults or saved inputs. It also takes no version, correlation id or deadline.
- **A real timeline of bars** (already G168), **compact cards or an automatic fit** (D-era decision: graphs open at a readable 1:1, T007-049), a **⌘K
  palette**, **saved views**, **batch actions**, **test-up-to-step** in the editor, a saga drawn as a group with a compensation lane.
- **A sticky runs list** beside a long run: the list scrolls away with the detail on wide screens.
- **Keeping the session across a reload** needs an ADR (D144 keeps the token in memory on purpose).
- Not measured: screen readers, touch devices, slow networks, runs of more than the 12 in the demo namespace.

## Menu hierarchy 2026-10-14 (T007-058)

The owner's reading: the namespace is the tenant of the product, so the menus that depend on it belong under it and must appear only once it
is chosen. The finding is UXR-10: the sidebar listed all six destinations at one level beside a selector, and the shell chose the first namespace by
itself, so the dependent menus were always there and the hierarchy was never visible.

| | Finding | Status |
|---|---|---|
| UXR-10 | Six peers in the menu, five of which depend on the namespace; the first namespace was chosen for the person. | Fixed (T007-058): *Namespaces*, then the selector, then Workflows, Runs, Operations, Scripts and Integrations under it, only while a namespace is chosen. No automatic choice. |

Decisions taken here (the owner may reverse any of them):

- **No automatic choice, not even for a single namespace.** With one, the person's click is one extra step on a first visit; the gain is that
  the menu says the same thing every time and that nothing is shown as chosen that the person did not choose. A remembered choice (it is a slug,
  not a secret: `studio.namespace`) is still kept across reloads while the tenant lists it.
- **A remembered choice the tenant no longer lists is forgotten**, not replaced by the first namespace as before.
- **There is no way back to "no namespace"** once one is chosen, other than choosing another. Nothing needs it today; add it if a tenant-level
  page other than *Namespaces* appears.
- **A page of a namespace without one says so in place** (*Choose a namespace*) instead of redirecting, so an address keeps its meaning and opens by
  itself when a namespace is chosen in the menu. The pages' own "Select a namespace…" lines (Operations, Integrations) stay as their own
  defence; the router reaches them only in the moment before a dropped remembered choice is cleared.
- **Selecting does not navigate.** On *Namespaces*, *Use namespace* opens the menu and stays; moving to a list is the person's next click.
- Changing the namespace while a *detail* page (`/runs/:id`, `/workflows/:id`) is open is unchanged: the page asks about its own id in the new
  namespace and says it was not found. It could return to the list instead; not built.

Not proved: the Playwright journeys (see T007-058), including their axe checks of the new nested list (a named list inside the navigation landmark),
and any assistive technology on it.
