# Feature Specification: Developer tooling — time-skipping tests, interceptors, VS Code, Git sync and a curated template hub

**Feature Branch**: `feature/028-developer-tooling`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Bring the developer tooling of Temporal and Windmill to the platform without a second engine. (1) A
time-skipping test environment: an SDK test harness over the **real engine** with an injectable clock
(`crates/application/src/clock.rs`), where virtual time can be advanced only in a test deployment and never in
production, held to the five-SDK conformance suite, plus replay checks over exported run histories (feature 020).
(2) SDK interceptors/middleware: client and worker hooks in the five SDKs; feature 031 orders its payload codec
stage against them (coordinator decision C5: 031 owns the only stage that transforms payload bytes). (3) A VS Code extension that validates the IR, previews the graph, runs and tests workflows and follows
their logs through the CLI. (4) Bidirectional Studio↔Git sync: a repository per namespace; a Studio publication
produces a commit or pull request in the feature-011 manifest format; repository changes create drafts; conflicts
are handled explicitly; credentials come from OpenBao; GitHub and GitLab are supported. (5) A curated, signed
catalog of workflows, scripts and resource types imported as drafts — curated, because the constitution excludes a
connector marketplace in V1.

## Context and delivery boundary

The platform already has the pieces this feature composes, and this feature must not replace any of them:

- **The first day already runs on the real engine** (ADR-0019, feature 012): `sagat worker dev` serves canned
  answers through the worker protocol, `sagat new` writes templates embedded from `templates/`, and the CLI
  validates with the server's own codec. There is no emulator (D207 of feature 012) and this feature adds none.
- **Time decisions are taken on the database clock** (ADR-0020, `docs/architecture/execution-semantics.md`). The
  application `Clock` port exists but most decisions sample `clock_timestamp()` or `now()` in SQL. Today the
  conformance suite "moves time" by back-dating rows (`tests/contract/sdk-conformance/control.rs`), which is not
  available to SDK users and does not keep the journal and the rows on one clock.
- **The SDKs have transport and trace hooks but no interceptors** (`sdks/SPEC.md` §4, §6, §10); G219 of feature
  012 records that no SDK ships a test context for an activity.
- **GitOps is a CLI pipeline** (feature 011, ADR-0018): `sagat export|diff|publish` keep Git as the source of the
  documents for a pipeline, the manifest lists workflows only (G199), and every authority stays in the server.
- **Templates are six directories** in `templates/`, embedded in the CLI and tested; there is no catalog, no
  signature and no import into a namespace.

What is new is the *test deployment* with a virtual workflow clock, the interceptor contract, the editor
integration, a server-side Git synchronizer and a signed catalog. Concepts of Temporal that exist only because
workflow code is replayed (workflow inbound/outbound interceptors, non-determinism errors, patching, sticky
queues, local activities) do not apply: the platform interprets a declarative IR and does not run workflow code
in an SDK (`sdks/SPEC.md`, "An SDK is not a workflow runtime").

## User Scenarios & Testing *(mandatory)*

### US01 — Test a long-waiting workflow in seconds on the real engine (Priority: P1)

A developer tests a workflow that waits 30 days for a reminder, retries a step with exponential backoff and
expires an approval, in any of the five SDKs, in seconds and against the same engine that runs production.

**Why this priority**: Durable workflows are about time; a test that waits real days is never written, and a
test against a fake engine proves nothing about the real one (ADR-0019).

**Independent Test**: Start a test server, run the same reminder/retry/approval-expiry workflow through each SDK's
test environment with automatic time skipping, and compare the normalized journals across the five languages and
with a run of the same workflow whose timers were short and real.

**Acceptance Scenarios**:

1. **Given** a test server and a published workflow with a 30-day timer, **When** the test starts a run and waits
   for it with time skipping, **Then** the run completes, the journal records the timer as started and fired 30
   days apart in workflow time, and the wall time spent is that of the work, not of the wait.
2. **Given** a step that fails twice with retryable errors, **When** time is skipped, **Then** the retries happen at
   the instants the retry policy computes, in order, each one leased, fenced and journaled as in production.
3. **Given** a daily cron trigger, **When** the test advances 30 days in `step` mode, **Then** 30 runs start, one per
   tick; **When** another test server advances 30 days in `jump` mode, **Then** the engine's existing catch-up rule
   applies and one late tick starts one run.
4. **Given** a timer and a signal due at the same virtual instant, **When** the clock reaches it, **Then** the first
   commit wins and the other is an audited no-op, exactly as in production.
5. **Given** a step that is `READY` with no worker serving it, **When** the test asks to advance, **Then** the clock
   does not move and the answer says what blocks it (one ready step), within a bounded wait.
6. **Given** an activity handler, **When** the developer unit-tests it with the SDK's activity test context, **Then**
   heartbeats, a cancellation and the declared secrets fixture are observable and its outcome is mapped exactly as
   the worker host maps it, without a server.
7. **Given** a production control plane or engine, **When** it is pointed at a test deployment's database, **Then** it
   refuses to start; **When** any production endpoint is asked for a testing route, **Then** the answer is
   `404 ROUTE_NOT_FOUND`.

### US02 — Wrap client and worker calls with interceptors in any SDK (Priority: P1)

A developer adds logging, metrics, tracing, gateway headers or authorization checks around every client call and
every activity execution, in the same way in the five SDKs. Interceptors observe and decorate calls; they never
transform payload bytes — that is feature 031's data converter, which runs between the interceptors and the transport
(coordinator decision C5).

