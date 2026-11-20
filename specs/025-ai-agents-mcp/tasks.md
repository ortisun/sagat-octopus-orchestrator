# Tasks: AI agents and MCP (025)

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/README.md), [quickstart.md](quickstart.md).

Status: proposed; nothing started. A task is checked only with the evidence it names, run for real
and recorded (constitution VIII). Test tasks are written first and must fail before implementation.

Format: `T025-NNN [P?] [USn?] Description — requirements — files — evidence`. `[P]` = different
files, no dependency on an unfinished task in the same phase. Test targets: `contract_ai_agents_mcp`,
`integration_ai_agents_mcp`, `fault_ai_agents_mcp`, `e2e_ai_agents_mcp`, `perf_ai_agents_mcp`
(declared in `tests/Cargo.toml`), plus `sdk_conformance`.

## Phase 1: Setup and foundations (blocks every story)

- [ ] T025-001 Record owner acceptance of ADR-0059/ADR-0060 and answers to O1280–O1282; record the
  coordinator's protocol-minor allocation (G1280) and the status of 021, 022 and 026; update the spec
  status line — all FRs — `specs/025-ai-agents-mcp/*`, `docs/adr/ADR-0059-*.md`, `docs/adr/ADR-0060-*.md`
  — owner record linked from research.md.
- [ ] T025-002 Merge the contract fragments into the canonical contracts and add their checks:
  `workflow.schema.json`, `openapi.yaml`, `worker.proto` + `worker.baseline.json`, `model-turn.schema.json`,
  remediation rows in `crates/api-http/src/error.rs` — FR-025-006, 042, 043 —
  `specs/001-platform-foundation/contracts/`, `tests/contract/ai-agents-mcp/errors.rs` — CI `contracts`
  lint and compatibility tests pass; old fixtures still accepted.
- [ ] T025-003 [P] Derived ids `<node>~m<n>`, `<node>~t<n>.<k>` and the tool-call compensation and
  reconciliation ids `<node>~t<n>.<k>~c`/`~r` in `NodeId`, `parse_stored` and every derived-id consumer;
  agent node ids ≤ 54 characters; forged ids and other two-suffix forms rejected — FR-025-008, 012, 025
  — `crates/domain/src/workflow.rs` — unit and property tests including the 64-character bound at
  `~t64.256~c`; existing `~c`/`~r` behaviour unchanged.
- [ ] T025-004 [P] Domain agent model: declaration, tools, budgets arithmetic (integer tokens, cost
  micros), pure planner returning typed commands — FR-025-006, 008, 010, 011, 013..015 —
  `crates/domain/src/agent.rs`, `crates/domain/src/agent/` — property tests: settled + reserved never
  exceeds a limit; exactly one next turn per settled turn; refusal results ordered.
- [ ] T025-005 [P] Domain provider, MCP and identity types: provider kinds, model entries, prices, data
  level; exposures, snapshot digests, token scope; six permissions and role `agent-runner`; AI quota
  resources — FR-025-001, 002, 005, 016, 026, 029 — `crates/domain/src/{ai_provider,mcp,identity,quota}.rs`
  — unit tests; permission catalog test updated (54 entries, eight roles).
- [ ] T025-006 Provider and MCP server catalog storage: on feature 021 `resources` if accepted (D1283),
  otherwise self-contained tables with a written migration path; revisions, receipts, tombstones,
  `run_ai_bindings` — FR-025-001, 002, 034, 044 — `crates/infra-postgres/migrations/`,
  `crates/infra-postgres/src/ai_providers.rs`, `storage-classification.json` — real PostgreSQL tests:
  immutability triggers, receipt replay/conflict, runtime-role privileges, classification manifest check.
- [ ] T025-007 Agent run-time tables: `agent_steps`, `agent_turns`, `agent_tool_calls`,
  `agent_transcript_entries`, `step_ai_credentials`, `ai_usage_daily`; budget CHECKs; down refuses live
  data — FR-025-008, 015, 016, 021 — `crates/infra-postgres/migrations/`, `src/agents.rs` — migration
  up/down tests, runtime-role sweep.
