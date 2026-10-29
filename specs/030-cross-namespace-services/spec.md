# Feature Specification: Cross-namespace service operations

**Feature Branch**: `feature/030-cross-namespace-services`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Nexus-like service contracts within one tenant. A handler namespace publishes a named
service whose operations have declared input and output schemas and a mode (`async` or `sync`). An
endpoint registry maps the service to its handler namespace and to the workflow that handles each
operation. A workflow in another namespace of the same tenant calls an operation through a new caller
node kind (additive IR). An asynchronous operation is backed by a handler run whose completion comes
back as a durable callback, deduplicated by operation id. Cancellation propagates, deadlines bound the
call, an allow-list of caller namespaces authorizes it, and calls across tenants are forbidden. The
result crosses the namespace boundary only under a declared disclosure with per-field classification
(ADR-0031, ADR-0033). Traces are linked across namespaces and quotas apply on both sides.

## Context and delivery boundary

A workflow can start another workflow only as a child run **of its own namespace** (`NodeKind::Child`
in `crates/domain/src/workflow.rs`; D36 of feature 002). A team owning `payments-prod` therefore has
no typed, authorized entry point to offer `orders-prod`: sharing workflow names, copying definitions
or wiring webhooks gives no contract, no allow-list, no declared disclosure and no durable
completion. This feature adds that entry point using only existing mechanisms: immutable versions
(011), one-transaction transitions and outbox (ADR-0002), inbox and delivery rows (ADR-0012/0025),
shard fences (ADR-0020), run results (G21), declared disclosure (ADR-0031), classification (ADR-0033),
durable quotas (ADR-0027) and journal-derived telemetry (ADR-0017). The proposed
[ADR-0065](../../docs/adr/ADR-0065-cross-namespace-operations-cross-only-through-durable-operation-messages.md)
records the hard-to-reverse choices. Nothing here is implemented; future evidence is named in
[tasks.md](tasks.md).

**What does not carry over from Temporal Nexus.** The engine interprets an IR; it does not replay
code. A handler is therefore a **workflow of the handler namespace** in both modes, not code on a
task queue. `sync` is a contract class (a short, flat handler with a short deadline), not a blocking
call: no thread waits in either mode. Task-queue routing, attaching several callers to one handler
run, copying callbacks at Continue-As-New, cross-account calls and a circuit breaker are not in the
first increment (D1481, D1493, G1480, G1482 in [research.md](research.md)).

## Domain vocabulary

- **Service**: a named contract owned by one **handler namespace**; its **revisions** are immutable
  documents identified by number and digest.
- **Operation**: an entry of a revision with a **mode** (`async`, `sync`), input and output schemas,
  declared error codes, timeouts and an explicit result disclosure.
- **Service endpoint** ("endpoint" below): a tenant-unique routing entry owned by the handler
  namespace. It exposes one service, serves one to four revisions, binds every operation to a
  **handler workflow**, lists the **caller namespaces** allowed to call it, and has limits and a state
  (`ACTIVE`, `PAUSED`, `DELETED`). Unrelated to the event endpoints of feature 004.
- **Service call**: a `serviceCall` node of a caller workflow and the caller-side record of one call.
- **Operation execution**: the handler-side record of one admitted operation, backed by exactly one
  **handler run**.
- **Operation message**: a durable row carrying a request (`START`, `CANCEL`) to the handler side or
  a notification (`STARTED`, `START_REFUSED`, `COMPLETED`) back; it is the sender's outbox and the
  receiver's inbox, unique per operation and kind.
- **Call depth**: the number of service calls between a run and the first caller.
- **Effect class**: what the caller side knows, when a call settles, about the effect on the handler
  side: `none` (no handler run was admitted), `failed` (the handler run reported failure), `applied`
  (the handler run succeeded), `partial` (the handler run ended `PARTIAL` or `COMPENSATION_FAILED`) or
  `unknown` (a handler run was admitted and its outcome was not known when the call settled).
  `applied`, `partial` and `unknown` are **effect-possible**.

## User Scenarios & Testing *(mandatory)*

### US01 — Publish a service contract and register an endpoint (Priority: P1)

The handler team publishes a contract, registers an endpoint that binds each operation to a handler
workflow, and names the namespaces allowed to call it.

**Why this priority**: nothing can be called without them, and an unsafe contract must fail at
publication, not at the first call.

**Independent Test**: publish a contract with one async and one sync operation, register an endpoint,
then submit one request per defect class and check each refusal and that nothing changed.

**Acceptance Scenarios**:

1. **Given** workflow `charge-card` in `payments-prod`, **when** a developer publishes service
   `payments` revision 1 with async operation `charge` whose schemas equal the handler version's and
   which declares `disclosure: declared-service-result-v1`, **then** the revision is immutable with a
   digest, and publishing the same document again returns revision 1 unchanged.
2. **Given** that revision, **when** an administrator registers endpoint `payments` serving revision 1,
   binding `charge` to `charge-card` and allowing `orders-prod`, **then** the endpoint is `ACTIVE` and
   the tenant registry lists it.
3. **Given** an output schema that is open, unbounded, marks a field `secret`, omits the disclosure,
   declares a reserved error code or exceeds a ceiling, **when** published, **then** it is refused with
   `VALIDATION_FAILED`, a closed reason and the offending JSON pointer, and no revision exists.
