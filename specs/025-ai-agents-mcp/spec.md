# Feature Specification: AI agents and MCP

**Feature Branch**: `feature/025-ai-agents-mcp`

**Created**: 2026-10-26

**Status**: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision 2026-11-24-1); Architecture decisions 0059 and 0060 accepted (delegate, Owner decisions 2026-11-26-123 and 2026-11-26-124); not started

**Revised**: 2026-11-26, against `develop` at `3f22b612`, after an independent audit (3 Blockers, 19 Majors, 14 Nits), with the decisions the coordinating author took under the owner's delegation (Owner decision 2026-11-24-4): Owner decisions 2026-11-26-120 to 2026-11-26-125

**Revised again**: 2026-11-27, after the security review of that date (`reviews/2026-11-27-security.md`: 9 Majors, 11 Nits, every one fixed in this set; Decision 025-016)

**Revised again (round 3)**: 2026-11-27, with the owner's acceptance of Gap 025-004 (Owner decision 2026-11-27-2) and the coordinating session's decisions under the owner's delegation (Owner decisions 2026-11-27-400 to 2026-11-27-403): Owner question 025-001 answered no, the Entra and AWS modes routed through design 047's broker (Decision 025-017), MCP tool classifications pinned by the administrator (Decision 025-018), and the redacted approval view with escalation (Decision 025-019)

**Input**: LLM provider resources (Anthropic, OpenAI-compatible, AWS Bedrock, Azure); a durable AI
agent step whose model calls and tool calls are durable attempts run by a managed executor, with
tools (published scripts, activities, managed operations, child workflows, MCP tools), structured
output, budgets (turns, tokens, cost, wall time) and human approval for sensitive tools; an MCP
server exposing declared workflows, scripts and operations as tools with OAuth or scoped tokens,
per-tool authorization, audit and declared result disclosure; an MCP client so agent steps can use
external MCP servers (static bearer-token servers now; OAuth-authenticated servers once constitution 5.1.0, whose third
declared set of Principle V the owner approved in Owner decision 2026-11-25-3, merges and design 047's token broker is built); token streaming through feature 026 Workflow Streams; and, at P3, a Studio
authoring assistant that drafts workflows and never publishes them.

## Context and delivery boundary

Authors want a step that asks a model to decide, extract or summarise, lets it call a bounded set of
tools and hands a validated result to the next step. Operators need that step to survive crashes
like any other step, to cost no more than declared, to leave evidence of every model and tool call
and to keep secrets and over-classified data away from a third-party provider. Model clients want to
call a namespace's workflows over the Model Context Protocol (MCP); agents want tools that others
publish over MCP.

This feature adds no second engine, approval mechanism, secret authority or streaming channel:

- The agent loop is a declarative node interpreted by the engine. Every model call and tool call is
  a **derived step** with its own lease, fence, heartbeat, retry policy and journal events
  ([ADR-0059](../../docs/adr/ADR-0059-agent-steps-are-engine-driven-loops-of-durable-derived-steps.md);
  derived-step precedent ADR-0010).
- Model calls and MCP client calls run in a separate managed executor, `apps/agent-runner`, on the
  feature-017 model (Architecture decisions 0035 and 0036). The reason is Principle V itself: a model
  prompt or an MCP tool call is a request a tenant authored, sent with a third-party credential, and
  its answer is untrusted content, so it is sent only by an isolated executor holding only that
  attempt's capability. Feature 021 proposes the same placement as a general rule (Architecture
  decision 0055 decision 11, still proposed, Owner question 1123 open); this feature does not depend on
  that rule being accepted. The control plane and scheduler never call a provider or an external MCP
  server.
- The credentials of a model call or an MCP call are delivered under Principle V's first declared set
  (ordinary task execution), not as a new exception: the immutable task definition of such a derived
  step is the agent node of the published version together with the provider revision and the
  MCP-server revision that version pins, so the secrets those pinned revisions name are the declared
  secrets of that task definition (Requirement 025-003). Only credential modes that hand the runner a
  static API key or bearer token are offered under that set. A mode that would hand it a client secret
  or a signing key, which constitution 5.1.0's third declared set excludes for a declared resource
  ("never a refresh token, client secret or signing key"), is refused: OAuth-authenticated MCP servers,
  the `azure-entra-client-credentials` mode and the `aws-access-key` mode reach a provider only through
  design 047's token broker, under the third declared set of Principle V that the owner approved for
  constitution 5.1.0 (Owner decision 2026-11-25-3, pull request 286: a declared resource's settings and
  its current access token); until 5.1.0 merges and the broker issues access tokens, those modes are
  refused at save (Gap 025-001). The first set does not cover a declared resource's client secret or
  signing key (Owner question 025-001, answered no by Owner decision 2026-11-27-400, Decision 025-017):
  for `azure-entra-client-credentials` the broker runs the client-credentials grant and the runner
  receives only the current access token; for `aws-access-key` the broker exchanges the long-lived key
  for a short-lived AWS STS session and mints from it an Amazon Bedrock short-term API key, which the
  runner receives as a bearer token (Owner decision 2026-11-27-401).
- Effects stay at-least-once (ADR-0004). A model call has no provider idempotency key: a lost
  response is asked again and paid twice. A tool call carries a stable key.
- Approvals use feature 014 (FR-014-007, FR-014-008); values follow ADR-0033; the transcript has its
  own audited disclosure; secrets are references resolved per attempt (ADR-0035), never prompt text.
- The MCP server and client follow
  [ADR-0060](../../docs/adr/ADR-0060-mcp-exposes-declared-tools-and-calls-pinned-tools.md): nothing
  is exposed by default, results leave only through a declared classified projection, and external
  tools are called only from pinned, allow-listed snapshots.

Depends on existing features 001–003, 005–009, 011, 014, 016, 017, 018 (merged: its reset-rule
registry is code, `crates/domain/src/execution/reset.rs`, and Architecture decision 0050, accepted, fixes
the lock slots), 035 (managed kinds and runner profiles, `ManagedKind` and `BindingProfile`), 038
(claim and transition statement budget), 041 (step logs, worker-protocol minor 1.7) and 043 (run-input
gate and validator), and on parallel drafts 021 (resource types, managed operations), 022 (standalone
steps) and 026 (Workflow Streams, `executeRun`); it registers in registries owned by drafts 020 (content
stores), 028 (time classes), 031 (encoded positions) and 032 (key inventory), uses the 029 access
evaluator when 029 is enabled, and adds rows to the audit-to-OCSF mapping of design 040. Nothing here
claims those drafts are implemented.

Agents are a graph-model feature. A procedural workflow (constitution Principle IV, design 044, not yet
written) reaches an agent only through a child graph workflow that its envelope declares; an envelope
entry for agents is left to design 044 (Gap 025-002).

## Domain vocabulary

- **Provider resource**: a namespace resource naming an LLM endpoint, credential references, allowed
  models with declared prices, and the most sensitive data level that may be sent to it.
- **Agent step**: one activation of an `agent` node; owns a transcript, budgets and turns.
- **Turn**: one model call (`<node>~m<turn>`) and the tool calls it requested
  (`<node>~t<turn>.<call>`); numbering starts at 1.
- **Transcript**: the append-only canonical record of what an agent step sent to and received from
  the model: rendered instructions, prompt and context, assistant blocks (including provider-opaque
  blocks such as thinking), tool calls and tool results.
- **Transcript level**: the strictest classification of anything that can enter an agent step's
  transcript, computed at publication (D1319). Model-written values (the result, tool arguments)
  are never classified below it.
- **Tool**: a declared capability the model may request: activity, script, managed database query,
  021 managed operation, child workflow or allow-listed MCP tool.
- **Exposure**: a declaration that makes one target callable through the platform's MCP server,
  with its closed result projection.
- **MCP server resource**: a registered external MCP server with immutable tool snapshots.
- **Budget**: a finite allowance per agent step for turns, tool calls, tokens, estimated cost and
  wall time.

## User Scenarios & Testing *(mandatory)*

### US01 — Register a provider and run a durable agent step with structured output (Priority: P1)

An administrator registers an Anthropic provider allowing `claude-opus-5-5`, `claude-sonnet-5-5` and
`claude-haiku-5-5` with declared prices and a data level. An author adds an `agent` node that
classifies a ticket into a closed schema; the next step reads the result.

**Why this priority**: every other story needs a provider, an agent node and a durable model call.

**Independent Test**: with the development model provider, kill the agent runner after the provider
answered and before acknowledgement; after restart there is one committed turn, two charged
attempts and a validated result.

**Acceptance Scenarios**:

1. **Given** a saved provider, **when** a user runs Test provider, **then** a durable probe lists
   models through the runner and records a safe result, the tested revision and the time, without
   generating tokens; saving is never shown as connectivity.
2. **Given** a published agent workflow, **when** a run reaches the node, **then** the agent step and
   its first model call are created in one transaction and a committed answer that validates
   against `outputSchema` becomes `steps.<node>.output.result`.
3. **Given** a lost model-call response, **when** the lease expires, **then** the model is asked
   again, only the first committed result is kept and both requests are charged.
4. **Given** an invalid answer, **when** repair turns remain, **then** a bounded validation summary
   is sent back; otherwise the step fails with `AI_OUTPUT_INVALID`.
5. **Given** a provider edited, paused or with a model removed after publication, **when** a retry
   runs, **then** it keeps the endpoint, settings and credentials of the revision the published version
   pins; a paused provider, a model no longer in the current revision's allowlist or a current ceiling
   below the agent's transcript level refuses the next resolution before dispatch; a run is never
   redirected.
6. **Given** a provider whose endpoint answers the probe with a `302` to another host, **when** the
   probe runs, **then** the redirect is not followed, no credential reaches the other host and the
   probe fails with `AI_DESTINATION_DENIED`.

### US02 — Give the agent tools whose effects are durable (Priority: P1)

An author declares an activity tool, a script tool, a database query tool and a child-workflow tool.
The model requests several in one turn; each runs as a durable step and all results return together.

**Why this priority**: tools make an agent useful, and their effects are where at-least-once matters.

**Independent Test**: a turn requests three tools; crash one worker after its effect and before
completion; the retry carries the same idempotency key; the next model call starts only after all
three results committed.

**Acceptance Scenarios**:

1. **Given** a turn within the per-turn limit, **when** the model call commits, **then** each valid
   request becomes `<node>~t<turn>.<call>` executed by its kind's executor with key
   `agent:{runId}:{node}:{turn}:{call}`, and results return in request order in one message.
