# Feature Specification: AI agents and MCP

**Feature Branch**: `feature/025-ai-agents-mcp`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: LLM provider resources (Anthropic, OpenAI-compatible, AWS Bedrock, Azure); a durable AI
agent step whose model calls and tool calls are durable attempts run by a managed executor, with
tools (published scripts, activities, managed operations, child workflows, MCP tools), structured
output, budgets (turns, tokens, cost, wall time) and human approval for sensitive tools; an MCP
server exposing declared workflows, scripts and operations as tools with OAuth or scoped tokens,
per-tool authorization, audit and declared result disclosure; an MCP client so agent steps can use
external MCP servers; token streaming through feature 026 Workflow Streams; and, at P3, a Studio
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
  feature-017 model (ADR-0035, ADR-0036) and under the placement rule that 021 writes in ADR-0055
  (coordinator C3 (a)): a request whose content a tenant authored in a workflow, such as a model
  prompt or a tool call, is sent only by a managed executor holding only that attempt's capability.
  The control plane and scheduler never call a provider or an external MCP server.
- Effects stay at-least-once (ADR-0004). A model call has no provider idempotency key: a lost
  response is asked again and paid twice. A tool call carries a stable key.
- Approvals use feature 014 (FR-014-007, FR-014-008); values follow ADR-0033; the transcript has its
  own audited disclosure; secrets are references resolved per attempt (ADR-0035), never prompt text.
- The MCP server and client follow
  [ADR-0060](../../docs/adr/ADR-0060-mcp-exposes-declared-tools-and-calls-pinned-tools.md): nothing
  is exposed by default, results leave only through a declared classified projection, and external
  tools are called only from pinned, allow-listed snapshots.

Depends on existing features 001–003, 005–009, 011, 014, 016 and 017, and on parallel drafts 021
(resource types, managed operations), 022 (standalone steps) and 026 (Workflow Streams, `executeRun`);
it registers in registries owned by drafts 018 (reset rules, ADR-0050 lock slots), 020 (content stores,
`pii` egress), 028 (time classes), 029 (access evaluator), 031 (encoded positions) and 032 (key
inventory). Nothing here claims those drafts are implemented.

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
5. **Given** a provider edited, paused or with a model removed after admission, **when** a retry
   runs, **then** it keeps the pinned revision; a paused provider refuses new resolution; a run is
   never redirected.

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
   and the model receives a bounded error result counted toward `maxToolCalls`.
3. **Given** a `write` tool with an unknown outcome, **when** it declares `reconcileWith`, **then** it
   is looked up before any retry; otherwise it waits in manual review and the agent does not
   continue until an operator resolves it. A `read` tool is retried.
4. **Given** an agent in a saga region that compensates, **when** compensation runs, **then** tool
   calls with `compensateWith` are undone in reverse completion order and a `write` tool without
   compensation ends the run `PARTIAL`.

### US03 — Require a person's approval before a sensitive tool runs (Priority: P1)

The `issue_refund` tool requires approval by the `operator` role. An eligible person sees the tool
input, as far as their classification rights allow, and decides.

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
   tool-call view, **then** fields above their level are withheld, every field being at least the
   agent's transcript level; a `secret` field is shown to nobody.

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
   **then** it is not sent and the call fails with `AI_PROMPT_CLASSIFICATION`; a value with no carrier
   counts as `pii`.
3. **Given** a tool output containing a secret its attempt received, **when** it completes, **then**
   the tool call fails with `SECRET_IN_OUTPUT` and nothing is appended to the transcript.
4. **Given** a user with `run:read` only, **when** they inspect the run, **then** they see turns, tool
   names, statuses and usage, never values; a holder of `payload:read-internal` and the namespace
   reveal grant reads the transcript through an audited `no-store` endpoint.
5. **Given** an agent whose transcript level is `pii` (a context field or a tool result is `pii`),
   **when** its `outputSchema` annotates `result.summary` as `public`, **then** publication fails with
   `CLASSIFICATION_LOOSENED`; a model cannot relabel what entered its transcript.

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

### US07 — Use tools of an external MCP server (Priority: P2)

An administrator registers an external MCP server with OAuth client credentials and refreshes its
tools; an author allows two of them in an agent node.

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
  expose it through MCP or a 026 public link: both refuse with `UNATTENDED_WRITE_TOOL`.
