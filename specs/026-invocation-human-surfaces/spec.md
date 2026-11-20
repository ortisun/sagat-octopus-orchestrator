# Feature Specification: Invocation and human-interaction surfaces

**Feature Branch**: `feature/026-invocation-human-surfaces`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Temporal/Windmill parity for invoking workflows and involving people: start forms generated
from a workflow's input schema (classification-aware, in the Studio and embeddable); public, shareable
form links (scoped, expiring, rate-limited, an anonymous principal kind, audited, under quota); a
synchronous run-and-wait endpoint with a bounded wait that returns the result when the run ends (or
returns early, feature 022) and `202` with the run handle otherwise, under an idempotency key; a
per-workflow OpenAPI document from the declared input/output schemas and Query/Update operations;
Workflow Streams (a bounded, append-only, classified log of progress and partial-result events emitted
by steps through the worker protocol, retained and delivered over SSE with a resume cursor, used by
feature 025); approval through Slack, Teams and e-mail with signed decision links and an optional
approval input form under the feature-014 approval semantics; and notification channels for run events
and operational alerts over the ADR-0025 outbound pipeline.

## Context and delivery boundary

Today a member with `run:start` starts a run through `startRun` (`202`); the Studio asks for raw JSON
(`frontend/apps/studio/src/features/runs/StartRun.tsx`); clients poll `getRun` for the end
(`waitForCompletion`, `sdks/SPEC.md` §3) and read the result through `discloseRunPayload`
(`source: result`, feature 016). The only live view is the payload-free graph stream (ADR-0016).
Native approvals exist (FR-014-007, FR-014-008) but nobody is told about them, the Studio has no
decision screen and a decision carries only `APPROVE`/`REJECT` and a reason code. Outbound deliveries
go to webhook and broker subscriptions for `run.finished` and operational alerts (ADR-0025). Every
caller is a tenant member with a role (`crates/domain/src/identity.rs`) or a signed webhook sender
(ADR-0012, ADR-0026); there is no anonymous principal. Evidence: [research.md](./research.md).

This feature adds surfaces and changes no execution semantics: runs started from a form, a link or the
synchronous endpoint use the existing admission transaction; a channel decision is the feature-014
decision; a stream event never changes run state. The hard-to-reverse choice, how anonymous and
link-carried access is bounded, is proposed in
[ADR-0061](../../docs/adr/ADR-0061-link-bearers-hold-one-narrow-capability-and-never-decide.md).
Workflow Streams, a second value-carrying store beside the journal with a value-carrying feed, are
proposed in [ADR-0068](../../docs/adr/ADR-0068-workflow-streams-are-a-fenced-classified-log-beside-the-journal.md).
Temporal's in-workflow stream buffer exists only because of code replay and does not apply.

## User Scenarios & Testing *(mandatory)*

### US01 — Start a run from a generated form (Priority: P1)

An operator opens a workflow in the Studio and starts it through a form built from the published
version's input schema instead of typing JSON: typed fields, required markers, enumerations as
choices, defaults, help text and the classification level of every field.

**Why this priority**: It is the everyday way people start workflows and the base of the public link
(US04) and of the approval form (US05).

**Independent Test**: Publish a workflow whose `inputSchema` has a string, an integer with bounds, an
enumeration, a nested object, an array of strings, a `pii` field and a `secret` field; open the form
in a real browser, submit an invalid value and then a valid one, and assert one run with the expected
input and no value in browser storage.

**Acceptance Scenarios**:

1. **Given** a published version whose input schema uses only the supported keywords, **When** an
   operator with `run:start` opens *Start run*, **Then** the form shows one control per declared
   field, ordered by its `x-sagat-form.order` hint and then by name, with its level marked by text
   and shape, and submits the canonical input through `startRun` with one idempotency key per form.
2. **Given** a value that violates the schema, **When** it is submitted, **Then** the server refuses
   it with `VALIDATION_FAILED` and JSON pointers, the form marks those fields, and no value is echoed
   in the error.
3. **Given** a required field that uses an unsupported keyword, **When** the form is requested,
   **Then** the form model is `UNAVAILABLE` with the reason and the pointer, and the Studio offers the
   JSON editor instead; it never draws a partial form that would submit an incomplete input.
4. **Given** a `secret` field, **When** the form is rendered, **Then** it is a write-only control,
   never prefilled, never shown back in a confirmation, with browser autocomplete off; a `pii` field
   is never kept in any browser store.
5. **Given** a required field that feature 031 declares encoded client-side, **When** the form is
   requested, **Then** the model is `UNAVAILABLE` with the feature-031 reason `ENCODING_READ_BY_FORM`.

### US02 — Call a workflow and wait for its result (Priority: P1)

An application calls one endpoint that starts a run and returns its classified result when the run
ends within a bounded wait, or `202` with the run handle when it does not, and can then wait again
on that run.

**Why this priority**: Request/response integrations (an API that fronts a workflow) are the most
common Windmill and Temporal use and today need a polling loop in every client.

**Independent Test**: Execute a workflow that finishes in about one second with a 10-second wait
(expect `200` and the result), one that takes 40 seconds with a 5-second wait (expect `202`, then
`GET …/outcome?waitMs=` until `200`), repeat the first call with the same idempotency key after
killing the control plane in the middle of the wait (expect the same run, no second run), and assert
that no database connection is held by a waiting request.

**Acceptance Scenarios**:

1. **Given** a run that reaches a terminal status before the effective wait ends, **When**
   `executeRun` returns, **Then** it is `200` with the terminal `Run`, `completion: TERMINAL`, the
   closed failure code when it failed, and the run result projected under feature 016 for the caller.
2. **Given** a run still running at the end of the wait, **When** the wait ends, **Then** the answer
   is `202` with the `Run`, `completion: PENDING`, a `Location` to its outcome and the effective wait;
   the run continues unaffected.
3. **Given** a retried call with the same `Idempotency-Key` and body, **When** it arrives during or
   after the first wait, **Then** it waits on the same run; a different body under the key is
   `409 IDEMPOTENCY_KEY_REUSED`.
4. **Given** feature 022 is present and the run committed its declared early result, **When** the
   waiter sees it, **Then** the answer is `200` with `completion: EARLY_RETURN` and that result while
   the run's trailing steps continue.
5. **Given** the waiter limit of the subject is reached, **When** another waiting call arrives,
   **Then** it is refused with `429 RATE_LIMITED` before any run is admitted.

### US03 — Follow progress and partial results through a Workflow Stream (Priority: P1)

A step reports its progress and partial results (rows processed, a percentage, text produced so far)
on declared stream channels; the Studio and API clients follow them live and resume after a
disconnection without missing or duplicating an event.

**Why this priority**: Long steps and AI agents (feature 025, token streaming) need live output that
is neither the final step output nor a log line.

**Independent Test**: A worker in each of the five SDKs publishes 500 progress events in batches on a
declared channel while a client follows the SSE stream, disconnects at event 200 and reconnects with
`Last-Event-ID`; the lease expires once and a second attempt publishes again; assert the delivered
offsets are gap-free and unique, both attempts are distinguishable, and an append after the lease
expired is refused with `STALE_ATTEMPT`.

**Acceptance Scenarios**:

1. **Given** a workflow version that declares channel `progress` and a node that may emit to it,
   **When** the leased attempt appends a batch, **Then** each event is validated against the channel
   schema, receives the next run stream offset and is readable by the SSE and page endpoints.