**Why this priority**: Applications need one ordered place for cross-cutting concerns in five languages, and feature
031 needs a fixed ordering of its codec stage relative to that place; without it each SDK would grow its own hooks.

**Independent Test**: Register two recording interceptors and a decorating interceptor that adds a gateway header in
each SDK, run the conformance scenario with a lost-response fault, and compare the call order, the bytes and headers
the server received on each retry, and the values the interceptors observed.

**Acceptance Scenarios**:

1. **Given** two client interceptors A then B, **When** a run is started, **Then** A runs before B on the way out and
   after B on the way back, once per logical call, whatever the number of transport retries.
2. **Given** a decorating interceptor and a lost response to `POST /runs`, **When** the SDK retries with the same
   idempotency key, **Then** the chain ran once, every attempt carries the same body and the same added header, and
   the server returns the one run it created.
3. **Given** a worker execute interceptor, **When** a forward, a compensation and a reconciliation task run, **Then**
   it wraps each handler and sees the task kind; the reports carry the identity the lease gave, unchanged.
4. **Given** an interceptor that tries to change the idempotency key, the namespace, a lease token or the payload,
   **When** the call proceeds, **Then** the change has no effect or the call is refused before it is sent, in every
   SDK.
5. **Given** a client interceptor that raises, **When** a call is made, **Then** the caller receives `INTERCEPTOR_FAILED`
   (not retried) and, when the server had already answered, the error says so; **Given** a worker interceptor that
   raises, **Then** the task is reported exactly as if the handler had raised it.

### US03 — Check exported histories against the current engine (Priority: P2)

Before an upgrade, an operator or a developer checks that the new release reads and folds the histories of real
runs exactly as the release that produced them.

**Why this priority**: Users' workflow versions are immutable IR, so the risk of a replay break is in the engine
and its event codec, not in user code; it matters at each upgrade, not at each commit.

**Independent Test**: Export the histories of the conformance corpus (feature 020's format), replay them on a test
server of the current build and require zero divergences; corrupt one event and require a typed divergence.

**Acceptance Scenarios**:

1. **Given** an exported history, **When** it is replayed, **Then** every event decodes with the server's codec, the
   domain reducer folds it without error and the derived projection equals the projection the history recorded.
2. **Given** a history whose pinned workflow version uses a capability this build does not support, **When** it is
   replayed, **Then** the report names the capability and nothing is published or started.
3. **Given** a history with a sequence gap, an unknown event type or an unknown event-schema version, **When** it is
   replayed, **Then** the answer is a typed refusal that names the first offending sequence number.
4. **Given** a history whose step outputs were exported under a disclosure policy, **When** the developer derives
   canned answers from it, **Then** only disclosed values become answers, undisclosed outputs are listed as missing
   and nothing classified `secret` or unrevealed `pii` is ever written.

### US04 — Validate, preview and run workflows from VS Code (Priority: P2)

An author edits a workflow document in VS Code, sees the platform's diagnostics in place and the graph beside it,
runs it in a development namespace or on a test server, and follows the run.

**Why this priority**: Authors who keep documents in Git edit them in an editor; the Studio stays the visual tool.

**Independent Test**: Open the diagnostics corpus of feature 012 in the extension's test host and compare each
diagnostic's position, code and help with the CLI's; run a template with canned answers and follow it to its end.

**Acceptance Scenarios**:

1. **Given** a document with an invalid node, **When** it is saved, **Then** the Problems view shows the same code,
   node, JSON pointer and help as `sagat workflow validate`, placed at the matching text position.
2. **Given** a valid document, **When** the graph preview is opened, **Then** it shows the same definition graph as
   the Studio for that document, updates as the document changes and offers a table alternative.
3. **Given** a development namespace and a signed-in developer, **When** they choose "Publish to dev and run",
   **Then** the CLI publishes and starts the run and the editor follows its status and journal without payloads;
   **Given** a staging or production namespace, **Then** the command refuses unless a user-level setting allows it.
4. **Given** an untrusted workspace, **When** it is opened, **Then** validation and preview work and every command that
   executes the CLI against a server or starts a worker is disabled.
5. **Given** a token stored by the extension, **When** any command runs, **Then** the token reaches the CLI only
   through its environment and never appears in an argument, a setting, the webview or an output channel; **Given** a
   trusted workspace whose settings name another server, **Then** the workspace value is ignored and the token is not
   sent to that origin.

### US05 — Keep a namespace and a Git repository in step, both ways (Priority: P2)

A team binds a namespace to a GitHub or GitLab repository. Publishing in the Studio opens a pull request (or
pushes a commit) with the 011 manifest and documents; a merge into the tracked branch becomes drafts in the
namespace; when both sides changed, the team resolves a conflict that the platform records and never merges by
itself.

**Why this priority**: Teams that review in Git and teams that build in the Studio work on the same workflows;
today only a pipeline running `sagat publish` connects them, in one direction.

**Independent Test**: Against a real GitHub (or GitLab) test repository — or a recorded provider stub for the fault
suite — publish twice, merge, edit in the Studio and in Git at once, lose a webhook and crash the synchronizer at
each boundary; require one pull request per workflow branch, drafts never overwritten and a recorded conflict.

**Acceptance Scenarios**:

1. **Given** a bound namespace with policy `PULL_REQUEST`, **When** version 4 of `orders` is published, **Then** the
   publication commits regardless of the provider's state and a pull request appears whose head contains the
   canonical document of version 4 and the manifest entry, once, even if the synchronizer crashes and runs again.
2. **Given** a merge into the tracked branch that changes `orders`, **When** the webhook arrives (or, if it was lost,
   at the next head reconciliation), **Then** the draft of `orders` becomes the repository's document, with the
   commit as its provenance, and nothing is published.
3. **Given** unexported Studio edits to the draft of `orders` and a repository change to the same document, **When**
   the import runs, **Then** the draft is unchanged, a conflict records base, draft and repository documents, and the
   Studio shows a three-way comparison with "take repository" and "keep draft".
4. **Given** the commit the synchronizer itself pushed, **When** its webhook arrives, **Then** nothing changes in the
   namespace (the repository document equals the active document).
5. **Given** a request signed with another binding's secret, a foreign binding id or a replayed delivery, **When** it
   reaches the webhook endpoint, **Then** it is refused with the same `401` or answered as a duplicate, and nothing
   about another tenant is revealed.
6. **Given** policy `DRAFT_AND_REQUEST` and a tracked branch the probe cannot show to require reviews, **When** a merge
   is imported, **Then** only drafts are created and the import reports `GIT_REVIEW_NOT_VERIFIED`; **Given** a branch
   that requires reviews, **Then** a publication request is opened, shows the commit author as unverified and needs one
   approval more than the namespace's policy.

### US06 — Import a curated, signed template as a draft (Priority: P3)

A developer browses the platform's curated catalog, reads a template's documents and requirements, and imports it
into a namespace as a draft to edit and publish.

**Why this priority**: Useful once the editor and sync paths exist; the six embedded templates already serve the
first day.

**Independent Test**: Import a workflow template and a script template into a development namespace; then tamper
with a file, sign with an unknown key, serve an older index version and an expired index, and require each to be
refused before anything is created.

**Acceptance Scenarios**:

1. **Given** a verified catalog entry, **When** it is imported under a name, **Then** a draft is created with the
   template's identity, version, digest and signer as provenance, and a list of what must be prepared (activities to
   bind, secrets to create, resource types, capabilities); nothing is published or executed.