- An external MCP server starts advertising another token endpoint: the next call is refused before
  the client secret is sent, and the change is audited.
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

## Requirements *(mandatory)*

Defaults and hard ceilings are in [plan.md](plan.md#finite-ceilings). Effective values are the
minimum of declaration, namespace policy and deployment ceiling; zero is not unlimited; admission
fails before work.

### Functional Requirements

#### Provider resources

- **FR-025-001 — Provider catalog**: The platform MUST provide namespace-scoped create, list, detail,
  update, pause, probe and delete for provider resources of kinds `anthropic`, `openai-compatible`,
  `aws-bedrock` and `azure-openai`, as a feature-021 resource kind when accepted, otherwise as a
  self-contained catalog with the same rules and a migration path (D1283). Settings are non-secret
  (HTTPS endpoint, region, deployment map, API version); credentials are secret references; unknown
  fields and credentials in settings or URLs are rejected. Resources have stable identity, a
  namespace-unique name and immutable revisions with optimistic concurrency; deletion is a tombstone
  that keeps referenced revisions.
- **FR-025-002 — Allowed models and pinning**: A revision MUST list allowed models with the
  provider's id, optional alias, maximum output and context tokens, capability flags (native
  structured output, parallel tool calls, streaming) and optional declared prices. Publication
  rejects other models. Admission pins the revision per agent node in the admission transaction;
  every attempt uses it. Pause and deletion block new admission and resolution; current revocation
  stays authoritative for pinned runs.
- **FR-025-003 — Credentials**: Credential values MUST live only in OpenBao behind the feature-017
  facade and be resolved per attempt: API-key header or bearer (`anthropic`, `openai-compatible`),
  access key pair with optional role (`aws-bedrock`), API key or Entra client credentials
  (`azure-openai`). No fallback; a deployment's own cloud identity is never used for a tenant
  account; adapters use explicit static credentials only, never SDK default credential chains or
  instance-metadata lookups (D1299).
- **FR-025-004 — Provider probe**: Test provider MUST admit a fixed durable probe (origin
  `AI_PROVIDER_PROBE`) run by the agent runner on the same destination, TLS, credential and quota
  path as a model call, calling a model-listing endpoint or equivalent. It generates no tokens and
  records a safe category, tested revision and time; success is evidence at that instant only.