2. **Given** a batch whose response was lost, **When** the worker sends it again with the same
   publisher sequence, **Then** the original offsets are returned and nothing is stored twice.
3. **Given** an expired lease or a completed step, **When** an append arrives, **Then** it is refused
   with `STALE_ATTEMPT`; **given** a channel the node does not declare, it is refused with
   `VALIDATION_FAILED`; in both cases nothing is stored.
4. **Given** a subscriber holding offset N, **When** it reconnects with `Last-Event-ID: N`, **Then**
   it receives the `attempts` snapshot and exactly the events after N; when a newer attempt of a node
   it has events for starts, it receives `superseded`; when the run is terminal and all events are
   sent it receives `event: end`.
5. **Given** an event whose fields are `public`, `internal` and `pii`, **When** a viewer, an operator
   and a holder of the reveal grant follow the stream, **Then** each receives only the fields of its
   audience (feature 016), the reveal is audited when the connection opens, and `secret` is never sent.
6. **Given** a channel's budget or the author pool is exhausted, **When** the step appends again,
   **Then** the append is refused with `QUOTA_EXCEEDED` (`STREAM_LIMIT`, naming the scope) and the step
   itself continues; a reserved `sagat.` channel of the same run is unaffected, and the reverse.

### US04 — Share a public form link (Priority: P2)

An administrator publishes a form for people outside the tenant (a support request, a partner
onboarding) as a link that expires, can be paused or revoked, is rate-limited and is embeddable in an
approved web site. The submitter needs no account and sees only a receipt.

**Why this priority**: High value for intake processes, and the riskiest surface (anonymous callers),
so it follows the authenticated form.

**Independent Test**: Configure the public-forms origin, enable public links in a namespace, create a
link for a pinned version with two exposed fields, one server-side preset and a live-run cap of 2,
submit from a browser without an account on the public origin (one run, actor
`system:public-link:<id>`), replay the same submission (same receipt token), exceed the per-link rate
and the live-run cap (429), revoke the link (opaque 404), demote the link owner (refused, audited,
alert), and assert that no tenant, namespace or workflow internals and no source address appear in
responses, logs or audit, and that the Studio origin answers the public paths `404`.

**Acceptance Scenarios**:

1. **Given** the namespace quota `public_links` is 0 (the default), **When** an administrator creates
   a link, **Then** it is refused with `QUOTA_EXCEEDED`; after the quota is raised it is created and
   its token is shown once.
2. **Given** an active link, **When** an anonymous person submits the exposed fields, **Then** the
   input is the presets merged with the submitted fields, validated against the pinned version, the
   run is admitted in one transaction with the submission receipt and audit, and the answer is a
   receipt with no run identifier.
3. **Given** an unknown, expired, paused, revoked or exhausted link, **When** it is used, **Then** the
   answer is the same `404` with no reason that distinguishes them.
4. **Given** a link owner who no longer holds `run:start` in the namespace, **When** a submission
   arrives, **Then** it is refused like an unavailable link, the refusal is audited and the namespace
   alert `PUBLIC_LINK_OWNER_UNAUTHORIZED` fires.
5. **Given** a link that declares `resultDisclosure: public-fields-v1`, **When** the submitter polls
   the receipt, **Then** it sees the run's coarse status and only the `public` fields of the result;
   without the declaration it sees the status only.
6. **Given** a link embedded in an approved origin, **When** another origin, or an origin approved
   only for another tenant's link, frames it, **Then** the browser refuses the frame: the page's
   `frame-ancestors` header is computed from that link alone.
7. **Given** a link at its live-run cap, or a namespace whose public-link pool is full, **When** a
   submission arrives, **Then** it is refused with `429` before admission, and members can still start
   runs up to the namespace's other active-run capacity.

### US05 — Decide an approval from Slack, Teams or e-mail (Priority: P2)

When an approval opens, the people who can decide it are told on the channels the workflow declares,
with a bounded, declared summary and a signed link to a decision page. They sign in, see the summary
their role may see, optionally fill the approval form, and decide; the existing feature-014 rules
decide whether the decision counts.

**Why this priority**: Approvals that nobody hears about expire; this closes the loop of US04 of
feature 014 without changing its semantics.

**Independent Test**: Open an approval with two notify targets (a local stand-in receiver for Slack
and an SMTP sink), follow the link from each, decide with an ineligible user (refused, audited), with
the initiator (refused), and with an eligible user who fills the form (accepted, input committed with
the decision); race a decision against expiry; replay the decision; assert one outcome and that the
channel message is updated with the outcome only.

**Acceptance Scenarios**:

1. **Given** an approval node with `notify` targets and a declared `summary`, **When** the approval
   opens, **Then** the opening, its summary, one delivery per target and one decision link per
   delivery commit in the opening transaction; a missing or paused channel does not prevent the
   approval from opening.
2. **Given** a decision link, **When** it is opened, **Then** the page requires sign-in; the link
   identifies the approval but grants nothing; a signed-in person who is not eligible, the excluded
   initiator or a member of another tenant cannot decide, and the denial is audited without disclosure.
3. **Given** an approval with a form whose schema requires a comment on rejection, **When** a
   rejection without the comment is submitted, **Then** it is refused with `VALIDATION_FAILED`; with
   it, the decision, its input and the route commit together and later steps read
   `steps.<node>.output.input`.
4. **Given** an expiry and a decision that race, **When** one commits, **Then** the other is a no-op;
   a link opened after settlement shows the settled status only.
5. **Given** a settled approval, **When** the settlement commits, **Then** an `approval.settled`
   delivery updates or follows up the channel message with the outcome and no actor or input.

### US06 — Tell a team on Slack, Teams or e-mail when runs end and alerts fire (Priority: P2)

An administrator registers notification channels and subscribes them to finished runs of selected
workflows and statuses, to approval requests, and to the namespace's operational alerts.

**Why this priority**: Operators live in chat; ADR-0025 already gives a durable pipeline, so this is
mostly new destinations and fixed templates.

**Independent Test**: Register a Slack, a Teams and an e-mail channel against local receivers;
subscribe them to `run.finished` (status `FAILED`) and `ops.alert.fired`; fail a run and trigger an
alert; assert one delivery per channel and event in the causing transaction, retries with the same
delivery id after a lost acknowledgement, `DEAD` after the attempt limit, redrive, and no payload
value in any message.

**Acceptance Scenarios**:

1. **Given** a channel subscribed to failed runs of workflow `orders`, **When** such a run ends,
   **Then** the delivery is inserted with `run.finished` in the same transaction and the message
   names the workflow, version, status, failure code and a link to the run, and nothing else.
2. **Given** a channel whose credential is unavailable, **When** delivery is attempted, **Then** it
   is retried with the same delivery id, becomes `DEAD` at the attempt limit, stays visible and can be
   redriven, and the channel is disabled after the existing consecutive-dead threshold.
3. **Given** *Send test message*, **When** an administrator requests it, **Then** a delivery is
   queued through the same pipeline; the API process never sends it itself.
4. **Given** a new channel, **When** it is created, **Then** its maximum classification is `public`;
   raising it to `internal` without `payload:read-internal`, or to `pii` without the namespace reveal
   grant, is refused with `FORBIDDEN`.

### US07 — Get the OpenAPI document of one workflow (Priority: P2)

A developer downloads an OpenAPI 3.1 document for one published version, with the typed start,
execute and outcome operations, its Query and Update operations and its stream channels, and
generates a client or imports it into an API gateway.