4. **Given** a binding whose handler schemas differ from the operation's, a `sync` operation bound to a
   waiting or floating handler version, or a handler that uses `continueAsNew`, **when** the endpoint is
   saved, **then** it is refused naming the binding, and the endpoint keeps its previous revision.
5. **Given** an allow-list entry naming another tenant's namespace, or a name already used in the
   tenant, **when** saved, **then** the foreign namespace is an opaque `NOT_FOUND` (audited) and the
   duplicate name is `CONFLICT`.

### US02 — Call an asynchronous operation from another namespace (Priority: P1)

An author in `orders-prod` adds a `serviceCall` node; a run calls the operation, a handler run executes
in `payments-prod`, and its result becomes the node's output.

**Why this priority**: it is the purpose of the feature; every other story refines it.

**Independent Test**: publish `place-order` with `serviceCall charge` followed by an activity reading
`steps.charge.output.receiptId`; start it; observe one handler run, the caller step succeeding with the
declared result, the activity receiving it, and both journals linked.

**Acceptance Scenarios**:

1. **Given** the published caller, **when** a run reaches `charge`, **then** the step waits, the call
   is recorded with its deadline and a start request is pending, all committed together; no handler
   run exists yet.
2. **Given** the pending start, **when** the engine admits it, **then** a handler run of `charge-card`
   starts with the operation input, and the start is consumed, in one commit.
3. **Given** the handler run succeeds with a result valid for the operation output schema, **when** its
   completion is applied, **then** the caller step succeeds with that result as `steps.charge.output`
   and the next node is scheduled.
4. **Given** the handler run fails with `CARD_DECLINED`, which the operation declares, **then** the
   caller step fails with `CARD_DECLINED` and `onError` applies; an undeclared code arrives only as
   `OPERATION_FAILED`.
5. **Given** a handler result that does not validate against the output schema, **then** no value
   crosses and the handler run stays `SUCCEEDED`; the caller step fails `OPERATION_RESULT_INVALID` with
   effect class `applied`, so it is compensated inside a saga and waits in manual review outside one,
   never following `onError` as if nothing had happened.

### US03 — Authorize calls and keep the tenant boundary (Priority: P1)

Only allow-listed namespaces of the same tenant can call, only a publisher with `service:call` can
publish a caller, and the result reaches the caller namespace only as declared and classified.

**Why this priority**: a confused deputy across namespaces (feature 009 edge case) or a cross-tenant
call is release-blocking (constitution V).

**Independent Test**: the authorization battery: non-allow-listed caller, allow-list removal after
publication and during a call, another tenant's endpoint, downgraded publisher, classified and secret
canaries.

**Acceptance Scenarios**:

1. **Given** a caller namespace absent from the allow-list, **when** a caller is published, **then** it is
   refused (`FORBIDDEN`, reason `SERVICE_CALLER_NOT_ALLOWED`); **when** the namespace is removed after
   publication and a run calls, **then** the start is refused and the step fails
   `OPERATION_NOT_ALLOWED` with no handler run.
2. **Given** a caller in tenant A naming an endpoint that exists only in tenant B, **when** validated,
   **then** the endpoint is not found, indistinguishable from a missing one, and the probe is audited.
3. **Given** an operation in flight, **when** the caller namespace is removed from the allow-list or the
   endpoint is deleted before the handler run ends, **then** the step fails `OPERATION_ACCESS_REVOKED`,
   no value crosses, the handler's effect class is still recorded, and an effect-possible outcome goes
   to compensation or manual review; pausing the endpoint does not affect it (default of O1481).
4. **Given** a result schema `public` at its root that marks `cardLast4` `internal` and `holder` `pii`,
   **when** the caller run's payload is disclosed (feature 016), **then** `status` is visible to readers
   of the run, `cardLast4` to holders of `payload:read-internal`, and `holder` only on an audited `pii`
   reveal; no value appears in the journal listing, graph, stream, telemetry, logs or audit of either
   namespace.
5. **Given** the caller version's publisher lost `service:call`, **when** a later run reaches the node,
   **then** the step fails `OPERATION_NOT_AUTHORIZED` and nothing is sent.

### US04 — Survive crashes, duplicates, lost hints and races (Priority: P1)

**Why this priority**: constitution I and VII and AGENTS.md rule 9; a lost or doubled transition
across two namespaces cannot be repaired afterwards.

**Independent Test**: crash before and after each of the eight runtime commit points of the plan,
deliver every message twice and in every order, drop every broker hint, let a `STARTED` go dead, race
completion against timeout and cancellation against admission 100 times each.

**Acceptance Scenarios**:

1. **Given** a crash at any commit point, **when** the engine restarts, **then** each operation has at
   most one handler run and exactly one applied caller outcome, and no acknowledged transition is lost.
2. **Given** one start claimed by two engines, or a completion delivered twice, **then** the second
   changes nothing.
3. **Given** every broker hint is lost, **then** every call still finishes through polling.
4. **Given** a completion and the caller's deadline at the same instant, **then** the first commit wins
   and the other is a recorded, audited no-op.
5. **Given** a cancellation while the start is being admitted, **then** either no handler run exists or
   the handler run exists and the cancellation policy applies to it; never both, never neither.
6. **Given** a fast handler whose `COMPLETED` is applied before its `STARTED` (or whose `STARTED` went
   dead), **then** the call settles from its scheduled state with the handler's outcome and the later
   or redriven `STARTED` is a no-op; the caller never times out a call whose handler finished in time.

### US05 — Cancel calls and bound them with deadlines (Priority: P2)