- **FR-025-005 — Data policy**: A revision MUST declare `maxPromptClassification` (`public`,
  `internal` or `pii`), with no default; `secret` is not a value (D1289, O1280). Declaring `pii`
  requires the saving administrator to hold the namespace reveal grant; the grant is rechecked at
  every resolution whose transcript level is `pii`, its revocation pauses the provider, and each such
  attempt is audited `ai.pii_egress` (D1637, the uniform rule of 020's O1081).

#### Agent node and loop

- **FR-025-006 — Agent node**: The IR MUST gain an additive `agent` node (ADR-0032) with `model`,
  literal `instructions` and `prompt`, named `context` expressions, `tools`, optional closed
  `outputSchema`, `budgets`, `modelOptions` (`effort`, `maxOutputTokens`), `retry`, optional `stream`,
  `timeoutMs`, `next` and `onError` (`contracts/agent-node.schema.json`). It is allowed only in the
  root graph and not with `chainPolicy` (D1281), refused unless the agent capability is enabled
  (FR-014-016); unknown fields are rejected.
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
- **FR-025-009 — Model call execution**: A model call MUST be the reserved activity `sagat.ai.model`,
  claimable only by a current `agent-runner` bound to the namespace and resolved by a two-phase
  `ResolveModelCall` that checks lease, session, pinned revision, provider status and the publisher's
  and initiator's rights before and after OpenBao I/O, commits the resolution audit and a
  metadata-only credential pin before returning, and holds no lock during provider I/O. Provider
  failures map to the closed vocabulary of `contracts/errors.md`; retries honour `retry-after`
  within the attempt deadline.
- **FR-025-010 — Tool declarations**: Each tool MUST declare a name matching `^[a-zA-Z0-9_-]{1,64}$`
  unique within the node, a literal description, a kind (`activity`, `script`, `database`,
  `operation`, `child`, `mcp`) with its reference, a closed bounded input schema (from the target
  when it declares one), `effect` (`read` or `write`, default `write`), an optional result
  projection, an optional closed `outputSchema` (the result's carrier when the target has none),
  bounded retry and timeout, and an approval policy. A `write` tool MUST declare either an approval
  policy or the explicit acknowledgement `approval: "none"`; `operation` tools always need a policy
  (G1293). A `database` tool is a parameterized `query` bound only from tool arguments; `execute`
  requires approval. An `mcp` tool names a server resource and a tool of its pinned snapshot.
- **FR-025-011 — Tool call validation**: An undeclared tool or input failing its schema MUST NOT
  execute; the model gets a bounded error result. Requests beyond the per-turn limit or the remaining
  `maxToolCalls` get refusal results. A turn's results return in request order in one message.
- **FR-025-012 — Tool execution**: Each valid call MUST be a derived step run by its kind's executor
  (SDK worker, sandbox runner, database runner, 021 executor, child run, agent runner for MCP) with
  key `agent:{runId}:{node}:{turn}:{call}` (D1296). `read` tools retry; a `write` tool with an unknown
  outcome uses `reconcileWith` when declared, otherwise manual review. `child` tools use the existing
  child semantics and tree budget. Results are bounded, scanned for resolved secrets, projected and
  checked against the provider's level before they are appended.
- **FR-025-013 — Structured output**: With `outputSchema`, the platform MUST validate the final
  answer against the closed bounded schema, use the adapter's native constraint where the model entry
  declares it (never forced tool choice), send at most `maxRepairTurns` bounded validation summaries
  back, and fail with `AI_OUTPUT_INVALID` when no valid answer fits the budget.
- **FR-025-014 — Stop reasons and output**: An answer without tool requests is final. `refusal` fails
  with `AI_MODEL_REFUSED`, a `max_tokens` stop without a complete answer or request with
  `AI_OUTPUT_TRUNCATED`; both non-retryable and routable by `onError`. The retained output is
  `{result, usage, stopReason, turns, toolCalls}` with a generated carrier in which every `result`
  field is at least the transcript level (FR-025-019) and the metadata is `internal`; a looser author
  annotation is refused at publication with `CLASSIFICATION_LOOSENED` (D1291). It is retained under
  the content quota and as a blob claim-check above the inline limit.
- **FR-025-015 — Budgets**: An agent step MUST enforce `maxTurns`, `maxToolCalls`, `maxInputTokens`,
  `maxOutputTokens`, optional `maxCost` and `timeoutMs`. Every model-call attempt, first or retry,
  takes its conservative reservation (D1286) against the agent's budgets and the namespace's daily
  row in the transaction that grants its lease; a reservation that does not fit fails the step with
  `AI_BUDGET_EXHAUSTED` before any lease reaches a runner. Completion settles reported usage; a
  provider error response releases the reservation; a lost response, timeout or lease expiry keeps it
  as charged. `maxCost` needs declared prices; cost is labelled estimated.
- **FR-025-016 — AI quotas**: The platform MUST enforce namespace and tenant concurrent model calls
  (the count of live `sagat.ai.*` leases, read in the claim transaction, so completion and expiry
  release capacity), namespace tokens and estimated cost per UTC day (settled plus reserved), and
  counts of providers, MCP servers, exposures and scoped tokens, through ADR-0027 resolution. Over a
  limit, claim or admission waits or fails before any provider request; a lowered limit does not
  cancel dispatched work.
- **FR-025-017 — Tool approval**: A tool with an approval policy MUST open an approval owned by its
  tool call in the transaction committing the model call, with feature-014 eligibility,
  decision-time authorization, deduplicated receipts, initiator exclusion and first-commit
  resolution against expiry and cancellation (D1297). `onReject`/`onExpire` are `refuse` or `fail`.
  The approver's view applies ADR-0033 with the tool's input schema as carrier, every field at least
  the transcript level (a looser annotation is refused at publication, D1292); the deadline is
  clamped to the remaining agent wall time.
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
  whose child input schema declares a field below it. At run time a value that does not validate
  against its carrier is not sent: the call fails with `AI_PROMPT_CLASSIFICATION` before dispatch (a
  tool result in that state returns `TOOL_FAILED` to the model).
- **FR-025-020 — Secrets never in prompts**: Secret values MUST NOT reach a rendered request,
  transcript, journal, log, span, audit record, stream event or response. Rendering has no secret
  capability; provider and MCP credentials exist only in the runner's transport; tool outputs are
  scanned for the values their attempt received (`SECRET_IN_OUTPUT`); canaries cover every surface.
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
  bulk export.

#### Execution boundary

- **FR-025-023 — Egress**: Provider and MCP destinations MUST use the guarded egress client and the
  namespace egress policy, with no redirects for non-GET, no environment proxy, bounded time and bytes
  and a bounded incremental reader for event streams. Private destinations need the operator-approved
  runner policy (ADR-0035 decision 4); metadata, link-local, loopback and platform endpoints stay denied.
- **FR-025-024 — Managed executor isolation**: The agent runner MUST be a separate process with
  machine role `agent-runner` (`worker:execute`, `ai:execute`), a current namespace binding and the
  next free additive protocol minor (G1280). It holds no platform database URL, OpenBao token or
  signing key, receives only the current attempt's capability, drops credentials when the attempt
  ends and cannot claim ordinary tasks; ordinary workers cannot claim or resolve its activities.
- **FR-025-025 — Sagas**: An agent in a saga region MUST compensate completed tool calls in reverse
  completion order with each tool's `compensateWith`; a `write` tool without compensation is
  irreversible and compensating past it ends the run `PARTIAL`; a failing compensation waits for a
  person.

#### MCP server

- **FR-025-026 — Exposures**: `mcp:expose` holders MUST be able to create, revise, disable and delete
  exposures naming a tool name, a target (a published workflow with an active- or pinned-version
  policy; script and operation targets only with feature 022 standalone steps, D1315), the input
  schema from the target, a closed result projection with `disclosure: "declared-mcp-result-v1"` and a
  `maxResultClassification`. Nothing is exposed by default. An exposure, and a call at run time, MUST
  refuse a target version that contains a `write` tool acknowledged with `approval: "none"`
  (`UNATTENDED_WRITE_TOOL`); feature 026 public links must apply the same refusal (T025-076).
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
  effective permission is the minter's current permissions ∩ scope at every call. Lifecycle and
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
  `sagat-octopus-orchestrator.io/idempotencyKey` (same key and arguments return the same run; changed
  arguments conflict). After a bounded wait a non-terminal run is a task for clients with the tasks
  extension or a run handle otherwise; the wait reuses feature 026 `executeRun` semantics
  (FR-026-007, FR-026-009) when available. `tasks/get` and `tasks/cancel` (which also needs
  `run:operate`) re-check `mcp:call`, token scope, exposure state and result-level rights on every
  call; a task handle is a name, not a capability.
- **FR-025-032 — Result disclosure**: A result MUST contain only the exposure's validated closed
  projection of the run result, with no field above `maxResultClassification` and none `secret`; a
  caller without the rights for the declared level is refused before admission (answered as an
  unknown tool). Every returned result is audited; arguments and results never enter logs, spans,
  metrics or audit reasons.
- **FR-025-033 — MCP limits and errors**: The server MUST bound request bytes, wait time, concurrent
  calls and rate per namespace and per token, and map platform errors to MCP protocol or tool
  execution errors as in `contracts/mcp-server.md`, without internal diagnostics.

#### MCP client

- **FR-025-034 — External MCP server resources**: `ai:manage` holders MUST be able to register,
  revise, pause and delete external MCP servers (HTTPS URL, protocol preference, a required
  `maxArgumentClassification` of `public`, `internal` or `pii` (`pii` under the same reveal-grant rule as
  FR-025-005), authentication none, static bearer
  secret or OAuth client credentials with a resource indicator, limits), as a 021 resource kind when
  available. For OAuth the revision records the issuer and token endpoint, confirmed by the saving
  `ai:manage` holder; the client secret is sent only there (D1306). User-delegated authorization is
  not supported (G1288).
- **FR-025-035 — Tool snapshots**: Refresh MUST be a durable runner job (origin `MCP_TOOL_REFRESH`)
  storing an immutable, bounded snapshot of tool names and description, input- and output-schema
  digests. Publication pins the snapshot; at call time a changed definition fails with
  `AI_TOOL_CHANGED` without calling. Descriptions and annotations are untrusted content.
- **FR-025-036 — MCP call execution**: An MCP tool call MUST be the reserved activity `sagat.ai.mcp`
  run by the agent runner with `ResolveMcpCall`, at-least-once, bounded in time and bytes, keeping
  text and structured content only (D1317), scanning results for resolved secrets, returning
  `input_required` and task results as `MCP_UNSUPPORTED_RESULT`, reporting a timeout after sending as
  `OUTCOME_UNKNOWN`, and refusing to authenticate when the server advertises an issuer or token
  endpoint other than the recorded ones (`AI_MCP_AUTH`, audit `mcp.oauth_metadata_changed`).

#### Streaming, Studio and assistant

- **FR-025-037 — Token streaming**: With `stream.tokens: true` and feature 026, the runner MUST append
  coalesced text-delta, tool-request and turn-end events to the reserved channel `sagat.agent.tokens`
  through 026's `AppendStreamEvents` under its lease (FR-026-013, FR-026-014), with the channel schema
  of `contracts/agent-stream-events.md` (text `pii`). The run's reserved-channel budget is the
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

- **FR-025-040 — SDKs and conformance**: The five SDKs MUST provide agent node and tool builders that
  emit byte-identical canonical IR, client calls for transcript reading and tool-call approvals, and
  pass the shared scenarios of `contracts/conformance.md` against the real engine and the dev provider.
- **FR-025-041 — Observability and audit**: Model calls, tool calls, approvals, MCP calls, probes and
  refreshes MUST emit OTel spans (journal-derived per ADR-0017, or runner spans), fixed-label metrics
  (no tenant, run, model or tool ids), redacted logs, and audit records committed with their decision
  for catalog changes, credential resolution, budget exhaustion, tool denials, transcript reads,
  exposures, token lifecycle and MCP calls.
- **FR-025-042 — Typed errors**: Errors MUST use the closed `Error.code` set with new `details.reason`
  values and durable failure codes from `contracts/errors.md`, each with a remediation row,
  `retryable` and trace id, never a prompt, response, argument, credential or provider message.
- **FR-025-043 — Contracts and compatibility**: OpenAPI, IR, worker protocol and any AsyncAPI change
  MUST be additive; older workers and SDKs never receive agent tasks; rollback disables new agent
  publication and admission while compatible readers drain existing runs.
- **FR-025-044 — Tenant boundary**: Tenant and namespace MUST come from the authenticated context on
  every row, query, task, credential path, MCP call and audit record; another tenant's provider,
  server, exposure, token, run or transcript is an opaque `NOT_FOUND` and the probe is audited.
- **FR-025-045 — Delivery**: The delivery MUST ship `apps/dev-model-provider` (development only, not
  in production images), runner images and Helm values with default-deny egress, runbooks for
  providers, the agent runner and MCP, and a reproducible happy and failure demo (quickstart.md).
  Real-provider tests are opt-in and never the only evidence of a guarantee.

- **FR-025-046 — Reset rules**: Every journal event type added here MUST declare its rule in 018's
  reset-rule registry: a reset point inside an open agent loop (after `agent.started`, before
  `agent.finished` of the same step) is `REFUSE`; the events of a finished agent step are
  `REMATERIALIZE`, re-creating its agent rows in the new run without repeating any call or charging any
  budget (D1635).
- **FR-025-047 — Encoded fields**: Agent context expressions, tool-result projections, tool inputs,
  `child` tool inputs, the agent result and MCP exposure projections MUST be registered in 031's
  position registry and checked with 031's codes (`ENCODING_READ_BY_EXPRESSION`,
  `ENCODING_CARRIAGE_MISMATCH`, `ENCODING_NOT_PRODUCIBLE`); an encoded value is never sent to a model
  or an MCP server, and a model-written value never fills an encoded location (D1636).

#### Amendment to feature 014

025 owns the single additive amendment of 014's `approval_requests` (owner kind `AGENT_TOOL_CALL`,
derived owner id, nullable routes for that kind, data-model.md §4); feature 026 cites it, and its
notifications and decision links apply to every owner kind, including tool calls (coordinator C22).
Task T025-032.

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
  token accepted from a query string or for another audience. Release-blocking.