2. **Given** a file whose digest does not match, an index signed by an unpinned key, an index older than the newest
   seen or an expired index, **When** the catalog is refreshed or an import is requested, **Then** the entry or index
   is unavailable with a typed reason and no draft is created.
3. **Given** an existing workflow of the same name, **When** the import is requested, **Then** it is refused as a
   conflict and the existing workflow is untouched; **Given** the same import repeated with its idempotency key,
   **Then** the original draft is returned.

### Edge Cases

- Advancing the virtual clock while an attempt is leased; a worker report arriving after the advance timed the
  attempt out (`STALE_ATTEMPT`); an advance requested by two tests at once (one epoch wins, the other conflicts).
- A virtual instant that crosses a DST transition of a cron's IANA time zone; an advance beyond the clock ceiling;
  an offset that would move a deadline past the representable range.
- Skipping until a run ends while the run waits for a signal nobody sends (`WAITING_EXTERNAL`, clock unmoved).
- A run whose deadline was computed by an SDK from its own system clock instead of the virtual clock.
- A crash of the test server between the clock commit and the scheduler pass that processes due work.
- An interceptor that adds a header whose value changes per call (a timestamp) under client retries: the chain runs
  once, so every attempt carries the first value.
- An interceptor that blocks forever: the call's own deadline and the lease still bound it.
- A history produced by a newer release than the replaying one; a history larger than the ceiling.
- Git: a force-push to the tracked branch; a manifest that names a file outside its directory or a link; a 30 MB
  push webhook; a branch protected against direct pushes; a deleted repository; a rotated GitHub App key; a pull
  request closed without merge (drift); two namespaces of one tenant bound to one repository with different paths;
  a published version whose document contains a secret-looking value; a workflow removed from the manifest.
- Templates: an entry whose kind requires feature 021 when it is absent; a template that names a secret value
  instead of a reference; a catalog refresh racing an import; an index whose expiry is beyond the allowed validity.
- VS Code: a CLI older than the extension supports; a workspace setting that points `cliPath` at another binary; a
  document that is YAML with anchors expanding beyond the CLI's budget.

## Requirements *(mandatory)*

### Functional Requirements

**Time-skipping test environment (US01)**

- **FR-028-001 — Test deployment:** The platform MUST provide a test server that composes the production control
  plane, engine loops, REST API and worker service from the same crates with a virtual workflow clock, a testing
  API and an embedded test identity issuer. It MUST NOT contain a second reducer, scheduler or store. It MUST start
  only when `SAGAT_ENV=dev`, only on an empty database or one it marked itself, MUST mark the database as a test
  deployment, and MUST hold exactly one tenant.
- **FR-028-002 — Never in production:** Production composition roots MUST refuse to serve a database that carries
  the test-deployment marker or a non-canonical workflow-time function, MUST NOT expose testing routes (they answer
  `404 ROUTE_NOT_FOUND`) and MUST NOT advertise the `virtual-clock` capability. The production image MUST NOT contain
  the test server binary and no production binary MAY depend on the test-only crates. Each rule MUST have an
  automated check that fails the build or the start (start-up refusal, route test, a CI check of the image's build
  and copy lines, and a dependency-tree check of the production binaries).
- **FR-028-003 — Two time classes:** Every instant the platform persists or compares MUST be classified as
  *workflow time* (timers, wait and approval deadlines, retry release, run/step/attempt and chain deadlines, loop
  delays, cron and schedule firing, and the recorded instants of a run's history) or *infrastructure time* (leases,
  worker sessions, shard and engine leases, rate buckets, deduplication and receipt windows, credential and token
  validity, cleanup, telemetry, synchronization jobs, audit). Two stored instants of different classes MUST NOT be
  compared, and a comparison or difference with "now" (a due check, a lag metric, a retention age) MUST use the now of
  the instant's class. Workflow time MUST be derived from one infrastructure sample, so that in production both are
  the same value and behaviour and query plans are unchanged. The classification is one published catalog covering
  the instants of every feature, including the parity drafts ([contract](contracts/time-classes.md), coordinator
  decision C13).
