# ADR-0059: An agent step is an engine-driven loop whose every model call and tool call is a durable derived step, run outside the control plane within reserved budgets and a declared data ceiling

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** Refines ADR-0010 (derived steps gain model-call and tool-call kinds),
ADR-0033 (two single-surface carriers, destination data ceilings and the `pii`-egress rule proposed
with 020's O1081), ADR-0035 and ADR-0036 (a third managed executor) and the feature-014 approval model
(an owner kind for tool calls). Supersedes none.
**Enforced by:** planned, not yet existing: a restricted-crate → allowed-dependents map in
`scripts/check-architecture.py` (only `sagat-agent-runner` may depend on `sagat-infra-ai`; task
T025-073); fault and integration suites under
`tests/{integration,fault}/ai-agents-mcp/` named in `specs/025-ai-agents-mcp/tasks.md` (crash at
each commit point, retry reservations, budget races, transcript-level publication fixtures,
classification, secret and MCP-argument canaries); contract compatibility tests
over `workflow.schema.json` and `worker.proto`. Until those exist this is review only.

## Context

Feature 025 adds an `agent` node: a model is asked, may request declared tools, sees their results
and eventually answers. Four forces make the execution model hard to reverse:

1. **Durability.** A model call takes seconds to minutes and costs money; a tool call may move money.
   The platform's guarantees (ADR-0002, constitution I) hold only for work the journal sees: a
   transition, its next tasks and the outbox commit together, and a stale attempt is fenced.
2. **Non-determinism.** Asking the model again does not give the same answer. Temporal's agent
   integrations make each model call an Activity and replay the loop code; the Sagat IR is
   declarative (ADR-0003), so there is no loop code to replay.
3. **Isolation.** Provider credentials are third-party secrets, and model output and MCP results are
   untrusted content. The control plane and scheduler must not hold them or run untrusted work
   (constitution V, ADR-0013).
4. **Disclosure and cost.** Sending a value to a provider is a disclosure to a third party
   (ADR-0033 decides who may see a value, not where it may be sent). A loop can spend without bound;
   providers have no request idempotency key, so a retried call is billed again.

## Decision

1. **The engine drives the loop.** The `agent` node is interpreted by a pure domain planner. Every
   model call is a derived step `<node>~m<turn>` and every tool call a derived step
   `<node>~t<turn>.<call>` (its compensation and reconciliation `<node>~t<turn>.<call>~c` and `~r`),
   each with the existing lease, generation fence, heartbeat, retry policy, idempotent report and
   journal events. Agent node ids are at most 54 characters so that every derived id fits the
   64-character node id bound. An executor MUST NOT decide the next turn or run the loop.
2. **Commits decide.** Activation creates the agent step, its budgets and the first model call in one
   transaction. The transaction that commits a model call MUST also plan what follows (tool calls,
   approvals, refusal results, repair turn, final output or failure). The next model call MUST be
   scheduled only by the transaction that settles the last tool call of the turn. The first committed
   result of a model call wins; a later attempt's result is rejected as stale.
3. **The transcript is append-only and authoritative.** It is stored as retained content referenced
   from the journal. Provider-opaque blocks (for example thinking) MUST be replayed byte-for-byte to
   the same provider kind and never edited, reordered or shown outside the transcript endpoint.
4. **Effects are at-least-once and said to be.** A model call has no provider idempotency; a lost
   response MUST be asked again and charged. A tool call MUST carry the key
   `agent:{runId}:{node}:{turn}:{call}`; a `write` tool with an unknown outcome MUST go to its
   reconciliation or to manual review, never to a blind retry. An agent in a saga region compensates
   tool calls in reverse completion order; a `write` tool without compensation is irreversible.
5. **Execution stays outside the control plane.** Model calls, MCP client calls, provider probes and
   MCP tool refreshes MUST run only in `apps/agent-runner` under the machine role `agent-runner`
   (`worker:execute`, `ai:execute`), which holds no platform database URL, OpenBao token or signing
   key and resolves each attempt's capability in two phases with no lock held during OpenBao or
   provider I/O. This applies the placement rule that 021 writes in ADR-0055 (coordinator C3 (a)):
   a request whose content a tenant authored in a workflow, such as a model prompt or a tool call, is
   sent only by a managed executor holding only that attempt's capability. Other tool kinds run in
   their own executors. A deployment's own cloud identity MUST NOT be used for a tenant's provider
   account, and adapters MUST NOT use SDK default credential chains or instance metadata.
6. **Budgets are reserved per attempt, not trusted.** Every model-call attempt, first or retry,
   MUST reserve a conservative bound (rendered request bytes as the text-token bound, plus the call's
   maximum output tokens, plus the estimated cost from declared prices) against the agent's budgets and
   the namespace's daily row in the transaction that grants its lease; a reservation that does not fit
   fails the step with `AI_BUDGET_EXHAUSTED` and no lease is granted. Completion settles reported usage;
   a provider error response releases the reservation; a lost response, a timeout or lease expiry keeps
   it as charged. AI concurrency is the count of live leases of agent-runner activities, read in the
   claim transaction, so no counter can leak capacity. Cost is always an estimate from
   operator-declared prices.
7. **Data leaves only below a declared ceiling, and model-written values are never classified below
   what entered the transcript.** A provider revision MUST declare `maxPromptClassification` and an
   external MCP server revision MUST declare `maxArgumentClassification` (`public`, `internal` or
   `pii`). Instructions and prompt are literals; run data enters only through context expressions and
   tool results. Publication MUST compute the agent's transcript level, the strictest of `internal`
   (literals and tool descriptions), every referenced context carrier path and every tool result
   carrier (no carrier counts as `pii`; `secret` is refused), and MUST refuse: a transcript level above
   the provider's ceiling; an `mcp` tool whose server ceiling is below it; and a `result` field, an
   approval-view tool input field or a child tool's input field annotated below it
   (`CLASSIFICATION_LOOSENED`). At run time a value that does not validate against its carrier is not
   sent, and an encoded (feature 031) value is never sent nor filled by the model (031's codes). A
   `pii` ceiling follows the uniform rule proposed with 020's O1081: it is set only by an administrator
   holding the reveal grant, the grant is rechecked at each resolution that sends `pii`, revocation
   pauses the provider or server, and each such attempt is audited. Rendering has no secret
   capability. The transcript is `pii` as a whole and is disclosed only to the feature-016 `pii`
   audience through an audited, no-store endpoint.
8. **Sensitive tools wait for a person through feature 014.** A tool approval is an
   `approval_requests` row of owner kind `AGENT_TOOL_CALL`, decided through the existing endpoint with
   decision-time authorization, deduplicated receipts, initiator exclusion and first-commit
   resolution against expiry and cancellation. A `write` tool MUST declare an approval policy or the
   explicit acknowledgement `approval: "none"`; an `operation` tool MUST declare a policy; a version
   with an acknowledged unattended write tool MUST NOT be exposed through MCP or a feature-026 public
   link.
9. **Scope is bounded until proven.** Agents are root-only and not combined with `chainPolicy`;
   modality is text and JSON; provider-side tools, provider-side MCP connectors and server-side model
   fallbacks are not used; every limit has a default and a hard ceiling (plan.md).

## Considered options

1. **Engine-driven loop of derived steps (chosen)** — every call is visible, fenced and recoverable;
   tools run in their own executors; budgets and approvals are transactional.
2. **One long managed activity that runs the whole loop in a runner** — simplest to build, but a crash
   loses every turn, tool effects bypass their executors' isolation and the journal, and approvals
   would need a second waiting mechanism inside a lease.
3. **Code-orchestrated loop in an SDK worker with replay (Temporal style)** — needs deterministic loop
   code per language, the procedural mode that `docs/architecture/workflow-ir-and-sdk-modes.md` defers,
   and five SDK implementations of the same planner.
4. **Provider-hosted agents and provider-side tools** — the provider runs the loop and the tools: no
   platform durability, egress control, audit or tenant boundary over the tool effects.

## Consequences

- Each turn costs several transactions and journal events; the performance report must measure
  activation, turn scheduling and resolution latency before any capacity is stated.
- `NodeId::parse_stored` and every reader of derived ids learn two suffixes; `approval_requests`
  gains an owner kind (a coordinated change with feature 014's open lock-order work).
- A third machine role and executor add operational cost (image, network policy, runbook), in
  exchange for a separate credential blast radius.
- Authors must classify data they want to send; an unclassified run payload is `pii` and is refused by
  a provider declared below `pii`.
- Retries and lost responses are billed; budgets are conservative, so a step can fail on budget
  before the provider would have reached it.
- A feature-018 reset point inside an open agent loop is refused; a finished agent step is
  re-materialized in the new run without repeating any call (reset-rule registry, coordinator C1).
- Rollback: disable agent publication and admission; keep the planner, runner and readers until agent
  runs drain; never drop agent tables while runs reference them.

## Evidence

None yet; this is a proposal. Required before acceptance of the implementation: the fault matrix and
verification plan of [plan.md](../../specs/025-ai-agents-mcp/plan.md), five-SDK conformance
([contracts/conformance.md](../../specs/025-ai-agents-mcp/contracts/conformance.md)), the
classification, secret and MCP-argument canary battery, and the measured performance report.
Decisions D1280–D1319 and D1635–D1638 and gaps G1280–G1297 and G1635 are in [research.md](../../specs/025-ai-agents-mcp/research.md).