- [ ] T025-008 MCP tables: snapshots, snapshot tools, exposures and revisions, tokens, call receipts,
  task handles — FR-025-026, 029, 031, 035 — `crates/infra-postgres/migrations/`, `src/mcp.rs` —
  migration tests, token hash uniqueness, receipt window on the database clock.
- [ ] T025-009 IR codec and publication validation of `agent`: capability gate, root-only, no
  `chainPolicy`, closed schemas and bounds, tool names, model allowlist, `UNPRICED_MODEL`, the
  transcript level (D1319) against the provider and MCP ceilings, the `result`, approval-view and
  child-input floors (`CLASSIFICATION_LOOSENED`), `write` tools with approval or `approval: "none"`,
  `operation` tools with a policy, and the version flag `agent_unattended_writes` — FR-025-006, 010,
  013..015, 017, 019 — `crates/infra-ir/src/agent.rs`, `crates/infra-ir/src/classification_check.rs`,
  `crates/infra-schema/src/` — publication fixtures for every reason in `contracts/errors.md`.
- [ ] T025-010 Application ports and the agent transactions of plan.md (activation, model completion,
  tool completion, approval outcome, cancellation, deadline) with `begin_safely` and the stated lock
  order — FR-025-008, 012, 017, 018 — `crates/application/src/agents.rs`, `agents/`,
  `crates/infra-postgres/src/agents.rs` — real PostgreSQL tests per transaction, including rollback of
  every statement.
- [ ] T025-073 [P] Restricted-dependents check: extend `scripts/check-architecture.py` with a
  restricted-crate → allowed-dependents map (`sagat-infra-ai` → `{sagat-agent-runner}`, `sagat-api-mcp`
  → `{sagat-control-plane}`) and negative tests with saved metadata in `scripts/tests/` — FR-025-024,
  ADR-0059, ADR-0060 — `scripts/check-architecture.py`, `scripts/tests/` — CI `architecture` fails on a
  forbidden dependent.

## Phase 2: US01 — provider and durable model call (P1, MVP)

- [ ] T025-011 [P] [US1] Dependency spike and pins: SSE parser, SigV4 signer, MCP codec (in-repo vs
  library, written comparison), advisory audit — FR-025-003, 027 — `Cargo.toml`, research.md —
  `cargo deny`/audit clean; comparison recorded (G1291).
- [ ] T025-012 [P] [US1] Bounded incremental body reader in the egress guard (bytes, idle and total
  deadline, same resolver checks) — FR-025-023 — `crates/infra-egress/src/lib.rs` — SSRF, redirect,
  slow-drip and oversize tests (G1283).
- [ ] T025-013 [US1] `crates/infra-ai`: canonical turn mapping and the `anthropic` adapter (Messages API,
  tools, `output_config.format`, streaming, usage, stop reasons, error mapping, `request-id`, no forced
  tool choice, opaque blocks replayed verbatim) — FR-025-007, 009, 013, 014 — `crates/infra-ai/src/` —
  adapter tests against `apps/dev-model-provider` recordings.
- [ ] T025-014 [P] [US1] `openai-compatible` adapter (Chat Completions, tools, JSON-schema format) —
  FR-025-009, 013 — `crates/infra-ai/src/openai_compatible.rs` — same adapter suite.
- [ ] T025-015 [P] [US1] `aws-bedrock` and `azure-openai` adapters with capability flags and validation
  fallback — FR-025-003, 009, 013 — `crates/infra-ai/src/{bedrock,azure}.rs` — adapter suite; SigV4
  vectors.