- **NFR-025-002 (durability)**: committed turns, tool calls, approval decisions and MCP admissions
  survive process and broker restart; duplicates create no second transition; effects are
  at-least-once with stable keys; no exactly-once claim.
- **NFR-025-003 (cost honesty)**: every cost is labelled estimated and comes from declared prices;
  reservations never fall below the documented bound; unknown usage is charged its reservation.
- **NFR-025-004 (privacy)**: no value above the declared level of its destination leaves toward a
  model provider or an external MCP server; model-written values are never classified below the
  transcript level; the transcript is disclosed only through its audited endpoint.
- **NFR-025-005 (compatibility)**: existing definitions, runs, workers and protocol minors keep their
  semantics; the agent capability is advertised only after five-SDK conformance passes.
- **NFR-025-006 (observability)**: every accepted model, tool and MCP call has persisted correlation
  and bounded OTel evidence; export failure never decides a commit.
- **NFR-025-007 (performance)**: activation, turn scheduling, resolution, runner overhead per call and
  MCP admission are measured (p50/p95/p99) on a declared environment with the dev provider; nothing is
  promised before measurement.

### Key Entities

- **Provider resource / revision**: identity, name, status; immutable kind, endpoint settings,
  credential references, allowed models with prices and capabilities, `maxPromptClassification`, limits.