**Why this priority**: Makes each workflow a typed API without new execution endpoints.

**Independent Test**: Generate the document for a version with input/output schemas, two queries,
one update and one stream channel; lint it as OpenAPI 3.1; send a request valid against it (accepted)
and one invalid against it (refused by the server with `VALIDATION_FAILED` at the same pointer).

**Acceptance Scenarios**:

1. **Given** a published version, **When** a caller with `definition:read` requests its document,
   **Then** the same version always yields byte-identical JSON with an `ETag`, containing only
   definition metadata and no run data.
2. **Given** a version without `inputSchema`, **When** the document is generated, **Then** the input
   is described as an open JSON object and the document says so in `x-sagat-untyped`.

### US08 — Decide inside Slack with a linked identity (Priority: P3)

An approver who linked their Slack identity to their platform subject once, and confirmed the link in
a signed-in session, approves or rejects from the Slack message buttons without opening a browser.
The namespace's Slack app binding ties a Slack workspace to the tenant at registration.

**Why this priority**: Convenience on top of US05, with the most new trust surface; it waits for US05.

**Independent Test**: Register a Slack app binding (team id from `auth.test`), link and confirm a
Slack user, click *Approve* (a signed interaction is admitted into the inbox and the decision is
attributed to the linked subject after the feature-014 checks); click from an unlinked user (an
ephemeral reply with the decision link, no decision); replay the interaction (no second decision);
send a bad or stale signature (uniform `401`); register the same Slack team and channel as interactive
in a second tenant (refused); use a stolen code from another workspace or without the victim's Studio
confirmation (no active link).

**Acceptance Scenarios**:

1. **Given** a linked identity whose subject is eligible, **When** the button is clicked, **Then** the
   decision is the feature-014 decision of that subject with `via: SLACK` attribution.
2. **Given** the subject lost its membership, the link was revoked or is still pending confirmation,
   **When** the button is clicked, **Then** no decision is applied and the person is sent to the
   decision page.
3. **Given** a button value whose decision link belongs to another tenant, namespace, team or channel
   than the binding that received the request, **When** it arrives, **Then** nothing is decided and the
   refusal is audited in the binding's tenant only.

### Edge Cases

- A version republished with a changed input schema: forms and the generated OpenAPI follow the
  version they were asked for; a public link keeps its pinned version until edited.
- A form submitted twice (double click, lost response): one run per idempotency key.
- A public link edited while a submission is in flight: the submission is judged against the link
  revision it locked; the edit waits or follows.
- The wait ends in the same instant the run ends: either `200` or `202` is correct; nothing changes.
- The control plane restarts with waiters, stream subscribers and decision pages open: connections
  drop, clients retry or reconnect, no state is lost and no run is started twice.
- The NATS hint of a finished run is lost: the batched poll observes the terminal status.
- A stream append races the step's completion: the first commit wins; a later append is stale.
- A stream event schema changes in a newer version: a run keeps the channels of its pinned version.
- An approval inside a collection or loop body (scoped instance): links and summaries are per
  instance; notifications follow the scoped approval only where feature 014 enables it.
- A channel message cannot be edited (Teams Workflows webhook, e-mail): the settlement is a follow-up
  message.
- A Slack delivery acknowledged by Slack whose acknowledgement was lost: the message is sent twice;
  this is stated, not hidden (at-least-once, ADR-0025).
- A decision link forwarded to another eligible person: that person decides as themself after
  signing in; the link's intended recipient is shown, not enforced.
- Data that left in a chat or e-mail message cannot be deleted by the platform (erasure boundary).
- An encrypted (feature 031) field in an input, a stream event or a summary: opaque, never rendered,
  never disclosed to anonymous callers.

## Requirements *(mandatory)*

### Functional Requirements

#### Generated start forms (US01)

- **FR-026-001 — Form model**: The server MUST derive a form model from the `inputSchema` of one
  published version (`GET /v1/workflows/{workflowId}/versions/{versionNumber}/form`): an ordered list
  of fields with JSON pointer, type, required flag, constraints, choices, default, title, description,
  hints and the feature-016 effective classification level, plus a status `AVAILABLE` or
  `UNAVAILABLE` with closed reasons. One Rust implementation computes it; the browser renders it and
  never infers schema semantics itself (ADR-0016). The model is deterministic for a version.
- **FR-026-002 — Supported subset**: The model MUST support exactly the keyword subset of
  [contracts/form-model.md](./contracts/form-model.md) (scalar types, objects, `required`, `enum`,
  `const`, `default`, titles, numeric and string bounds, four formats, arrays of scalars, local `$ref`).
  Any other keyword makes its field `UNSUPPORTED` with its pointer; an unsupported required field, or a
  schema above the form ceilings, makes the model `UNAVAILABLE`; no `inputSchema` is `NO_SCHEMA`.
- **FR-026-003 — Form hints**: The IR MUST accept an optional, closed `x-sagat-form` annotation on
  carrier schema nodes of `inputSchema` and of the approval form (FR-026-033) with only `widget`
  (`text`, `textarea`, `select`, `radio`, `checkbox`, `password`, `date`), `order` (integer) and
  `placeholder` (≤ 120 characters). Publication and `validate` reject unknown keys, wrong types and a
  widget incompatible with the field type. Hints never change validation of the input.
- **FR-026-004 — Classification-aware rendering**: Each field MUST show its effective level by text
  and shape, not colour alone. A `secret` field MUST be write-only (password control, never prefilled,
  never echoed, autocomplete off). No field value is put in a URL, a browser store, a persisted query
  cache or telemetry; form state is cleared on submission, on navigation and on session change.
  Values are never prefilled from a URL.
- **FR-026-005 — Submission**: A form MUST submit through the existing `startRun` admission with one
  idempotency key per opened form, kept across retries. Server validation errors map to fields by
  JSON pointer only, without echoing values.
- **FR-026-006 — Encrypted fields**: A location that feature 031 declares encoded (its draft annotation
  `x-sagat-encoding`) MUST be opaque to every surface of this feature, which registers its positions in
  feature 031's encoded-field registry (forms and public submissions as read positions and write paths,
  stream channel schemas and the approval form as carriers, approval summaries as expression positions)
  and uses only that feature's codes. The derived start form treats such a field as unsupported
  (reason `ENCODING_READ_BY_FORM`; `UNAVAILABLE` when required); a declared artefact that reads it (an
  `x-sagat-form` hint, an approval `form` or `summary`) is refused at publication through the
  feature-031 check (`ENCODING_READ_BY_FORM`); a public link cannot expose or preset it (the same code);
  no form accepts, renders or presets it.

#### Synchronous invocation (US02)

- **FR-026-007 — Run and wait**: The platform MUST provide `POST /v1/runs/execute` (`executeRun`)
  that takes the `StartRunRequest` body plus `waitMs`, requires `Idempotency-Key`, admits the run with
  exactly the `startRun` transaction and authorization (`run:start`, plus `run:read` to receive the
  outcome), and waits at most the effective wait (min of request, namespace setting and deployment
  ceiling). It answers `200` with a `RunOutcome` when the run is terminal (or returned early,
  FR-026-011) and `202` with `completion: PENDING`, the `Run` and a `Location` header otherwise.