- [ ] T025-016 [US1] `apps/agent-runner`: config, registration, finite credit polling, heartbeat,
  cancellation, closed failure vocabulary, no DB/OpenBao/signing key, panic hook without payload,
  explicit static provider credentials with SDK default credential chains disabled — FR-025-003, 009,
  024 — `apps/agent-runner/` — integration tests; a test that the process environment and config
  contain no platform credential; a test that link-local and metadata addresses are unreachable from
  the runner process and its adapters.
- [ ] T025-017 [US1] `ResolveModelCall` two-phase capability, credential pins and `secret.resolved` audit
  — FR-025-003, 009, 024 — `crates/api-grpc/src/agent.rs`, `crates/application/src/agent_capability.rs`
  — revocation between phases, stale lease, expired session, ordinary worker/database runner denial.
- [ ] T025-018 [US1] Server-side request rendering (literals, context evaluation, canonical JSON blocks,
  tool definitions, transcript, digest) — FR-025-007, 020 — `crates/application/src/agents/render.rs` —
  determinism and no-secret-capability tests.
- [ ] T025-019 [US1] Engine wiring of activation and model completion, outbox hints, run graph
  `AgentProgress` — FR-025-008, 014 — `crates/scheduler/`, `crates/application/src/agents.rs` — real
  engine test: one-turn agent succeeds end to end.
- [ ] T025-020 [US1] Structured output validation, repair turns, generated output carrier — FR-025-013,
  014 — `crates/application/src/agents/output.rs` — valid, repaired and `AI_OUTPUT_INVALID` cases.
- [ ] T025-021 [US1] Fault tests: crash before and after the provider answered, duplicate completion,
  lost broker hint, engine and runner restart, a lost response followed by a retry at the budget edge,
  and a runner killed while holding leases — FR-025-008, 009, 015, 016, NFR-025-002 —
  `tests/fault/ai-agents-mcp/model_calls.rs` — one committed turn; settled + reserved ≤ limit for the
  agent and the daily row at every step; concurrency free after lease expiry; journal equals
  projection (`verify_run`).
- [ ] T025-022 [US1] Provider routes, audit and probe admission (origin `AI_PROVIDER_PROBE`) —
  FR-025-001..005, 044 — `crates/api-http/src/routes/ai_providers.rs` — HTTP tests incl. cross-tenant
  opacity, paused provider, receipts.
- [ ] T025-023 [US1] `apps/dev-model-provider`: scripted scenarios (answer, tool requests, invalid JSON,
  `429`/`529`, stream failure, refusal, hold), test MCP server, refuses to start outside development —
  FR-025-045 — `apps/dev-model-provider/`, `infra/docker/compose.ai.yaml` — its own tests; production
  image check that it is absent.
- [ ] T025-024 [US1] Studio provider screens (create/rotate secret inline, probe, data-level choice,
  provider-retention notice) and agent node editor emitting canonical IR — FR-025-001, 005, 021, 038 —
  `frontend/apps/studio/src/features/ai/`, `features/workflows/AgentFields.tsx` — component tests;
  generated-client check.

## Phase 3: US02 — durable tools (P1)

- [ ] T025-025 [US2] Tool planning: declared-name check, input validation, per-turn and total limits,
  refusal results in request order — FR-025-010, 011 — `crates/application/src/agents/tools.rs` —
  integration tests for every refusal reason.
- [ ] T025-026 [US2] Tool-completion transaction and last-of-turn scheduling — FR-025-008, 012 —
  `crates/application/src/agents.rs` — concurrent completion test: exactly one next turn over 100 rounds.
- [ ] T025-027 [US2] Executors per kind: activity (ordinary lease of the derived step), script, database
  query (execute only with approval), child (tree budget); stable keys; unknown outcome to
  reconciliation or manual review — FR-025-012 — `crates/application/src/agents/execution.rs` —
  `tests/fault/ai-agents-mcp/tools.rs`: crash after effect, duplicate delivery, lost hint.
- [ ] T025-028 [P] [US2] `operation` tool kind on feature 021 `resourceOperation` (gated) — FR-025-010,
  012 — `crates/application/src/agents/execution.rs` — refused while 021 is absent; fixture when present.