**Why this priority**: FR-003-006 asks that cancellation and deadlines propagate to sub-workflows; an
unbounded or unstoppable call across namespaces breaks it.

**Independent Test**: cancel a caller run under each policy; exceed the timeout and the start timeout;
cancel the handler run from its own namespace.

**Acceptance Scenarios**:

1. **Given** `cancelPolicy: abandon`, **when** the caller run is cancelled, **then** the step is cancelled
   at once with effect class `unknown` (or `none` if the start was not yet admitted), nothing reaches the
   handler run, and the outcome of a later completion is kept on the call for lookup.
2. **Given** `request_cancel` (default), **then** the step is cancelled at once and the handler run is
   asked to stop.
3. **Given** `wait_cancelled`, **then** the handler run is asked to stop and the step stays open until its
   outcome arrives, recorded as it is.
4. **Given** a call not started by `startTimeoutMs`, **then** the start is withdrawn and the step fails
   `OPERATION_START_TIMED_OUT` with effect class `none`. **Given** a started call not finished by
   `timeoutMs`, **then** the handler run is asked to stop and the platform looks the outcome up for at
   most the outcome lookup window: the step fails `OPERATION_TIMED_OUT` with the handler's effect class,
   or `unknown` when the window passes, and an effect-possible failure goes to compensation or manual
   review (constitution III).
5. **Given** an operator of `payments-prod` cancels the handler run, **then** the step fails
   `OPERATION_CANCELLED` and `onError` applies.

### US06 — Bound both sides with quotas and backpressure (Priority: P2)

**Why this priority**: ADR-0027 requires finite admission; calls across namespaces would otherwise
escape per-namespace limits and the 1,024-run tree budget.

**Independent Test**: exhaust each ceiling on each side; check refusal before work, deferral without
consumed attempts and call depth across namespaces and child runs.

**Acceptance Scenarios**:

1. **Given** the caller namespace at its pending-call quota or the run at its active-call limit, **when**
   a node is reached, **then** the step fails `OPERATION_QUOTA_EXCEEDED` and nothing is sent.
2. **Given** the handler namespace at its active-run or active-operation quota, the endpoint at its
   concurrency or start rate, or the endpoint `PAUSED`, **then** the start is deferred with a closed
   reason and retried with bounded backoff until room appears or the start timeout passes.
3. **Given** a chain of calls (A → B → C, with child runs between), **when** the depth would exceed the
   ceiling, **then** the start is refused with `OPERATION_DEPTH_EXCEEDED`.

### US07 — Offer short synchronous operations (Priority: P2)

**Why this priority**: low-latency lookups are a common Nexus use; the mode reuses the async path.

**Independent Test**: bind a flat pinned handler to a sync operation, call it from all five SDKs, then
make the handler exceed the deadline.

**Acceptance Scenarios**:

1. **Given** a sync operation bound to a pinned flat handler version, **then** the caller receives the
   result as in US02, with a default timeout of 10 s.
2. **Given** the handler exceeds the timeout, **then** the handler run is asked to stop and, after the
   outcome lookup, the step fails `OPERATION_TIMED_OUT` with the handler's effect class (`unknown` if
   none arrived in the window).
3. **Given** a caller node with `wait_cancelled` or a timeout over 60 s for a sync operation, **when**
   published, **then** it is refused.

### US08 — Observe and trace a call across namespaces (Priority: P2)

**Why this priority**: constitution VI; a failure across namespaces must be explainable from durable
evidence on both sides.

**Independent Test**: run US02 and a failing call; check one trace with links, closed-label metrics,
audit on both sides and the Studio links, with an unauthorized viewer seeing metadata only.

**Acceptance Scenarios**:

1. **Given** a completed call, **then** the handler run's spans are in the caller's trace under the
   service-call span, and both spans link to each other.
2. **Given** the caller run in the Studio, **when** the `serviceCall` node is opened, **then** it shows
   endpoint, operation, mode, state, deadlines, outcome code and a link to the handler run, which shows
   "called by" with a link back; following a link needs the viewer's own access to the other namespace.
3. **Given** a namespace, **when** its operations are listed by direction and state, **then** the list is
   payload-free and paginated, and dead messages offer a redrive to holders of `run:operate`.

### US09 — Compensate a service call inside a saga (Priority: P3)

**Why this priority**: valuable for business transactions across namespaces, but it needs a new
derived step kind and comes after the forward path is proven.

**Independent Test**: a saga with `charge` compensated by `refund` and a failing later step; then make
`refund` fail.

**Acceptance Scenarios**:

1. **Given** a completed `charge` in a saga that aborts, **then** `refund` is called as a derived
   service call with an input over the forward call's input and result.
2. **Given** `refund` fails permanently, **then** the run reaches the existing `MANUAL_REVIEW` or
   `COMPENSATION_FAILED` outcome; the compensation is never assumed done.
3. **Given** a `serviceCall` in a saga with neither `compensateWith` nor `irreversible: true`, or with a
   policy other than `wait_cancelled`, **when** published, **then** it is refused.
4. **Given** `charge` failed in a saga with an effect-possible class (`OPERATION_TIMED_OUT` with
   `unknown`, `OPERATION_ACCESS_REVOKED` or `OPERATION_RESULT_INVALID` with `applied`), **when** the saga
   compensates, **then** `refund` runs for it, as a compensation must tolerate an effect that did not
   happen; an `irreversible` call ends the saga `PARTIAL`.

### Edge Cases