2. **Given** an undeclared tool or input failing its schema, **when** planned, **then** nothing runs
   and the model receives a bounded error result counted toward `maxToolCalls`; an undeclared request
   is recorded on every run-read surface as the fixed marker `UNKNOWN_TOOL` with its call index, never
   under the name the model wrote.
3. **Given** a `write` tool with an unknown outcome, **when** it declares `reconcileWith`, **then** it
   is looked up before any retry; otherwise it waits in manual review and the agent does not
   continue until an operator resolves it. A `read` tool is retried.
4. **Given** an agent in a saga region that compensates, **when** compensation runs, **then** tool
   calls with `compensateWith` are undone in reverse completion order and a `write` tool without
   compensation ends the run `PARTIAL`.

### US03 — Require a person's approval before a sensitive tool runs (Priority: P1)

The `issue_refund` tool requires approval by the `operator` role. An eligible person sees a redacted
view of the call (the decision context, a redacted outline of the transcript and the tool input, with
every field their classification rights do not disclose masked) and decides when they can see every
field the tool's approval policy names as needed for the decision.

**Why this priority**: a model must not cause a sensitive effect alone.

**Independent Test**: open a tool-call approval, restart the engine, submit an unauthorized then an
authorized decision, and race a decision against expiry; exactly one outcome wins.

**Acceptance Scenarios**:

1. **Given** a requested tool with an approval policy, **when** the model call commits, **then** an
   approval owned by that tool call opens in the same transaction, no executor receives the call,
   and the deadline is clamped to the agent's remaining wall time.
2. **Given** a pending approval, **when** an eligible person approves through
   `POST /approvals/{approvalId}/decisions`, **then** the tool call becomes ready with decision and
   audit in one transaction; rejection or expiry applies `refuse` (a refusal result goes to the
   model) or `fail` (`AI_TOOL_DENIED`).
3. **Given** a revoked role, another tenant or the excluded initiator, **when** they decide, **then**
   nothing is applied and the denial is audited without disclosure.
4. **Given** an approver without `payload:read-internal` or the reveal grant, **when** they open the
   tool-call view, **then** they see the decision context, a redacted outline of the transcript and the
   tool input, with every field above their level masked (its path and level only, never its length, a
   digest or a prefix), every field being at least the agent's transcript level; revealing a masked
   field needs the namespace reveal grant and is audited `payload.pii_revealed`; a tool input field
   annotated `secret` is refused at publication.
5. **Given** an eligible approver to whom a field named in the tool's `decisionFields` (every input
   field unless the author narrowed it) would be withheld, **when** they decide, **then** the decision
   is refused with `APPROVAL_INPUT_WITHHELD`, nothing is applied and the denial is audited: nobody
   approves the arguments that matter without seeing them. An approver of a tool whose author narrowed
   `decisionFields` to `["/amount"]` decides on the redacted view when they can see `amount`.
6. **Given** an agent whose transcript level is `pii` and an approval policy whose eligible deciders
   all lack the reveal grant, **when** the approval opens, **then** it is escalated to the policy's
   `escalation` route and audited `ai.approval_escalated`; when nobody there can see the decision fields
   either, or no route is declared, the approval stays pending and visible (the gauge
   `agent_approvals_without_decider` and its alert), and at its deadline it ends through `onExpire`
   with the recorded reason `NO_DECIDER_WITH_CONTEXT`, audited: it never expires unnoticed.

### US04 — Keep spend inside declared budgets and quotas (Priority: P1)

An author sets turn, tool-call, token, estimated-cost and wall-time budgets; an administrator sets
namespace limits on concurrent model calls and daily tokens.

**Why this priority**: an agent loop can otherwise spend without bound.

**Independent Test**: a scripted model that never stops calling tools is refused before the call
that would cross a budget, with a typed failure.

**Acceptance Scenarios**:

1. **Given** remaining budget below a model call's reservation, **when** the next turn would start,
   **then** nothing is dispatched and the step fails with `AI_BUDGET_EXHAUSTED`, routable by `onError`.
2. **Given** parallel tool results committing together, **when** the last commits, **then** exactly
   one next model call is scheduled with its reservation in that transaction.
3. **Given** a call with unknown usage, **when** it is retried, **then** its first reservation stays
   charged.
4. **Given** a cost budget on an unpriced model, **when** published, **then** publication fails with
   `VALIDATION_FAILED`, reason `UNPRICED_MODEL`.
5. **Given** a namespace at its concurrency or daily-token limit, **when** a model call is claimed,
   **then** it waits or is refused under the quota rules and no provider request is made.

### US05 — Keep secrets and over-classified data out of prompts (Priority: P1)

A provider declares `maxPromptClassification: internal`; a context expression references a `pii`
field; a tool output carries a secret value the tool was given.

**Why this priority**: sending data to a model provider is a disclosure to a third party.

**Independent Test**: publish definitions referencing `pii`, `secret` and unmarked fields; route
canary secrets through tools; no canary reaches the dev provider, a log, span, audit record, stream
or response, and the transcript is readable only through its audited endpoint.

**Acceptance Scenarios**:

1. **Given** a context expression referencing a carrier path above the provider's level, **when**
   published, **then** publication fails with `PROMPT_CLASSIFICATION_EXCEEDED` and the JSON pointer;
   `secret` is always refused.
2. **Given** a run value that does not validate against its carrier, **when** it would be rendered,
   **then** it is not sent and the call fails with `CLASSIFICATION_EGRESS_REFUSED` (the existing
   worker-origin code of `sdks/error-catalog.json`); a value with no carrier counts as `pii`.
3. **Given** a tool output containing a secret its attempt received, **when** it completes, **then**
   the tool call fails with `SECRET_IN_OUTPUT` and nothing is appended to the transcript.
4. **Given** a user with `run:read` only, **when** they inspect the run, **then** they see turns,
   declared tool names, statuses and usage, never values nor a string the model or the provider wrote;
   a holder of `payload:read-internal` and the namespace reveal grant reads the transcript through an
   audited `no-store` endpoint, and the run's payload-disclosure and payload-inspection endpoints refuse
   agent payloads.
5. **Given** an agent whose transcript level is `pii` (a context field or a tool result is `pii`),
   **when** its `outputSchema` annotates `result.summary` as `public`, **then** publication fails with
   `CLASSIFICATION_LOOSENED`; a model cannot relabel what entered its transcript.