- [ ] T025-029 [US2] Saga compensation of tool calls in reverse completion order; irreversible writes;
  failed compensation to manual review — FR-025-025 — `crates/domain/src/agent/`, saga reducer —
  `tests/fault/ai-agents-mcp/sagas.rs`.
- [ ] T025-030 [US2] Tool result handling: bounds and truncation, secret scan, projection, carrier
  validation, data-level check — FR-025-012, 019, 020 — `crates/application/src/agents/results.rs` —
  tests per rule.
- [ ] T025-031 [US2] Cancellation and deadline: heartbeat stop, approvals cancelled, child policy,
  first-commit races — FR-025-018 — `crates/application/src/agents.rs`, `crates/scheduler/` —
  `tests/fault/ai-agents-mcp/races.rs` (deadline vs completion, cancel vs completion, 50 rounds each).

## Phase 4: US03 — tool approval (P1)

- [ ] T025-032 [US3] `approval_requests` owner kind `AGENT_TOOL_CALL`, the single shared amendment of
  feature 014 that 026 cites (coordinator C22): migration, constraint versions, lock-order review with
  feature 014 (G1289) — FR-025-017 — `crates/infra-postgres/migrations/`,
  `src/primitives.rs` — 014 approval suites still pass; new owner-kind tests.
- [ ] T025-033 [US3] Open, decide, expire and cancel tool approvals; `refuse`/`fail`; clamped deadline —
  FR-025-017, 018 — `crates/application/src/agents/approvals.rs` — races decision vs expiry vs cancel
  (50 rounds), unauthorized/revoked/cross-tenant/initiator denials audited.
- [ ] T025-034 [US3] Approver view of the tool input with the input schema as carrier — FR-025-017 —
  `crates/api-http/src/routes/agent_tool_calls.rs` — disclosure matrix per level; no-store; audit.
- [ ] T025-035 [US3] Studio approval panel for tool calls — FR-025-017, 038 — `frontend/apps/studio/src/features/runs/ToolApproval.tsx` — component and accessibility tests.

## Phase 5: US04 — budgets and quotas (P1)

- [ ] T025-036 [US4] Per-attempt reservation in the claim transaction (`agent_call_attempts`), settlement,
  release on provider error responses, charged-unknown on loss and expiry, daily reserved columns,
  exhaustion before any lease; per-adapter verification of the text-token bound (G1284) and of the
  "error responses are not billed" assumption (G1294) — FR-025-015, NFR-025-003 —
  `crates/domain/src/agent/budget.rs`, `crates/application/src/agents.rs`, `crates/infra-postgres/src/agents.rs`
  — property tests; counter-equals-sum trigger tests; adapter fixtures comparing bound and reported usage.
- [ ] T025-037 [P] [US4] Estimated cost from declared prices, currency rules, `UNPRICED_MODEL` —
  FR-025-002, 015 — `crates/domain/src/ai_provider.rs` — unit and publication tests.
- [ ] T025-038 [US4] AI quotas: concurrency as the count of live `sagat.ai.*` leases in the claim
  transaction (no counter), daily tokens and cost (settled + reserved), resource counts, zero refuses,
  lowered limits — FR-025-016 — `crates/domain/src/quota.rs`, `crates/infra-postgres/src/agents.rs` —
  quota tests with the runtime role; kill-the-runner test frees capacity at lease expiry.
- [ ] T025-039 [US4] Every ceiling of plan.md enforced at publication or admission, at and beyond the
  boundary — FR-025-015, 016, 033 — `crates/infra-ir/src/agent.rs`, routes — boundary tests.

## Phase 6: US05 — data protection (P1)

