# ADR-0059: An agent step is an engine-driven loop whose every model call and tool call is a durable derived step, run outside the control plane within reserved budgets and a declared data ceiling

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal (2026-10-26, revised 2026-10-27 after the independent architecture and
security reviews); accepted on 2026-11-26 by the agent preparing feature 025 for build, under the owner's
delegation (Owner decision 2026-11-24-4), recorded as Owner decision 2026-11-26-123; revised the same day
against `develop` (`3f22b612`) after an independent audit of the feature 025 set; revised on 2026-11-27
after the security review of that date (`specs/025-ai-agents-mcp/reviews/2026-11-27-security.md`,
Decision 025-016), which tightened decisions 5, 7 and 8 and left their direction unchanged; revised
again on 2026-11-27 (round 3) under Owner decisions 2026-11-27-400 to 2026-11-27-403: decision 5 states
the broker routes of Decision 025-017 and decision 8 the pinned tool classification and the redacted
approval view with escalation of Decisions 025-018 and 025-019.
**Refines / supersedes:** Refines ADR-0010 (derived steps gain model-call and tool-call kinds),
Architecture decision 0071 (its derived-id grammar of decision 5, its egress ceiling of decision 8 and
its managed kinds and runner profiles of decision 1 gain the agent forms, provider and MCP-server
ceilings and an agent profile), ADR-0033 (two single-surface carriers, destination data ceilings and the
`pii`-egress rule decided for Owner question 1081), ADR-0035 and ADR-0036 (a third managed executor),
Architecture decision 0097 (its usage-lease pattern applied to the daily AI usage) and the feature-014
approval model (an owner kind for tool calls). Supersedes none.
**Enforced by:** planned, not yet existing: an extension of the restricted-dependent rules that
`scripts/check-architecture.py` already has (`ICEBERG_TEST_ONLY_PACKAGES`, `LAKE_RUNNER_PACKAGES` and
`CONTAINER_ENGINE_CRATES`): only `sagat-agent-runner` may depend on `sagat-infra-ai`, and neither may
link a container engine (Task 025-073); `crates/application/tests/managed_kind_sites.rs` extended to the
AI managed kinds (Task 025-083); the reset-rule arms in `crates/domain/src/execution/reset.rs`, whose
exhaustive `match` does not compile without them (Task 025-078); fault and integration suites under
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
   journal events. These are the only agent forms `NodeId::parse_stored` accepts beside a declared
   id, `~c`, `~r` and the destination form `<source>~d.<name>` of Architecture decision 0071 decision
   5; an agent node declares no destinations, so agent node ids stay at most 54 characters and every
   derived id fits the 64-character node id bound. Agents exist only in graph-model versions; a
   procedural workflow reaches one through a child graph workflow (design 044, not yet written,
   decides any envelope entry). An executor MUST NOT decide the next turn or run the loop.
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
5. **Execution stays outside the control plane, and credentials are declared secrets.** Model calls,
   MCP client calls, provider probes and MCP tool refreshes MUST run only in `apps/agent-runner`, as
   managed steps (`ManagedKind::{AiModel, AiMcp, AiProbe, AiMcpRefresh}`) under a new machine role
   `agent-runner` (`worker:execute`, `ai:execute`, never `database:execute`) bound with
   `BindingProfile::Agent`. The process holds no platform database URL, OpenBao token, signing key,
   database credential or container engine and resolves each attempt's capability in two phases with no
   lock held during OpenBao or provider I/O. The placement follows from Principle V itself: a model
   prompt or an MCP call is a tenant-authored request sent with a third-party credential whose answer is
   untrusted, so only an isolated executor holding only that attempt's capability sends it
   (Architecture decision 0055 decision 11 proposes the same rule for every parity draft; it is still
   proposed, Owner question 1123, and this decision does not depend on it). The credentials are
   disclosed under Principle V's **first** declared set, ordinary task execution: the immutable task
   definition of a model-call or MCP-call derived step is the agent node of the published version plus
   the provider revision and MCP-server revision that version pins, so the secrets those pinned
   revisions name are the declared secrets of that task definition, delivered all or nothing to the
   lease-owning agent runner under the caller's current membership, its worker/activity binding
   (`BindingProfile::Agent`), the pool-delivery policy, and the publisher's `secret:use` on every one of
   them, checked at publication and again at delivery. Only a static API key or bearer token is
   delivered this way. These secrets are disclosed only by the agent resolve calls: the node of every
   AI kind is materialized with no `secrets`, and the ordinary `ResolveSecrets` refuses every managed
   lease. MCP servers authenticated by OAuth, and the provider modes that need a client secret or a
   signing key (`azure-entra-client-credentials`, `aws-access-key`), reach tokens only through design
   047's token broker, under the third declared set of Principle V that the owner approved for
   constitution 5.1.0 (Owner decision 2026-11-25-3), which excludes a resource's client secret and
   signing key (5.1.0 adopted, pull request 286); they are refused until the broker issues access
   tokens. The first set never covers those secrets (Owner decision 2026-11-27-400): the broker runs
   the Entra client-credentials grant and the runner receives only that access token, under the third
   set with the resource's authorities (`resource:use`, and `resource:effect` for `write` MCP tools),
   never as a first-set secret (Decisions 025-017 and 025-020). The AWS exchange for an STS session and
   an Amazon Bedrock short-term API key waits for the owner's answer to Owner question 025-002, and
   `aws-access-key` stays refused until then. The
   runner follows no redirect (`max_redirects = 0` for every provider and MCP request, the probe
   included) and reads its namespace egress policy from operator-owned configuration before every
   attempt. The agent runner sends no step logs (design 041) and the transcript is
   the authoritative record. Other tool kinds run in their own executors. A deployment's own cloud
   identity MUST NOT be used for a tenant's provider account, and adapters MUST NOT use SDK default
   credential chains or instance metadata.