- **FR-026-008 — Wait on an existing run**: The platform MUST provide
  `GET /v1/runs/{runId}/outcome?waitMs=` that answers `200` immediately for a terminal run and
  otherwise waits like FR-026-007, answering `202` at the end of the wait. It requires `run:read`.
- **FR-026-009 — Waiting is not holding**: A waiting request MUST NOT hold a database transaction,
  connection, row lock, lease or worker slot. Waiters register in a bounded in-process registry woken
  by the existing `run.finished` hint and by feature 022's `run.result_returned` hint, to which every
  control-plane replica subscribes without a queue group, and by one batched poll per process at a
  bounded interval that reads both terminal status and a committed early result; a hint is never the
  record. Waiters are bounded per subject, tenant and process; a refusal happens before admission. On
  shutdown or at the end of the wait a waiter answers `202`. The wait deadline is infrastructure time.
- **FR-026-010 — Outcome content**: A `RunOutcome` MUST carry the `Run`, `completion`
  (`TERMINAL`, `EARLY_RETURN`, `PENDING`), the closed failure code of a failed run, and, for a run
  with a result, the result as the feature-016 classified projection for the caller's audience
  (`reveal: [pii]` opt-in, audited as `payload.pii_revealed` in the read transaction). It never
  carries step outputs or error payloads. Responses are `Cache-Control: private, no-store`.
- **FR-026-011 — Early return**: When feature 022 is present, a run that committed its declared early
  result MUST complete the wait with `completion: EARLY_RETURN` and that result projected under its
  declared schema; until then `EARLY_RETURN` is reserved and never produced. Feature 022 announces an
  early result with its `run.result_returned` hint (not `run.finished`); the batched poll covers a lost
  hint within one interval.
- **FR-026-012 — Synchronous custom routes**: When feature 021 custom HTTP routes exist, a route MAY
  declare a wait (`response.mode: wait`, `waitMs`) served by the same waiter. Its caller is a webhook
  identity, not a member: it receives a result only when the route declares
  `resultDisclosure: public-fields-v1`, and then only the fields of FR-026-026; otherwise `202` with no
  run identifier. This requirement is a contract hook implemented with feature 021.

#### Workflow Streams (US03)

- **FR-026-013 — Declaration**: The IR MUST accept an optional top-level `streams` map (channel name
  → `{schema, display, description, budget}`) and an optional `emits` list on `activity`, `script` and
  managed nodes. A channel schema is a closed, bounded carrier classified per feature 016; `display` is
  `list` or `progress` (the latter requires a top-level number `fraction` in [0, 1]); `budget`
  (`maxEvents`, `maxBytes`) is the channel's per-run budget, defaulted and capped by the plan.
  Publication rejects undeclared channels in `emits`, open schemas, budgets above the ceilings and
  author channels whose budgets sum above the author pool. Channel names starting with `sagat.` are
  reserved: their schema, classification and budget are fixed by the owning feature (for example
  `sagat.agent.tokens` of feature 025), declared by that feature's node declaration, and counted in a
  separate reserved pool (FR-026-019). Declarations are pinned with the version (ADR-0068).
- **FR-026-014 — Append**: A leased attempt MUST append batches through a new worker-protocol rpc,
  `AppendStreamEvents` ([contracts/worker-protocol.md](./contracts/worker-protocol.md)). The server
  checks the lease on the database clock, that the node declares each channel, the channel schema, the
  ceilings, and that no value given to the step as a secret appears (`SECRET_IN_OUTPUT`); accepted
  events commit atomically; a stale lease is `STALE_ATTEMPT` and stores nothing.
- **FR-026-015 — Stream log**: Events MUST be stored in a per-run append-only log separate from the
  journal (ADR-0068), with a per-run monotonic offset allocated in the append transaction, the
  channel, node, instance path, attempt, generation, publisher sequence, server time and the event
  data inline in the same row (never in the shared content-addressed `payloads` table), so that
  deleting the row deletes the value and releases its bytes. An append MUST NOT change the run
  version, the journal, the run state or anything a workflow expression can read. Deduplication is by
  `(run, node instance, attempt, generation, publisherSeq)`: a replayed batch returns its original
  offsets; the same key with different content is `CONFLICT` (`IDEMPOTENCY_KEY_REUSED`).
- **FR-026-016 — Attempts and continuation**: Events of a superseded attempt MUST remain and carry
  their attempt; every read marks an event `superseded` when a later attempt of the same node instance
  exists. A live subscriber MUST be told: the feed sends an `attempts` snapshot (current attempt and
  settled state of every node instance that has events) when it opens or resumes, and a control event
  `superseded` when a newer attempt of such a node instance starts and `settled` when its step
  settles; both are derived from committed attempt state at read time, carry no offset and add no write
  to the claim path. A client discards events of attempts older than the current one. A stream belongs
  to one run (one Continue-As-New segment); a successor starts its own log at offset 1.
- **FR-026-017 — Delivery**: The platform MUST serve `GET /v1/runs/{runId}/stream-events` as
  `text/event-stream` and as a bounded JSON page for the same query, filtered by channel. The SSE `id`
  and the resume cursor are `<epoch>.<offset>`, where `epoch` is the cell epoch of feature 032 (0 when
  032 is absent); a client resumes with `Last-Event-ID` or `after`, and a cursor whose epoch differs
  from the current one makes the server send `event: reset` and restart from the beginning with the
  `attempts` snapshot, so a reader never mixes offsets of two epochs. The stream sends `event: end`
  when the run is terminal and every event has been sent, `event: reconnect` at the lifetime ceiling,
  and a ping comment at the existing interval. Open streams are bounded per subject and tenant,
  rechecked for membership like the graph stream, and need `run:read`.
- **FR-026-018 — Disclosure in streams**: Each event's `data` MUST be the feature-016 projection for
  the subscriber's audience; `reveal: [pii]` is allowed only with the namespace grant and audited
  when the connection opens and at each membership recheck. An event that does not validate is never
  stored, so a stored event always has a carrier.
- **FR-026-019 — Bounds**: Event and batch sizes, per-attempt rate and three budgets MUST be enforced
  before storage: each channel's own budget; the author pool (every author channel of the run
  together); the reserved pool (every `sagat.` channel together); all under a per-run hard ceiling
  that bounds both pools (plan, Finite ceilings). A reserved channel never consumes the author pool,
  and the reverse. Stream bytes count against `tenant_content_bytes`. Exhaustion is `QUOTA_EXCEEDED`
  (`STREAM_LIMIT`, with the exhausted scope) or `RATE_LIMITED` (`STREAM_RATE`) to the publisher; it
  never fails the step by itself.
- **FR-026-020 — Publishers**: The five SDK workers MUST publish through FR-026-014; scripts publish
  through the reserved, capability-gated `stream.append` host call that feature 023 defines for every
  script language, which the supervisor turns into an append under the attempt's lease; managed
  executors (features 017/025) use the same rpc under their capability. No publisher other than a
  current lease holder can append.