- Caller paused during a call: the completion settles the step; nothing new is scheduled until resume.
- Caller at `continueAsNew` with a call in flight: refused by quiescence (ADR-0030 §16).
- Caller ended (abandon, request_cancel) or purged before the completion: the completion is discarded
  and audited, and its outcome (code and effect class, no value) is written once to the call record for
  lookup when the record still exists.
- Handler workflow's active version changes to other schemas under an `active` binding: new starts fail
  `OPERATION_HANDLER_UNAVAILABLE`; the endpoint shows the binding as incompatible; calls in flight are
  unaffected.
- Endpoint stops serving the pinned contract revision: new starts fail `OPERATION_CONTRACT_UNAVAILABLE`.
- Endpoint deleted and recreated under the same name: pinned callers fail
  `OPERATION_ENDPOINT_UNAVAILABLE`; nothing is retargeted.
- Input or result over 256 KiB or a blob descriptor: refused; blobs do not cross namespaces (G1495).
- A notification that keeps failing becomes `DEAD`, alerts the receiving namespace and is redrivable;
  it is never dropped.
- A handler run that is rerun or reset produces an unrelated run (ADR-0039 provenance only).
- A call into the caller's own namespace: allowed only if that namespace is allow-listed.
- An encrypted or compressed field (feature 031): never crosses namespaces in v1. A caller whose input
  maps an encoded location is refused at publication, and an envelope found at admission or in a
  handler result is refused at runtime (FR-030-039, 031 D1667).
- A reset (feature 018) of a caller whose prefix contains an open call is refused; finished calls are
  copied as provenance (FR-030-038).
- A draft test run (feature 022) must mock or pin a `serviceCall` node, or it is refused.

## Requirements *(mandatory)*

### Functional Requirements

**Contracts and the registry**

- **FR-030-001 — Contract document**: The platform MUST accept a `kind: Service` document under
  `apiVersion: sagat-octopus-orchestrator.io/v1` with a name and 1–32 operations, each with a name, a
  mode (`async`, `sync`), `inputSchema`, `outputSchema`, 0–16 declared `errors`, `timeouts`
  (`maxMs`; optional `defaultMs`, 10 s for sync and 24 h for async when absent) and
  `disclosure: "declared-service-result-v1"`. Unknown fields are refused.
- **FR-030-002 — Contract validation**: Publication MUST refuse, with `VALIDATION_FAILED`, a closed
  reason and a JSON pointer: an output schema that is not closed and bounded (ADR-0031 §4); a `secret`
  level in an output schema; any classification defect of FR-016-004; a declared code with a reserved
  prefix (`OPERATION_`, `CHILD_`) or equal to a platform code; a sync `maxMs` over 60 s or an async one
  over 60 days; a schema over 64 KiB.
- **FR-030-003 — Immutable revisions**: Publishing MUST create the service on first use and an
  immutable revision with a canonical digest; a document equal to an existing revision returns it.
  Referenced revisions are never deleted; deleting a service is a tombstone.
- **FR-030-004 — Endpoint registry**: An endpoint MUST have a tenant-unique name, belong to its handler
  namespace, expose one service, serve 1–4 of its revisions, bind every operation of each, and carry
  limits (`maxConcurrent`, `startsPerSecond`) and a state. Every change is a new immutable endpoint
  revision under optimistic concurrency; delete is a tombstone. Holders of `service:read` in the tenant
  can list the registry with served revisions, state and binding health.
- **FR-030-005 — Handler bindings**: A binding MUST name a workflow of the handler namespace and a policy,
  `pinned` (a version) or `active`; a sync binding MUST be pinned. At save and at every start the
  resolved version's input and output schemas MUST equal the operation's canonically (classification
  included) and the version MUST NOT use `continueAsNew`; a sync handler MUST NOT contain `timer`,
  `waitEvent`, `approval`, `child`, `serviceCall`, `forEach`, `while` or `repeatUntil`.
- **FR-030-006 — Allow-list**: An endpoint MUST carry an explicit list of caller namespaces of its
  tenant; an empty list admits nobody; there is no wildcard; another tenant's namespace is
  `NOT_FOUND`. Every change is audited.

**The caller**

- **FR-030-007 — `serviceCall` node**: The IR MUST gain, additively (ADR-0032), `type: serviceCall`
  with `endpoint`, `service`, `operation`, a required `input` (an expression producing an object),
  and optional `timeoutMs` (the operation's `defaultMs` when absent), `startTimeoutMs`,
  `cancelPolicy` (`abandon`, `request_cancel` default, `wait_cancelled`), `contractRevision`, `next`
  and `onError`, and the required `inputDisclosure: "declared-service-input-v1"`, by which the caller
  publisher declares that the input values leave the caller namespace. Its output schema is the pinned
  operation's and cannot be authored. It is gated by a server capability (`UNSUPPORTED_NODE` when
  disabled), allowed only in the root graph in the first increment, at most 64 per definition.
- **FR-030-008 — Caller publication**: Publishing MUST resolve the endpoint by name in the publisher's
  tenant, check service, operation and contract revision (given, or the newest served), the caller
  namespace's allow-listing and the publisher's `service:call`, the input classification and encoding
  rules (FR-030-021, FR-030-039), and pin endpoint identity and contract revision per node, immutably
  with the version. Promotion (011) resolves names again in the destination.