6. **Given** the same agent, **when** the workflow's `end.outputExpr` is
   `steps.triage.output.result.summary` and the workflow `outputSchema` annotates `summary` as
   `public`, or a parent's `child` tool or child node annotates the output of a child workflow that
   wraps such an agent as `public`, **then** publication (or, for a child under an active-version
   policy, the child's admission) fails with `CLASSIFICATION_LOOSENED`: the level a value inherits from
   an agent's output follows it into every output carrier it reaches.

### US06 — Expose declared workflows to MCP clients (Priority: P2)

An administrator exposes `lookup-order` as the MCP tool `lookup_order` with a closed result
projection; a model client calls it with an OAuth token issued for the platform's MCP resource URI.

**Why this priority**: model clients reach the platform without custom integration code.

**Independent Test**: list tools, call the tool, lose the response, retry with the same idempotency
key; there is one run, one audited call and only the declared projection in the result.

**Acceptance Scenarios**:

1. **Given** no exposure, **when** a client calls `tools/list`, **then** the list is empty.
2. **Given** an exposure, **when** an authorized caller calls it, **then** a durable run is admitted
   in the normal admission transaction and the server returns the projected result within a bounded
   wait, otherwise a task (tasks extension) or a run handle.
3. **Given** a token for another audience, an expired token or a token in the query string, **when**
   presented, **then** the server answers `401` with `WWW-Authenticate` and the Protected Resource
   Metadata URL and admits nothing.
4. **Given** a caller lacking the rights for the projection's declared level, **when** they call,
   **then** the call is refused before admission; no value is partially returned.
5. **Given** another tenant's identity or a foreign exposure, **when** called, **then** the answer is
   indistinguishable from an unknown tool and the attempt is audited.
6. **Given** scoped token A, scoped to exposure Y, that called with idempotency key `order-42`, **when**
   token B, scoped only to exposure Z, calls Z with the same key and arguments, **then** B gets its own
   run (a receipt belongs to one credential and one exposure) and never Y's run or result; when A
   reuses `order-42` on another exposure, the call is refused with `IDEMPOTENCY_KEY_REUSED`.
7. **Given** an exposed workflow that starts a child workflow (a child node or a `child` tool) whose
   version contains an agent `write` tool acknowledged `approval: "none"`, **when** the exposure is
   saved or the child is admitted under a run that MCP admitted, **then** the save or the child's
   admission is refused with `UNATTENDED_WRITE_TOOL`.
8. **Given** a caller whose token is revoked while their call waits, **when** the run finishes inside
   the bounded wait, **then** no result is returned: the credential and the rights are checked again
   before any result leaves.

### US07 — Use tools of an external MCP server (Priority: P2)

An administrator registers an external MCP server with a static bearer token and refreshes its
tools; an author allows two of them in an agent node. (OAuth client credentials wait for constitution 5.1.0 to merge, approved by the owner in Owner
decision 2026-11-25-3, and for design 047's broker, Gap 025-001.)

**Why this priority**: many tool providers publish MCP servers.

**Independent Test**: change a tool's schema on the test MCP server after publication; the run fails
that call with `AI_TOOL_CHANGED` without sending it.

**Acceptance Scenarios**:

1. **Given** a registered server, **when** refreshed, **then** a durable runner job stores an
   immutable snapshot of names and digests from which authors choose.
2. **Given** a pinned snapshot, **when** a tool's schema or description changed, **then** the call
   fails with `AI_TOOL_CHANGED` and is not sent.
3. **Given** a metadata, loopback or out-of-policy destination, **when** refreshed or called, **then**
   egress refuses and the denial is audited.
4. **Given** a `write` external tool that timed out after the request was sent, **when** the attempt
   ends, **then** the outcome is unknown and the call waits in manual review.
5. **Given** an MCP server declaring `maxArgumentClassification: internal` and an agent whose
   transcript level is `pii`, **when** the agent allows one of its tools, **then** publication fails
   with `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED`; argument canaries never reach the test server.
6. **Given** a published agent whose provider or MCP server an administrator later lowers (a new
   revision with `maxPromptClassification` or `maxArgumentClassification` below the agent's transcript
   level), **when** a run is started or an already running agent reaches its next model or MCP call,
   **then** admission is refused (`AI_DESTINATION_RESTRICTED`) or the call fails with
   `CLASSIFICATION_EGRESS_REFUSED` before dispatch, without republication.
7. **Given** an external MCP server whose tool `delete_records` falsely states `readOnlyHint: true`
   and an MCP-server revision whose administrator did not classify that tool (so it is `write`), **when**
   an author declares it `effect: "read"` without an approval policy, **then** publication is refused
   with `WRITE_TOOL_APPROVAL_REQUIRED`; the disagreement between the annotation and the pinned
   classification is audited `ai.mcp.tool_classification_mismatch` at refresh and at publication, and
   a call of the tool needs a person's approval.

### US08 — Follow an agent while it works (Priority: P2)

An operator sees each turn, tool call, approval and estimated usage in Studio from the journal; with
feature 026, an authorized client reads token deltas from the run's Workflow Stream.

**Why this priority**: agents are long and non-deterministic; operators need to see where one is.

**Independent Test**: stream a three-turn agent, disconnect and resume, compare with the committed
transcript, kill the runner mid-stream and see superseded deltas marked.

**Acceptance Scenarios**:

1. **Given** a running agent, **when** a `run:read` holder opens the run, **then** Studio shows turns,
   tool calls, approvals and usage with projection lag, and no transcript values.
2. **Given** `stream.tokens: true` and feature 026, **when** a model call streams, **then** bounded
   delta events tagged with turn, attempt and generation are appended; the committed turn is
   authoritative.
3. **Given** feature 026 absent, **when** a workflow declares `stream.tokens: true`, **then**
   publication fails with an unsupported-capability error.

### US09 — Draft a workflow with the authoring assistant (Priority: P3)

A developer describes a process; the assistant proposes a draft shown as a diff, which the developer
may apply.

**Why this priority**: useful, not needed for durable agents.

**Independent Test**: apply a proposal and confirm no version, activation or run was created.

**Acceptance Scenarios**:

1. **Given** an assistant provider, **when** a developer asks, **then** an internal durable run returns
   a proposal validated by the existing validator; nothing changes until the developer applies it
   with `definition:write`.
2. **Given** an applied proposal, **when** the developer stops, **then** nothing is published,
   activated or run.
3. **Given** a proposal still invalid after repair turns, **when** returned, **then** the developer
   sees the problems and no draft changes.

### Edge Cases

- A model requests 20 tools with a per-turn limit of 4: four run, sixteen get refusal results, all
  count toward `maxToolCalls`.
- A lost model-call response is retried: the retry's lease is granted only if its own reservation
  fits beside the first one, which stays charged; otherwise the step fails with `AI_BUDGET_EXHAUSTED`.
- A runner holding three model-call leases is killed: when the leases expire the namespace's
  concurrency is free again, without any counter repair.
- A workflow with a `write` tool acknowledged `approval: "none"` is published, then someone tries to
  expose it through MCP or a 026 public link: both refuse with `UNATTENDED_WRITE_TOOL`. The same holds
  when the unattended tool is in a child workflow the exposed one can start, at any depth; for a child
  under an active-version policy the refusal comes at the child's admission.
- A model requests 256 undeclared tools whose names spell out a customer's tax number: each is refused,
  and the run graph, journal, stream and spans show 256 `UNKNOWN_TOOL` markers with call indexes, never
  the names.
- A JSON-RPC batch (an array body) reaches the MCP endpoint: it is refused with `-32600` before any
  member is read.
- An external MCP server starts advertising another token endpoint (OAuth servers, once constitution 5.1.0
  merges and design 047's broker is built): the next call is refused before any
  credential is used, and the change is audited.
- Two tool calls of a turn complete at once: the run lock serializes them; only the last commit
  schedules the next turn.
- Cancellation while a model call streams, an approval is pending and a child tool runs: the call is
  asked to stop at its next heartbeat, the approval is cancelled, the child follows its policy, no
  turn starts.
- The agent deadline fires as the last tool result commits: first commit wins; the loser is an
  audited no-op.
- Repeated `429` without `retry-after` (a provider spend cap): the retry budget ends with
  `AI_RATE_LIMITED`.
- A provider reports a different model id: recorded on the turn, not a failure (G1292).
- A provider stream fails after `200`: the attempt fails retryably; streamed deltas are superseded.
- A tool result above its bound is cut, flagged truncated, and the model is told.
- A transcript reaching its retained-bytes ceiling fails the step with `RUN_DATA_LIMIT` before dispatch.
- An MCP client retries without an idempotency key: a second run starts, as documented.
- An external MCP server returns `input_required` or a task: the call returns
  `MCP_UNSUPPORTED_RESULT` to the model.
- A provider deleted while a pinned run waits on approval: the tombstone keeps the revision; the
  next resolution fails closed.
- The only reveal-grant holder among an approval's eligible deciders loses the grant while it is
  pending: the next sweep (at most 60 seconds later) finds no decider with context and escalates it;
  a decision that holder sends after the loss is refused `APPROVAL_INPUT_WITHHELD`.
- An MCP server's refresh changes a tool's annotation from `readOnlyHint: true` to
  `destructiveHint: true`: published versions keep their pinned snapshot and revision, the mismatch is
  audited, and nothing becomes looser.

## Requirements *(mandatory)*

Defaults and hard ceilings are in [plan.md](plan.md#finite-ceilings). Effective values are the
minimum of declaration, namespace policy and deployment ceiling; zero is not unlimited; admission
fails before work.

### Functional Requirements

#### Provider resources

- **FR-025-001 — Provider catalog**: The platform MUST provide namespace-scoped create, list, detail,
  update, pause, probe and delete for provider resources of kinds `anthropic`, `openai-compatible`,
  `aws-bedrock` and `azure-openai`, as platform-provided resource types of feature 021's resource
  model; there is no second catalog (Owner decision 2026-11-26-122, Decision 1283). The provider work
  starts after 021's resource core merges; until then the engine, runner and adapters are built against
  a test provider written through the store port by test support only. Settings are non-secret
  (HTTPS endpoint, region, deployment map, API version); credentials are secret references; unknown
  fields and credentials in settings or URLs are rejected. Resources have stable identity, a
  namespace-unique name and immutable revisions with optimistic concurrency; deletion is a tombstone
  that keeps referenced revisions.
- **FR-025-002 — Allowed models and pinning**: A revision MUST list allowed models with the
  provider's id, optional alias, maximum output and context tokens, capability flags (native
  structured output, parallel tool calls, streaming) and optional declared prices. Publication
  rejects other models. Publication records in the published version, per agent node, the provider
  revision it checked (the author's explicit `revision`, otherwise the current one) and, per `mcp` tool,
  the MCP-server revision and the snapshot identifier; admission copies those records into the run's
  pins in the admission transaction, and every attempt uses them for endpoint, settings, credentials
  and prices. Restrictions saved later still apply at once: at admission and at every resolution the
  transcript level is checked against the stricter of the pinned revision's ceiling and the current
  revision's ceiling, and the pinned model must be in the current revision's allowlist as well as the
  pinned one; a failure refuses admission (`AI_DESTINATION_RESTRICTED`) or the call before dispatch
  (`CLASSIFICATION_EGRESS_REFUSED` for a ceiling, `AI_CAPABILITY_UNAVAILABLE` for a model). A later
  revision never widens what a published version may send. Pause and deletion block new admission and
  resolution; current revocation stays authoritative for pinned runs (Decision 025-016).
- **FR-025-003 — Credentials**: Credential values MUST live only in OpenBao behind the feature-017
  facade and be resolved per attempt. The modes offered are those that hand the runner a static API key
  or bearer token: API-key header or bearer (`anthropic`, `openai-compatible`), an Amazon Bedrock API
  key sent as a bearer token (`aws-bedrock`, mode `bearer`; its use by the adapter is verified in Task
  025-011, Gap 1291) and an API key (`azure-openai`, mode `api-key`). The modes `aws-access-key` (an
  access key pair, whose secret access key is a long-lived signing key, with or without a role) and
  `azure-entra-client-credentials` (an Entra client secret) are refused at save with
  `AI_CREDENTIAL_MODE_UNAVAILABLE` until design 047's broker issues access tokens to the runner (Gap
  025-001): no client secret or signing key ever reaches the agent runner (Owner question 025-001,
  answered no by Owner decision 2026-11-27-400, Decision 025-017). Once the broker exists, both modes
  are delivered through it and the runner receives only an access token: for
  `azure-entra-client-credentials` the broker holds the client secret and runs the client-credentials
  grant against the recorded Entra authority host (HTTPS, confirmed by the saving administrator, never
  discovered), and the token request goes only there; for `aws-access-key` the broker holds the
  long-lived key pair, calls AWS STS (`AssumeRole` when the revision records a role, `GetSessionToken`
  otherwise; a session of 900 seconds by default and at most 3,600) and from that session mints an
  Amazon Bedrock short-term API key, which the runner sends as a bearer token on the adapter's existing
  `bearer` path, so the runner holds neither the long-lived key nor the session's signing key (Owner
  decision 2026-11-27-401). Their disclosure fits Principle V's first declared set (ordinary task execution):
  the immutable task definition of a model-call or MCP-call derived step is the agent node of the
  published version plus the provider revision and the MCP-server revision that version pins, so the
  logical secrets those pinned revisions name are the declared secrets of that task definition (a probe
  or refresh run's private definition pins its revision the same way). They are delivered all or
  nothing, only to the lease-owning agent runner, and only while the caller's current membership, its
  worker/activity binding (`BindingProfile::Agent` for the reserved activity), the pool-delivery policy
  of every secret and the publisher's permission `secret:use` on every one of them authorize it; the
  publisher's `secret:use` is checked at publication and again at delivery. No fallback; a deployment's
  own cloud identity is never used for a tenant account; adapters use explicit static credentials only,
  never SDK default credential chains or instance-metadata lookups (D1299). These secrets are disclosed
  only by `ResolveModelCall` and `ResolveMcpCall`: the node the agent declaration materializes for every
  `sagat.ai.*` managed kind has an empty `secrets` list, and the ordinary `ResolveSecrets` call, and
  every other secret-bearing worker call except the resolve call of the lease's own managed kind,
  refuses a lease whose node has a managed kind. MCP servers authenticated by
  OAuth are outside this rule: they reach tokens only through design 047's token broker, under
  the third declared set of Principle V that the owner approved for constitution 5.1.0 (Owner decision 2026-11-25-3, pull request 286: a declared resource's settings and its current access token) (Gap 025-001).
- **FR-025-004 — Provider probe**: Test provider MUST admit a fixed durable probe (origin
  `AI_PROVIDER_PROBE`) run by the agent runner on the same destination, TLS, credential and quota
  path as a model call, calling a model-listing endpoint or equivalent. It generates no tokens and
  records a safe category, tested revision and time; success is evidence at that instant only. The
  probe, the MCP tool refresh (Requirement 025-035) and the authoring assistant (Requirement 025-039)
  are private runs created by `start_private` and so pass row 13 of the design 043 run-input gate.
- **FR-025-005 — Data policy**: A revision MUST declare `maxPromptClassification` (`public`,
  `internal` or `pii`), with no default at the API; `secret` is not a value (D1289). The value is the
  existing domain type `EgressCeiling` (`crates/domain/src/classification.rs`, Architecture decision
  0071 decision 8): `EgressCeiling::requires_reveal_grant` decides who may set it and
  `EgressCeiling::admits` decides what may leave. The rule for `pii` is the uniform `pii`-egress rule of
  Owner question 1081 (decided on 2026-11-24 for feature 020, Decision 1617), applied to provider
  revisions and MCP-server revisions by Owner decision 2026-11-26-120: a ceiling of `pii` is set only by
  an administrator holding the namespace reveal grant; that administrator's grant (the actor recorded on
  the revision that set `pii`, the reading of Decision 1617) is rechecked at every resolution whose
  transcript level is `pii`; its revocation, or that administrator leaving the namespace, pauses the
  provider until an administrator who holds the grant saves a new revision; each such attempt is audited
  `ai.pii_egress`; a secret never leaves; and the default is `internal` (the type's default, which the
  Studio preselects; the API still requires the field to be sent) (D1637).

#### Agent node and loop

- **FR-025-006 — Agent node**: The IR MUST gain an additive `agent` node (ADR-0032) with `model`,
  literal `instructions` and `prompt`, named `context` expressions, `tools`, optional closed
  `outputSchema`, `budgets`, `modelOptions` (`effort`, `maxOutputTokens`), `retry`,
  `attemptTimeoutMs`, optional `stream`, `timeoutMs`, `next` and `onError`
  (`contracts/agent-node.schema.json`). It is allowed only in the root graph of a graph-model version
  and not with `chainPolicy` (D1281); it declares no step-output destinations (feature 035), so no
  `<agent>~d.<name>` id exists (Decision 025-004). Publication refuses it unless the operator has
  recorded the platform requirement `ai-agents-v1` in the instance registry (Architecture decisions
  0082 and 0050 decision 9; the capability is listed by control planes and execution engines whose
  configuration runs agents); unknown fields are rejected.
- **FR-025-007 — Request rendering**: The server MUST render each request from the pinned
  instructions, the prompt, context values evaluated by the bounded evaluator and serialized as
  canonical JSON in named delimited blocks, declared tools, the committed transcript and the output
  schema, with no secret, network or callback capability, and record the rendered request's digest.
- **FR-025-008 — Loop semantics**: Activation MUST create the agent step, its budgets and the first
  model call in one transaction. A committed model call appends the assistant message and, in the
  same transaction, plans its tool calls or the final answer. The next model call is scheduled only
  by the transaction settling the turn's last tool call. The first committed result wins, a stale
  attempt is rejected, and the transcript is append-only with provider-opaque blocks replayed
  unchanged (D1282).
- **FR-025-009 — Model call execution**: A model call MUST be the reserved activity `sagat.ai.model`
  (a managed step of `ManagedKind::AiModel`), claimable only by a current `agent-runner` identity whose
  binding has `BindingProfile::Agent` in the namespace, and resolved by a two-phase `ResolveModelCall`.
  Phase 1, in a run transaction closed before OpenBao I/O, checks the current tenant and namespace
  scope, the exact task, attempt and generation, the matching capability token and unexpired lease, the
  recorded lease-owning executor subject and live session, the pinned revision and provider status, and
  the Principle V authorities of Requirement 025-003 (caller membership, binding, pool-delivery policy,
  the publisher's `secret:use` on every declared secret) plus the initiator's `ai:use`. Phase 2
  re-checks all of them and that the selected secret revisions still match, then commits the
  value-free `secret.resolved` audit and a metadata-only credential pin per secret slot before any value
  is returned; a failed check or audit refuses disclosure. Both phases also recheck the restrictions of
  Requirement 025-002 (current ceiling and allowlist). No lock is held during OpenBao or provider
  I/O. Provider failures map to the closed vocabulary of `contracts/errors.md`; retries honour
  `retry-after` within the attempt deadline. The claim path offers a `sagat.ai.*` lease only for a node
  whose `managed_kind` is that AI kind, never by the activity name alone; the `sagat.ai.` prefix is
  reserved (an ordinary activity, worker or version naming it is refused at publication and
  registration), and recording `ai-agents-v1` is refused while any existing version has an ordinary
  activity under that prefix (Task 025-095).
- **FR-025-010 — Tool declarations**: Each tool MUST declare a name matching `^[a-zA-Z0-9_-]{1,64}$`
  unique within the node, a literal description, a kind (`activity`, `script`, `database`,
  `operation`, `child`, `mcp`) with its reference, a closed bounded input schema (from the target
  when it declares one), `effect` (`read` or `write`, default `write`), an optional result
  projection, an optional closed `outputSchema` (the result's carrier when the target has none),
  bounded retry and timeout, and an approval policy. A `write` tool MUST declare either an approval
  policy or the explicit acknowledgement `approval: "none"`; `operation` tools always need a policy
  (G1293). A `database` tool is a parameterized `query` bound only from tool arguments; `execute` is
  always `write` and always needs an approval policy (`approval: "none"` and `effect: "read"` are
  refused with `WRITE_TOOL_APPROVAL_REQUIRED`). A `child` tool declared `effect: "read"` needs an
  approval policy (the child workflow can write; `WRITE_TOOL_APPROVAL_REQUIRED`). An `mcp` tool names a
  server resource and a tool of its pinned snapshot, and may be declared `effect: "read"` without an
  approval policy only when the pinned MCP-server revision classifies that tool `read`
  (`toolClassifications`, Requirement 025-034); a tool the revision does not classify is `write`. The
  snapshot's annotations (`readOnlyHint`, `destructiveHint`) are untrusted hints: they never turn a
  `write` into a `read` nor remove an approval requirement, and a disagreement between an annotation and
  the pinned classification is audited (Decision 025-018). A tool input field annotated `secret` is
  refused at publication (`SECRET_TOOL_INPUT`): the model never holds a secret to write.
- **FR-025-011 — Tool call validation**: An undeclared tool or input failing its schema MUST NOT
  execute; the model gets a bounded error result. Tool inputs are validated by the platform's run-input
  validator of design 043 (`RunInputValidator`, `crates/infra-schema/src/run_input/`), not by a second
  validator; `agentClosedSchema` is a restriction of the 043 subset plus closure
  (`additionalProperties: false` on every object, `maxLength` on every string, `maxItems` on every
  array), so every agent schema compiles there. The error result is built only from the validator's
  value-free rule and field, never from the value. Requests beyond the per-turn limit
  (`maxParallelToolCalls`) or the remaining `maxToolCalls` get refusal results. A turn's results return
  in request order in one message. An undeclared request is recorded as the fixed marker `UNKNOWN_TOOL`
  and its call index: the name the model wrote exists only in the transcript (`pii`), never in
  `agent_tool_calls`, the run graph, the journal, the stream, spans, metrics, logs or audit.
- **FR-025-012 — Tool execution**: Each valid call MUST be a derived step run by its kind's executor
  (SDK worker, sandbox runner, database runner, 021 executor, child run, agent runner for MCP) with
  key `agent:{runId}:{node}:{turn}:{call}` (D1296). `read` tools retry; a `write` tool with an unknown
  outcome uses `reconcileWith` when declared, otherwise manual review. `child` tools use the existing
  child semantics and tree budget; the child's admission goes through the run-input gate of design 043
  (its child row), and a refused input creates no child: the tool-call step fails with
  `CHILD_INPUT_INVALID` (or `RUN_INPUT_SCHEMA_UNUSABLE`), as FR-043-008 says of a parent step, and the
  model receives `TOOL_FAILED` with that code and the value-free rule and field; the agent continues and
  the call counts toward `maxToolCalls` (Decision 025-007). A child's admission is also refused, with
  no child created and `TOOL_FAILED` to the model, when the child version's recorded output levels are
  stricter than the tool's result carrier (`CLASSIFICATION_LOOSENED`, Requirement 025-019) or when the
  run's root was admitted through MCP or a 026 public link and the child version is unattended
  (`UNATTENDED_WRITE_TOOL`, Requirement 025-026). Results are bounded, scanned for resolved
  secrets, projected and checked against the provider's level before they are appended.
- **FR-025-013 — Structured output**: With `outputSchema`, the platform MUST validate the final
  answer against the closed bounded schema, use the adapter's native constraint where the model entry
  declares it (never forced tool choice), send at most `maxRepairTurns` bounded validation summaries
  back, and fail with `AI_OUTPUT_INVALID` when no valid answer fits the budget. Validation uses the 043
  validator (Requirement 025-011); a summary lists only its value-free rule and field pairs.
- **FR-025-014 — Stop reasons and output**: An answer without tool requests is final. `refusal` fails
  with `AI_MODEL_REFUSED`, a `max_tokens` stop without a complete answer or request with
  `AI_OUTPUT_TRUNCATED`; both non-retryable and routable by `onError`. The retained output is
  `{result, usage, stopReason, turns, toolCalls}` with a generated carrier in which every `result`
  field is at least the transcript level (FR-025-019) and the metadata is `internal`; a looser author
  annotation is refused at publication with `CLASSIFICATION_LOOSENED` (D1291). It is retained under
  the content quota and as a blob claim-check above the inline limit.
- **FR-025-015 — Budgets**: An agent step MUST enforce `maxTurns`, `maxToolCalls`,
  `maxParallelToolCalls`, `maxRepairTurns`, `maxInputTokens`, `maxOutputTokens`, optional `maxCost` and
  `timeoutMs`. Every model-call attempt, first or retry, takes its conservative reservation (D1286)
  against the agent's budgets and the namespace's daily budget in the transaction that grants its lease
  (the daily budget is charged to a per-tenant, per-namespace lease of the claiming server backend,
  refilled from the namespace's `ai_usage_daily` row only when short, never written at every commit:
  the pattern of Architecture decision 0097); a reservation that does not fit fails the step with
  `AI_BUDGET_EXHAUSTED` before any lease reaches a runner. Completion settles reported usage; a
  provider error response releases the reservation; a lost response, timeout or lease expiry keeps it
  as charged. `maxCost` needs declared prices; cost is labelled estimated.
- **FR-025-016 — AI quotas**: The platform MUST enforce namespace and tenant concurrent agent-runner
  attempts (`ai_runner_concurrency` and `tenant_ai_runner_concurrency`: the count of live leases of
  every `sagat.ai.*` activity, model calls, MCP calls, probes and refreshes alike, read in the claim
  transaction, so completion and expiry release capacity), namespace tokens and estimated cost per UTC
  day (settled plus reserved, through the leases of Requirement 025-015), an exposure's optional daily
  tokens (`maxDailyTokens`, charged through the same leases under the exposure's own budget scope),
  live agent runs per MCP caller (`mcp_agent_runs_per_caller`, Requirement 025-033), and
  counts of providers, MCP servers, exposures and scoped tokens, through ADR-0027 resolution. Over a
  limit, claim or admission waits or fails before any provider request; a lowered limit does not
  cancel dispatched work.
- **FR-025-017 — Tool approval**: A tool with an approval policy MUST open an approval owned by its
  tool call in the transaction committing the model call, with feature-014 eligibility,
  decision-time authorization, deduplicated receipts, initiator exclusion and first-commit
  resolution against expiry and cancellation (D1297). `onReject`/`onExpire` are `refuse` or `fail`.
  The approver's view is a redacted view under ADR-0033 with the tool's input schema as carrier, every
  field at least the transcript level (a looser annotation is refused at publication, D1292): the
  decision context (run, workflow version, agent node, turn and call index, the tool's declared name,
  description, kind, target and effect, and the budgets used), a redacted outline of the transcript
  (the turns, the declared names of the tool calls and their outcomes, with every model-written text and
  every tool input and result value masked) and the tool input with each field the decider's current
  rights disclose shown and every other field masked (its path and level only, never its length, a
  digest or a prefix). Revealing a masked field needs the namespace reveal grant and is audited
  `payload.pii_revealed`. The deadline is clamped to the remaining agent wall time. The tool's approval
  policy names the fields a decider must see in clear (`decisionFields`, JSON pointers into the input
  schema; every input field by default). An author may narrow `decisionFields`, even to none, for a tool
  that may run with `approval: "none"`; for a tool that may not (`operation` tools, database `execute`
  tools) narrowing is refused at publication (`DECISION_FIELDS_REQUIRED`). At decision time, a decider to
  whom a decision field would be withheld is refused with `FORBIDDEN` reason `APPROVAL_INPUT_WITHHELD`,
  nothing is applied, and the denial is audited (no tool input field is `secret`, Requirement 025-010).
  When the approval opens, after every `APPROVAL_INPUT_WITHHELD` refusal and on every pass of the
  approval sweeper (every 60 seconds), the platform checks whether at least one eligible decider
  (feature 014 eligibility, the initiator excluded) currently sees every decision field; when none does,
  the approval is escalated to the policy's `escalation` route (an eligibility policy of the same shape),
  audited `ai.approval_escalated` and counted. When nobody on that route can see them either, or the
  policy declares no route, the approval stays pending and visible (the gauge
  `agent_approvals_without_decider` and its alert), and at its deadline it ends through `onExpire` with
  the recorded reason `NO_DECIDER_WITH_CONTEXT`, audited: an approval nobody can decide never expires
  unnoticed (Decision 025-019).
- **FR-025-018 — Cancellation and deadlines**: Cancellation and the agent deadline MUST stop new turns
  and tool calls, ask in-flight model calls to stop through heartbeats, cancel pending approvals and
  apply each tool kind's cancellation semantics. Deadline, decision, expiry, cancellation and
  completion races are decided by the first commit; the loser is an audited no-op. Cancellation is
  never presented as undoing an effect.

#### Data protection

- **FR-025-019 — Prompt classification**: Publication MUST compute each agent node's transcript
  level (D1319): the strictest of `internal` (literals and tool descriptions), every carrier path a
  context expression references and every tool's result carrier (a value without a carrier counts as
  `pii`; a tool result uses its target's output carrier or the tool's closed `outputSchema`). It MUST
  reject with `PROMPT_CLASSIFICATION_EXCEEDED` and the JSON pointer when that level exceeds the
  provider's `maxPromptClassification` or a referenced path is `secret`; reject with
  `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED` an `mcp` tool whose server revision's
  `maxArgumentClassification` is below it; and reject with `CLASSIFICATION_LOOSENED` a `child` tool
  whose child input schema declares a field below it. The levels are compared with the domain's
  `Level::stricter` and `EgressCeiling::admits`, not a second implementation. At run time a value that
  does not validate against its carrier is not sent: the call fails with `CLASSIFICATION_EGRESS_REFUSED`
  before dispatch (a tool result in that state returns `TOOL_FAILED` to the model). The ceiling checks
  are repeated at admission and at every resolution against the current restrictions (Requirement
  025-002).

  **Inherited output levels.** A value drawn from an agent's output keeps at least that agent's
  transcript level in every output carrier it reaches. Publication computes the effective level of each
  field of the workflow `outputSchema`, of each child node `outputSchema` and of each `child` tool's
  result carrier with the existing domain rule `written_level` (`crates/domain/src/classification.rs`):
  the stricter of the field's annotation and the level it inherits. A workflow output field inherits from
  the paths `end.outputExpr` references (when the expression is one path, field `x` inherits from that
  path's `x`; otherwise every field inherits the strictest referenced path); a path into
  `steps.<agent>.output.result` inherits the agent's transcript level; a path into a child's output
  inherits the child version's recorded output levels. Publication refuses an annotation below the
  inherited level with `CLASSIFICATION_LOOSENED` and the pointer, and records the version's effective
  output levels. A child under an active-version policy, whose version is unknown at publication, is
  checked at its admission against the levels recorded in the version it starts, and refused there. The
  transcript level of a parent agent uses the effective level of each `child` tool's result carrier.
  The MCP exposure checks (Requirement 025-032) use these effective levels at save and at call time. A
  value an activity or script computes is a new value under that step's declared carrier (feature 016)
  and inherits nothing (Decision 025-016).
- **FR-025-020 — Secrets never in prompts**: Secret values MUST NOT reach a rendered request,
  transcript, journal, log, span, audit record, stream event or response. Rendering has no secret
  capability; provider and MCP credentials exist only in the runner's transport; tool outputs and
  model-call outputs are scanned for the values their attempt received (`SECRET_IN_OUTPUT`); canaries
  cover every surface. The existing scan matches a value of at least 8 bytes verbatim and JSON-escaped
  (`crates/application/src/worker.rs`); for agent tool results and agent-runner outputs it is extended to
  the base64 (standard and URL-safe, all three byte alignments) and hexadecimal (both cases) forms of
  each value. A secret shorter than 8 bytes, or transformed any other way (compressed, encrypted,
  split, re-encoded twice), is not detected: that is a stated bound of the guarantee, not a claim
  (Gap 025-004, accepted by the owner as a documented known limit, Owner decision 2026-11-27-2). The
  limit is stated and not hidden, in the form of Architecture decision 0085 rule 12: the platform MUST
  NOT claim otherwise in its documentation, and the runbook MUST say how a run's agent content is purged
  and that the secret is then rotated. The primary defence stays that the agent runner never receives a
  secret the task definition does not declare (Requirement 025-003).
- **FR-025-021 — Retention**: Transcripts, tool inputs and results, agent outputs and request staging
  blobs MUST be retained content under the tenant content quota and the storage classification
  manifest, registered as content stores in 020's registry and purged with the run's content under
  020 (D1638). Data sent to a provider or an external MCP server is outside platform deletion; the
  provider form and runbook say so (G1285).
- **FR-025-022 — Transcript disclosure**: Ordinary run, graph, journal, SSE, telemetry and audit
  surfaces MUST stay payload-free. `GET /namespaces/{namespaceId}/runs/{runId}/agent-transcripts/{nodeId}`
  returns the bounded transcript only to the feature-016 `pii` audience (`run:read`,
  `payload:read-internal` and the namespace reveal grant), audits every read as `payload.pii_revealed`
  (never collapsed) before any byte is returned, and answers `Cache-Control: private, no-store`. No
  bulk export. The existing run endpoints `payload-disclosures` and `payload-inspections` (feature 016)
  refuse every agent derived-step payload (model-call requests and responses, tool inputs, MCP results)
  and every transcript entry, answering as for a payload that does not exist; the agent node's retained
  output stays disclosable there under its generated carrier (Requirement 025-014). Run-read surfaces
  (the run graph's `AgentProgress`, the journal, the stream's `internal` fields, spans, metrics, logs and
  audit) carry no string the model or the provider wrote: a tool name is a declared name or the marker
  `UNKNOWN_TOOL`; the provider's tool-use id (`providerCallId`) is transcript content only; the model the
  provider reports is shown only as `substituted: true|false` against the pinned model, its value kept
  in the transcript; the provider request id is kept on spans only when it matches the closed pattern
  `^[A-Za-z0-9._:-]{1,128}$`, otherwise dropped.

#### Execution boundary

- **FR-025-023 — Egress**: Provider and MCP destinations MUST use the guarded egress client and the
  namespace egress policy, with no redirect followed for any method (`max_redirects = 0`, the guard's
  default, for every provider and MCP request, the probe and the refresh included: a `3xx` answer is a
  failure, `AI_DESTINATION_DENIED` audited `egress.denied`), no environment proxy, bounded time and
  bytes and a bounded incremental reader for event streams. The namespace egress policy is
  operator-owned runner configuration, per namespace, reread before every attempt (the precedent of
  `apps/database-runner/src/config.rs`); a namespace absent from it is denied, and nothing the server
  sends can widen it. Private destinations need the operator-approved
  runner policy (ADR-0035 decision 4); metadata, link-local, loopback and platform endpoints stay denied.
- **FR-025-024 — Managed executor isolation**: The agent runner MUST be a separate process with the
  new machine role `agent-runner` (`worker:execute`, `ai:execute`; never `database:execute` and never a
  database credential, which the `database-runner` role holds), a current namespace binding of
  `BindingProfile::Agent`, and the next free minor of the worker-protocol register of `sdks/SPEC.md`
  section 9 when its contract merges (1.7 is feature 041's, Owner decision 2026-11-15-8; G1280). It
  holds no platform database URL, OpenBao token, signing key or container-engine access, receives only
  the current attempt's capability and the credentials of Requirement 025-003, drops them when the
  attempt ends and cannot claim ordinary tasks; ordinary workers, sandbox, database and lake runners
  cannot claim or resolve its activities. Its steps are managed steps (`Node::managed_kind` is an AI
  kind), so the step logs of design 041 are refused for them: an `agent-runner` token is refused by the
  step-log ingest (its checks P1 and T7), a carry on a `sagat.ai.*` step is ignored (check C0), its
  registration carries no `StepLogPolicy`, and the runner never sends step logs; the transcript is the
  separate authoritative record (Decision 025-005).
- **FR-025-025 — Sagas**: An agent in a saga region MUST compensate completed tool calls in reverse
  completion order with each tool's `compensateWith`; a `write` tool without compensation is
  irreversible and compensating past it ends the run `PARTIAL`; a failing compensation waits for a
  person.

#### MCP server

- **FR-025-026 — Exposures**: `mcp:expose` holders MUST be able to create, revise, disable and delete
  exposures naming a tool name, a target (a published workflow with an active- or pinned-version
  policy; script and operation targets only with feature 022 standalone steps, D1315), the input
  schema from the target, a closed result projection with `disclosure: "declared-mcp-result-v1"` and a
  `maxResultClassification`, and an optional daily token budget (`maxDailyTokens`). Nothing is exposed
  by default. An exposure, and a call at run time, MUST refuse an unattended target version
  (`UNATTENDED_WRITE_TOOL`). A version is unattended when it contains an agent `write` tool
  acknowledged with `approval: "none"`, or when any child workflow it can start (child nodes and `child`
  tools, at any depth, under their version policy) is unattended; publication records the flag for
  pinned children, and for a child under an active-version policy the flag of the version it resolves
  to is checked at save and call time and again at the child's admission. Every run whose root was
  admitted through MCP or a 026 public link carries that origin to all its descendants, and a child
  admission under it refuses an unattended child version (`UNATTENDED_WRITE_TOOL`). Feature 026 public
  links must apply the same refusals (T025-076).
- **FR-025-027 — Protocol surface**: The server MUST implement MCP 2026-07-28 over Streamable HTTP and
  serve 2025-11-25 clients statelessly, with `server/discover`, `tools/list`, `tools/call` and the
  tasks extension, and without sampling, roots, logging, resources or prompts (D1302). A call admits
  durable work and nothing else.
- **FR-025-028 — Authentication**: The server MUST act as an OAuth 2.1 resource server: publish
  Protected Resource Metadata (RFC 9728), answering identically for every well-formed namespace id;
  accept only `Authorization` bearer tokens whose only audience is the configured canonical MCP URI
  (RFC 8707), or scoped MCP tokens; answer `401` with `WWW-Authenticate`, or `403` with
  `insufficient_scope`; and never pass a received token to a target. REST MUST refuse any token whose
  audience contains an MCP URI (D1303, G1296).
- **FR-025-029 — Scoped MCP tokens**: `mcp:token:manage` holders MUST be able to mint, list and revoke
  tokens bound to one namespace, a list of exposures and an expiry, shown once and stored as a hash;
  effective permission is the minter's current permissions ∩ scope at every call. The request names
  exposures by tool name and the mint resolves them to exposure identifiers, which is what the token
  stores: a deleted exposure recreated under the same name is not in an old token's scope. A mint
  replayed with the same `Idempotency-Key` and content returns the token's metadata without the token
  (no plaintext is kept in any receipt); changed content is `IDEMPOTENCY_KEY_REUSED`. Lifecycle and
  denials are audited.
- **FR-025-030 — Tool authorization**: A call MUST require `mcp:call`, the exposure in the token
  scope, the target's own permission (`run:start`) and the rights of the declared result level at
  call time; when 029 is enabled, visibility of exposures and their targets is decided by the 029
  access evaluator. `tools/list` returns only callable exposures in name order with
  `cacheScope: "private"`.
  A foreign, unknown, out-of-scope or uncallable exposure is answered as an unknown tool and the
  attempt is audited (D1316).
- **FR-025-031 — Call admission and waiting**: A call MUST admit the target through the existing
  durable admission transaction, deduplicated by the optional `_meta` key
  `sagat-octopus-orchestrator.io/idempotencyKey`. A receipt belongs to one caller, defined as the
  credential identity (the scoped token's identifier, or the OAuth issuer and subject), and to one
  exposure: the same caller, exposure, key and argument digest return the same run; changed arguments,
  or the same key on another exposure, are refused with `IDEMPOTENCY_KEY_REUSED`; another caller never
  sees the receipt. Authorization runs before the receipt is read, and a replayed result is disclosed
  only through the replaying call's own exposure and its current revision's projection, after that
  call's authorization. The arguments are the run input and pass the run-input gate of design 043 as a
  new row of its table: the call builds a `CheckedRunInput` and admits through the existing `create`,
  so no second validation path exists. `RUN_INPUT_INVALID` is answered as JSON-RPC `-32602` with the
  value-free rule and field (never a value), `RUN_INPUT_SCHEMA_UNUSABLE` as a tool error, and the
  transient refusals (no permit, the backstop, an interrupted check) as a retryable JSON-RPC `-32603`;
  in `report` mode the refusal is counted and audited, the run is admitted and the MCP caller sees an
  ordinary call. An exposure's `inputSchema` is the target version's, inside the 043 subset
  (`specs/043-run-input-validation/contracts/input-schema-subset.md`); a target outside it cannot be
  exposed. After a bounded wait a non-terminal run is a task for clients with the tasks
  extension or a run handle otherwise; the wait reuses feature 026 `executeRun` semantics
  (FR-026-007, FR-026-009) when available. Before a result leaves at the end of the bounded wait, the
  call re-checks the credential (not revoked, not expired, the minter's or subject's current membership),
  `mcp:call`, the token scope, the exposure's state and the result-level rights; a failed check returns
  no result and is answered as an unknown tool. `tasks/get` and `tasks/cancel` (which also needs
  `run:operate`) re-check the same on every call; a task handle is a name, not a capability, bound to the
  caller as defined above.
- **FR-025-032 — Result disclosure**: A result MUST contain only the exposure's validated closed
  projection of the run result, with no field whose effective level (the inherited output levels of
  Requirement 025-019, recorded in the target version) is above `maxResultClassification` and none
  `secret`, checked when the exposure is saved and again at call time against the version the call
  resolves; a caller without the rights for the declared level is refused before admission (answered
  as an unknown tool). Every returned result is audited; arguments and results never enter logs, spans,
  metrics or audit reasons.
- **FR-025-033 — MCP limits and errors**: The server MUST bound request bytes, wait time, concurrent
  calls and rate per namespace and per token, and map platform errors to MCP protocol or tool
  execution errors as in `contracts/mcp-server.md`, without internal diagnostics. A JSON-RPC batch
  (an array body) is refused with `-32600` before any member is read, so the per-call rate limit counts
  every call. Remote callers cannot exhaust the namespace's AI budget: the live runs admitted by one
  caller whose target version can contain agents are bounded (`mcp_agent_runs_per_caller`, default 4,
  ceiling 64, counted in the admission transaction; beyond it the call is refused with `QUOTA_EXCEEDED`),
  and an exposure's optional `maxDailyTokens` is charged by every model-call reservation of a run it
  admitted and of that run's descendants, through the usage leases of Requirement 025-015.

#### MCP client

- **FR-025-034 — External MCP server resources**: `ai:manage` holders MUST be able to register,
  revise, pause and delete external MCP servers (HTTPS URL, protocol preference, a required
  `maxArgumentClassification` of `public`, `internal` or `pii` (`pii` under the same reveal-grant rule as
  FR-025-005), authentication none or static bearer secret, limits, and `toolClassifications`: the
  classification `read` or `write` of each tool by name, set by the saving `ai:manage` holder and pinned
  with the revision; a tool it does not name is `write`), as a 021 resource type (Owner decision
  2026-11-26-122). The classification, never the server's annotations, decides whether an `mcp` tool may
  run without approval (Requirement 025-010, Decision 025-018). The static bearer secret is a declared secret of the task definition as in
  Requirement 025-003. OAuth client credentials with a resource indicator rest on the third declared set of Principle V that the owner approved for constitution 5.1.0 (Owner decision 2026-11-25-3, pull request 286: a declared resource's settings and its current access token), and are refused at save until
  5.1.0 merges and design 047's broker is built
  (`MCP_AUTH_MODE_UNAVAILABLE`, Gap 025-001); when enabled, the revision records the issuer and token
  endpoint, confirmed by the saving `ai:manage` holder, and tokens come only from design 047's broker
  (D1306). User-delegated authorization is not supported (G1288).
- **FR-025-035 — Tool snapshots**: Refresh MUST be a durable runner job (origin `MCP_TOOL_REFRESH`)
  storing an immutable, bounded snapshot of tool names and description, input- and output-schema
  digests. Publication pins the snapshot: the published version records the snapshot identifier and the
  MCP-server revision, and admission copies them into `run_mcp_snapshot_pins` (Requirement 025-002), so
  a later refresh changes nothing a published version reads or compares against; at call time a
  changed definition fails with `AI_TOOL_CHANGED` without calling. Descriptions and annotations are
  untrusted content; the snapshot keeps the annotations `readOnlyHint` and `destructiveHint` as hints.
  A refresh, and a publication, that finds a tool whose annotation disagrees with the classification of
  the revision it pins (`readOnlyHint: true` on a tool classified or defaulted `write`, or
  `destructiveHint: true` on one classified `read`) records an audit event
  `ai.mcp.tool_classification_mismatch` with the server, revision, snapshot and tool name, and the
  publication proceeds under the pinned classification.
- **FR-025-036 — MCP call execution**: An MCP tool call MUST be the reserved activity `sagat.ai.mcp`
  (a managed step of `ManagedKind::AiMcp`) run by the agent runner with `ResolveMcpCall` under the same
  two phases and Principle V authorities as Requirement 025-009, at-least-once, bounded in time and
  bytes, keeping text and structured content only (D1317), scanning results for resolved secrets,
  returning `input_required` and task results as `MCP_UNSUPPORTED_RESULT`, and reporting a timeout
  after sending as `OUTCOME_UNKNOWN`. A server's `isError: true` result reaches the model as the reason
  `MCP_TOOL_ERROR`; its text (bounded to 4 KiB, secret-scanned, marked untrusted) has no carrier and so
  counts as `pii`: it is forwarded only when the agent's transcript level is already `pii`, otherwise
  the model receives the reason alone (Decision 025-008). For OAuth servers, once enabled (Gap 025-001),
  the call is refused when the server advertises an issuer or token endpoint other than the recorded
  ones (`AI_MCP_AUTH`, audit `mcp.oauth_metadata_changed`).

#### Streaming, Studio and assistant

- **FR-025-037 — Token streaming**: With `stream.tokens: true` and feature 026, the runner MUST append
  coalesced text-delta, tool-request and turn-end events to the reserved channel `sagat.agent.tokens`
  through 026's `AppendStreamEvents` under its lease (FR-026-013, FR-026-014), with the channel schema
  of `contracts/agent-stream-events.md` (text `pii`; a `tool_requested` event names a declared tool or
  the marker `UNKNOWN_TOOL`, never the name the model wrote). The run's reserved-channel budget is the
  effective `maxTurns` × the per-call event and byte caps, clamped to 026's per-run hard ceiling for
  reserved channels and counted apart from author channels (D1307); 026 marks superseded attempts and
  projects events per reader (FR-026-016, FR-026-018, G1295). Deltas are not authoritative and never
  include thinking. Without 026, publication refuses the declaration.
- **FR-025-038 — Studio**: Studio MUST provide provider, MCP server, exposure and token screens; an
  agent node editor emitting the same canonical IR as the SDKs; a run view of turns, tool calls,
  approvals and estimated usage from the journal-derived graph with its lag; a gated transcript viewer
  fetching on explicit action into ephemeral state; and tool-call approval decisions; design tokens
  only, WCAG 2.2 AA with a table alternative, no sample values (FR-007-010).
- **FR-025-039 — Authoring assistant (P3)**: The assistant MUST run as an internal durable run
  (origin `AUTHORING_ASSISTANT`) on a configured provider whose `maxPromptClassification` is at least
  `internal` (draft content and the request text count as `internal`, D1308), with no tools, receive
  only the current draft, the IR schema and metadata the requester may read (never run payloads or
  secrets), return a proposal validated by the existing validator, and change nothing until the
  requester applies it as a draft write. It cannot publish, activate, start runs or approve.

#### Cross-cutting

- **FR-025-040 — SDKs and conformance**: The six official SDKs (TypeScript, Go, Python, C#, Java and
  Rust, `sdks/rust`) MUST provide agent node and tool builders that emit byte-identical canonical IR,
  client calls for transcript reading and tool-call approvals, and pass the shared scenarios of
  `contracts/conformance.md` against the real engine and the dev provider.
- **FR-025-041 — Observability and audit**: Model calls, tool calls, approvals, MCP calls, probes and
  refreshes MUST emit OTel spans (journal-derived per ADR-0017, or runner spans), fixed-label metrics
  (no tenant, run, model or tool ids), redacted logs, and audit records committed with their decision
  for catalog changes, credential resolution, budget exhaustion, tool denials, transcript reads,
  approval escalations and approvals ended without a decider (`ai.approval_escalated`,
  `ai.approval_no_decider`), MCP tool classification mismatches (`ai.mcp.tool_classification_mismatch`),
  exposures, token lifecycle and MCP calls. Every new audit action needs its row in the audit-to-OCSF
  mapping of design 040 (`docs/designs/040-ocsf-event-standard.md`, Technical Plan 1: one arm per
  action in `crates/domain/src/audit_ocsf.rs` once that design is built).
- **FR-025-042 — Typed errors**: Errors MUST use the closed `Error.code` set with new `details.reason`
  values and durable failure codes from `contracts/errors.md`, each with a remediation row,
  `retryable` and trace id, never a prompt, response, argument, credential or provider message. The
  durable failure codes of managed steps are added to `sdks/error-catalog.json` as `worker`-origin
  entries and to `sdks/SPEC.md` section 2, as the Iceberg codes of feature 035 were, and every SDK's
  embedded copy follows.
- **FR-025-043 — Contracts and compatibility**: OpenAPI, IR, worker protocol and any AsyncAPI change
  MUST be additive; older workers and SDKs never receive agent tasks; rollback disables new agent
  publication and admission while compatible readers drain existing runs. The capabilities
  `ai-agents-v1` (control plane and execution engine) and `mcp-server-v1` (control plane) are added to
  the closed capability list of the instance registry (Architecture decision 0082), so a fleet with a
  process that cannot serve agents or MCP cannot have them required.
- **FR-025-044 — Tenant boundary**: Tenant and namespace MUST come from the authenticated context on
  every row, query, task, credential path, MCP call and audit record; another tenant's provider,
  server, exposure, token, run or transcript is an opaque `NOT_FOUND` and the probe is audited.
- **FR-025-045 — Delivery**: The delivery MUST ship `apps/dev-model-provider` (development only, not
  in production images), runner images and Helm values with default-deny egress, runbooks for
  providers, the agent runner and MCP, and a reproducible happy and failure demo (quickstart.md).
  Real-provider tests are opt-in and never the only evidence of a guarantee.

- **FR-025-046 — Reset rules**: Every journal event type added here MUST have its arm in the reset-rule
  registry of feature 018, which is code (`crates/domain/src/execution/reset.rs`, the `registry!` macro;
  an event without a rule does not compile): every event of an open agent loop (`agent.started` to
  `agent.budget_exhausted`) is `ResetRule::Refuse { until: Some("agent.finished") }`, so no point
  between them and the `agent.finished` of the same agent node is offered; `agent.finished` is
  `ResetRule::Rematerialize`, re-creating the finished step's agent rows in the new run without
  repeating any call or charging any budget (D1635). Every agent event names the agent node, not a
  derived id, so the window closes on the right step.
- **FR-025-047 — Encoded fields**: Agent context expressions, tool-result projections, tool inputs,
  `child` tool inputs, the agent result and MCP exposure projections MUST be registered in 031's
  position registry and checked with 031's codes (`ENCODING_READ_BY_EXPRESSION`,
  `ENCODING_CARRIAGE_MISMATCH`, `ENCODING_NOT_PRODUCIBLE`); an encoded value is never sent to a model
  or an MCP server, and a model-written value never fills an encoded location (D1636).

#### Amendment to feature 014

025 owns the single additive amendment of 014's `approval_requests` (owner kind `AGENT_TOOL_CALL`,
derived owner id, nullable routes for that kind, data-model.md §4); feature 026 cites it, and its
notifications and decision links apply to every owner kind, including tool calls (coordinator C22).
Task T025-032. Feature 014 has no escalation ("no quorum, delegation or escalation",
`specs/014-workflow-primitives/research.md`); the escalation of a tool-call approval nobody can decide
with context (Requirement 025-017, Decision 025-019) is part of this same amendment and applies only to
the owner kind `AGENT_TOOL_CALL`: nullable columns `escalation_policy` (the eligibility policy of the
`escalation` route, recorded from the published version), `escalated_at`, `context_checked_at` and
`no_decider_reason` (only `NO_DECIDER_WITH_CONTEXT`), written by the opening transaction, the decision
path and the approval sweeper. One eligible decision still settles a request; escalation changes who is
eligible, never how many decide. Tasks T025-096 and T025-103.

#### Amendment to feature 016

FR-016-002 lists its carriers as an exact set. This feature adds, each for one surface only: the
generated carrier of an agent node's retained output (`result` fields at least the transcript level,
metadata `internal`; D1291); a tool's `inputSchema` for the approval view of that tool call (fields
at least the transcript level; D1292); and a tool's `outputSchema` as a classification input of the
transcript level only, never a disclosure surface. The schema of the reserved stream channel
`sagat.agent.tokens` is a 026 stream-channel carrier fixed by this feature. The first parity feature
implemented opens the one consolidated amendment of FR-016-002 (coordinator C6); task T025-075.

### Non-functional requirements

- **NFR-025-001 (security)**: zero tested cross-tenant access, zero canary leakage of secrets or
  credentials into prompts, transcripts, journal, logs, spans, audit, streams or responses, and no
  token accepted from a query string or for another audience. Release-blocking. The canaries are
  verbatim, JSON-escaped, base64 and hexadecimal forms of values of at least 8 bytes, the forms the scan
  detects (Requirement 025-020); other transformations are outside the claim (Gap 025-004).
- **NFR-025-002 (durability)**: committed turns, tool calls, approval decisions and MCP admissions
  survive process and broker restart; duplicates create no second transition; effects are
  at-least-once with stable keys; no exactly-once claim.
- **NFR-025-003 (cost honesty)**: every cost is labelled estimated and comes from declared prices;
  reservations never fall below the documented bound; unknown usage is charged its reservation.
- **NFR-025-004 (privacy)**: no value above the declared level of its destination leaves toward a
  model provider or an external MCP server, under the destination's current ceiling; model-written
  values are never classified below the transcript level in any output carrier they reach, through
  workflow outputs and child outputs included; the transcript is disclosed only through its audited
  endpoint, and no run-read surface carries a string the model or the provider wrote.
- **NFR-025-005 (compatibility)**: existing definitions, runs, workers and protocol minors keep their
  semantics; the agent capability is advertised only after six-SDK conformance passes.
- **NFR-025-006 (observability)**: every accepted model, tool and MCP call has persisted correlation
  and bounded OTel evidence; export failure never decides a commit.
- **NFR-025-007 (performance)**: activation, turn scheduling, resolution, runner overhead per call and
  MCP admission are measured (p50/p95/p99) on a declared environment with the dev provider; nothing is
  promised before measurement.

### Key Entities

- **Provider resource / revision**: identity, name, status; immutable kind, endpoint settings,
  credential references, allowed models with prices and capabilities, `maxPromptClassification`, limits.
- **Run provider binding**: the provider revision recorded per agent node in the published version and
  copied into the run at admission.
- **Agent step**: run, node, status, effective budgets, reserved and settled usage, turn counter,
  transcript head, deadline.
- **Turn / attempt reservation**: model-call derived step, request digest, response reference, usage,
  stop reason, reported model; one reservation per attempt.
- **Tool call**: turn, index, declared name (or the marker `UNKNOWN_TOOL`), kind, input, status,
  approval, derived step, result, effect outcome; the provider's tool-use id is transcript content.
- **External MCP server / revision / tool snapshot**: URL, `maxArgumentClassification`,
  authentication with recorded issuer and token endpoint, credential references; immutable tool
  digests.
- **Exposure / revision**: tool name, target, version policy, projection, disclosure, level, optional
  daily token budget.
- **Scoped MCP token**: hash, minter, namespace, scope (exposure identifiers), expiry, revocation.
- **MCP call receipt**: caller (the credential identity), exposure, idempotency key, argument digest,
  admitted run.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All criteria are future evidence; none is met by this document.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-025-001 | Real browser registers a provider with a new OpenBao credential, reloads without values, probes, and distinguishes save, pause and probe; another tenant cannot discover it | FR-025-001..005, 038, 044 |
| SC-025-002 | Real PostgreSQL, agent runner and dev provider: an agent run returns a validated structured result; a crash after the provider answered and before acknowledgement yields one committed turn and two charged attempts | FR-025-006..009, 013..015 |
| SC-025-003 | Crash at every commit point of activation, model completion, tool completion, approval decision and deadline leaves no orphan step, no duplicate turn and no lost acknowledged transition | FR-025-008, 012, 017, 018 |
| SC-025-004 | Tool calls of every kind run through their own executor with the stable key; duplicate delivery and lost broker hints change nothing; unknown write outcomes reach reconciliation or manual review | FR-025-010..012, 025 |
| SC-025-005 | Tool-call approval: unauthorized, revoked, cross-tenant and initiator decisions denied and audited; the redacted view masks every field the decider's rights withhold (path and level only); a decider to whom a decision field would be withheld is refused `APPROVAL_INPUT_WITHHELD` and nothing is applied; narrowed `decisionFields` on an `operation` or database `execute` tool are refused at publication; an approval no eligible decider can decide with context is escalated, and with no decider on the escalation route ends at its deadline with the recorded reason `NO_DECIDER_WITH_CONTEXT`, audited; decision versus expiry, versus escalation and versus cancellation pick exactly one outcome over 50 rounds each | FR-025-017, 018 |
| SC-025-006 | Budget, quota and ceiling exhaustion refuse before dispatch; every retry reserves in its claim (lost response then retry keeps settled + reserved ≤ limit for the agent and the daily row); parallel completions schedule exactly one next turn; killing a runner releases concurrency when its leases expire | FR-025-015, 016 |
| SC-025-007 | `pii`, `secret`, unmarked and non-validating values are refused at publication or before dispatch per the provider's and MCP servers' levels, and again after a ceiling is lowered without republication; looser `result`, approval-view, child-input, workflow-output and child-output annotations below an inherited level are refused (a child under an active-version policy at its admission); a `secret` tool input field is refused; secret (verbatim, JSON-escaped, base64 and hexadecimal) and MCP-argument canaries appear in no prompt, MCP request, transcript, journal, log, span, audit, stream or response; `write` tools without approval or acknowledgement, `child` tools declared `read` without approval, `mcp` tools declared `read` without approval on a tool the pinned MCP-server revision does not classify `read` (whatever the snapshot's annotations say, a mismatch being audited), and database `execute` tools without a policy are refused | FR-025-010, 014, 017, 019..021, 026 |
| SC-025-008 | Transcript endpoint: each combination of the three rights, revocation of the reveal grant between requests, no-store headers and per-read audit; audit failure returns no bytes; the `payload-disclosures` and `payload-inspections` endpoints refuse agent payloads; 256 undeclared tool requests leave only `UNKNOWN_TOOL` markers on run-read surfaces | FR-025-022, 041 |
| SC-025-009 | Metadata, loopback, link-local, private and redirect destinations refused for providers and MCP servers, a `302` on the probe and on the refresh included (no credential reaches the redirect target); the event-stream reader enforces bytes and deadlines; a namespace absent from the runner's configuration is denied | FR-025-023, 036 |
| SC-025-010 | Ordinary workers, sandbox, database and lake runners cannot claim or resolve agent-runner activities; the agent runner cannot claim ordinary tasks, is refused by the step-log ingest, by `ResolveSecrets` for every managed lease and never receives a database credential, a client secret or a signing key; an ordinary activity named `sagat.ai.*` is refused at publication, blocks the recording of `ai-agents-v1` when it predates it, and is never leased to the agent runner; stale leases, expired sessions and a publisher who lost `secret:use` are refused | FR-025-003, 009, 024 |
| SC-025-011 | MCP server tested with an MCP client library at 2026-07-28 and 2025-11-25: empty default list, single-audience tokens and REST refusal of MCP tokens, uniform metadata answers, insufficient scope, scoped tokens and revocation (also during a bounded wait), idempotent retry per credential and exposure (another token's key never replays), tasks re-authorized on every `tasks/get`, foreign and uncallable exposures answered as unknown tools, batches refused, unattended child versions refused at exposure and at child admission, the per-caller agent-run limit and an exposure's daily token budget | FR-025-026..033 |
| SC-025-012 | MCP client against the test server: snapshot pinning at publication (a later refresh changes nothing a published version compares against), drift refusal, egress denial, unknown outcome to manual review, unsupported results and `MCP_TOOL_ERROR` as tool errors (text forwarded only at transcript level `pii`), the `oauth-client-credentials`, `azure-entra-client-credentials` and `aws-access-key` modes refused while Gap 025-001 is open (and, once design 047's broker is built, refusal on a changed token endpoint and a runner that receives only an access token: an Entra access token, or an Amazon Bedrock short-term API key minted from a short-lived STS session) | FR-025-003, 034..036 |
| SC-025-013 | With feature 026, a 16-turn streaming agent stays within its reserved-channel budget, a resumed stream matches the committed transcript and superseded attempts are marked; without 026, publication refuses streaming | FR-025-037 |
| SC-025-014 | All six SDKs (Rust included) build byte-identical agent definitions and pass the shared runtime scenarios | FR-025-040, 043 |
| SC-025-015 | Telemetry carries only closed label values; spans and logs carry no prompt, response, argument, credential or provider message; every audit action is written with its decision | FR-025-041, 042 |
| SC-025-016 | Assistant proposals never create a version, activation or run; failed validation changes no draft | FR-025-039 |
| SC-025-017 | Reproducible Compose demo of the happy path and of budget-exhaustion and runner-crash failures, with measured latency and resource figures on a declared environment | FR-025-045, NFR-025-007 |
| SC-025-018 | Reset points inside an open agent loop are refused; a reset after a finished agent step re-materializes its rows without a provider or tool request and without budget or daily-usage charges; encoded-field fixtures are refused with 031's codes | FR-025-046, 047 |

## Assumptions and dependencies

- **021** (Architecture decision 0055, proposed): providers and MCP servers are its platform-provided
  resource types, whose executor is the agent runner, and `operation` tools are its
  `resourceOperation`s run by its integration runner (Owner decision 2026-11-26-122 answering Owner
  question 1282). The provider and MCP-server catalog work starts after 021's resource core merges; the
  other work proceeds with a test provider; `operation` tools stay refused until 021 offers them (G1281).
- **022**: standalone steps gate script and operation exposures (D1315).
- **026**: Workflow Streams (FR-026-013..021, `AppendStreamEvents`, reserved `sagat.` channels) gate
  token streaming; `executeRun` semantics (FR-026-007..009) are reused by the MCP wait (G1282).
- **014**: approvals are reused through an additive owner kind (G1289), with the escalation columns of
  that same amendment for tool calls (Decision 025-019); scoped instance identity is not used, so agents
  stay root-only.
- **016** levels, carriers and reveal grants and **017** OpenBao facade, credential pins, private
  runner policy and private definitions are reused as they are.
- **013** (federated login, draft) may later issue MCP tokens; until then the configured OIDC issuer
  of 001 does. **029** custom roles and folders would allow narrower MCP audiences (G1290); when 029 is
  enabled its evaluator decides MCP visibility and `agent-runner` is an additive built-in role.
- **018** is merged: its reset-rule registry is code (Requirement 025-046) and Architecture decision
  0050 (accepted) already names 025's lock slots, 4b for provider and MCP-server heads and 8b for the
  daily usage row. The `pii`-egress rule of Owner question 1081 is decided (Decision 1617 of feature
  020). **020** content-store registry, **028** time classes, **031** position registry
  (Requirement 025-047) and **032** key inventory are registries owned by drafts; 025 registers its
  entries when they exist (D1635–D1638, G1635).
- **041** owns worker-protocol minor 1.7 (step logs); 025 takes the next free minor of the register in
  `sdks/SPEC.md` section 9 when its contract merges (Owner decision 2026-11-15-8, G1280). **043** owns
  the run-input gate and validator that MCP admission and agent schemas reuse. **038** owns the claim
  and transition statement budget that agent claims and completions are measured against. **040** owns
  the audit-to-OCSF mapping. **047** owns the OAuth token broker that OAuth MCP servers wait for, and
  the broker routes of the `azure-entra-client-credentials` and `aws-access-key` provider modes (Gap
  025-001, Decision 025-017). **044** (not yet written) owns the procedural model (Gap 025-002). Model ids are the
  provider's; the platform interprets them only against the revision's allowlist.

## Out of scope

Image, audio and file modalities; provider-side server tools, MCP connectors and model fallbacks;
memory across runs; user-delegated OAuth for external MCP servers; the client side of the MCP tasks
extension and `input_required`; MCP resources, prompts, sampling and roots; fine-tuning, embeddings
and batch inference; billing reconciliation; a connector or tool marketplace (constitution, Delivery
Constraints; since 5.0.0 an open marketplace stays out and only the curated hub of 021, 028 and 047 is admitted); agents in loop or collection bodies or with continue-as-new; publishing by the assistant.

## Owner questions

The first three are decided under the owner's delegation (Owner decision 2026-11-24-4): Owner question 1280
(data sent to providers) by Owner decision 2026-11-26-120, Owner question 1281 (role grants) by Owner
decision 2026-11-26-121 and Owner question 1282 (resource model) by Owner decision 2026-11-26-122; see
[research.md](research.md#5-owner-questions), with decisions D1280–D1319, D1635–D1638 and
D025-001 onward, and gaps G1280–G1297, G1635 and G025-001 onward. See the
[requirements checklist](checklists/requirements.md).

Owner question 025-001 (raised 2026-11-27 by the security review's Major 9) asked whether Principle
V's first declared set may cover the client secret or signing key of a declared feature-021 resource
(an Entra client secret, an AWS secret access key), which constitution 5.1.0's third declared set
excludes for resources. It is decided, under the owner's delegation, by Owner decision 2026-11-27-400:
no. The two modes reach the runner only as access tokens issued by design 047's broker (Owner
decisions 2026-11-27-400 and 2026-11-27-401, Decision 025-017), and until that broker exists they are
refused (Gap 025-001). The one security residual risk the review left, the bound of the output secret
scan (Gap 025-004), is accepted by the owner as a documented known limit (Owner decision
2026-11-27-2).