- **FR-026-021 — Retention and reset**: Stream events MUST be deleted, data included, a bounded time
  after the run is terminal (namespace setting within the deployment ceiling, never later than the
  run's retention), one run per transaction, releasing exactly the bytes they were charged; the purge
  instant is infrastructure time set when the run becomes terminal. The store is registered in feature
  020's content-store registry with this purge rule and the erasure rule that run erasure deletes the
  rows at once. A reset of feature 018 does not copy stream events: the new run starts its own log at
  offset 1 (the rule this feature declares in feature 018's reset-rule registry).

#### Public form links (US04)

- **FR-026-022 — Link resource**: A namespace administrator MUST be able to create, list, read,
  edit, pause, resume, rotate and revoke public links (`/v1/namespaces/{namespaceId}/public-links`).
  A link pins one published version, lists the exposed input fields (pointers of a form that is
  `AVAILABLE` for them), holds optional server-side presets (classified, never shown to the
  submitter), a public title and description, an expiry (required, infrastructure time), an optional
  submission cap, a required cap on concurrently live runs, a `resultDisclosure` (`none` or
  `public-fields-v1`), up to eight embedding origins and a non-secret embed id. A field whose
  effective level is `secret`, or that is encrypted (FR-026-006), can be neither exposed nor preset.
  Edits create immutable revisions with optimistic concurrency and are audited. The namespace count
  quota `public_links` defaults to 0, so no link exists until an administrator raises it.
- **FR-026-023 — Link token**: The token MUST be 256 random bits, shown once at creation or rotation
  and stored only as a SHA-256. The shareable URL is `https://<public-forms-origin>/f/<embedId>#t=<token>`:
  the embed id is non-secret and lets the server choose the page's headers; the token stays in the
  fragment, so it never reaches a server or proxy log or a `Referer`, and the page sends it in a
  header. Rotation invalidates the previous token at once and keeps the embed id.
- **FR-026-024 — Public-link principal**: A submission MUST be served by a distinct principal kind
  (ADR-0061) that carries tenant, namespace, link and link revision and holds exactly one capability:
  submit the pinned version's form through this link. It is not a `Subject`, has no membership or
  role, cannot call any other operation, and is recorded as actor `system:public-link:<linkId>`.
  The run is admitted on behalf of the link owner: each submission rechecks under lock that the owner
  is still a member holding `run:start` in the namespace and, when feature 029 is enabled, that its
  evaluator still lets the owner see and start the pinned workflow (folder visibility), and fails
  closed otherwise.
- **FR-026-025 — Submission admission**: Before any database work: token format and body size, then
  a per-process keyed rate guard (keyed on the connection's peer address, or on a forwarded address
  only from operator-configured trusted proxies, right-most untrusted hop). Then an unlocked lookup of
  the link by token hash (unusable links end here as the same `404`) and the debit of the durable
  per-link and per-namespace (`public_link_submissions_per_minute`) token buckets in their own short
  committed decision, as ADR-0027 does for API rates (a refusal is `429`; a debit is not refunded when
  the admission later fails). Then the existing admission transaction, in the slots of ADR-0050
  decision 7: slot 1 — the link row `FOR UPDATE` (state, expiry, submission cap, the exact live-run
  count of the link) and the read-only 029 access check when enabled; slot 2 — the submission receipt;
  slot 4a — the owner's membership `FOR SHARE`, the 029 access fences and the namespace row (the owner
  recheck); the merge of presets with exactly the exposed fields and schema validation (any other
  field or a schema violation is `VALIDATION_FAILED` `FORM_INPUT_INVALID`, checked before the
  transaction on the unlocked link and again against the locked revision); slots 4b to 6 as for any
  admission, where slot 6's active-run admission also takes the exact count of the namespace pool of
  live public-link runs (`public_link_active_runs`, a sub-pool of `active_runs` under the feature-009
  lock, so anonymous callers cannot take the capacity members need); slot 8b — usage rows last. The run
  (origin `PUBLIC_LINK`), its first tasks, journal, outbox, audit, quotas and the submission receipt
  commit together. A client-generated submission key (at least 128 random bits, one per opened page)
  replays the original receipt.
- **FR-026-026 — Receipt**: The submitter MUST receive `202` with a receipt token (a second
  capability bound to one submission, 256 bits, derived as an HMAC of the link and submission-key hash
  so that a replay returns the same token without the server storing it; kept in page memory only) and
  no run, workflow, namespace or tenant identifier. The receipt endpoint returns a coarse status
  (`ACCEPTED`, `RUNNING`, `COMPLETED`, `FAILED`) and, only when the link declares `public-fields-v1`,
  the result fields whose effective level is `public`; for a value written by a model (feature 025) the
  effective level is at least the highest level of everything that entered its transcript (feature
  025's model-output floor), whatever the author's annotation says. It never returns `internal`,
  `pii` or `secret` values, failure codes or step data.
- **FR-026-027 — Public page and embedding**: The public page MUST be a separate minimal entry with
  no Studio shell, no sign-in, no cookie and no third-party resource, served together with the
  `/v1/public/*` routes only on an operator-configured public-forms origin that MUST differ from the
  Studio origin (it may share a host with the feature-027 apps origin; ADR-0062 of feature 027 holds
  the combined refinement of ADR-0016 for these origins and their CSP); the Studio origin answers those
  paths `404`, and the control plane refuses to start when the two origins are equal. The page's
  `frame-ancestors` header is computed from the link found by the embed id; an unknown embed id or an
  unusable link gets `'none'` and the same page. Every unusable link (unknown, expired, paused,
  revoked, exhausted, at capacity, owner unauthorized) answers the same `404` from the API. The page
  shows only the link's public title, description and exposed fields.
- **FR-026-028 — Abuse, audit and privacy**: Every accepted submission MUST be audited
  (`public_link.submitted`, link, revision, run) in its transaction; refusals are audited with a
  closed reason and collapsed per link and reason for 300 seconds. No source address, user agent or
  submitted value is written to audit, logs, spans or metrics. A link refused for its owner raises the
  namespace alert `PUBLIC_LINK_OWNER_UNAUTHORIZED`.

#### Approval through channels (US05)

- **FR-026-029 — Declaration**: The `approval` node MUST accept optional `notify` (1–4 targets, each
  naming a notification channel of the namespace) and `summary` (1–16 fields `{key, label, value}` with
  a closed, classified result schema and `disclosure: declared-approval-summary-v1`). Publication
  computes the effective level of every location each `value` expression reads (an undeclared location
  is `pii`; a model-written value follows feature 025's floor) and refuses a summary field annotated
  below that level (`SUMMARY_LEVEL_BELOW_SOURCE`). The summary is evaluated with the existing bounded
  evaluator over the committed run snapshot when the approval opens; it receives no secret, capability
  or network access.
- **FR-026-030 — Opening**: The approval opening transaction (FR-014-007) MUST also commit the
  evaluated summary as retained content, one outbound delivery of `approval.requested` per active
  target and one decision link per delivery. A target that is missing, paused or disabled MUST NOT
  prevent the approval from opening; it is audited as `notification.skipped` and visible as an alert.
  A summary evaluation failure opens the approval without a summary, audited with a closed reason.
  This applies to every approval owner kind, including feature 025's `AGENT_TOOL_CALL` (025 owns that
  amendment of the feature-014 approval rows). A draft test run (feature 022, origin `DRAFT_TEST`)
  sends no notification and creates no decision link.
- **FR-026-031 — Decision links**: A decision link MUST be a signed opaque token (256 random bits and
  an HMAC-SHA-256 tag under a rotatable platform key, format in
  [contracts/channel-messages.md](./contracts/channel-messages.md)) stored only as a hash, bound to
  tenant, namespace, approval and delivery, and carried in the URL fragment of
  `/studio/approvals/decide`. Its validity is infrastructure time (creation plus the lesser of the
  approval timeout and 30 days) and, separately, the approval still being `PENDING`; the approval's
  deadline is workflow time and is never compared with the link's validity (the time-class catalog of
  feature 028, FR-028-003, ADR-0063 draft). When feature 029 is enabled, resolving a link also
  requires that its evaluator lets the caller see the run's workflow.
  It identifies the approval and grants nothing: a decision always needs an authenticated principal
  that passes FR-014-008 at commit (ADR-0061).
- **FR-026-032 — Decision page**: The Studio MUST provide a decision page that resolves the link after
  sign-in (the token is kept in `sessionStorage` only across the sign-in redirect and removed on return;
  never in local storage, a URL or a query), shows the approval status, deadline and the summary
  projected for the viewer's own audience (not the channel's), the optional form, and submits the
  existing decision operation with a decision key generated per page and the link token. The server
  derives the decision's attribution (`via`: the link's channel kind, or `API` without a valid link of
  that approval); a client cannot assert it. A settled or expired approval shows its status only.
  Ineligible callers see a refusal without policy details.