- **Run provider binding**: the provider revision pinned per agent node at admission.
- **Agent step**: run, node, status, effective budgets, reserved and settled usage, turn counter,
  transcript head, deadline.
- **Turn / attempt reservation**: model-call derived step, request digest, response reference, usage,
  stop reason, reported model; one reservation per attempt.
- **Tool call**: turn, index, name, kind, provider tool-use id, input, status, approval, derived step,
  result, effect outcome.
- **External MCP server / revision / tool snapshot**: URL, `maxArgumentClassification`,
  authentication with recorded issuer and token endpoint, credential references; immutable tool
  digests.
- **Exposure / revision**: tool name, target, version policy, projection, disclosure, level.
- **Scoped MCP token**: hash, minter, namespace, scope, expiry, revocation.
- **MCP call receipt**: idempotency key, argument digest, caller, admitted run.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All criteria are future evidence; none is met by this document.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-025-001 | Real browser registers a provider with a new OpenBao credential, reloads without values, probes, and distinguishes save, pause and probe; another tenant cannot discover it | FR-025-001..005, 038, 044 |
| SC-025-002 | Real PostgreSQL, agent runner and dev provider: an agent run returns a validated structured result; a crash after the provider answered and before acknowledgement yields one committed turn and two charged attempts | FR-025-006..009, 013..015 |
| SC-025-003 | Crash at every commit point of activation, model completion, tool completion, approval decision and deadline leaves no orphan step, no duplicate turn and no lost acknowledged transition | FR-025-008, 012, 017, 018 |
| SC-025-004 | Tool calls of every kind run through their own executor with the stable key; duplicate delivery and lost broker hints change nothing; unknown write outcomes reach reconciliation or manual review | FR-025-010..012, 025 |
| SC-025-005 | Tool-call approval: unauthorized, revoked, cross-tenant and initiator decisions denied and audited; decision versus expiry and versus cancellation pick exactly one outcome over 50 rounds each | FR-025-017, 018 |
| SC-025-006 | Budget, quota and ceiling exhaustion refuse before dispatch; every retry reserves in its claim (lost response then retry keeps settled + reserved ≤ limit for the agent and the daily row); parallel completions schedule exactly one next turn; killing a runner releases concurrency when its leases expire | FR-025-015, 016 |
| SC-025-007 | `pii`, `secret`, unmarked and non-validating values are refused at publication or before dispatch per the provider's and MCP servers' levels; looser `result`, approval-view and child-input annotations are refused; secret and MCP-argument canaries appear in no prompt, MCP request, transcript, journal, log, span, audit, stream or response; `write` tools without approval or acknowledgement are refused | FR-025-010, 014, 017, 019..021, 026 |
| SC-025-008 | Transcript endpoint: each combination of the three rights, revocation of the reveal grant between requests, no-store headers and per-read audit; audit failure returns no bytes | FR-025-022, 041 |
| SC-025-009 | Metadata, loopback, link-local, private and redirect destinations refused for providers and MCP servers; the event-stream reader enforces bytes and deadlines | FR-025-023, 036 |
| SC-025-010 | Ordinary workers, sandbox and database runners cannot claim or resolve agent-runner activities; the agent runner cannot claim ordinary tasks; stale leases and expired sessions are refused | FR-025-009, 024 |
| SC-025-011 | MCP server tested with an MCP client library at 2026-07-28 and 2025-11-25: empty default list, single-audience tokens and REST refusal of MCP tokens, uniform metadata answers, insufficient scope, scoped tokens and revocation, idempotent retry, tasks re-authorized on every `tasks/get`, foreign and uncallable exposures answered as unknown tools | FR-025-026..033 |
| SC-025-012 | MCP client against the test server: snapshot pinning, drift refusal, egress denial, unknown outcome to manual review, unsupported results as tool errors, refusal on a changed token endpoint | FR-025-034..036 |
| SC-025-013 | With feature 026, a 16-turn streaming agent stays within its reserved-channel budget, a resumed stream matches the committed transcript and superseded attempts are marked; without 026, publication refuses streaming | FR-025-037 |
| SC-025-014 | All five SDKs build byte-identical agent definitions and pass the shared runtime scenarios | FR-025-040, 043 |
| SC-025-015 | Telemetry carries only closed label values; spans and logs carry no prompt, response, argument, credential or provider message; every audit action is written with its decision | FR-025-041, 042 |
| SC-025-016 | Assistant proposals never create a version, activation or run; failed validation changes no draft | FR-025-039 |
| SC-025-017 | Reproducible Compose demo of the happy path and of budget-exhaustion and runner-crash failures, with measured latency and resource figures on a declared environment | FR-025-045, NFR-025-007 |
| SC-025-018 | Reset points inside an open agent loop are refused; a reset after a finished agent step re-materializes its rows without a provider or tool request and without budget or daily-usage charges; encoded-field fixtures are refused with 031's codes | FR-025-046, 047 |