- **FR-028-004 — Virtual clock:** A test deployment MUST hold one non-negative offset added to workflow time only.
  The offset MUST only grow, each change MUST be one audited transaction with a new epoch, an advance MAY name the
  epoch it expects, and the total offset and each advance MUST be bounded. There is no reset; a fresh test server
  is the way back.
- **FR-028-005 — Advance modes:** `step` mode (the default) MUST stop at every workflow-due instant up to the target
  in order, set the clock to it, let the real scheduler passes process what became due and reach quiescence before
  the next stop. `jump` mode MUST make one change to the target, after which due work is processed late under the
  engine's existing rules (as after an outage). The number of stops and the wall time of one call MUST be bounded;
  progress already made MUST remain committed and be reported.
- **FR-028-006 — Skip targets:** A skip MUST accept an instant, a duration, "the next due instant" or "until a run
  (or a chain) is terminal". When the deployment is quiescent, nothing is due and the target is not reached, it MUST
  return `WAITING_EXTERNAL` with the closed kinds of what the run waits for (event, signal, approval, update, child,
  worker) without moving the clock.
- **FR-028-007 — Quiescence:** Quiescence MUST mean that no step instance is ready, leased or running, no workflow
  item at or before the virtual now is unprocessed, no admitted inbox event is undispatched and no run or child
  admission is pending. Waiting for it MUST be bounded in real time; when it is not reached the clock MUST NOT move
  and the error MUST be `CONFLICT` with reason `TIME_ADVANCE_BLOCKED` and counts per closed kind. An explicit advance
  MAY waive quiescence only in `jump` mode.
- **FR-028-008 — Semantics preserved:** Under a virtual clock the engine MUST use the same reducer, transaction
  boundaries, leases and fencing; the journal MUST record workflow-time instants; replay MUST equal the projection;
  races MUST keep "first commit wins". The existing engine suites MUST pass unchanged with a zero offset and the
  shifted-clock regression suite MUST pass with a non-zero offset.