- **FR-026-033 — Approval input form**: The `approval` node MUST accept an optional `form`
  (`schema`: a closed, bounded carrier classified per feature 016 and rendered by FR-026-001..004;
  `requiredFor`: `APPROVE`, `REJECT` or both). `ApprovalDecisionRequest` gains optional `input`,
  validated against the pinned schema; the decision, the input, the receipt, the route, the journal,
  the audit and the outbox commit together; the decision-key fingerprint includes the input when it is
  present and stays byte-identical to today's fingerprint when it is absent, so decision retries in
  flight across the upgrade replay instead of conflicting. The input is the approval step's output
  under `input`; the approval form schema becomes a feature-016 carrier (an additive extension of
  FR-016-002).
- **FR-026-034 — Settlement**: The settlement transaction (decision, expiry or cancellation) MUST
  insert one `approval.settled` delivery for each delivery of `approval.requested`, which edits the
  original message where the destination allows it and otherwise posts a follow-up; it carries the
  outcome and time only, never the actor, the reason code or the input. Settlement closes the
  decision links.

#### Notification channels (US06)

- **FR-026-035 — Channel resource**: The platform MUST provide namespace notification channels
  (`/v1/namespaces/{namespaceId}/notification-channels`) as additional typed destinations of the
  ADR-0025 outbound subscription aggregate: `SLACK` (a Slack app binding of the namespace, FR-026-042,
  and a channel id), `TEAMS` (Workflows webhook URL secret), `EMAIL` (deployment SMTP relay, recipient
  list, optional relay credential secret). Settings are non-secret; credentials are secret bindings
  (ADR-0035 authority); each channel has a namespace-unique name used by `notify`, a
  `maxClassification` (default `public`) and the ADR-0025 status, rate, health and dead-letter view.
- **FR-026-036 — Subscriptions**: A channel MUST subscribe to a closed set of event kinds:
  `run.finished` (optional filters: up to 32 workflow names and a set of terminal statuses),
  `ops.alert.fired`, `ops.alert.resolved`, `approval.requested`, `approval.settled`; other features
  add kinds additively (for example a schedule auto-pause of feature 018). Deliveries are
  inserted in the transaction that commits the fact (ADR-0025 decision 2) and follow its rate, lease,
  attempt fence, retry, `DEAD`, redrive and disable rules.
- **FR-026-037 — Message content and placement**: Channel messages are fixed-shape, platform-defined
  deliveries to a tenant-configured destination: the engine sends them through the guarded egress
  client holding only that channel's own credential (the placement rule of feature 021, ADR-0055 rule
  (b)); channels keep their own credentials like webhook subscriptions and never reuse feature-021
  resources. Messages MUST be rendered at send time from closed platform templates per event kind and
  destination with identifiers, names, statuses, counts, times and links built from the deployment's
  base URL only. Approval summary fields are included only when their effective level is at or below
  the channel's `maxClassification`; `secret` never. Values are escaped for the destination (Slack
  mrkdwn, Adaptive Card text, HTML) and bounded; no author-supplied link, image or markup is rendered.
- **FR-026-038 — E-mail**: E-mail MUST use the deployment-configured SMTP relay with TLS required,
  the egress guard, a deployment sender address, `text/plain` with a simple escaped HTML alternative,
  and recipient addresses classified `pii` (never logged, labelled or put in spans). No inbound reply
  is ever treated as a decision.
- **FR-026-039 — Test, health and the `pii` egress rule**: *Send test message* MUST queue a delivery
  through the pipeline; channel health shows the last attempt outcome, consecutive dead count and
  status. Setting `maxClassification` to `internal` requires `event:manage` and
  `payload:read-internal`. A `pii` value leaves through a channel only under the uniform rule proposed
  by feature 020 (owner question O1081, refining ADR-0033): the channel's ceiling `pii` is set by an
  administrator holding the namespace `payload:reveal-pii` grant; that grant is rechecked at each
  delivery that would carry a `pii` field; its revocation pauses the channel; each such delivery is
  audited; `secret` never leaves. Each ceiling change is audited.

#### Per-workflow OpenAPI (US07)

- **FR-026-040 — Generated document**: The platform MUST serve
  `GET /v1/workflows/{workflowId}/versions/{versionNumber}/openapi` (`definition:read`): a
  deterministic OpenAPI 3.1 document that describes the real generic operations specialized for that
  version (`startRun` and `executeRun` with `workflowId` and `versionId` as `const` and `input` as the
  `inputSchema`; `getRunOutcome` with the `outputSchema` as result; each declared Query and Update
  with its argument and result schemas; `streamRunEvents` with one schema per channel), with
  classification annotations kept and no run data; a version without `inputSchema` or
  `outputSchema` describes it as an open object marked `x-sagat-untyped: true`. The answer has a
  strong `ETag` derived from the version digest and the generator version. No new execution path is
  created.
- **FR-026-041 — Agreement with the server**: A request that validates against the generated
  document MUST pass the server's schema validation for that version and one that fails it MUST be
  refused at the same JSON pointer; this is a contract test, not an assumption.

#### Decisions inside chat (US08)

- **FR-026-042 — Slack app binding, identity links and in-chat decisions**: One Slack app model: a
  tenant-owned Slack app registered as a namespace *Slack app binding* (bot token and signing secret as
  secret bindings). At registration the platform calls Slack `auth.test` with the bot token and stores
  the returned team id (never client-supplied); a Slack channel marked interactive is unique by
  `(team, channel)` across all tenants. Interaction and slash-command requests arrive at the binding's
  own URL and are verified with that binding's signing secret
  ([contracts/channel-messages.md](./contracts/channel-messages.md), uniform `401`). The tenant and
  namespace of a decision come only from the decision-link row carried in the button value, which MUST
  match the binding's tenant, team and channel; the chat identity is looked up only inside that
  tenant. Linking an identity: the signed-in subject obtains a single-use
  code (≥ 128 random bits, 10 minutes, consumed atomically); the slash command from a team the code's
  tenant bound creates a pending link; it becomes active only after the subject confirms "Slack user U
  of workspace T" in the signed-in Studio session; code attempts are rate-limited per team. The verified
  request is admitted into the inbox with deduplication and applied by the dispatcher as the
  feature-014 decision of the linked subject (`via: SLACK`); an unlinked, pending, revoked or ineligible
  identity gets the decision link, never a decision. Teams in-chat decisions are excluded (G1324).

#### Cross-cutting