- [ ] T025-040 [US5] Transcript level and prompt classification: publication computation (literals,
  context paths, tool-result carriers, no carrier as `pii`), provider and MCP ceilings, output,
  approval-view and child-input floors, runtime refusal of non-validating values
  (`AI_PROMPT_CLASSIFICATION`) — FR-025-014, 017, 019 — `crates/infra-ir/src/classification_check.rs`,
  `crates/application/src/agents/render.rs` — classification battery, including a `pii` context with a
  `public` result field (refused) and an MCP server below the transcript level (refused).
- [ ] T025-041 [US5] Transcript storage and audited disclosure endpoint (016 `pii` audience,
  `payload.pii_revealed` before bytes, no-store, bounds) — FR-025-021, 022 —
  `crates/application/src/agent_transcripts.rs`, `routes/agent_transcripts.rs` — rights matrix,
  revocation between pages, audit failure returns no bytes.
- [ ] T025-042 [US5] Secret and MCP-argument canary battery across prompts sent to the dev provider,
  arguments received by the test MCP server, transcripts, journal, logs, spans, audit, stream events
  and responses — FR-025-019, 020, NFR-025-001 — `tests/integration/ai-agents-mcp/canaries.rs` — zero
  canary occurrences; no argument above the server's `maxArgumentClassification` reaches it.
- [ ] T025-043 [P] [US5] Retention: manifest entries, deletion with the run, snapshot maintenance job —
  FR-025-021 — `storage-classification.json`, `crates/application/src/maintenance.rs` — retention tests.

## Phase 7: US06 — MCP server (P2)

- [ ] T025-044 [US6] `crates/api-mcp`: Streamable HTTP JSON-RPC, 2026-07-28 and stateless 2025-11-25,
  `server/discover`, header checks, error mapping — FR-025-027, 033 — `crates/api-mcp/`,
  `apps/control-plane/` — protocol unit tests.
- [ ] T025-045 [US6] OAuth resource server: Protected Resource Metadata answered identically for every
  well-formed namespace id, single-audience check, `WWW-Authenticate`, `insufficient_scope`,
  query-token refusal, no passthrough — FR-025-028 — `crates/api-mcp/src/auth.rs` — security tests,
  including a multi-audience token (refused) and an existence-oracle probe on the metadata endpoint.
- [ ] T025-046 [US6] Scoped MCP tokens: mint, list, revoke, hash, intersection with the minter's current
  rights, audit — FR-025-029 — `crates/application/src/mcp_tokens.rs`, `routes/mcp_tokens.rs` — tests
  incl. minter demotion.
- [ ] T025-047 [US6] Exposures: routes, revisions, projection compile and level checks, exposer rights,
  refusal of target versions with unattended write tools (`UNATTENDED_WRITE_TOOL`, also at call time
  for ACTIVE-policy exposures), Studio screens — FR-025-026, 032, 038 —
  `crates/application/src/mcp_exposures.rs`, `frontend/apps/studio/src/features/ai/` — tests per reason.
- [ ] T025-048 [US6] `tools/list` filtering and order; `tools/call` admission with receipts, bounded wait
  (026 `executeRun` when present), uncallable exposures answered as unknown tools, task handles, and
  full re-authorization on every `tasks/get` and `tasks/cancel` — FR-025-030, 031 —
  `crates/application/src/mcp_calls.rs` — integration tests, including revocation between `tasks/get`
  calls and a handle presented by another caller.
- [ ] T025-049 [US6] Result disclosure: projection validation, level and caller-rights checks,
  `mcp.result_disclosed` audit — FR-025-032 — `crates/application/src/mcp_calls.rs` — disclosure tests;
  secret field never returned.
- [ ] T025-050 [US6] MCP faults and abuse: lost response with and without key, duplicate calls, rate and
  concurrency limits, cross-tenant opacity, restart during wait — FR-025-031, 033, 044 —
  `tests/fault/ai-agents-mcp/mcp_server.rs` — one run per key; opaque foreign answers.
- [ ] T025-051 [US6] Conformance with independent MCP client libraries (TypeScript and Python, pinned)
  at both revisions; runbook section — FR-025-027..033 — `tests/contract/ai-agents-mcp/mcp_server/`,
  `docs/runbooks/mcp.md` — conformance report.