- **FR-028-009 — SDK test environment:** Each of the five SDKs MUST offer a test environment that connects to a test
  server, checks its `virtual-clock` capability, creates clients and worker hosts configured for it (binding the
  worker's activity names through the public binding API), and exposes `now`, `advance`, `skip`,
  `waitForCompletion` with time skipping and namespace creation. Behaviour MUST be held to shared conformance
  scenarios.
- **FR-028-010 — Injectable SDK clock:** An SDK client MUST take an optional clock used only to turn a duration option
  (such as a run deadline) into an instant; the default is the system clock and the test environment supplies the
  virtual one. Heartbeat intervals and retry backoff MUST stay on the SDK's monotonic clock. A deadline instant sent
  through REST is interpreted in workflow time; the test server's clock endpoint gives non-SDK callers the workflow
  now to compute it from.
- **FR-028-011 — Activity test context:** Each SDK MUST offer a context for unit-testing an activity handler without
  a server: given metadata, recorded heartbeats, cancellation with the closed reasons, a secrets fixture, and the
  outcome mapped by the host's own mapping rules (`sdks/SPEC.md` §6.5–6.6). It MUST NOT emulate leases, retries or
  any server decision, and its documentation MUST say so.
- **FR-028-012 — Test identity:** The test server MUST issue tokens only for its own seeded tenant, from a key
  generated at each start, on a loopback listener, with the rules of the development identity provider
  (ADR-0019 decision 3), and MUST write them to files with mode `0600`. Its issuer MUST be unique per start so its
  tokens are accepted nowhere else.
- **FR-028-013 — Testing API:** Clock, quiescence and replay operations MUST be REST operations under `/v1/testing/`,
  described in a contract separate from the canonical OpenAPI document, authorized by the test server's own
  `testing:clock` check (admin and developer of the test deployment's tenant; the permission is not added to the
  production catalog or to any production role) or `definition:read` for replay, audited, `no-store`, and refused
  with typed errors.

**History replay (US03)**

- **FR-028-014 — Replay check:** The test server MUST accept an exported run history (feature 020's
  `sagat-run-history-v1`, FR-020-036, in `STRUCTURE` or `DISCLOSED` mode), decode every event with the server's event codec, fold it with the domain reducer and compare the derived
  projection (run status and stop, step statuses, attempts, chosen routes, chain links) with the recorded one. The
  report MUST list structural divergences by sequence, kind and field and MUST NOT contain payload values.
- **FR-028-015 — Version compatibility:** The replay MUST decode the history's pinned workflow version document with
  the current codec and capability set and report each unsupported capability; it MUST NOT publish, start or store
  anything other than its audit record.
- **FR-028-016 — Input contract and bounds:** The replay MUST refuse, with a typed reason naming the first offending
  sequence, a history with a gap, an unknown event type, an unknown event-schema version or a size above the
  ceiling. The test server MUST NOT fetch histories from another deployment; the caller supplies the document.
- **FR-028-017 — History-derived answers (P3):** The CLI MUST derive a `worker dev` answer file (feature 012 D208
  format) from a history's disclosed step outputs and failures only, list every step it could not answer, and never
  write a value classified `secret` or `pii` that the export did not reveal. It MUST compare a candidate run's path
  with the history's and report differences, without claiming that a different version is compatible.
- **FR-028-018 — Entry points:** Each SDK test environment MUST expose the replay call and the CLI MUST offer
  `sagat history replay|answers|compare`; the SDK call MUST be held to a conformance scenario.

**Interceptors (US02)**

- **FR-028-019 — Client interceptors:** Each SDK client MUST accept an ordered list of interceptors that wrap every
  client operation through one generic call shape naming the operation from a closed, additively growing set. The
  first registered interceptor MUST be the outermost. The chain MUST run once per logical call, outside the SDK's
  retry loop, and every retry MUST send identical bytes and the same decorations. Interceptors MAY observe the
  operation, its metadata and the application value it carries (read-only), MAY add request headers outside the
  protected set, and MAY refuse a call; they MUST NOT replace or transform a payload.
- **FR-028-020 — Worker interceptors:** Each worker host MUST accept an ordered list of interceptors around task
  execution (forward, compensation and reconciliation), around the heartbeat (metadata only) and around each report.
  A report's chain MUST run once per report; retries of the report MUST resend identical bytes. Worker interceptors
  observe the decoded step document and the handler's result read-only. The host's lease, heartbeat, cancellation and
  shutdown rules MUST be unchanged.
- **FR-028-021 — Position against the data converter:** This feature MUST NOT define a payload-transforming hook.
  The interceptor chains MUST sit on the application side of feature 031's data converter (FR-031-016): on the way
  out the chain runs first, then the converter, then the transport; on the way in the transport, the converter, then
  the chain in reverse. Interceptors therefore observe decoded application values and nothing can be placed between
  the converter and the transport. With no converter configured the order is the same with the converter absent.
  Streamed blobs are observed as metadata only (G1402).
- **FR-028-022 — Protected fields:** Interceptors MUST receive read-only views of the token, tenant, namespace,
  idempotency, update and decision keys, lease token, task identity, run, step, attempt, generation, protocol version
  and `traceparent`; an attempt to change one MUST have no effect or be refused as `VALIDATION_FAILED` with reason
  `PROTECTED_FIELD` before anything is sent. Trace context continues to come only from the existing trace hook. The
  protected request headers are `Authorization`, `X-Octopus-Namespace`, `Idempotency-Key`, `traceparent`,
  `Content-Type`, `Content-Length`, `User-Agent` and every `X-Octopus-*` and `X-Sagat-*` header; added headers are
  bounded in number and size and never logged by value.
- **FR-028-023 — Interceptor failures:** A client interceptor that raises MUST produce `INTERCEPTOR_FAILED` (a new
  SDK-origin, non-retryable catalog code whose message names only the interceptor's type), and when the server had
  already answered, the reason `AFTER_RESPONSE` so the caller retries with the same key. A worker execute interceptor
  that raises MUST be treated exactly as the handler raising that error. Neither MAY stop the host's loop or drop a
  leased task silently.
- **FR-028-024 — Interceptor conformance and documentation:** `sdks/SPEC.md` MUST gain the interceptor contract and
  the five SDKs MUST pass shared scenarios for order, once-per-call bytes and decorations under lost responses,
  protected fields, error mapping and coverage of every client operation and task kind.

**VS Code extension (US04)**

- **FR-028-025 — Diagnostics:** The extension MUST obtain diagnostics only from the CLI's machine-readable
  validation (the server's codec), place them at the text position of their JSON pointer, keep their code, help and
  documentation link, and validate locally without a server; server-side checks (secrets, scripts) run only when a
  server is configured.
- **FR-028-026 — Graph preview:** The preview MUST render the definition graph from the same graph model the Studio
  uses (one shared package), update on edit, follow the design tokens, meet WCAG AA with a table alternative, and
  never draw sample values (FR-007-010).
- **FR-028-027 — Run and test:** The extension MUST offer, through the CLI only, publishing and starting a run in a
  development namespace, serving canned answers with `worker dev`, advancing a test server's clock and following a
  run's status and journal without payloads. It MUST refuse staging and production namespaces unless a user-level
  setting allows it, mirroring the CLI's own guard (D223 of feature 012).
- **FR-028-028 — Trust and credentials:** The token MUST be kept in the editor's secret storage and passed to the CLI
  only through its environment, never in an argument, a setting, the webview, a log or an output channel. A stored
  token MUST be bound to the server origin it was stored for and passed only when the configured server has that
  origin. The `cliPath`, server, worker-endpoint and `allowNonDev` settings MUST be read from user or machine scope
  only, never from a workspace, trusted or not. In an untrusted workspace only validation and preview MUST work. The webview MUST use a content security policy starting from `default-src 'none'`,
  load only bundled resources and validate every message. The extension MUST check the CLI's version range.
- **FR-028-029 — CLI machine output:** The CLI MUST offer a versioned JSON output (`--json`) for document and manifest
  validation, publish, run, runs, inspect, history and test-server commands, and `workflow inspect --follow` as
  newline-delimited JSON; human output and exit codes MUST stay as they are.

**Git sync (US05)**

- **FR-028-030 — Repository binding:** A namespace MUST have at most one repository binding (the first increment
  synchronizes workflow documents only; app documents of feature 027 wait for a later increment, coordinator decision
  C20): provider (GitHub or
  GitLab), an API host from the operator's allowlist, the repository, the tracked branch, the manifest directory,
  the authentication (GitHub App id, installation id and private-key secret, or a GitLab project/group access token
  secret), the webhook secret, an outbound policy (`PULL_REQUEST` by default or `DIRECT_PUSH`) and an inbound policy
  (`DRAFT` by default or `DRAFT_AND_REQUEST`). Settings MUST be immutable revisions changed with optimistic
  concurrency; a binding can be paused, resumed, probed with a fixed read and removed with its history kept. The probe
  MUST also report whether the tracked branch requires at least one approving review before a merge, as the provider
  states it (GitHub branch protection or rulesets, GitLab merge-request approval rules).
- **FR-028-031 — Export obligation:** Every activation of a workflow in a bound, active namespace (publish, promote
  into it, rollback, activate) MUST insert its export obligation in the same transaction as the activation. A failed
  or unavailable provider MUST NOT roll back or delay the publication. Each export pins the binding revision it was
  created under.
- **FR-028-032 — Export execution:** Exports MUST run in the engine under a lease with an attempt fence (Git sync is a
  delivery of a fixed, platform-defined shape to a tenant-configured destination, sent through the guarded egress
  client with only that binding's credential: category (b) of coordinator decision C3, recorded by feature 021 in
  ADR-0055), render the
  active version's canonical document and its manifest entry in the 011 format, and per workflow keep one branch
  `sagat-sync/<workflow>` and at most one open pull request, coalescing newer activations. `DIRECT_PUSH` MUST only
  fast-forward the tracked branch and MUST NOT force-push. Before creating a branch, commit or pull request the
  synchronizer MUST look up what already exists; an ambiguous provider outcome MUST be looked up before it is
  repeated. Rendered content MUST be scanned for secret material before it leaves the platform.
- **FR-028-033 — Inbound changes:** Each binding MUST have its own webhook endpoint that verifies GitHub's
  `X-Hub-Signature-256` or GitLab's signing-token signature (with its timestamp window; GitHub signs no timestamp, so
  delivery-id deduplication within the receipt window is its replay guard), answers every failure with the same
  `401`, deduplicates by delivery id and, for a push to the tracked branch, enqueues one coalesced import of the branch
  head. Other events MUST be acknowledged and ignored. A periodic head reconciliation MUST enqueue an import
  when the head differs from the last imported commit: webhooks are hints.
- **FR-028-034 — Import:** An import MUST read the manifest and documents at the head commit within the ceilings and,
  per workflow in its own transaction, create the workflow or replace its draft by compare-and-swap with the commit
  as provenance. It MUST NOT publish; with `DRAFT_AND_REQUEST` it MUST open a publication request bound to the
  document's digest (ADR-0018), but only while the binding's latest probe shows required reviews on the tracked
  branch; otherwise it imports as `DRAFT` and reports `GIT_REVIEW_NOT_VERIFIED`. A publication of a draft last saved by
  `system:git-sync` MUST require one approval more than the namespace's policy (at least one), counted at publication,
  and the request MUST show the commit, its author and its committer as unverified provenance. It MUST NOT overwrite a
  draft that holds unexported changes, MUST report entries removed from the manifest without deleting anything, and
  MUST be resumable.
- **FR-028-035 — Conflicts:** A conflict MUST record the base (last synchronized) document, the draft revision and the
  repository document and commit, be displayed through the server's diff (ADR-0016), and be resolved only by a person
  choosing `TAKE_REPOSITORY` (compare-and-swap of the draft) or `KEEP_DRAFT` (the repository is left alone; the next
  publication exports). The platform MUST NOT merge documents itself. A newer repository commit supersedes an open
  conflict.
- **FR-028-036 — Status and drift:** Each bound workflow MUST have one status from a closed set (`IN_SYNC`,
  `EXPORT_PENDING`, `PULL_REQUEST_OPEN`, `DRAFT_FROM_REPOSITORY`, `CONFLICT`, `DRIFT`, `REMOVED_IN_REPOSITORY`,
  `ERROR`), derived by comparing digests; the synchronizer's own commits MUST cause no draft change.
- **FR-028-037 — Credentials and egress:** Credentials MUST be namespace secret references (OpenBao authority, FR-017-005)
  resolved per job in the engine, with the binding author's secret access rechecked at each job. A GitHub App token
  MUST be minted per job from an RS256 assertion valid at most 10 minutes, restricted to the bound repository, kept in
  memory only and never shared between jobs or tenants. Every provider call MUST go through the egress guard; a refused
  destination is `GIT_DESTINATION_DENIED`. No credential MUST appear in a log, span, journal, audit record, response
  or commit. The webhook route MUST apply its rate limits before resolving any secret and MUST answer an unknown
  binding after the same verification work as a known one.
- **FR-028-038 — Sync failures:** Jobs MUST retry with bounded jittered backoff honouring the provider's rate-limit
  answers, become `DEAD` after 12 attempts (visible, redrivable), and pause the binding after 20 consecutive dead jobs,
  with a closed set of error codes and a namespace alert when exports are stuck.

**Template hub (US06)**

- **FR-028-039 — Curated sources:** The catalog MUST come only from the index bundled with the release and from
  indexes the operator configures for the deployment with pinned public keys. There MUST be no tenant-configured
  source, no public submission, no ratings, no payment and nothing executable at browse or import time. Entry kinds
  are workflow, script and — only when feature 021 is enabled — resource type.
- **FR-028-040 — Verification:** An index MUST carry an Ed25519 signature over its RFC 8785 canonical form by a pinned
  key, a version that never decreases for that index, a signing time and an expiry within the maximum validity, and a
  SHA-256 digest for every file. Any failure MUST make the index or the entry unavailable with a typed reason;
  verification MUST happen at refresh and again at import.
- **FR-028-041 — Import as a draft:** An import MUST create only a workflow draft or a script draft, validated with the
  platform's codec, with the template's identity, version, digest and signer as provenance and the list of what must
  be prepared. It MUST be idempotent by key, refuse an existing name as a conflict, carry no secret values, and count
  against the existing workflow, script and content quotas.
- **FR-028-042 — Browse:** The catalog MUST be browsable through REST, the Studio and the CLI by anyone with
  `definition:read`; files are shown as text and never executed in the browser.

**Cross-cutting**

- **FR-028-043 — Observability and audit:** Every new operation MUST emit OTel spans and fixed-label metrics, write
  redacted structured logs, and audit clock changes, replays, binding changes, exports, imports, conflict
  resolutions, webhook refusals, catalog refreshes and imports in the transaction of the decision.
- **FR-028-044 — Finite ceilings:** Every new resource MUST have a default and a hard ceiling (plan), with the
  effective value the minimum of request, namespace and deployment, zero refusing new admission, and admission
  failing before work starts.
- **FR-028-045 — Contracts and documentation:** REST additions MUST be additive draft fragments merged into the
  canonical contracts at implementation; `PlatformVersion` gains an additive `capabilities` list; the error catalog,
  `sdks/SPEC.md`, the CLI output schema, runbooks and quickstarts MUST change in the same delivery.

### Non-functional requirements

- **NFR-028-SEC:** Virtual time reachable from a production deployment, a cross-tenant read or write through a
  binding, webhook or import, a credential canary (GitHub App key, installation token, GitLab token, CLI token in the
  editor) in any log, span, journal, audit record, response, commit or webview, or an unsigned template accepted, is
  release-blocking. Signature, trust and authorization checks fail closed.
- **NFR-028-DUR:** An activation and its export obligation commit together; a clock change is one monotonic
  transaction; imports replace drafts only by compare-and-swap; crashes at every synchronizer boundary leave one
  branch, one pull request and no lost obligation. Provider effects are at-least-once with lookup; nothing claims
  exactly-once.
- **NFR-028-COMP:** Production behaviour, query plans and in-flight runs are unchanged by the time-class refactor
  (plan equality checked for the scheduler passes); REST, AsyncAPI, protobuf and IR change additively (no protobuf or
  IR change is needed); SDKs without interceptors behave exactly as before; the CLI's human output and exit codes
  are unchanged.
- **NFR-028-OBS:** Telemetry follows ADR-0017: no tenant, binding, workflow, run or template identifier in a metric
  label; payload-free spans; the Git sync backlog and its age are measured and alerted per namespace.
- **NFR-028-PERF:** The workflow-time function in production, the stops per second of time skipping, the
  publish-to-commit latency of the synchronizer and catalog refresh are benchmark criteria measured on a declared
  environment; no throughput or latency is promised.
- **NFR-028-A11Y:** Studio screens and the extension's webview meet WCAG AA with keyboard operation, a table or list
  alternative to every graph, and state shown by text and shape as well as colour.

### Key Entities

- **Test deployment and virtual clock:** a deployment marked as a test deployment, holding one monotonic workflow-time
  offset with an epoch and an append-only history of advances.
- **Time class:** the classification of every persisted or compared instant as workflow or infrastructure time.
- **Skip request and report:** a target, a mode, the stops taken, the outcome and what blocked or awaited.
- **History replay report:** the verdict of folding an exported history with the current codec and reducer, with
  structural divergences and unsupported capabilities.
- **Interceptor chain:** SDK-local ordered wrappers around client calls, task executions, heartbeats and reports,
  on the application side of feature 031's data converter.
- **Repository binding and revision:** a namespace's repository, branch, manifest directory, credentials references and
  policies, as immutable revisions.
- **Export, import, document sync state and conflict:** the obligations and jobs that carry activations to Git and
  commits to drafts, the per-workflow base digest and status, and the three-way record awaiting a person.
- **Draft provenance:** where a draft revision came from (a commit or a template import).
- **Template index, entry and import:** a signed, versioned, expiring catalog; its entries with file digests and
  requirements; the record of an import into a namespace.

## Success Criteria *(mandatory)*

All evidence below is required future evidence, not completed tests.

### Measurable Outcomes

- **SC-028-001** (FR-028-001, 004..011): The reminder/retry/approval-expiry workflow reaches its terminal state through
  each of the five SDKs' test environments with identical normalized journals, and with the same event types and order
  as a real-time run with short timers; the shared activity-context fixture passes in all five SDKs; wall time per
  language is recorded with its environment as a benchmark criterion (target under 60 seconds on the reference CI
  runner).
- **SC-028-002** (FR-028-002, 012): 100% of the production-refusal checks pass: production control plane and engine exit
  with a typed refusal on a test-marked database or a replaced clock function, every `/v1/testing/*` path answers
  `404 ROUTE_NOT_FOUND` in production, the production image contains no test server, and test-server tokens are refused
  by a production deployment.
- **SC-028-003** (FR-028-003, 008): the time-classification manifest covers 100% of time columns of the migrated schema
  (a catalog test fails on an unclassified one); the plans of the scheduler pass queries are equal before and after the
  refactor; the existing engine suites pass with offset zero and the shifted-clock suite passes with a 400-day offset.
- **SC-028-004** (FR-028-005..007): a daily cron fires 30 times over a 30-day `step` advance and once over a `jump`; a
  timer/signal race at a stop is decided by the first commit in 20 of 20 rounds; a crash between the clock commit and the
  pass loses no due item; a blocked advance never moves the clock.
- **SC-028-005** (FR-028-019..024): in all five SDKs the conformance scenario shows identical interceptor order, identical
  request bytes and decorations across every retry under lost responses, refused protected-field and payload changes,
  the error mapping above and every client operation and task kind reaching the chain; the golden is reviewed by a
  person.
- **SC-028-006** (FR-028-014..018): the exported histories of the conformance corpus replay with zero divergences on the
  build under test; each injected corruption (gap, unknown type, changed status, unsupported capability) yields its typed
  report; the replay call passes in five SDKs; derived answers contain no undisclosed value (canary test).
- **SC-028-007** (FR-028-025..029): for the 012 diagnostics corpus the extension's positions, codes and help equal the
  CLI's; a token canary never appears in a child-process argument, a setting, the webview or a log; an untrusted
  workspace exposes no executing command; the webview passes an automated accessibility check.
- **SC-028-008** (FR-028-030..032, 037, 038): crash injected at each export boundary (after commit before record, after
  pull request before record, duplicate claim, stale lease) leaves exactly one branch and one open pull request per
  workflow with the active document; a publication commits while the provider is down; a dead export can be redriven.
- **SC-028-009** (FR-028-033..036): duplicate and replayed webhooks change nothing; a lost webhook is caught by head
  reconciliation within one interval; a concurrent Studio edit always produces a conflict and never an overwrite; the
  synchronizer's own commit changes no draft; foreign-binding and bad-signature requests are indistinguishable `401`s
  (also in timing, within the test's tolerance); a request opened by import needs policy + 1 approvals and is never
  opened without verified branch review.
- **SC-028-010** (FR-028-039..042): a tampered file, an unpinned key, a rolled-back version and an expired index are each
  refused before any write; an import creates only a draft and is idempotent; quota exhaustion is refused before work.
- **SC-028-011** (FR-028-013, 030..042, NFR-028-SEC): the cross-tenant battery covers every new REST operation with zero
  findings; credential canaries are absent from logs, spans, journal, audit, responses, commits and webviews.
- **SC-028-012** (FR-028-043, 044): every new operation has a span, a fixed-label metric and its audit action; each
  ceiling is exercised at its limit with a typed refusal.
- **SC-028-013** (FR-028-045, DoD): the quickstart's happy and failure demos run from a clean checkout, and every
  skipped or unproven guarantee is listed with its gap.

## Assumptions

- Feature 020 defines the exported run-history document; this feature states the fields it needs
  ([contract](contracts/history-replay.md)) and is blocked on that format for US03 (G1404).
- Feature 031 owns the only payload-transforming SDK stage (its data converter, FR-031-016) and codec semantics; this
  feature defines the interceptor chains and their position on the application side of that stage (decision C5).
- GitHub (github.com and GitHub Enterprise Server REST v3) and GitLab 19.1 or later (signing-token webhooks) are the
  supported providers; older GitLab can use head reconciliation only (G1409).
- A test deployment is ephemeral, single-tenant and development-only; its data is disposable.
- Release distribution follows the owner's decision 12 (GitHub Releases only) unless O1400 decides otherwise.
- The publication and version rules of feature 011 (ADR-0018) apply to drafts created by import and to publications
  that trigger exports, with one stated weakening and its compensation: the platform does not know who wrote an
  imported document, so the separation of duties of ADR-0018 decision 4 cannot exclude the commit author; FR-028-034
  requires one extra approval and verified branch review for requests opened by import (extends feature 011 G210).

## Dependencies

- **Existing:** 001 (contracts, identity), 002 (engine, journal, leases), 003 (sagas, reconciliation), 004 (webhook
  ingress patterns, delivery pipeline), 005 (scripts for script templates), 006 (SDK conformance), 007 (Studio), 008
  (telemetry, alerts), 009 (RBAC, quotas, egress, secrets), 010 (scheduler shards), 011 (publication, manifest, CLI),
  012 (CLI, `worker dev`, templates, development identity provider), 014 (approvals, loops and chains carry workflow
  time), 016 (classification for derived answers), 017 (OpenBao credential authority).
- **New features:** 020 (history export `sagat-run-history-v1`; US03 depends on it), 031 (orders its data converter on
  the transport side of the interceptor chains; no other dependency in either direction, decision C5), 018, 019, 020,
  022, 025, 026, 030 and 032 (their instants are classified in the [time-class catalog](contracts/time-classes.md),
  decision C13), 021 (resource-type templates; ADR-0055 holds the placement rule of decision C3), 022
  (owns draft test mode and pinned outputs; the extension may call it when present, this feature does not provide it),
  029 (custom roles may grant the new permissions), 027 (app documents are not synchronized by this feature's first
  increment; 027 marks Git sync of app manifests as a later dependency, decision C20).

## Out of Scope

- A connector or template marketplace, third-party publishing, ratings or payments (constitution, Delivery
  Constraints); executable plugins in the server.
- Workflow-code interceptors, determinism checks of user code and code replay (not applicable to the declarative IR).
- Per-namespace virtual clocks, several clocks in one test server, and SDK-managed test-server processes (G1400,
  G1401).
- Synchronizing scripts, connections, secrets, resources or app documents through Git (G1406); automatic merge of
  documents; automatic publication on merge (O1401).
- A web or remote-only VS Code (vscode.dev) without a local CLI (G1415); an IDE other than VS Code.
- Any payload-transforming hook (owned by feature 031); payload bytes of streamed blobs reaching interceptors (G1402).

## Owner questions

Recorded with recommended defaults in [research.md](research.md#owner-questions): O1400 (where the extension is
published), O1401 (whether a merge may publish automatically), O1402 (whether operators may add catalog indexes).
The [checklist](checklists/requirements.md) records the specification's quality review. No task or success criterion
is complete because this document exists.