- **FR-026-043 — Authorization**: The catalog MUST gain `link:read` (admin, developer, operator) and
  `link:manage` (admin); every other operation reuses existing permissions as the plan's matrix
  states. Every check fails closed and is repeated on receipt replay.
- **FR-026-044 — Tenant boundary**: Every row, query, message, waiter key, stream subscription and
  audit MUST carry tenant and namespace; a foreign link, channel, approval, run or stream is an opaque
  `NOT_FOUND`; anonymous and link-carried calls derive tenant and namespace only from the stored
  link, never from the request.
- **FR-026-045 — Observability and audit**: Every surface MUST emit the spans, fixed-label metrics
  and audit actions listed in the plan, with no value, token, address or high-cardinality identifier
  in a label, and audit written in the transaction of the decision it records.
- **FR-026-046 — Errors**: Errors MUST reuse the closed `Error.code` set and add only `details.reason`
  refinements listed in [contracts/errors.md](./contracts/errors.md), named by the shared convention
  (`<AREA>_INPUT_INVALID`, `<AREA>_DESTINATION_DENIED`) and using feature 031's codes for encoded
  fields; worker-protocol refusals reuse the existing `x-sagat-error` codes.
- **FR-026-047 — Ceilings and quotas**: Every new resource MUST have the defaults and hard ceilings of
  the plan; effective = min(request, namespace, deployment); zero is not unlimited; admission fails
  before work; new quota resources reuse ADR-0027 administration.
- **FR-026-048 — Five SDKs**: TypeScript, Go, Python, C# and Java MUST offer `execute`,
  `awaitOutcome`, `streamEvents` (resume by offset), `getStartForm`, `getWorkflowOpenApi`,
  `decideApproval` with `input`, worker stream publishing with flush and a bounded buffer, and the
  builder fields `streams`, `emits`, `notify`, `summary` and `form`, with byte-identical canonical
  documents, and pass shared conformance scenarios.
- **FR-026-049 — Studio**: The Studio MUST add the start form, the run's *Streams* tab, the decision
  page, public link management, notification channel management and the public page entry, using
  design tokens only, WCAG AA, a table/list alternative and FR-007-010 unavailable states.
- **FR-026-050 — Compatibility and rollout**: OpenAPI, IR, worker protocol and AsyncAPI changes MUST
  be additive; existing `startRun`, approval decisions, subscriptions and Studio keep working. A
  worker that does not negotiate the new protocol minor keeps receiving every task it serves today:
  publishing is optional for a node that declares `emits`, and such a worker simply never appends.
  Each capability is enabled separately and rolled back by disabling new publication or creation
  while readers and stored data remain.

### Amendment to feature 016

This feature adds three carriers to the exact list of FR-016-002 (coordinator decision C6); the single
amendment of FR-016-002 and of classification rules version 2 is opened by the first parity feature
implemented, with the consolidated list kept in `SPECIFICATION-INDEX.md` (task T026-067):

- the `schema` of each declared Workflow Streams channel (`streams.<channel>.schema`), including the
  fixed schemas of reserved `sagat.` channels supplied by their owning feature;