6. **Budgets are reserved per attempt, not trusted.** Every model-call attempt, first or retry,
   MUST reserve a conservative bound (rendered request bytes as the text-token bound, plus the call's
   maximum output tokens, plus the estimated cost from declared prices) against the agent's budgets and
   the namespace's daily row in the transaction that grants its lease; a reservation that does not fit
   fails the step with `AI_BUDGET_EXHAUSTED` and no lease is granted. Completion settles reported usage;
   a provider error response releases the reservation; a lost response, a timeout or lease expiry keeps
   it as charged. The namespace's daily budget is charged to per-tenant, per-namespace leases of the
   claiming server backend, refilled from the daily usage row only when short and never written at
   every commit (the pattern of Architecture decision 0097). AI concurrency is the count of live leases
   of every agent-runner activity, read in the claim transaction, so no counter can leak capacity; an
   ordinary claim runs none of these statements (the budget of design 038). Cost is always an estimate
   from operator-declared prices.
7. **Data leaves only below a declared ceiling, and model-written values are never classified below
   what entered the transcript where an expression or a child output carries them** (a value an activity
   or script returns is a new value under that step's carrier and inherits nothing; feature 025's threat
   model states this pass-through, Decision 025-020). A provider revision MUST declare `maxPromptClassification` and an
   external MCP server revision MUST declare `maxArgumentClassification` (`public`, `internal` or
   `pii`), each the domain's `EgressCeiling` of Architecture decision 0071 decision 8. Instructions and prompt are literals; run data enters only through context expressions and
   tool results. Publication MUST compute the agent's transcript level, the strictest of `internal`
   (literals and tool descriptions), every referenced context carrier path and every tool result
   carrier (no carrier counts as `pii`; `secret` is refused), and MUST refuse: a transcript level above
   the provider's ceiling; an `mcp` tool whose server ceiling is below it; and a `result` field, an
   approval-view tool input field or a child tool's input field annotated below it
   (`CLASSIFICATION_LOOSENED`). The level a value inherits from an agent's output follows it: the
   effective level of a workflow output field, a child node output field or a `child` tool result
   carrier field is the stricter of its annotation and what it inherits through `end.outputExpr` and
   child outputs (the domain's `written_level`), refused at publication, or at admission for a child
   under an active-version policy. Publication records the provider revision, MCP-server revision and
   snapshot it checked in the version; admission and every resolution recheck the transcript level
   against the stricter of the pinned and current ceilings and the model against the current
   allowlist, so a restriction applies at once and a later revision never widens a published version.
   No string a model or a provider wrote (an undeclared tool name, a tool-use id, a reported model)
   reaches a run-read surface. At run time a value that does not validate against its carrier is not
   sent (`CLASSIFICATION_EGRESS_REFUSED`), and an encoded (feature 031) value is never sent nor filled
   by the model (031's codes). Agent schemas are validated by the run-input validator of design 043,
   and a validation summary carries only its value-free rule and field. A `pii` ceiling follows the
   uniform rule decided for Owner question 1081 (Decision 1617 of feature 020, applied here by Owner
   decision 2026-11-26-120): it is set only by an administrator holding the reveal grant, the grant is
   rechecked at each resolution that sends `pii`, revocation pauses the provider or server, each such
   attempt is audited, a secret never leaves and the default ceiling is `internal`. Rendering has no secret
   capability. The transcript is `pii` as a whole and is disclosed only to the feature-016 `pii`
   audience through an audited, no-store endpoint.
8. **Sensitive tools wait for a person through feature 014.** A tool approval is an
   `approval_requests` row of owner kind `AGENT_TOOL_CALL`, decided through the existing endpoint with
   decision-time authorization, deduplicated receipts, initiator exclusion and first-commit
   resolution against expiry and cancellation. The approver sees a redacted view (the decision context,
   a redacted outline of the transcript and the tool input with every field their rights withhold
   masked by path and level only); revealing a masked field needs the reveal grant. A decision is
   applied only when the decider's current rights disclose every field the tool's approval policy names
   in `decisionFields` (every field by default; never narrowed for a tool that may not run with
   `approval: "none"`; a tool input field cannot be `secret`); nobody approves the arguments that matter
   without seeing them. An approval no eligible decider can decide with context is escalated to the
   policy's `escalation` route, and with nobody there it stays pending and visible and ends at its
   deadline with the recorded reason `NO_DECIDER_WITH_CONTEXT`, never unnoticed (Decision 025-019). A `write` tool MUST declare an approval policy or the
   explicit acknowledgement `approval: "none"`; an `operation` tool and a database `execute` tool MUST
   declare a policy; a `child` tool declared `read` MUST declare a policy, and an `mcp` tool may be
   `read` without one only when the pinned MCP-server revision classifies it `read`; the server's
   annotations are hints, and their disagreement with that classification is audited (Decision
   025-018). A version with an acknowledged
   unattended write tool, in itself or in any child workflow it can start, MUST NOT be exposed through
   MCP or a feature-026 public link, and a child admission under a run those surfaces admitted MUST
   refuse such a child version.
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
   and six SDK implementations of the same planner. *Note (2026-11-24):* constitution 5.0.0 no longer
   defers the procedural mode (design 044 and its records Architecture decisions 0103 to 0109, all not
   yet written); the reasons above for an engine-driven agent loop still hold.
4. **Provider-hosted agents and provider-side tools** — the provider runs the loop and the tools: no
   platform durability, egress control, audit or tenant boundary over the tool effects.

## Consequences

- Each turn costs several transactions and journal events; the performance report must measure
  activation, turn scheduling and resolution latency before any capacity is stated.
- `NodeId::parse_stored` and every reader of derived ids learn the agent forms; `ManagedKind` and
  `BindingProfile` gain AI variants; `approval_requests` gains an owner kind and, for that kind only,
  the escalation columns of Decision 025-019 (a coordinated change with feature 014's open lock-order
  work).
- A new machine role and executor add operational cost (image, network policy, runbook), in exchange
  for a separate credential blast radius: the agent runner never holds a database credential.
- The worker protocol takes the next free minor of the register in `sdks/SPEC.md` section 9 (1.7 is
  feature 041's); agent and MCP publication wait for the instance-registry capabilities `ai-agents-v1`
  and `mcp-server-v1`.
- Authors must classify data they want to send; an unclassified run payload is `pii` and is refused by
  a provider declared below `pii`.
- Retries and lost responses are billed; budgets are conservative, so a step can fail on budget
  before the provider would have reached it.
- A feature-018 reset point inside an open agent loop is not offered (`ResetRule::Refuse { until:
  Some("agent.finished") }`); a finished agent step is re-materialized in the new run without
  repeating any call (`ResetRule::Rematerialize`, in the code registry
  `crates/domain/src/execution/reset.rs`).
- Rollback: disable agent publication and admission; keep the planner, runner and readers until agent
  runs drain; never drop agent tables while runs reference them.

## Evidence

None yet: accepted as a design, not as an implementation. Required before the implementation is
declared done: the fault matrix and verification plan of
[plan.md](../../specs/025-ai-agents-mcp/plan.md), six-SDK conformance
([contracts/conformance.md](../../specs/025-ai-agents-mcp/contracts/conformance.md)), the
classification, secret and MCP-argument canary battery, and the measured performance report.
Decisions D1280–D1319, D1635–D1638 and D025-001 to D025-019, gaps G1280–G1297, G1635 and
G025-001 to G025-004, and Owner question 025-001 (decided) are in
[research.md](../../specs/025-ai-agents-mcp/research.md).