- [ ] T025-074 [US6] REST refuses any token whose audience contains an MCP URI (feature-001 token
  verification, coordinated, G1296) — FR-025-028 — `crates/api-http/src/auth/` — REST contract test with
  an MCP-audience token and a multi-audience token.

## Phase 8: US07 — MCP client (P2)

- [ ] T025-052 [US7] External MCP server resources: routes, Studio, auth modes, secrets — FR-025-034 —
  `routes/mcp_servers.rs`, `frontend/apps/studio/src/features/ai/` — HTTP and component tests.
- [ ] T025-053 [US7] Refresh job (origin `MCP_TOOL_REFRESH`) in the runner; immutable snapshots with
  digests and bounds — FR-025-035 — `crates/infra-ai/src/mcp_client.rs`, `apps/agent-runner/` — tests
  against the test MCP server (both revisions).
- [ ] T025-054 [US7] `mcp` tool declarations pinned to snapshots at publication and admission —
  FR-025-010, 035 — `crates/infra-ir/src/agent.rs` — publication tests (`MCP_TOOL_NOT_ALLOWED`).
- [ ] T025-055 [US7] Untrusted-content handling and injection canaries: instructions in descriptions and
  results cannot call undeclared tools, skip approvals, exceed budgets or reach denied destinations
  (G1286) — FR-025-011, 017, 023, 035 — `tests/integration/ai-agents-mcp/untrusted.rs` — documented
  threat model in the runbook.
- [ ] T025-056 [US7] Drift detection before each call (`AI_TOOL_CHANGED`, `mcp.tool_changed`) —
  FR-025-035 — `crates/infra-ai/src/mcp_client.rs` — test: changed schema is never called.
- [ ] T025-057 [US7] `ResolveMcpCall` and MCP call execution: at-least-once, unknown outcome,
  unsupported results, text-only, secret scan, egress denial — FR-025-023, 036 —
  `crates/api-grpc/src/agent.rs`, `crates/infra-ai/src/mcp_client.rs` — `tests/fault/ai-agents-mcp/mcp_client.rs`.
- [ ] T025-058 [P] [US7] OAuth client credentials with resource indicator: issuer and token endpoint
  recorded in the revision and confirmed by an `ai:manage` holder, secret sent only to the recorded
  endpoint, refresh-time comparison with discovered metadata (`mcp.oauth_metadata_changed`, calls
  refused until revised) — FR-025-034, 036 — `crates/infra-ai/src/mcp_auth.rs`,
  `crates/application/src/mcp_servers.rs` — tests against the test authorization server, including a
  server that switches its advertised token endpoint.

## Phase 9: US08 and US09 (P2, P3)

- [ ] T025-059 [US8] Journal-derived agent graph and telemetry (turns, tool calls, approvals, usage;
  unknown kinds degrade) — FR-025-038, 041 — `crates/application/src/telemetry.rs`,
  `crates/telemetry/src/` — projection equals replay; unsupported-kind test.
- [ ] T025-060 [US8] Studio agent timeline and gated transcript viewer (explicit fetch, ephemeral
  state, WCAG 2.2 AA, table alternative, no sample values) — FR-025-022, 038 —
  `frontend/apps/studio/src/features/runs/{AgentTimeline,AgentTranscriptViewer}.tsx` — component,
  accessibility and real-browser tests.
- [ ] T025-061 [US8] Token streaming on `sagat.agent.tokens` through 026 `AppendStreamEvents` (gated):
  coalescing, per-call caps, the per-run reserved-channel budget clamped to 026's hard ceiling,
  publication refusal without 026 — FR-025-037 — `apps/agent-runner/`, `crates/infra-ir/src/agent.rs` —
  `agent-stream-multiturn` conformance (16 turns all streamed within budget); resumed stream equals
  committed text; superseded attempts marked (G1295).