## Assumptions and dependencies

- **021** (ADR-0055, proposed): if accepted before T025-006 with types that can name the agent runner
  as executor, providers and MCP servers are its platform-provided resource types and `operation`
  tools are its `resourceOperation`s run by its integration runner; otherwise the self-contained
  catalog ships and `operation` tools stay disabled (O1282, G1281).
- **022**: standalone steps gate script and operation exposures (D1315).
- **026**: Workflow Streams (FR-026-013..021, `AppendStreamEvents`, reserved `sagat.` channels) gate
  token streaming; `executeRun` semantics (FR-026-007..009) are reused by the MCP wait (G1282).
- **014**: approvals are reused through an additive owner kind (G1289); scoped instance identity is
  not used, so agents stay root-only.
- **016** levels, carriers and reveal grants and **017** OpenBao facade, credential pins, private
  runner policy and private definitions are reused as they are.
- **013** (federated login, draft) may later issue MCP tokens; until then the configured OIDC issuer
  of 001 does. **029** custom roles and folders would allow narrower MCP audiences (G1290); when 029 is
  enabled its evaluator decides MCP visibility and `agent-runner` is an additive built-in role.
- **018** reset rules (FR-025-046) and ADR-0050 lock slots, **020** content-store registry and `pii`
  egress rule (O1081), **028** time classes, **031** position registry (FR-025-047) and **032** key
  inventory are registries owned by those drafts; 025 registers its entries (D1635–D1638, G1635).
- The worker protocol minor is allocated by the coordinator (G1280). Model ids are the provider's;
  the platform interprets them only against the revision's allowlist.

## Out of scope

Image, audio and file modalities; provider-side server tools, MCP connectors and model fallbacks;
memory across runs; user-delegated OAuth for external MCP servers; the client side of the MCP tasks
extension and `input_required`; MCP resources, prompts, sampling and roots; fine-tuning, embeddings
and batch inference; billing reconciliation; a connector or tool marketplace (constitution, Delivery
Constraints); agents in loop or collection bodies or with continue-as-new; publishing by the assistant.

## Owner questions

O1280 (data sent to providers), O1281 (role grants) and O1282 (resource model) with recommended
defaults are in [research.md](research.md#5-owner-questions), with decisions D1280–D1319 and
D1635–D1638 and gaps G1280–G1297 and G1635. See the [requirements checklist](checklists/requirements.md).