- **FR-030-009 — Scheduling**: When a `serviceCall` activates, the platform MUST evaluate the input and
  validate it against the pinned schema, check
  caller-side quotas, call depth and the publisher's current
  `service:call`, and commit together the waiting step with its deadlines, the call record, the start
  request, the journal event, the audit record and the hint. A refusal fails the step with a typed code
  and sends nothing.
- **FR-030-010 — Identity**: The operation id MUST be derived deterministically from tenant, caller run,
  node instance and call attempt (1, incremented only by an operator retry of the step), and the
  handler run id from the operation id; at most one handler run exists per operation, enforced by the
  database.

**The handler side**

- **FR-030-011 — Admission**: Admitting a start MUST, in one commit, check that the endpoint exists,
  the caller namespace is allow-listed, the pinned revision is served, the binding resolves to a
  compatible version, that the input holds no envelope (031, FR-030-039), and
  depth and handler-side limits allow it, then start the handler run through the
  existing admission semantics (input = operation input, deadline = operation deadline, the handler
  version publisher's authority, actor `system:service-operation`), record the execution and send
  `STARTED`.
- **FR-030-012 — Refusal and deferral**: A non-retryable refusal (endpoint deleted, caller not allowed,
  revision not served, handler unavailable, depth exceeded) MUST end the start and send
  `START_REFUSED` with its code, audited. A refusal for room (endpoint paused, handler-namespace or
  endpoint limits, start rate) MUST defer the start with a closed reason and bounded backoff, consuming
  no attempts, until room appears or the caller's start deadline withdraws it.
- **FR-030-013 — Completion**: The handler run's terminal commit MUST also record the operation outcome
  and send `COMPLETED` with the effect class: a result (G21) that validates against the output schema
  and holds no envelope (FR-030-039) by digest on success; a failure code verbatim only if declared. If the
  endpoint was deleted or the caller namespace removed from the allow-list, it sends
  `OPERATION_ACCESS_REVOKED`, the effect class and no result.
- **FR-030-014 — Completion apply**: Each notification MUST be applied at most once per operation and
  kind, in any arrival order: the call record, the journal event, the step outcome (output charged to
  the caller run's retained data, `RUN_DATA_LIMIT` otherwise), the next work and the hint commit
  together. A `COMPLETED` MUST be applicable while the call is still `SCHEDULED` (the handler run id is
  derived), and a `STARTED` that arrives later, or is redriven, is a no-op. A `COMPLETED` or
  `START_REFUSED` for a settled or purged call is discarded and audited; the outcome of a discarded
  `COMPLETED` (code and effect class) is written once to the call record for lookup.
- **FR-030-015 — Outcomes**: The caller step MUST succeed only on a handler `SUCCEEDED` with a valid
  result, and otherwise fail with a code of [contracts/errors.md](contracts/errors.md): the declared
  code, `OPERATION_FAILED`, `OPERATION_CANCELLED`, `OPERATION_TIMED_OUT`, `OPERATION_COMPENSATED`,
  `OPERATION_PARTIAL`, `OPERATION_COMPENSATION_FAILED`, `OPERATION_RESULT_INVALID`,
  `OPERATION_ACCESS_REVOKED`, or a refusal or caller-side code; each settlement carries its effect
  class (FR-030-037).

**Deadlines and cancellation**

- **FR-030-016 — Deadlines**: The effective timeout MUST be the minimum of `timeoutMs`, the operation's
  `maxMs`, the caller run's remaining deadline and the deployment ceiling; the start timeout is at most
  the timeout; the handler run's deadline is the `deadline_at` carried by the `START`, that is the
  caller's effective deadline. Deadlines are workflow time in 028's time-class catalog; first commit
  wins between a deadline and a notification. When the deadline passes after admission, the caller
  sends `CANCEL` and waits at most the outcome lookup window (default 60 s, hard 15 min) for the
  handler outcome before settling with `unknown`.
- **FR-030-017 — Cancellation**: Cancelling the caller step MUST withdraw a start that is not yet
  admitted, whatever the policy (effect class `none`). Otherwise `abandon` settles the step and sends
  nothing; `request_cancel` settles it and sends `CANCEL` (both with effect class `unknown`);
  `wait_cancelled` sends `CANCEL` and waits for the outcome. A `CANCEL` stops the handler run; for a
  terminal or purged handler run it is a no-op.
- **FR-030-018 — Handler-side actions**: Cancel, pause and resume of a handler run MUST keep their
  meaning, its outcome mapping by FR-030-015; a rerun or reset of a handler run carries no operation.

**Sync operations**

- **FR-030-019 — Sync mode**: A sync operation MUST run through the async machinery with a default
  timeout of 10 s, a maximum of 60 s, a pinned flat handler and the fixed `request_cancel` policy. No
  latency is promised (NFR-030-005).

**Disclosure, classification and payloads**

- **FR-030-020 — Declared disclosure**: A result MUST leave the handler namespace only for an operation
  that declares `disclosure: "declared-service-result-v1"`, only as the validated value of its closed
  output schema, and only to an allow-listed caller namespace. Nothing is inferred from a schema.
- **FR-030-021 — Classification in both directions**: The operation output schema MUST be a
  classification carrier for the caller (see "Amendment to feature 016" below), pinned with the caller
  version, that the caller cannot loosen. The input crosses only under the node's `inputDisclosure`,
  and, when `input` is a direct path into a carrier of the caller (`input…`, `steps.<node>.output…`;
  any other source counts as `pii`), publication MUST refuse an operation input field whose effective
  level is looser than the level of the source field it receives (`VALIDATION_FAILED`, reason
  `SERVICE_CALL`, issue `INPUT_CLASSIFICATION_LOOSENED`). A computed input expression is not compared
  (no taint tracking, D283 of 016); `inputDisclosure` is then the only control (G1496). The handler
  run's input is classified by the handler's input schema, which equals the operation's. Until 016
  accepts the extension, results are `structure-only` for disclosure (G1493) and still usable by
  expressions.
- **FR-030-022 — Payload-free projections**: Journal listings, graphs, streams, telemetry, logs,
  metrics, audit, operation listings and error bodies of both namespaces MUST carry no input, result,
  schema fragment or caller text.
- **FR-030-023 — Inline payloads**: Inputs and results MUST be inline and at most 256 KiB; blob
  descriptors are refused. Encoded fields follow FR-030-039.

**Authorization, tenant boundary, quotas**

- **FR-030-024 — Permissions**: The catalog MUST gain `service:read` (admin, developer, operator,
  viewer), `service:publish` (admin, developer), `service:endpoint:manage` (admin) and `service:call`
  (admin, developer); the seven built-in roles stay and these are additive grants to them. Listing a
  namespace's operations needs `run:read` there; redriving a dead message needs `run:operate` in the
  receiving namespace. When feature 029 is enabled, its evaluator decides the effective permissions
  and the visibility of services, endpoints and operation listings (custom roles may hold the four
  permissions; folders may hold services and endpoints, G1488).
- **FR-030-025 — Tenant boundary**: No endpoint, allow-list entry, pin, call record, execution record or
  message MUST be able to reference two tenants; a foreign or missing resource is the same opaque
  `NOT_FOUND`; a cross-tenant attempt is audited.
- **FR-030-026 — Quotas on both sides**: The platform MUST bound pending calls per caller namespace and
  active calls per run; active operations per handler namespace, concurrency and starts per second per
  endpoint (handler runs also count against the existing active-run quotas); and services, revisions,
  endpoints, served revisions and allow-list entries. Effective limits are the minimum of request,
  namespace, tenant and deployment values; zero refuses; admission fails before work.
- **FR-030-027 — Call depth**: Every run MUST carry a call depth (0 for ordinary runs), a handler run the
  caller's plus one and a child run its parent's; a start beyond the ceiling (default 4, hard 8) is
  refused with `OPERATION_DEPTH_EXCEEDED`.

**Delivery, recovery, retention, observability**

- **FR-030-028 — Delivery**: Pending messages MUST be found by polling, broker hints only shortening the
  wait. A notification or `CANCEL` that keeps failing becomes `DEAD` after 12 attempts, raises an alert
  in the receiving namespace and can be redriven; nothing is deleted unprocessed.
- **FR-030-029 — Retention**: Call and execution records MUST live as long as their runs; settled
  messages are purged after 7 days (up to 30 by setting); services and endpoints leave tombstones and
  keep referenced revisions. Operation messages and call records are registered as a content store in
  020's registry with their purge and erasure rules (data-model.md).
- **FR-030-030 — Telemetry**: The handler run MUST continue the caller's trace under the service-call
  span, both spans linking to each other; spans derive from the journal (ADR-0017); metric labels come
  only from the closed sets of the plan; logs are redacted.
- **FR-030-031 — Audit**: Contract publication and deletion, endpoint and allow-list changes, pause,
  resume, scheduling, admission, refusal, cancellation requests, completion (outcome and whether a
  result was disclosed), discarded completions, dead messages, redrives and every denial MUST be audited
  in the transaction of the decision, with ids, names, codes and trace id only.

**Surfaces and compatibility**

- **FR-030-032 — REST API**: Additive `/v1` routes MUST manage services, revisions, endpoints and
  allow-lists, list the registry, list and read operations by direction and state, and redrive dead
  messages; writes take an `Idempotency-Key` (same key and body replays; a changed body conflicts).
- **FR-030-033 — Studio**: The Studio MUST offer registry, contract, endpoint and operation screens, a
  `serviceCall` inspector that picks endpoint and operation from the registry, and a runtime node with
  links to the other side; it follows the journal sequence (ADR-0016), uses design tokens, meets WCAG AA
  with table alternatives, draws no sample values (FR-007-010) and shows unknown nodes as unsupported
  (FR-014-017).
- **FR-030-034 — SDKs and CLI**: The five SDK builders MUST emit byte-identical `serviceCall` nodes and
  `Service` documents for the same program, their clients and the CLI MUST cover publication, endpoint
  management and operation reads, and one shared conformance suite MUST cover builders, outcomes,
  cancellation, sync, security and faults.
- **FR-030-035 — Compatibility**: IR, OpenAPI, AsyncAPI and the catalog MUST change additively;
  existing versions, runs and clients keep their behaviour; the worker protocol is unchanged.
  Disabling the capability stops new publications and calls, while calls in flight still finish.
- **FR-030-036 — Saga compensation (P3)**: A `serviceCall` in a saga MUST declare `irreversible: true`
  or `compensateWith` naming an operation of the same pinned contract, and use `wait_cancelled`; the
  compensation is a derived call under ADR-0010 with the existing retry, review and failure semantics.

**Effects, resets and encoded payloads**

- **FR-030-037 — Effect classes and unknown outcomes**: Every settlement MUST record its effect class
  (vocabulary). A step that fails with an effect-possible class (`applied`, `partial`, `unknown`) MUST
  NOT be treated as "not done": inside a saga it is planned for its `compensateWith` (which tolerates an
  effect that did not happen) or, if `irreversible`, ends the saga `PARTIAL`; outside a saga it enters
  the existing manual review (`step.needs_review`) instead of following `onError`, and is settled by the
  existing operator actions (`RESOLVE_STEP` with `saga:override`, or `RETRY_STEP`, which starts a new
  call attempt with a new operation id). A failure with class `none` or `failed` follows `onError`. A
  cancelled caller run records the class and keeps the later outcome for lookup.
- **FR-030-038 — Reset rules**: Every journal event type this feature adds MUST declare its rule in
  018's reset-rule registry: `service_call.*` events of a call whose `service_call.finished` is in the
  prefix are `COPY` (the operation id kept as provenance; the reset run creates no call record, message
  or handler run); any `service_call.*` event of a call still open at the reset point is `REFUSE`;
  `service_operation.cancel_requested` and `service_operation.completed` are `EXCLUDE`; the `cause`
  field of a handler run's `run.started` is copied as provenance and the reset run backs no operation.
- **FR-030-039 — No encoded values across namespaces (031 D1667)**: Encoded values MUST NOT cross the
  boundary in v1, because a 031 envelope binds its tenant, namespace and location (031 D1556) and no
  platform component may re-encode. Therefore: a `Service` document's `inputSchema` and `outputSchema`
  refuse `x-sagat-encoding` (`ENCODING_NOT_APPLICABLE`); an endpoint cannot bind a handler version whose
  input or output schema declares encoding (`ENCODING_NOT_APPLICABLE`); caller publication refuses a
  `serviceCall.input` or `compensateWith.input` whose source holds an encoded location
  (`ENCODING_CARRIAGE_MISMATCH`); at runtime an envelope found in the input at admission refuses the
  start, and one found in a handler result withholds the value, each with 031's
  `ENCODING_CARRIAGE_MISMATCH` or `ENVELOPE_CONTEXT_MISMATCH` (G1497, 031 G1536).

### Amendments and registrations in other features

- **Amendment to feature 016 (FR-016-002):** the closed carrier list gains the `serviceCall` node,
  whose carrier schema is the pinned operation's `outputSchema` (never authored on the node); the
  `inputSchema` and `outputSchema` of a `Service` document accept `x-sagat-classification` under
  FR-016-003/004. This joins the single amendment of FR-016-002 and classification rules version 2
  consolidated in `SPECIFICATION-INDEX.md`, opened by the first of the drafts to be implemented
  (T030-069).
- **031 encoded-field registry:** `serviceCall.input` and `compensateWith.input` (refused when the source
  holds an encoded location), the operation input and output schemas (`ENCODING_NOT_APPLICABLE`) and
  handler admission and result return (runtime envelope check) are registered there (FR-030-039).
- **018 reset-rule registry** and **ADR-0050 admission lock order:** FR-030-038; the plan cites the
  slot of the service-endpoint heads.
- **020 content-store registry:** operation messages and call records (FR-030-029).
- **028 time-class catalog:** the call deadlines are workflow time; delivery backoff and purge times are
  infrastructure time (data-model.md).
- **029 evaluator:** FR-030-024. **022 draft test runs:** a `serviceCall` must be pinned or mocked, or
  the test run is refused (`TEST_RUN_UNMOCKED_EFFECT`, 022's code).

### Non-functional requirements

- **NFR-030-001 (durability)**: No committed scheduling, admission, completion or cancellation is lost or
  applied twice under crash, duplicate or lost hint. One handler run per operation is a platform
  guarantee; effects inside it stay at-least-once (ADR-0004).
- **NFR-030-002 (security)**: Zero cross-tenant references, zero calls from non-allow-listed namespaces
  and zero canary values in any projection of either namespace; authorization fails closed.
- **NFR-030-003 (compatibility)**: Existing workflows, runs, SDK clients and worker protocol minors keep
  their semantics; an older Studio shows the node as unsupported.
- **NFR-030-004 (observability)**: Every call is explainable from durable evidence on both sides
  without telemetry; telemetry failures never affect commits.
- **NFR-030-005 (performance)**: Measure on a declared environment p50/p95/p99 of scheduled→started and
  scheduled→finished for trivial sync and async operations, admission throughput per endpoint, the
  duration of each commit point and the write cost per call. Nothing is promised.
- **NFR-030-006 (resource bounds)**: Every resource has a default and a hard ceiling (plan); no path
  allows unbounded fan-out across namespaces.

### Key Entities

- **Service / service revision**: owned by a handler namespace; immutable contract documents.
- **Service endpoint / endpoint revision / allow-list entry**: tenant-unique routing with served
  revisions, bindings, limits, state and the allowed caller namespaces of the same tenant.
- **Caller pin**: per caller version and node, the endpoint identity and contract revision.
- **Service call**: the caller-side record of one call: state, deadlines, policy, outcome, result digest,
  handler run link.
- **Operation execution**: the handler-side record: handler run, caller link, cancellation, completion.
- **Operation message**: `START`, `CANCEL`, `STARTED`, `START_REFUSED`, `COMPLETED`, unique per operation
  and kind, with delivery state and attempts.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All evidence below is future evidence, not completed tests.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-030-001 | Every contract and endpoint defect class (at least 14) is refused with its reason and pointer; an identical document returns the same revision; endpoint revision conflicts are detected | FR-030-001..006 |
| SC-030-002 | The same async call succeeds and fails with a declared code identically in all five SDKs on the real engine, with equal normalized journals of both runs | FR-030-007..015, 034 |
| SC-030-003 | A crash before and after each of the eight runtime commit points, a `COMPLETED` applied before its `STARTED` and a dead `STARTED` leave at most one handler run and exactly one applied caller outcome per operation, in 100% of injected cases, with no timeout of a call whose handler finished in time | FR-030-009..014, 028 |
| SC-030-004 | Each message kind delivered twice and every hint dropped: no second effect, and every call finishes within the polling bound; settled messages are purged after their retention while pending and dead ones never are | FR-030-010, 014, 028, 029 |
| SC-030-005 | 100 rounds each of completion-vs-timeout and cancellation-vs-admission end with exactly one winner and an audited no-op loser | FR-030-014, 016, 017 |
| SC-030-006 | The isolation battery yields zero unauthorized handler runs and zero disclosed results | FR-030-006, 008, 011, 013, 024, 025 |
| SC-030-007 | Secret, unmarked, undeclared-field and undeclared-code canaries appear in no projection, log, span, metric or audit of either namespace; classified fields reach only their audience; an input field looser than its caller source is refused at publication | FR-030-020..023, 030 |
| SC-030-008 | Each quota and ceiling of both sides refuses or defers before work with its reason; deferrals consume no attempts; depth holds across namespaces and child runs | FR-030-012, 026, 027 |
| SC-030-009 | The three cancellation policies, both deadlines and a handler-side cancel behave as specified in all five SDKs | FR-030-016..018 |
| SC-030-010 | A flat pinned sync handler is accepted and a waiting or floating one refused; a late sync call fails `OPERATION_TIMED_OUT` and stops its handler run | FR-030-005, 019 |
| SC-030-011 | One exported trace spans both namespaces with reciprocal links; every metric label is in its closed set; every decision of FR-030-031 has its audit record; the Studio shows both links and the lag | FR-030-030..033 |
| SC-030-012 | Existing workflows, runs, SDK clients and an older Studio pass unchanged; with the capability disabled publication fails `UNSUPPORTED_NODE` and calls in flight finish | FR-030-035 |
| SC-030-013 | A saga compensates through a derived call; a failing compensation reaches manual review | FR-030-036 |
| SC-030-014 | A benchmark report states environment, configuration, p50/p95/p99 and failures for NFR-030-005; no figure is published without it | NFR-030-005 |
| SC-030-016 | For every effect-possible outcome (timeout after admission inside and outside the lookup window, revocation, invalid result, abandon, handler `PARTIAL`) the step is compensated in a saga or enters manual review outside one; never `onError`; the late outcome is visible for lookup | FR-030-015..017, 037 |
| SC-030-017 | A reset with a finished call in its prefix replays equal to its projection and creates no call record, message or handler run; a reset point with an open call is refused | FR-030-038 |
| SC-030-018 | In all five SDKs: an encoding annotation in a `Service` document, a binding to a handler that declares encoding and a `serviceCall` whose input maps an encoded location are refused with 031's codes; an envelope smuggled at an undeclared location is refused at admission and withheld from a result at runtime; no encoded value is ever stored in the other namespace | FR-030-039 |
| SC-030-015 | An engine restart and a shard takeover during 1,000 concurrent calls finish all of them without operator action or fence violation | FR-030-011, 014, 028 |

## Assumptions

- Calls stay inside one tenant and one region; cross-tenant calls are forbidden by design, calls
  between regions wait for feature 032 (G1484).
- Roles are tenant-wide today (one membership per subject and tenant), so the namespace allow-list is
  the boundary between environments; feature 029 may add finer grants.
- The handler's answer is its run result (G21), returned through `end.outputExpr`.
- First-increment restrictions (root-path calls, no handler chains) are explicit capability errors.
- Numbers here are ceilings to enforce, not measured capacity.

## Dependencies

- **Existing**: 001 identity and contracts; 002 runs, child protocol D36, run result G21; 003 sagas
  (ADR-0010); 004 inbox and deliveries (ADR-0012/0025); 006 SDK conformance; 007 Studio (ADR-0016);
  008 telemetry and alerts (ADR-0017); 009 RBAC, quotas, audit; 010 shards (ADR-0020); 011 versions and
  promotion; 014 additive IR, declared disclosure, chains (ADR-0030/0031/0032/0037); 016
  classification (ADR-0033; the carrier extension needs its owner); 017 pinning pattern.
- **Parallel drafts, by scope only**: 018 business workflow ids (attaching callers, G1482), reset-rule
  registry and ADR-0050 lock order; 020 search attributes and content-store registry; 022 standalone
  steps (more sync handler kinds, O1480) and draft-test-run mocks; 028 time-skipping tests and
  time-class catalog; 029 custom roles, folders and evaluator (G1488); 031 encoded-field registry,
  codes and D1667 (G1497); 032 multi-region placement (G1484).

## Out of scope

Calls across tenants or regions; callers outside workflows; handler kinds other than workflows;
attaching several callers to one handler run; handler chains; calls inside structured bodies; blobs
across namespaces; schema subtyping between revisions; a circuit breaker; GitOps manifests for
services and endpoints; a service marketplace.

## Owner questions

[research.md](research.md#owner-questions) records three choices with recommended defaults, which this
specification applies until answered: O1480 (handler kinds for sync operations), O1481 (in-flight
results after allow-list removal or endpoint deletion) and O1482 (publication approval for contract
and endpoint changes in protected namespaces).