- [ ] T025-062 [US9] Authoring assistant run (origin `AUTHORING_ASSISTANT`), provider level at least
  `internal`, proposal validation, apply as draft — FR-025-039 — `crates/application/src/agents/assistant.rs`,
  routes — tests: no version, activation or run created; a provider declared `public` is refused.
- [ ] T025-063 [US9] Studio assistant panel with diff and explicit apply — FR-025-038, 039 —
  `frontend/apps/studio/src/features/workflows/AuthoringAssistant.tsx` — component and browser tests.

## Phase 10: Polish and gates

- [ ] T025-064 [P] Telemetry: spans, fixed-label metrics, audit actions; no content in spans or logs —
  FR-025-041, 042 — `crates/telemetry/src/metrics.rs`, runner — telemetry tests with closed label sets.
- [ ] T025-065 [P] TypeScript and Go agent builders, transcript and approval clients, conformance —
  FR-025-040 — `sdks/typescript/src/agent.ts`, `sdks/go/workflow/agent.go` — scenarios of
  `contracts/conformance.md`.
- [ ] T025-066 [P] Python, C# and Java builders and clients; all-five conformance run — FR-025-040, 043
  — `sdks/python/sagat_octopus_orchestrator/agent.py`,
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/FlowAgent.cs`,
  `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/AgentOptions.java` —
  `sdk_conformance` all five.
- [ ] T025-067 Security battery: every new route, gRPC call and MCP method in the cross-tenant battery;
  runtime-role sweep; egress battery for providers and MCP — FR-025-023, 044, NFR-025-001 —
  `tests/contract/security-multitenancy/rest/battery.rs`, `tests/integration/ai-agents-mcp/` — zero
  findings.
- [ ] T025-068 Deployment: Dockerfile target `agent-runner` (non-root, read-only root, no platform
  credentials), Helm templates with default-deny egress, compose overlay — FR-025-024, 045 —
  `infra/docker/Dockerfile`, `infra/helm/templates/`, `infra/docker/compose.ai.yaml` — rendered chart
  tests; network policy test on a real cluster.
- [ ] T025-069 Documentation: runbooks (providers, agent runner, MCP; incident steps for spend, provider
  outage, leaked token), architecture sections (worker protocol, security, observability) — FR-025-045
  — `docs/runbooks/{ai-agents,agent-runner,mcp}.md`, `docs/architecture/*.md` — docs contract check.
- [ ] T025-070 Performance report with the dev provider at 1/16/64 concurrent agents, including
  `ai_usage_daily` contention — NFR-025-007 — `tests/performance/ai-agents-mcp/`,
  `tests/performance/reports/ai-agents-mcp.md` — measured p50/p95/p99 with environment and failures.
- [ ] T025-071 [P] Opt-in real-provider smoke tests (for example `claude-haiku-5-5` on Anthropic),
  reported separately and never the only evidence — FR-025-045 — `tests/e2e/ai-agents-mcp/real_providers.rs`
  — report states provider, date and cost; skipped when credentials are absent.
- [ ] T025-072 Final gates: workspace tests, clippy, architecture, contracts, five SDKs, browser e2e,
  quickstart walkthrough (`e2e_ai_agents_mcp quickstart::`); record evidence and remaining gaps — all
  FRs and SCs — `specs/025-ai-agents-mcp/verification.md` (created then) — every command and outcome
  recorded; no unchecked gate reported as passed.
- [ ] T025-075 [P] Amend feature 016's carrier text (FR-016-002 "exactly") for the two single-surface
  carriers added here, with the floors of D1291/D1292 (G1297) — FR-025-014, 017 —
  `specs/016-payload-classification/spec.md` (in the implementation delivery) — reviewed diff.
- [ ] T025-076 Coordinate with feature 026: public-link creation refuses versions with
  `agent_unattended_writes`, public-link runs take the link's creating principal as initiator for
  `ai:use`, reserved-channel budgets and the superseded control event (G1295) — FR-025-026, 037 —
  026 contracts and tests — 026 tests reference these rules.
- [ ] T025-077 Register 025's entries in the shared registries when their owners publish them (G1635):
  018 reset rules, ADR-0050 lock slots 4/5/8, 020 content stores, 028 time classes
  (`crates/infra-postgres/time-classification.json`), 031 encoded positions, 032 key inventory — FR-025-021,
  046, 047 — those drafts' contract files and the manifests — each registry's own consistency test passes
  with 025's entries.
- [ ] T025-078 Reset rules in 018's reset transaction: `REFUSE` inside an open agent loop,
  `REMATERIALIZE` of a finished agent step (rows copied, payloads shared, no call, no budget or daily
  charge) — FR-025-046 — `crates/application/src/agents/reset.rs` (new), 018 reset path —
  `tests/integration/ai-agents-mcp/reset.rs`: reset points before, inside and after a loop.
- [ ] T025-079 [P] Encoded-field checks for agent positions with 031's `EncodingCheck` and codes —
  FR-025-047 — `crates/infra-ir/src/agent.rs`, `crates/infra-ir/src/encoding_check.rs` (031) —
  publication fixtures for context, tool results, child inputs, agent output and exposure projections.
- [ ] T025-080 `pii` egress rule: reveal grant required to set a `pii` ceiling, rechecked at each
  `pii` resolution, revocation pauses the provider or MCP server, `ai.pii_egress` audit per attempt —
  FR-025-005, 034 — `crates/application/src/{ai_providers,mcp_servers,agent_capability}.rs` —
  integration tests: grant revoked between two turns pauses the provider and fails closed.
- [ ] T025-081 MCP visibility through the 029 access evaluator when 029 is enabled; `agent-runner` as an
  additive built-in role in 029's model — FR-025-030 — `crates/application/src/mcp_calls.rs` —
  tests with folder restrictions hiding an exposure's target from `tools/list` and `tools/call`.

## Dependencies and order

Phase 1 (including T025-073) blocks all stories. US01 (Phase 2) blocks US02–US09. US03, US04 and US05 depend on US02's
tool planning (T025-025, T025-026). US06 needs Phase 1 and the admission path only; US07 needs the
runner (T025-016, T025-017). US08 streaming needs feature 026; T025-028 needs feature 021; script and
operation exposures need feature 022. Phase 10 closes after every story it covers.

## Traceability

| Requirement group | Tasks | Success criteria |
|---|---|---|
| FR-025-001..005 | T025-005, 006, 022, 024, 080 | SC-025-001 |
| FR-025-006..009, 013, 014 | T025-003, 004, 009, 013..021 | SC-025-002, SC-025-003 |
| FR-025-010..012, 025 | T025-025..030 | SC-025-004 |
| FR-025-015, 016 | T025-036..039 | SC-025-006 |
| FR-025-017, 018 | T025-031..035 | SC-025-005 |
| FR-025-019..022 | T025-040..043, 075, 077 | SC-025-007, SC-025-008 |
| FR-025-023, 024 | T025-012, 016, 017, 057, 067, 068, 073 | SC-025-009, SC-025-010 |
| FR-025-026..033 | T025-044..051, 074, 076, 081 | SC-025-011 |
| FR-025-034..036 | T025-052..058, 080 | SC-025-012 |
| FR-025-037 | T025-061, 076 | SC-025-013 |
| FR-025-038, 039 | T025-024, 035, 047, 060, 062, 063 | SC-025-001, SC-025-016 |
| FR-025-040, 043 | T025-002, 065, 066 | SC-025-014 |
| FR-025-041, 042 | T025-002, 059, 064 | SC-025-015 |
| FR-025-044 | T025-022, 050, 067 | SC-025-001, SC-025-011 |
| FR-025-045 | T025-023, 068..072 | SC-025-017 |
| FR-025-046, 047 | T025-077..079 | SC-025-018 |