- the `schema` of an approval `summary` (`declared-approval-summary-v1`);
- the `schema` of an approval `form` (the decision input, the approval step's `output.input`).

Each follows FR-016-003 to FR-016-005 unchanged (unmarked is `pii`, no loosening, pinned with the
version). Query/Update schemas stay non-carriers (FR-016-004).

### Non-functional requirements

- **NFR-026-001 (security)**: Zero tested cross-tenant access across every new operation, zero token
  or secret canary leakage, and every anonymous or link-carried path fails closed; the security battery
  is extended to the new operations, including the public ones.
- **NFR-026-002 (privacy)**: No new surface stores or logs source addresses, form values outside the
  run input, recipient addresses outside channel settings, or values above the audience's level; the
  erasure boundary of data sent to chat and e-mail is documented.
- **NFR-026-003 (durability)**: Every admission, append, opening, decision and delivery is atomic with
  its audit and outbox; duplicates and lost responses never create a second run, event, decision or
  delivery; external sends stay at-least-once.
- **NFR-026-004 (compatibility)**: Runs, clients, workers and Studio builds from before this feature
  keep their behaviour; contracts only grow.
- **NFR-026-005 (observability)**: Every accepted operation has correlated traces and bounded
  metrics; telemetry export failure never decides an admission or a decision.
- **NFR-026-006 (performance)**: Waiter wake-up latency, stream publish-to-delivery latency, append
  throughput per run and the database cost of the batched poll are measured on a declared environment;
  the ceilings are design bounds to verify, not capacity promises.
- **NFR-026-007 (accessibility)**: Forms, the decision page, the public page and the streams tab meet
  WCAG 2.2 AA, are keyboard operable and announce validation and status changes.

### Key Entities

- **Form model**: the derived, versioned description of one carrier's fields; not stored.
- **Public link / link revision / submission receipt**: a namespace resource pinning a version, its
  immutable revisions, and one receipt per accepted submission key.
- **Public-link principal**: the anonymous caller of one link, with one capability.
- **Waiter**: an in-process registration of a request waiting for a run outcome; never durable.
- **Stream channel / stream event / stream head**: a declared channel of a version with its budget, one
  appended event with its offset and inline data, and the per-run allocation and budget row.
- **Approval summary / decision link / decision input**: the evaluated declared summary of an opened
  approval, the signed pointer sent with a delivery, and the validated form input of a decision.
- **Notification channel**: an outbound subscription with a chat or e-mail destination and a name.
- **Slack app binding**: a namespace's registration of its own Slack app, with the team id verified
  through `auth.test` (P3 for interactivity; also the credential of `SLACK` channels).
- **Chat identity link**: a subject's link to one Slack identity, active only after confirmation (P3).
- **Workflow OpenAPI document**: a deterministic, derived document per version; not stored.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All criteria below are future evidence; nothing is complete because this document exists.

- **SC-026-001**: In a real browser, a form generated for each supported keyword starts exactly one run
  per submission key, refuses each invalid value at its pointer, renders every level by text, and
  leaves no input value in any browser store (FR-026-001..006).
- **SC-026-002**: `executeRun` returns `200` for runs that end within the wait and `202` otherwise in
  100 % of 1,000 mixed calls, with zero duplicate runs across retries, a control-plane kill and a
  lost NATS hint, and zero database connections held by waiting requests (FR-026-007..010).
- **SC-026-003**: The five SDKs produce identical normalized results for execute, outcome wait,
  stream publish/subscribe with resume, start form, OpenAPI retrieval and decisions with input
  (FR-026-048).
- **SC-026-004**: 10,000 appended events across two attempts, with injected lost responses, a lease
  expiry and a completion race, yield gap-free unique offsets, zero duplicates and zero events after
  the lease ended, for SDK workers, a TypeScript and a Python script and a managed executor;
  exhausting an author channel, the author pool or the reserved pool refuses only that scope
  (FR-026-013..016, 019, 020).
- **SC-026-005**: A subscriber that disconnects at random points 50 times receives every event exactly
  once by offset, the `attempts` snapshot on every resume, a `superseded` event for each newer attempt
  and `end` after the last one; a cursor of another cell epoch gets `reset`; a reset run of feature 018
  starts with an empty log; after the retention period the run's events and head are gone and the
  tenant's content bytes equal their value before the first append (FR-026-016, 017, 021).
- **SC-026-006**: For every audience and level combination, streams, outcomes, receipts, summaries and
  messages disclose exactly the fields of that audience, and `secret` and canary values never appear
  (FR-026-010, 018, 026, 032, 037).
- **SC-026-007**: Public links: unknown, expired, paused, revoked, exhausted, at-capacity and
  owner-unauthorized links are indistinguishable to the caller; replays return the original receipt
  token; rate, live-run cap, public pool and quota limits refuse before any run is admitted; the page is
  framed only by its own link's origins and never served on the Studio origin; no source address
  appears in audit, logs or spans (FR-026-022..028).
- **SC-026-008**: A crash injected at every commit point of submission, append, approval opening,
  decision with input, settlement and delivery leaves either the whole effect or none of it, and a
  retry completes it once (NFR-026-003).
- **SC-026-009**: A decision and an expiry raced 50 times select one outcome every time; ineligible,
  initiator and cross-tenant attempts through a decision link are refused and audited; a link is never
  sufficient to decide; every opened approval with targets has exactly one delivery per active target
  and one `approval.settled` delivery per request delivery (FR-026-029..034).
- **SC-026-010**: Notification deliveries to Slack, Teams and e-mail stand-ins are inserted with their
  fact, retried with a stable delivery id after a lost acknowledgement, become `DEAD` at the limit and
  are redrivable; messages contain no value above the channel's level; revoking the reveal grant of
  the administrator who set a `pii` ceiling pauses the channel before its next `pii` delivery; draft
  test runs produce no notification (FR-026-030, 034..039).
- **SC-026-011**: Generated OpenAPI documents lint as OpenAPI 3.1, are byte-identical for a version,
  and agree with the server on 100 % of a generated valid/invalid request corpus (FR-026-040, 041).
- **SC-026-012**: Every ceiling of the plan is exercised at its limit and one past it with the stated
  typed refusal (FR-026-047).
- **SC-026-013**: The security battery covers every new operation, including the anonymous ones, with
  zero cross-tenant disclosure, and the authorization matrix of the plan holds for every role
  (NFR-026-001, FR-026-043, FR-026-044).
- **SC-026-014**: Studio journeys for the start form, streams tab, decision page, public page, link
  and channel management pass automated WCAG AA checks and keyboard-only operation (FR-026-049,
  NFR-026-007).
- **SC-026-015**: Slack in-chat decisions with confirmed identities are applied once per interaction
  and refused for unlinked, pending, revoked or ineligible identities, bad signatures, button values of
  another tenant, team or channel, a second tenant binding the same interactive channel, and a code
  used from another workspace, twice, or without the subject's confirmation (FR-026-042).
- **SC-026-016**: The benchmark report states environment, wake-up latency p50/p95/p99, stream
  publish-to-delivery latency and append throughput, with failures disclosed (NFR-026-006).
- **SC-026-017**: With features 021 and 022 absent, no response carries `EARLY_RETURN` and no route
  waits; when they are integrated, their waits use the same waiter, ceilings and disclosure rules,
  proven by their own contract tests (FR-026-011, FR-026-012).
- **SC-026-018**: Telemetry tests find every span, metric and audit action of the plan with closed
  labels only, and every refusal uses a listed reason with no value in the message (FR-026-045,
  FR-026-046, NFR-026-005).
- **SC-026-019**: The previous release's fixture corpus, SDK clients, workers and Studio build pass
  unchanged against the new server, and each down migration refuses while its data exists
  (FR-026-050, NFR-026-004).

## Assumptions

- Feature 014's approval semantics are authoritative and unchanged: one eligible decision, decision-time
  authorization, initiator exclusion, first commit wins, decision key deduplication, raw signals
  never decide. This feature adds ways to reach the decision operation and an optional input.
- Feature 016 classification (`x-sagat-classification`, audiences of ADR-0033, reveal grants) is the
  only disclosure rule for authenticated callers; ADR-0061 adds the anonymous audience.
- The run result (G21, `runs.result_digest`) and its disclosure through `source: result` exist.
- E-mail goes through an SMTP relay the operator configures; deliverability (SPF, DKIM, DMARC) is the
  deployment's responsibility.
- Recipients are channel-level lists (a Slack channel, a Teams channel, an e-mail list); routing a
  notification to each eligible person individually needs a directory that does not exist (G1326).
- Microsoft retired Office 365 Connectors (incoming webhooks) in Teams in May 2026; Teams delivery uses
  Workflows webhooks, which do not support interactive buttons (D1341).
- Single region (ADR-0009); public links and decision pages are served by the control plane. The
  operator configures a public-forms origin distinct from the Studio origin before public links can be
  used (it may share a host with the feature-027 apps origin).
- Each tenant that wants Slack interactivity creates and registers its own Slack app; there is no
  platform-distributed Slack app (G1335).

## Dependencies

- **Existing features**: 001 (identity, contracts), 002 (admission, journal), 004 (inbox, outbound
  pipeline, ADR-0012/0025/0026), 007 (Studio, ADR-0016), 008 (telemetry, alerts, ADR-0017), 009
  (permissions, quotas, egress, ADR-0027), 011 (immutable publication), 014 (approvals, Query/Update,
  ADR-0030/0031/0032), 016 (classification, ADR-0033), 017 (secret authority, managed executors,
  ADR-0035), 006 (SDK conformance, ADR-0007/0015).
- **New features specified in parallel (drafts)**: 018 (reset-rule registry, FR-026-021; the merged
  admission lock order of ADR-0050 decision 7, where the link row is slot 1), 020 (content-store registry; the
  uniform `pii` egress rule of O1081, FR-026-039), 021 (custom HTTP routes: FR-026-012 hook; the
  placement rule of ADR-0055 that channel messages follow; channels do not reuse its resources), 022
  (early return and its `run.result_returned` hint: FR-026-009, 011; no notifications for draft test
  runs), 023 (the `stream.append` script host call, FR-026-020), 025
  (consumes Workflow Streams for token streaming through a reserved `sagat.` channel, and reuses the
  waiter for its MCP call wait; its model-output classification floor governs model-written values
  in receipts, summaries and streams; it aligns its `sagat.agent.tokens` budget to the reserved pool of
  this plan), 027 (may share its apps origin's host with the public-forms origin), 028 (ADR-0063 draft:
  link validity is infrastructure time), 031 (encoded-field registry and codes: FR-026-006), 032 (the
  cell epoch in stream cursors, FR-026-017; decision-link and receipt keys in its key inventory), 029
  (its evaluator rechecks links when enabled, FR-026-024, 031; groups could later be
  approval-notification recipients), 013 (federated login is not required). None is required for P1.

## Out of Scope

- Possession-only decisions (deciding by holding a link, without signing in) — O1320.
- Inbound e-mail replies as decisions; Slack modals and Teams adaptive-card forms for approval input.
- Teams in-chat interactive decisions (needs a registered bot) — G1324.
- CAPTCHA or third-party bot protection on public links — G1320.
- Authenticated embedding of forms in third-party pages — G1321.
- An ephemeral (non-durable) stream mode, a chain-level unified stream offset, per-person notification
  routing, digests and quiet hours, a low-code app builder (feature 027), and client code generation
  from the per-workflow OpenAPI inside the platform.

## Owner questions

See [research.md](./research.md#owner-questions): O1320 (possession-only decision links), O1321
(public links default enablement), O1322 (`pii` in channel messages).

See also the [requirements checklist](./checklists/requirements.md).
