# Tasks: AI agents and MCP (025)

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/README.md), [quickstart.md](quickstart.md).

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decisions 0059 and 0060 accepted (delegate, Owner decisions 2026-11-26-123
and 2026-11-26-124); not started. A task is checked only with the evidence it names, run for real and
recorded (constitution VIII). Test tasks are written first and must fail before implementation.
Revised 2026-11-26 against `develop` at `3f22b612` after an independent audit; tasks 025-082 to
025-092 were added then. Revised 2026-11-27 after the security review of that date (Decision
025-016): tasks 025-093 to 025-099 were added and the tests of the existing tasks it touched were
extended in place. Revised again on 2026-11-27 (round 3) for Decisions 025-017 to 025-019 (Owner
decisions 2026-11-27-2 and 2026-11-27-400 to 2026-11-27-403): tasks 025-100 to 025-103 were added,
Task 025-096 was rewritten in place, and the tasks that stated the replaced rules were amended in place.

Format: `T025-NNN [P?] [USn?] Description — requirements — files — evidence`. `[P]` = different
files, no dependency on an unfinished task in the same phase. Test targets: `contract_ai_agents_mcp`,
`integration_ai_agents_mcp`, `fault_ai_agents_mcp`, `e2e_ai_agents_mcp`, `perf_ai_agents_mcp`
(declared in `tests/Cargo.toml`), plus `sdk_conformance`. An indented `Owner-gate:` line names the
owner decision a task builds on; the OAuth tasks cite Owner decision 2026-11-25-3, the owner's approval of
constitution 5.1.0's third declared set of Principle V (Gap 025-001). How the tasks are cut into
slices and two parallel lanes is in [Build slices](#build-slices).

## Phase 1: Setup and foundations (blocks every story)

- [ ] T025-001 Confirm at build start that Owner decisions 2026-11-26-120 to 2026-11-26-125 (the answers
  to Owner questions 1280 to 1282, the acceptance of Architecture decisions 0059 and 0060, the
  checklist dispositions) are rows of `docs/owner-decisions.md`; record the status of features 021,
  022, 026 and of designs 040, 044 and 047 at that date, and the worker-protocol minor that is next
  free in the register of `sdks/SPEC.md` section 9 — all FRs — `specs/025-ai-agents-mcp/*`,
  `docs/adr/ADR-0059-*.md`, `docs/adr/ADR-0060-*.md` — the rows and statuses linked from research.md.
- [ ] T025-002 Merge the contract fragments into the canonical contracts and add their checks:
  `workflow.schema.json`, `openapi.yaml`, `worker.proto` + `worker.baseline.json` at the next free
  minor, the row of that minor in the register of worker-protocol minors of `sdks/SPEC.md` section 9
  (Gap 1280), `model-turn.schema.json`, remediation rows in `crates/api-http/src/error.rs`, and the five
  test targets in `tests/Cargo.toml` — FR-025-006, 042, 043 — `specs/001-platform-foundation/contracts/`,
  `sdks/SPEC.md`, `tests/Cargo.toml`, `tests/contract/ai-agents-mcp/errors.rs` — CI `contracts` lint and
  compatibility tests pass; old fixtures still accepted; the register row names feature 025.
- [ ] T025-003 [P] Derived ids `<node>~m<n>`, `<node>~t<n>.<k>` and the tool-call compensation and
  reconciliation ids `<node>~t<n>.<k>~c`/`~r` in `NodeId`, `parse_stored` and every derived-id consumer,
  beside the existing `~c`/`~r` and, when feature 035's destinations land, `~d.<name>` (Architecture
  decision 0071 decision 5); agent node ids ≤ 54 characters; forged ids, `<agent>~m<n>~c`, any `~d.`
  after an agent form and other two-suffix forms rejected — FR-025-006, 008, 012, 025 —
  `crates/domain/src/workflow.rs` — unit and property tests including the 64-character bound at
  `~t64.256~c`; existing `~c`/`~r` behaviour unchanged.
- [ ] T025-004 [P] Domain agent model: declaration, tools, budgets arithmetic (integer tokens, cost
  micros), pure planner returning typed commands, and the derived-node function that materializes the
  `Node` of a stored derived id (Decision 025-003) — FR-025-006, 008, 010, 011, 013..015 —
  `crates/domain/src/agent.rs`, `crates/domain/src/agent/` — property tests: settled + reserved never
  exceeds a limit; exactly one next turn per settled turn; refusal results ordered.
- [ ] T025-005 [P] Domain provider, MCP and identity types: provider kinds, model entries, prices, the
  data level as the existing `EgressCeiling`; exposures, snapshot digests, token scope; six permissions
  (`Permission::ALL` 55 to 61) and role `agent-runner` with the default grants of Owner decision
  2026-11-26-121; AI quota resources (`ai_runner_concurrency`, `tenant_ai_runner_concurrency`, ...) —
  FR-025-001, 002, 005, 016, 026, 029 — `crates/domain/src/{ai_provider,mcp,identity,quota}.rs` — unit
  tests; permission catalog test updated (61 entries, eight roles; `agent-runner` has no
  `database:execute`).
  Owner-gate: OD-2026-11-26-121
- [ ] T025-006 Provider and MCP-server resource types on feature 021's resource model (no second
  catalog): type registrations `llm_anthropic`, `llm_openai_compatible`, `llm_aws_bedrock`,
  `llm_azure_openai`, `mcp_server` with the agent runner as executor, their closed settings schemas,
  pins in `run_resource_bindings`, and the secret slot in the primary key of 021's
  `step_resource_credentials` if 021 has not added it (Decision 025-014); starts after 021's resource
  core merges — FR-025-001, 002, 034, 044 — `crates/infra-postgres/migrations/`,
  `crates/infra-postgres/src/` (021's resource store), `storage-classification.json` — real PostgreSQL
  tests: immutability, receipt replay/conflict, runtime-role privileges, one pin per slot of a
  two-slot test provider, classification manifest check.
  Owner-gate: OD-2026-11-26-122
- [ ] T025-007 Agent run-time tables: `agent_steps`, `agent_turns`, `agent_call_attempts` (with the
  trigger that keeps the reserved and settled counters of `agent_steps` equal to the sum of its attempt
  rows), `agent_tool_calls`, `agent_transcript_entries`; budget CHECKs; down refuses live data —
  FR-025-008, 015, 016, 021 — `crates/infra-postgres/migrations/`, `src/agents.rs` — migration up/down
  tests, counter-equals-sum trigger tests (insert, settle, release, charged-unknown), runtime-role
  sweep against the grants table of data-model.md section 8 (every privilege not listed is refused to
  `sagat_app`); `agent_tool_calls.tool_name` refuses a value that is neither a declared name of the
  node nor `UNKNOWN_TOOL`, and the table has no provider tool-use id column.
- [ ] T025-008 MCP tables: `mcp_server_heads`, snapshots, snapshot tools, `version_agent_pins`,
  `run_mcp_snapshot_pins`, exposures and revisions (with `max_daily_tokens`), tokens (exposure
  identifiers, the mint request digest, no plaintext), call receipts keyed by the caller as the
  credential identity, task handles — FR-025-026, 029, 031, 035 —
  `crates/infra-postgres/migrations/`, `src/mcp.rs` — migration tests, token hash uniqueness, receipt
  window on the database clock, a receipt row of one scoped token never matched by another token of the
  same minter, runtime-role sweep against data-model.md section 8.
- [ ] T025-009 IR codec and publication validation of `agent`: the `ai-agents-v1` requirement recorded
  (Task 025-082), graph-model version, root-only, no `chainPolicy`, no destinations, `agentTool`
  closed (`unevaluatedProperties: false`), schemas inside `agentClosedSchema` compiled by the design 043
  `RunInputValidator`, bounds, tool names, model allowlist, `UNPRICED_MODEL`, the publisher's
  `secret:use` on every credential secret of every provider and MCP-server revision the version will
  pin, the transcript level (D1319) against the provider and MCP ceilings (`EgressCeiling::admits`),
  the `result`, approval-view and child-input floors (`CLASSIFICATION_LOOSENED`), the inherited output
  levels of Task 025-093, `write` tools with approval or `approval: "none"`, `operation` tools and
  database `execute` tools with a policy (`approval: "none"` and `effect: "read"` refused for
  `execute`), `child` tools declared `read` only with a policy, `mcp` tools declared `read` without a
  policy only when the pinned MCP-server revision classifies the tool `read` (Task 025-102, whatever the
  snapshot's annotations say), `decisionFields` narrowed only on tools that may run with
  `approval: "none"` (`DECISION_FIELDS_REQUIRED`), no tool input field annotated `secret`
  (`SECRET_TOOL_INPUT`), and the transitive version flag
  `agent_unattended_writes` of Task 025-094 — FR-025-003, 006, 010, 011, 013..015, 017, 019 —
  `crates/infra-ir/src/agent.rs`, `crates/infra-ir/src/classification_check.rs` — publication fixtures
  for every reason in `contracts/errors.md`, a tool with a misspelled key (refused), a publisher
  without `secret:use` on a provider secret (refused), a database `execute` tool with
  `approval: "none"` (refused `WRITE_TOOL_APPROVAL_REQUIRED`), a `child` tool declared `read` without a
  policy (refused), an `mcp` tool declared `read` on a tool its pinned revision does not classify, and
  on one classified `write` whose snapshot states `readOnlyHint: true` (both refused), an `operation`
  tool with `decisionFields: []` (refused `DECISION_FIELDS_REQUIRED`), a `secret` tool input field
  (refused).
- [ ] T025-010 Application ports and the agent transactions of plan.md (activation, model completion,
  tool completion, approval outcome, cancellation, deadline) with `begin_safely` and the stated lock
  order — FR-025-008, 012, 017, 018 — `crates/application/src/agents.rs`, `agents/`,
  `crates/infra-postgres/src/agents.rs` — real PostgreSQL tests per transaction, including rollback of
  every statement; the slot-order barrier test of Architecture decision 0050
  (`tests/fault/schedules-bulk-operations/slot_order.rs`) extended to the agent admission path (slots 4b
  and 8b).
- [ ] T025-073 [P] Architecture check: extend the restricted-dependent rules `scripts/check-architecture.py`
  already has (lines 60-67, the `ICEBERG_CRATE`/`ICEBERG_TEST_ONLY_PACKAGES` and
  `LAKE_RUNNER_PACKAGES`/`CONTAINER_ENGINE_CRATES` patterns): `sagat-infra-ai` usable only by
  `sagat-agent-runner` and `sagat-tests`, `sagat-api-mcp` only by `sagat-control-plane` and
  `sagat-tests`, and `sagat-agent-runner` and `sagat-infra-ai` added to the packages that may not link a
  container engine — FR-025-024, Architecture decisions 0059 and 0060 — `scripts/check-architecture.py`,
  `scripts/tests/test_check_architecture.py` — CI `architecture` fails on each forbidden dependent, seen
  failing with saved metadata first.
- [ ] T025-082 Instance-registry capabilities `ai-agents-v1` (kinds `CONTROL_PLANE`,
  `EXECUTION_ENGINE`) and `mcp-server-v1` (kind `CONTROL_PLANE`): one migration extending the closed
  CHECKs `serving_instances_capabilities_closed` and `platform_requirements_capability_valid`,
  `Capability::ALL` and `as_str` in `crates/application/src/instances.rs`, the process lists each only
  when its configuration runs agents or the MCP endpoint (Architecture decision 0082 decision 7);
  publication refused and the MCP endpoint `404` until recorded (Decision 025-002) — FR-025-006, 043 —
  `crates/infra-postgres/migrations/`, `crates/application/src/instances.rs` — the registry test that
  keeps the schema list and `Capability::ALL` equal; activation refused while a live process lacks the
  capability; publication refused before activation.
- [ ] T025-083 [P] Managed kinds and the agent profile: `ManagedKind::{AiModel, AiMcp, AiProbe,
  AiMcpRefresh}` with their `activity()` and `profile()`, `BindingProfile::Agent` (stored `agent`),
  registration of an `agent-runner` identity only for that profile, and the derived-id to managed-kind
  mapping of data-model.md section 4 through the agent declaration's derived-node function (the node
  of every `sagat.ai.*` kind materialized with an empty `secrets` list, a unit test per kind); extend
  `crates/application/tests/managed_kind_sites.rs` to refuse a worker path that tests a derived-id
  suffix or a `sagat.ai.` name instead of asking `Node::managed_kind` — FR-025-009, 024, 036 —
  `crates/domain/src/database.rs`, `crates/domain/src/binding.rs`, `crates/domain/src/agent/`,
  `crates/application/tests/managed_kind_sites.rs` — unit tests per mapping row; the site check seen
  failing on a planted suffix test; a database or lake binding cannot register for `sagat.ai.*` and an
  agent binding cannot register for `sagat.database.*`.
  Owner-gate: OD-2026-11-26-121
- [ ] T025-087 [P] SDK error catalog: every `AiFailure` code of `contracts/errors.md` added to
  `sdks/error-catalog.json` as a `worker`-origin entry (gRPC status, `retryable`, remediation) and to
  `sdks/SPEC.md` section 2, every SDK's embedded copy regenerated — FR-025-042 — `sdks/error-catalog.json`,
  `sdks/SPEC.md`, the six embedded copies (`sdks/rust/src/error-catalog.json` and the others) — each
  SDK's copy-equals-catalog test passes in all six SDKs.

## Phase 2: US01 — provider and durable model call (P1, MVP)

- [ ] T025-011 [P] [US1] Dependency spike and pins: SSE parser, MCP codec (in-repo vs library, written
  comparison), advisory audit, and the check that `aws-bedrock` accepts an Amazon Bedrock API key as a
  bearer token, including a short-term Bedrock API key such as design 047's broker mints for the
  `aws-access-key` mode (Task 025-101); no SigV4 signer is pinned in the runner (Decision 025-017) —
  FR-025-003, 027 — `Cargo.toml`, research.md — `cargo deny`/audit clean; comparison recorded (G1291);
  if the Bedrock bearer mode is not usable, `aws-bedrock` is recorded as waiting for design 047's broker
  instead.
- [ ] T025-012 [P] [US1] Bounded incremental body reader in the egress guard as a new variant of
  `ResponseBody` taken by the existing `send_with` (bytes, idle and total deadline, same resolver
  checks), not a second client — FR-025-023 — `crates/infra-egress/src/lib.rs` — SSRF, redirect,
  slow-drip and oversize tests (G1283); a `GET` answered `302` with `max_redirects = 0` returns the
  `302` and makes no request to the `Location` host (the guard copies every request header to each hop,
  lines 257-259, so zero hops is what keeps a credential header at the first host).
- [ ] T025-013 [US1] `crates/infra-ai`: canonical turn mapping and the `anthropic` adapter (Messages API,
  tools, `output_config.format`, streaming, usage, stop reasons, error mapping, `request-id`, no forced
  tool choice, opaque blocks replayed verbatim) — FR-025-007, 009, 013, 014 — `crates/infra-ai/src/` —
  adapter tests against `apps/dev-model-provider` recordings.
- [ ] T025-014 [P] [US1] `openai-compatible` adapter (Chat Completions, tools, JSON-schema format) —
  FR-025-009, 013 — `crates/infra-ai/src/openai_compatible.rs` — same adapter suite.
- [ ] T025-015 [P] [US1] `aws-bedrock` and `azure-openai` adapters with capability flags and validation
  fallback, in the modes that hand the runner a static API key or bearer token only: `aws-bedrock` with
  an Amazon Bedrock API key as a bearer token (verified by the Task 025-011 spike, Gap 1291) and
  `azure-openai` with an API key; the runner never builds a SigV4 signing path nor an Entra token
  exchange: once design 047's broker exists, both modes reach the adapters as access tokens (Tasks
  025-100 and 025-101) — FR-025-003, 009, 013 — `crates/infra-ai/src/{bedrock,azure}.rs` — adapter
  suite; a test that no adapter accepts a secret access key or a client secret slot.
- [ ] T025-016 [US1] `apps/agent-runner`: config, registration under `BindingProfile::Agent`, finite
  credit polling, heartbeat, cancellation, closed failure vocabulary, no DB/OpenBao/signing key/database
  credential/container engine, panic hook without payload, explicit static provider credentials with SDK
  default credential chains disabled, never calls `AppendStepLogs` and sends no `StepLogCarry` —
  the egress client built with `max_redirects = 0` for every provider and MCP request, and the
  per-namespace egress policy read from operator-owned runner configuration before every attempt (the
  `apps/database-runner/src/config.rs` precedent) — FR-025-003, 009, 023, 024 — `apps/agent-runner/` —
  integration tests; a test that the process environment and config contain no platform credential; a
  test that link-local and metadata addresses are unreachable from the runner process and its adapters;
  a test that the runner's client has `max_redirects = 0`; a probe answered `302` by the dev provider
  fails `AI_DESTINATION_DENIED` and the redirect target receives no request; a namespace removed from
  the configuration is denied at the next attempt without a restart.
- [ ] T025-017 [US1] `ResolveModelCall` and `ResolveMcpCall` application use cases: two phases, the
  Principle V first-set authorities of Requirement 025-003 (membership, `BindingProfile::Agent`
  binding, pool-delivery policy, the publisher's `secret:use` on every declared secret, the initiator's
  `ai:use`), the recorded lease-owning subject and live session, revalidation after OpenBao I/O,
  credential pins per secret slot and the `secret.resolved` audit before values, the current-restriction
  recheck of Task 025-097 in both phases, each resolve call refusing every kind but its own; and the
  ordinary `ResolveSecrets` use case (`crates/application/src/worker/secrets.rs`, which today delivers
  `node.secrets()` of any node, `secret_resolution.rs` lines 19-28) refusing a lease whose materialized
  node has any managed kind — FR-025-003, 009, 024, 036 — `crates/application/src/agent_capability.rs`,
  `crates/application/src/worker/secrets.rs` — revocation of `secret:use` between phases, stale
  lease, expired session, a database or lake runner and an ordinary worker denied, all or nothing on a
  two-slot provider, audit failure returns no value; an agent-runner token calling `ResolveSecrets` on
  a model-call, MCP-call, probe and refresh lease is denied and nothing is resolved; `ResolveSecrets` on
  a managed database lease is denied too; the database-runner suites still pass.
- [ ] T025-018 [US1] Server-side request rendering (literals, context evaluation, canonical JSON blocks,
  tool definitions, transcript, digest) — FR-025-007, 020 — `crates/application/src/agents/render.rs` —
  determinism and no-secret-capability tests.
- [ ] T025-019 [US1] Engine wiring of activation and model completion, outbox hints, run graph
  `AgentProgress` — FR-025-008, 014 — `crates/scheduler/`, `crates/application/src/agents.rs` — real
  engine test with a test harness that plays the runner over the application ports: one-turn agent
  succeeds end to end.
- [ ] T025-020 [US1] Structured output validation through the design 043 `RunInputValidator`, repair
  turns whose summary carries only the validator's value-free rule and field, generated output carrier
  — FR-025-013, 014 — `crates/application/src/agents/output.rs` — valid, repaired and
  `AI_OUTPUT_INVALID` cases; a canary value in an invalid answer never appears in the summary.
- [ ] T025-021 [US1] Fault tests: crash before and after the provider answered, duplicate completion,
  lost broker hint, engine and runner restart, a lost response followed by a retry at the budget edge,
  and a runner killed while holding leases — FR-025-008, 009, 015, 016, NFR-025-002 —
  `tests/fault/ai-agents-mcp/model_calls.rs` — one committed turn; settled + reserved ≤ limit for the
  agent and for the daily budget (folded usage plus leases) at every step; concurrency free after lease
  expiry; journal equals projection (`verify_run`).
- [ ] T025-022 [US1] Provider routes on 021's resource model, audit and probe admission (origin
  `AI_PROVIDER_PROBE`, through `start_private`, row 13 of the design 043 gate); `ai:manage` and
  `secret:use` with each secret's access list to save; the `pii` ceiling under the rule of Owner
  decision 2026-11-26-120; credential modes `aws-access-key` and `azure-entra-client-credentials`
  refused with `AI_CREDENTIAL_MODE_UNAVAILABLE` until design 047's broker exists (Gap 025-001; Tasks
  025-100 and 025-101 enable them through it), and the `entraAuthority` setting recorded for when the
  Entra mode is enabled — FR-025-001..005, 044 —
  `crates/api-http/src/routes/ai_providers.rs` — HTTP tests incl. cross-tenant opacity, paused provider,
  receipts, `SECRET_NOT_USABLE`, `REVEAL_GRANT_REQUIRED`, `AI_CREDENTIAL_MODE_UNAVAILABLE` for both
  modes.
  Owner-gate: OD-2026-11-26-120, OD-2026-11-26-122
- [ ] T025-023 [US1] `apps/dev-model-provider`: scripted scenarios (answer, tool requests, invalid JSON,
  `429`/`529`, stream failure, refusal, hold, `redirect-probe` answering the model listing with a `302`,
  `undeclared-tools` requesting 256 undeclared tools named after a canary, a reported model other than
  the requested one), test MCP server (bearer mode; an `isError` tool; a tool answering `302`), refuses
  to start outside development — FR-025-045 — `apps/dev-model-provider/`, `infra/docker/compose.ai.yaml`
  (layered on `compose.database.yaml` and its OpenBao; no second OpenBao) — its own tests; production
  image check that it is absent.
- [ ] T025-024 [US1] Studio provider screens (create/rotate secret inline, probe, data-level choice with
  `internal` preselected, provider-retention notice) and agent node editor emitting canonical IR —
  FR-025-001, 005, 021, 038 — `frontend/apps/studio/src/features/ai/`,
  `features/workflows/AgentFields.tsx` — component tests; generated-client check.
  Owner-gate: OD-2026-11-26-120, OD-2026-11-26-122

## Phase 3: US02 — durable tools (P1)

- [ ] T025-025 [US2] Tool planning: declared-name check, input validation through the design 043
  `RunInputValidator` (results carry only its value-free rule and field), per-turn
  (`maxParallelToolCalls`) and total limits, refusal results in request order — FR-025-010, 011 —
  `crates/application/src/agents/tools.rs` — integration tests for every refusal reason.
- [ ] T025-026 [US2] Tool-completion transaction and last-of-turn scheduling — FR-025-008, 012 —
  `crates/application/src/agents.rs` — concurrent completion test: exactly one next turn over 100 rounds.
- [ ] T025-027 [US2] Executors per kind: activity (ordinary lease of the derived step), script, database
  query (execute only with approval), child (tree budget; admission through the design 043 gate's child
  row, a refused input failing the tool-call step `CHILD_INPUT_INVALID` and returning `TOOL_FAILED` to
  the model, Decision 025-007); stable keys; unknown outcome to reconciliation or manual review per the
  kind rules of Decision 025-011 — FR-025-012 — `crates/application/src/agents/execution.rs` —
  `tests/fault/ai-agents-mcp/tools.rs`: crash after effect, duplicate delivery, lost hint; a refused
  child input creates no child and the agent continues.
- [ ] T025-028 [P] [US2] `operation` tool kind on feature 021 `resourceOperation` (gated) — FR-025-010,
  012 — `crates/application/src/agents/execution.rs` — refused while 021 is absent; fixture when present.
  Owner-gate: OD-2026-11-26-122
- [ ] T025-029 [US2] Saga compensation of tool calls in reverse completion order; irreversible writes;
  failed compensation to manual review — FR-025-025 — `crates/domain/src/agent/`, saga reducer —
  `tests/fault/ai-agents-mcp/sagas.rs`.
- [ ] T025-030 [US2] Tool result handling: bounds and truncation, secret scan, projection, carrier
  validation, data-level check (`CLASSIFICATION_EGRESS_REFUSED` before dispatch); the scan for agent
  tool results and agent-runner outputs extended from the existing verbatim and JSON-escaped forms
  (`crates/application/src/worker.rs`, minimum 8 bytes) to base64 (standard and URL-safe, three byte
  alignments) and hexadecimal (both cases) — FR-025-012, 019, 020 —
  `crates/application/src/agents/results.rs` — tests per rule; one test per encoded form of a resolved
  secret (each refused `SECRET_IN_OUTPUT`); a secret shorter than 8 bytes recorded as a known gap
  (Gap 025-004), not as a pass.
- [ ] T025-031 [US2] Cancellation and deadline: heartbeat stop, approvals cancelled, child policy,
  first-commit races — FR-025-018 — `crates/application/src/agents.rs`, `crates/scheduler/` —
  `tests/fault/ai-agents-mcp/races.rs` (deadline vs completion, cancel vs completion, 50 rounds each).

## Phase 4: US03 — tool approval (P1)

- [ ] T025-032 [US3] `approval_requests` owner kind `AGENT_TOOL_CALL`, the single shared amendment of
  feature 014 that 026 cites (coordinator C22): migration, constraint versions, the escalation columns
  of Decision 025-019 (`decision_fields`, `escalation_policy`, `escalated_at`, `context_checked_at`,
  `no_decider_reason`, null for every other owner kind) and their partial index, `GRANT UPDATE
  (escalated_at, context_checked_at, no_decider_reason) ON approval_requests TO sagat_app` (Decision
  025-020), lock-order review with feature 014 (G1289) — FR-025-017 — `crates/infra-postgres/migrations/`,
  `src/primitives.rs` — 014 approval suites still pass; new owner-kind tests; the runtime-role sweep
  (Task 025-007) runs the escalation and the context check with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
- [ ] T025-033 [US3] Open, decide, expire and cancel tool approvals; `refuse`/`fail`; clamped deadline;
  the decision refusal of Task 025-096 and the escalation of Task 025-103 — FR-025-017, 018 —
  `crates/application/src/agents/approvals.rs` — races decision vs expiry vs escalation vs cancel (50
  rounds), unauthorized/revoked/cross-tenant/initiator denials audited.
- [ ] T025-034 [US3] Approver redacted view of the tool call with the input schema as carrier: the
  decision context, the redacted transcript outline and the input with withheld fields masked (path and
  level only), with the `decidable` flag and the masked-field list of Task 025-096 — FR-025-017 —
  `crates/api-http/src/routes/agent_tool_calls.rs` — disclosure matrix per level; no length, digest or
  prefix of a masked field in the body; no-store; audit; `decidable` false exactly when a decision field
  is withheld.
- [ ] T025-035 [US3] Studio approval panel for tool calls — FR-025-017, 038 — `frontend/apps/studio/src/features/runs/ToolApproval.tsx` — component and accessibility tests.

## Phase 5: US04 — budgets and quotas (P1)

- [ ] T025-036 [US4] Per-attempt reservation in the claim transaction (`agent_call_attempts`), settlement,
  release on provider error responses, charged-unknown on loss and expiry, the daily budget through the
  usage leases of Task 025-086, exhaustion before any lease; per-adapter verification of the text-token
  bound (G1284) and of the "error responses are not billed" assumption (G1294) — FR-025-015,
  NFR-025-003 — `crates/domain/src/agent/budget.rs`, `crates/application/src/agents.rs`,
  `crates/infra-postgres/src/agents.rs` — property tests; counter-equals-sum trigger tests; adapter
  fixtures comparing bound and reported usage; the claim lines of Task 025-085's budget hold.
- [ ] T025-037 [P] [US4] Estimated cost from declared prices, currency rules, `UNPRICED_MODEL` —
  FR-025-002, 015 — `crates/domain/src/ai_provider.rs` — unit and publication tests.
- [ ] T025-038 [US4] AI quotas: `ai_runner_concurrency` and `tenant_ai_runner_concurrency` as the count
  of live leases of every `sagat.ai.*` activity in the claim transaction (no counter), daily tokens and
  cost (settled + reserved through the leases), resource counts, zero refuses, lowered limits —
  FR-025-016 — `crates/domain/src/quota.rs`, `crates/infra-postgres/src/agents.rs` — quota tests with
  the runtime role; kill-the-runner test frees capacity at lease expiry; a probe and an MCP call count
  toward the same limit; ordinary claim statement count unchanged (Task 025-085).
- [ ] T025-039 [US4] Every ceiling of plan.md enforced at publication or admission, at and beyond the
  boundary — FR-025-015, 016, 033 — `crates/infra-ir/src/agent.rs`, routes — boundary tests.
- [ ] T025-085 [US4] Claim and transition budget of design 038: the agent claim and completion get their
  own lines in the statement budget (`tests/integration/claim-transition-efficiency/work_budget.rs`), the
  domain predicate "a run of this version can have agent rows" lets ordinary completions skip every
  agent-row load (design 038 step 8), with its property test that no agent row exists for a run the
  predicate excludes (operator commands, cancellation and timeouts included) — FR-025-008, 016,
  NFR-025-007 — `crates/domain/src/agent/`, `crates/application/src/worker.rs` and its claim path,
  `tests/integration/claim-transition-efficiency/` — `integration_claim_transition_efficiency` passes
  with the ordinary budget unchanged and the new lines recorded; the predicate's property test.
- [ ] T025-086 [US4] AI usage leases on the pattern of Architecture decision 0097: `ai_usage_leases` keyed
  by tenant, namespace, day, currency and writer (`pg_backend_pid()`), charge to the writer's own lease,
  refill from `ai_usage_daily` only when short (the daily row first, then the own lease `NOWAIT`), the
  purge pass folding idle and ended-backend leases, the exact daily check, the daily invariant test
  (Decision 025-010), the `budget_scope` key for an exposure's `maxDailyTokens` (Task 025-099), and the
  definer functions of data-model.md section 8 (fixed `search_path = pg_catalog, pg_temp`, every name
  schema-qualified, `REVOKE ALL … FROM PUBLIC`, the writer read from `pg_backend_pid()` inside each
  function, never an argument) — FR-025-015, 016 — `crates/infra-postgres/migrations/`,
  `crates/infra-postgres/src/ai_usage_leases.rs`, `crates/application/src/maintenance.rs` — invariant
  test (folded plus leased equals the sum of the day's attempts after a random interleaving), a
  backend ended mid-day folded back, the limit exact under 16 concurrent claimers, contention measured in
  Task 025-070; the runtime role has no privilege on `ai_usage_leases`, cannot write `ai_usage_daily`
  directly, and a definer function asked to charge another backend's lease has no argument through which
  to do so (a catalog test over the functions' signatures and `proconfig`).

## Phase 6: US05 — data protection (P1)

- [ ] T025-040 [US5] Transcript level and prompt classification: publication computation (literals,
  context paths, tool-result carriers, no carrier as `pii`), provider and MCP ceilings
  (`EgressCeiling::admits`, `Level::stricter`), output, approval-view and child-input floors, runtime
  refusal of non-validating values (`CLASSIFICATION_EGRESS_REFUSED`) — FR-025-014, 017, 019 —
  `crates/infra-ir/src/classification_check.rs`, `crates/application/src/agents/render.rs` —
  classification battery, including a `pii` context with a `public` result field (refused), an MCP
  server below the transcript level (refused), the inherited output levels of Task 025-093, and the
  case "ceiling lowered after publication" of Task 025-097 (a provider and an MCP server each lowered by
  a new revision without republication: the next admission refused `AI_DESTINATION_RESTRICTED`, the next
  call of a running agent refused `CLASSIFICATION_EGRESS_REFUSED` with no request reaching the dev
  provider or the test MCP server).
- [ ] T025-041 [US5] Transcript storage and audited disclosure endpoint (016 `pii` audience,
  `payload.pii_revealed` before bytes, no-store, bounds); the existing `payload-disclosures` and
  `payload-inspections` run endpoints refusing every agent derived-step payload and transcript entry as
  a payload that does not exist, the agent node's retained output still disclosable under its generated
  carrier — FR-025-021, 022 — `crates/application/src/agent_transcripts.rs`, `routes/agent_transcripts.rs`,
  the feature-016 payload selection in `crates/application/src/` — rights matrix, revocation between
  pages, audit failure returns no bytes; a holder of all three rights asking `payload-disclosures` and
  `payload-inspections` for a model-call response, a tool input, an MCP result and a transcript entry
  gets the not-found answer and no bytes.
- [ ] T025-042 [US5] Secret and MCP-argument canary battery across prompts sent to the dev provider,
  arguments received by the test MCP server, transcripts, journal, logs, spans, audit, stream events,
  step logs and responses — FR-025-019, 020, NFR-025-001 — `tests/integration/ai-agents-mcp/canaries.rs`
  — zero canary occurrences, for verbatim, JSON-escaped, base64 (three alignments, standard and
  URL-safe) and hexadecimal canaries of at least 8 bytes; no argument above the server's
  `maxArgumentClassification` reaches it; an `MCP_TOOL_ERROR` text never reaches an agent whose
  transcript level is below `pii`. Canaries shorter than 8 bytes and doubly encoded or compressed ones
  are run and reported as known gaps (Gap 025-004), never counted as passes, as are a base64 canary
  resolved by a child workflow's activity and returned through a `child` tool, and a non-UTF-8 canary
  (Decision 025-020).
- [ ] T025-043 [P] [US5] Retention: manifest entries, deletion with the run, snapshot maintenance job —
  FR-025-021 — `storage-classification.json`, `crates/application/src/maintenance.rs` — retention tests.
- [ ] T025-084 [US5] Step logs refused for the agent runner (design 041): an `agent-runner` token refused
  at the ingest's pre-check P1 and authority check T7, a carry on a `sagat.ai.*` managed step ignored at
  carry check C0, no `StepLogPolicy` in an agent-runner registration (Decision 025-005); this touches
  design 041's paths, so the CI gate runs `integration_step_logs` — FR-025-020, 024 — the step-log ingest
  and report-carry paths of design 041 in `crates/application/src/` and the registration response —
  `integration_step_logs` with an agent-runner token (refused, audited `AUTHORITY`), a carry on a model
  call (ignored, nothing stored), and the existing step-log suite unchanged.

## Phase 7: US06 — MCP server (P2)

- [ ] T025-044 [US6] `crates/api-mcp`: Streamable HTTP JSON-RPC, 2026-07-28 and stateless 2025-11-25,
  `server/discover`, header checks, error mapping, `404` until `mcp-server-v1` is recorded, batch
  bodies refused with `-32600` before any member is read — FR-025-027, 033 — `crates/api-mcp/`,
  `apps/control-plane/` — protocol unit tests; a batch of two `tools/call` members admits nothing and
  charges no rate-limit token twice.
- [ ] T025-045 [US6] OAuth resource server: Protected Resource Metadata answered identically for every
  well-formed namespace id, single-audience check, `WWW-Authenticate`, `insufficient_scope`,
  query-token refusal, no passthrough — FR-025-028 — `crates/api-mcp/src/auth.rs` — security tests,
  including a multi-audience token (refused) and an existence-oracle probe on the metadata endpoint.
- [ ] T025-046 [US6] Scoped MCP tokens: mint, list, revoke (minter with `mcp:token:manage` or any
  `ai:manage` holder), hash, intersection with the minter's current rights, audit; exposure names
  resolved to identifiers at mint; a mint replay returning metadata without the token — FR-025-029 —
  `crates/application/src/mcp_tokens.rs`, `routes/mcp_tokens.rs` — tests incl. minter demotion; a
  replayed mint returns `200` with no `token` field and no table or receipt holds the plaintext; an
  exposure deleted and recreated under the same name is not callable with the old token.
- [ ] T025-047 [US6] Exposures: routes, revisions, projection compile and level checks against the
  target version's recorded effective output levels (Task 025-093), exposer rights (`namespace:read` to
  read, `mcp:expose` to delete), refusal of unattended target versions at any child depth (Task
  025-094; `UNATTENDED_WRITE_TOOL`, also at call time for ACTIVE-policy exposures) or with an
  `inputSchema` outside the design 043 subset (`RUN_INPUT_SCHEMA_UNUSABLE`), Studio screens —
  FR-025-026, 031, 032, 038 — `crates/application/src/mcp_exposures.rs`,
  `frontend/apps/studio/src/features/ai/` — tests per reason; a projection of a field annotated
  `public` and drawn from an agent's `pii` result cannot be saved (publication refuses the target
  first); a delete without `mcp:expose` is refused.
- [ ] T025-048 [US6] `tools/list` filtering and order; `tools/call` admission through the run-input gate
  row of Task 025-089 with receipts, bounded wait (026 `executeRun` when present), uncallable exposures
  answered as unknown tools, task handles, and full re-authorization on every `tasks/get` and
  `tasks/cancel`; the caller as the credential identity (scoped token identifier, or OAuth issuer and
  subject) for receipts and handles; authorization before the receipt; the exposure matched in the
  receipt (`IDEMPOTENCY_KEY_REUSED` on another exposure); a replayed result disclosed only through the
  replaying call's exposure and its current revision; the recheck of credential and rights before a
  result leaves the bounded wait — FR-025-030, 031 — `crates/application/src/mcp_calls.rs` —
  integration tests, including revocation between `tasks/get` calls, a handle presented by another
  caller, token B of the same minter reusing token A's key and arguments on another exposure (its own
  run, never A's run or result), token A reusing its key on another exposure (refused), and a token
  revoked while its call waits (no result, `mcp.call_denied`).
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
- [ ] T025-089 [US6] `tools/call` as a new row of the design 043 run-input gate: a `CheckedRunInput` of
  the arguments against the pinned target version's `inputSchema`, admitted through the existing
  `create` (no validator in `crates/api-mcp`); `RUN_INPUT_INVALID` to JSON-RPC `-32602` with the
  value-free rule and field, `RUN_INPUT_SCHEMA_UNUSABLE` to a tool error, transient refusals to a
  retryable `-32603`; `report` mode admits and audits (Decision 025-006); the gate's row added to design
  043's table in the implementation delivery — FR-025-031, 033 — `crates/application/src/mcp_calls.rs`,
  `crates/api-mcp/` — integration tests per answer in `enforce` and in `report`; a canary argument
  value never appears in a `-32602` answer, a log or an audit; 043's fitness test that every
  run-creating path goes through `CheckedRunInput` passes with the MCP path.

## Phase 8: US07 — MCP client (P2)

- [ ] T025-052 [US7] External MCP server resources on 021's resource model: routes, Studio, auth modes
  `none` and `bearer`, secrets (`ai:manage` and `secret:use` to save), the `pii` argument ceiling under
  the rule of Owner decision 2026-11-26-120, the `toolClassifications` field of Task 025-102 in the
  form and the API; `oauth-client-credentials` refused with `MCP_AUTH_MODE_UNAVAILABLE` — FR-025-034 — `routes/mcp_servers.rs`, `frontend/apps/studio/src/features/ai/`
  — HTTP and component tests. The OAuth part waits for design 047's token broker (constitution 5.1.0
  is adopted, pull request 286).
  Owner-gate: OD-2026-11-26-120, OD-2026-11-26-122, OD-2026-11-25-3
- [ ] T025-053 [US7] Refresh job (origin `MCP_TOOL_REFRESH`, through `start_private`) in the runner;
  immutable snapshots with digests and bounds — FR-025-035 — `crates/infra-ai/src/mcp_client.rs`,
  `apps/agent-runner/` — tests against the test MCP server (both revisions).
- [ ] T025-054 [US7] `mcp` tool declarations pinned to snapshots at publication, the snapshot
  identifier and server revision recorded in `version_agent_pins` and copied at admission (Task
  025-097) — FR-025-010, 035 — `crates/infra-ir/src/agent.rs`, `crates/application/src/` publication
  path — publication tests (`MCP_TOOL_NOT_ALLOWED`); a refresh after publication leaves the run's pinned
  snapshot, its descriptions and the drift check's digests unchanged.
- [ ] T025-055 [US7] Untrusted-content handling and injection canaries: instructions in descriptions and
  results cannot call undeclared tools, skip approvals, exceed budgets or reach denied destinations
  (G1286) — FR-025-011, 017, 023, 035 — `tests/integration/ai-agents-mcp/untrusted.rs` — documented
  threat model in the runbook, which also states the pass-through of an activity or script that
  returns model-written values: they become a new value under that step's declared carrier and inherit
  no transcript level, so the author's annotation of that carrier decides where they may go (Decision
  025-020).
- [ ] T025-056 [US7] Drift detection before each call (`AI_TOOL_CHANGED`, `mcp.tool_changed`) —
  FR-025-035 — `crates/infra-ai/src/mcp_client.rs` — test: changed schema is never called.
- [ ] T025-057 [US7] MCP call execution in the runner: at-least-once, unknown outcome, unsupported
  results, `isError` as `MCP_TOOL_ERROR` (text forwarded only at transcript level `pii`, Decision
  025-008), text-only, secret scan, egress denial, a `3xx` answer to a call or a refresh failing
  `AI_DESTINATION_DENIED` — FR-025-023, 036 — `crates/infra-ai/src/mcp_client.rs`,
  `apps/agent-runner/` — `tests/fault/ai-agents-mcp/mcp_client.rs`; a test server answering `302`
  receives one request and the redirect target none (the bearer header never leaves the first host).
- [ ] T025-058 [P] [US7] OAuth client credentials with resource indicator, with tokens only from design
  047's broker: issuer and token endpoint recorded in the revision and confirmed by an `ai:manage`
  holder, refresh-time comparison with discovered metadata (`mcp.oauth_metadata_changed`, calls refused
  until revised). Rests on constitution 5.1.0's third declared set (Owner decision 2026-11-25-3; adopted, pull request 286); waits for design 047's broker (Gap 025-001) —
  FR-025-034, 036 — `crates/infra-ai/src/mcp_auth.rs`, `crates/application/src/mcp_servers.rs` — tests
  against the test authorization server, including a server that switches its advertised token
  endpoint.

## Phase 9: US08 and US09 (P2, P3)

- [ ] T025-059 [US8] Journal-derived agent graph and telemetry (turns, tool calls, approvals, usage;
  unknown kinds degrade), carrying only declared tool names or `UNKNOWN_TOOL` and `modelSubstituted`
  (Task 025-098) — FR-025-022, 038, 041 — `crates/application/src/telemetry.rs`,
  `crates/telemetry/src/` — projection equals replay; unsupported-kind test.
- [ ] T025-060 [US8] Studio agent timeline and gated transcript viewer (explicit fetch, ephemeral
  state, WCAG 2.2 AA, table alternative, no sample values) — FR-025-022, 038 —
  `frontend/apps/studio/src/features/runs/{AgentTimeline,AgentTranscriptViewer}.tsx` — component,
  accessibility and real-browser tests.
- [ ] T025-061 [US8] Token streaming on `sagat.agent.tokens` through 026 `AppendStreamEvents` (gated):
  coalescing, per-call caps, the per-run reserved-channel budget clamped to 026's hard ceiling,
  publication refusal without 026, `toolName` a declared name or `UNKNOWN_TOOL` — FR-025-037 —
  `apps/agent-runner/`, `crates/infra-ir/src/agent.rs` — an undeclared streamed request emits
  `UNKNOWN_TOOL`;
  `agent-stream-multiturn` conformance (16 turns all streamed within budget); resumed stream equals
  committed text; superseded attempts marked (G1295).
- [ ] T025-062 [US9] Authoring assistant run (origin `AUTHORING_ASSISTANT`, through `start_private`),
  provider level at least `internal`, proposal validation, apply as draft — FR-025-039 —
  `crates/application/src/agents/assistant.rs`, routes — tests: no version, activation or run created; a
  provider declared `public` is refused.
- [ ] T025-063 [US9] Studio assistant panel with diff and explicit apply, reusing the existing
  `frontend/apps/studio/src/features/workflows/DiffView.tsx` for the proposal diff — FR-025-038, 039 —
  `frontend/apps/studio/src/features/workflows/AuthoringAssistant.tsx` — component and browser tests.

## Phase 10: Polish and gates

- [ ] T025-064 [P] Telemetry: spans, fixed-label metrics, audit actions; no content in spans or logs —
  FR-025-041, 042 — `crates/telemetry/src/metrics.rs`, runner — telemetry tests with closed label sets.
- [ ] T025-065 [P] TypeScript and Go agent builders, transcript and approval clients, conformance —
  FR-025-040 — `sdks/typescript/src/agent.ts`, `sdks/go/workflow/agent.go` — scenarios of
  `contracts/conformance.md`.
- [ ] T025-066 [P] Python, C# and Java builders and clients; the six-language conformance run (with
  Task 025-088) — FR-025-040, 043 — `sdks/python/sagat_octopus_orchestrator/agent.py`,
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/FlowAgent.cs`,
  `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/AgentOptions.java` —
  `sdk_conformance` all six.
- [ ] T025-067 Security battery: every new route, gRPC call and MCP method in the cross-tenant battery;
  runtime-role sweep; egress battery for providers and MCP — FR-025-023, 044, NFR-025-001 —
  `tests/contract/security-multitenancy/rest/battery.rs`, `tests/integration/ai-agents-mcp/` — zero
  findings.
- [ ] T025-068 Deployment: Dockerfile target `agent-runner` (non-root, read-only root, no platform
  credentials), Helm templates with default-deny egress, compose overlay layered on
  `compose.database.yaml` (its OpenBao, `docs/runbooks/openbao.md`) — FR-025-024, 045 —
  `infra/docker/Dockerfile`, `infra/helm/templates/`, `infra/docker/compose.ai.yaml` — rendered chart
  tests; network policy test on a real cluster.
- [ ] T025-069 Documentation: runbooks (providers, agent runner, MCP; incident steps for spend, provider
  outage, leaked token), architecture sections (worker protocol, security, observability); the bound of
  the output secret scan published in the form of Architecture decision 0085 rule 12 (Gap 025-004,
  accepted by the owner in Owner decision 2026-11-27-2): a section "The limit, stated and not hidden"
  saying that a resolved value shorter than 8 bytes, or transformed other than JSON-escaped, base64 or
  hexadecimal, a value that is not valid UTF-8 and any output that is not valid UTF-8 are not detected,
  and that base64 and hexadecimal are looked for only among the values the scanned attempt received
  (so not for a `child` tool's child activities or an earlier step's output entering through
  `context`), with no sentence elsewhere claiming otherwise, and the incident steps
  that purge the run's agent content and then rotate the secret; the approvals runbook section for an
  approval marked `NO_DECIDER_WITH_CONTEXT` (grant the reveal grant, declare an `escalation` route, or
  narrow `decisionFields`) — FR-025-017, 020, 045 — `docs/runbooks/{ai-agents,agent-runner,mcp}.md`,
  `docs/architecture/*.md` — docs contract check; a grep that no document under `docs/` claims the scan
  detects every secret.
- [ ] T025-070 Performance report with the dev provider at 1/16/64 concurrent agents, including
  `ai_usage_leases` refills and `ai_usage_daily` contention — NFR-025-007 —
  `tests/performance/ai-agents-mcp/`, `tests/performance/reports/ai-agents-mcp.md` — measured
  p50/p95/p99 with environment and failures.
- [ ] T025-071 [P] Opt-in real-provider smoke tests (for example `claude-haiku-5-5` on Anthropic),
  reported separately and never the only evidence — FR-025-045 — `tests/e2e/ai-agents-mcp/real_providers.rs`
  — report states provider, date and cost; skipped when credentials are absent.
- [ ] T025-072 Final gates: workspace tests, clippy, architecture, contracts, six SDKs, browser e2e,
  quickstart walkthrough (`e2e_ai_agents_mcp quickstart::`); record evidence and remaining gaps; close
  the checklist item "Feature meets measurable outcomes" — all FRs and SCs —
  `specs/025-ai-agents-mcp/verification.md` (created then) — every command and outcome recorded; no
  unchecked gate reported as passed.
- [ ] T025-075 [P] Amend feature 016's carrier text (FR-016-002 "exactly") for the two single-surface
  carriers added here, with the floors of D1291/D1292 (G1297) — FR-025-014, 017 —
  `specs/016-payload-classification/spec.md` (in the implementation delivery) — reviewed diff.
- [ ] T025-076 Coordinate with feature 026: public-link creation refuses versions with
  `agent_unattended_writes` (transitive, Task 025-094), public-link roots are marked `remote_root` so
  their child admissions refuse unattended child versions, public-link runs take the link's creating principal as initiator for
  `ai:use`, reserved-channel budgets and the superseded control event (G1295) — FR-025-026, 037 —
  026 contracts and tests — 026 tests reference these rules.
- [ ] T025-077 Register 025's entries in the registries that are still drafts when their owners publish
  them (G1635): 020 content stores, 028 time classes (`crates/infra-postgres/time-classification.json`,
  to be created by 028), 031 encoded positions, 032 key inventory — FR-025-021, 046, 047 — those
  drafts' contract files and the manifests — each registry's own consistency test passes with 025's
  entries.
- [ ] T025-078 Reset rules as arms of the code registry: every open-loop agent event
  `ResetRule::Refuse { until: Some("agent.finished") }`, `agent.finished` `ResetRule::Rematerialize`,
  and the rematerialization of a finished agent step in 018's reset transaction (rows copied, payloads
  shared, no call, no budget, lease or daily charge) — FR-025-046 —
  `crates/domain/src/execution/reset.rs`, `crates/application/src/reset_completion.rs` and its store,
  `crates/application/src/agents/reset.rs` (new) — the exhaustive reset-rule table test of feature 018;
  `tests/integration/ai-agents-mcp/reset.rs`: reset points before, inside (not offered) and after a
  loop.
- [ ] T025-079 [P] Encoded-field checks for agent positions with 031's `EncodingCheck` and codes —
  FR-025-047 — `crates/infra-ir/src/agent.rs`, `crates/infra-ir/src/encoding_check.rs` (031) —
  publication fixtures for context, tool results, child inputs, agent output and exposure projections.
- [ ] T025-080 `pii` egress rule (Owner decision 2026-11-26-120): reveal grant required to set a `pii`
  ceiling, the grant of the administrator who saved that revision rechecked at each `pii` resolution,
  its revocation or that administrator leaving the namespace pauses the provider or MCP server until a
  grant holder saves a new revision, `ai.pii_egress` audit per attempt, default `internal` —
  FR-025-005, 034 — `crates/application/src/{ai_providers,mcp_servers,agent_capability}.rs` —
  integration tests: grant revoked between two turns pauses the provider and fails closed; the setting
  administrator removed from the namespace pauses it the same way; another member's grant does not
  stand in for theirs.
  Owner-gate: OD-2026-11-26-120
- [ ] T025-081 MCP visibility through the 029 access evaluator when 029 is enabled; `agent-runner` as an
  additive built-in role in 029's model — FR-025-030 — `crates/application/src/mcp_calls.rs` —
  tests with folder restrictions hiding an exposure's target from `tools/list` and `tools/call`.
  Owner-gate: OD-2026-11-26-121
- [ ] T025-088 [P] Rust SDK: agent node and tool builders emitting the canonical IR, transcript and
  tool-call approval clients — FR-025-040, 043 — `sdks/rust/src/workflow/agent.rs`,
  `sdks/rust/src/client/agents.rs`, `sdks/rust/conformance/` — the Rust per-SDK checks of
  `sdks/AGENTS.md`; byte-identical IR fixture with the other five; its four lock files moved together.
- [ ] T025-090 [P] Audit-to-OCSF mapping rows for every new audit action (design 040): an arm of the
  exhaustive `match` in `crates/domain/src/audit_ocsf.rs` and the regenerated
  `docs/reference/ocsf-mapping.md`, in the change that adds each action once design 040 is built
  (Gap 025-003) — FR-025-041 — `crates/domain/src/audit_ocsf.rs`, `docs/reference/ocsf-mapping.md` —
  design 040's reference and validation tests pass with the new actions.
- [ ] T025-091 gRPC handlers `ResolveModelCall` and `ResolveMcpCall` in the worker service, calling the
  use cases of Task 025-017, wired in the execution engine's composition root — FR-025-009, 024, 036 —
  `crates/api-grpc/src/agent.rs`, `apps/execution-engine/src/lib.rs` — gRPC tests: ordinary worker,
  database and lake runner tokens denied; stale lease; the response carries no OpenBao routing, platform
  URL or key; `worker.baseline.json` compatibility; over gRPC, `ResolveSecrets` with an agent-runner
  token on each `sagat.ai.*` lease is denied with nothing resolved and no `secret.resolved` audit,
  `ResolveDatabase` on an AI lease is denied, and `ResolveModelCall` on an MCP lease (and the reverse) is
  denied.
- [ ] T025-092 Test provider for the work that precedes 021's resource core: a provider revision and an
  MCP-server revision written through the store port by test support only (never a table or a route of
  its own), so the engine, runner and adapter tasks run before Task 025-006 — FR-025-001, 009 —
  `tests/support/` — the engine and runner suites use it; a check that no production crate depends on it.

Tasks added on 2026-11-27 after the security review of that date (Decision 025-016):

- [ ] T025-093 [US5] Inherited output levels: publication computes the effective level of every field
  of the workflow `outputSchema`, every child node `outputSchema` and every `child` tool result carrier
  with the domain's `written_level` (`crates/domain/src/classification.rs`, no second implementation),
  inheriting from the paths `end.outputExpr` references (an agent's `result` paths carry its transcript
  level; a child's output carries the child version's recorded levels), refuses an annotation below
  the inherited level (`CLASSIFICATION_LOOSENED` with the pointer), records
  `workflow_versions.effective_output_levels`, uses the effective child-tool carrier level in the
  parent agent's transcript level, and checks an active-version child at its admission against the
  levels its version recorded — FR-025-012, 019, 032, NFR-025-004 — `crates/infra-ir/src/classification_check.rs`,
  `crates/application/src/agents/execution.rs` (child admission), the publication path in
  `crates/application/src/` — publication fixtures: `end.outputExpr: steps.triage.output.result.summary`
  with `summary` annotated `public` under a `pii` transcript (refused); the same through a pinned child
  node and a pinned `child` tool (refused); an outputExpr of several paths (every field takes the
  strictest); an active-version child whose new version raises its levels refused at its admission
  (`TOOL_FAILED` with `CLASSIFICATION_LOOSENED` for a tool, step failure for a node); a value recomputed
  by an activity keeps that activity's carrier (the stated bound); existing workflows without agents
  publish unchanged (the 016 fixtures pass).
- [ ] T025-094 [US6] Transitive unattended flag and remote roots: `agent_unattended_writes` true when
  the version, or any child workflow it can start under a pinned version policy at any depth, has an
  agent `write` tool acknowledged `approval: "none"` or whose `decisionFields` leave out any input
  field (Decision 025-020); `starts_active_children` recorded; exposure save
  and call resolve active-version children to their current versions and refuse an unattended one;
  `runs.remote_root` set for a root admitted through MCP or a 026 public link and copied to every
  descendant; a child admission (node or `child` tool) under a remote root refusing an unattended child
  version (`UNATTENDED_WRITE_TOOL`, no child created) — FR-025-012, 026 — `crates/infra-ir/src/agent.rs`,
  `crates/application/src/mcp_exposures.rs`, `crates/application/src/mcp_calls.rs`, the child admission
  path in `crates/application/src/`, `crates/infra-postgres/migrations/` (lane A) — fixtures and
  integration tests: a pinned child two levels down with an unattended tool makes the root unattended
  (exposure refused); an active-version child activated with an unattended tool after the exposure was
  saved is refused at its admission under an MCP-admitted run and admitted under an ordinary run; a
  version whose `issue_refund` tool narrows `decisionFields` to none is refused at exposure save, at
  call time, as a public-link target and as a child under a remote root; the flag survives a reset
  (Task 025-078).
- [ ] T025-095 Reserved `sagat.ai.` prefix and claims by managed kind: `check_node`
  (`crates/domain/src/workflow.rs`, which reserves only `sagat.database.` today) and
  `managed_capabilities_allowed` (`crates/application/src/worker.rs`) refuse an ordinary activity,
  worker registration or version naming a `sagat.ai.` activity (`RESERVED_ACTIVITY_NAME`); an activation
  check and a migration-time query list existing versions with such a name and refuse recording
  `ai-agents-v1` while any exists; the claim path offers a `sagat.ai.*` lease only for a node whose
  `managed_kind` is that AI kind — FR-025-009, 024 — `crates/domain/src/workflow.rs`,
  `crates/application/src/worker.rs`, `crates/application/src/instances.rs` — unit tests for both
  refusals; a version published before the prefix was reserved, with an ordinary `sagat.ai.model`
  activity and declared secrets, blocks the activation and is never leased to an agent runner (seen
  failing first against the current name-only match).
- [ ] T025-096 [US3] Redacted approval view and decisions on the fields that matter (rewritten
  2026-11-27, round 3, Decision 025-019): the approval view of an `AGENT_TOOL_CALL` request returns the
  decision context, the redacted transcript outline and the tool input with every field the caller's
  current rights withhold masked by path and level only, plus `decisionFields`, `maskedFields` and
  `decidable` (false exactly when a decision field is withheld); revealing a masked field goes through
  the transcript audience (reveal grant) and is audited `payload.pii_revealed`; a decision through
  `POST /approvals/{approvalId}/decisions` by a decider whose current rights withhold any decision field
  is refused (`403` `APPROVAL_INPUT_WITHHELD`), applies nothing and is audited
  `ai.approval_input_withheld`; publication refuses narrowed `decisionFields` on `operation` and
  database `execute` tools (`DECISION_FIELDS_REQUIRED`) and pointers outside the input schema
  (`INVALID_DECISION_FIELD`) — FR-025-010, 017 — `crates/application/src/agents/approvals.rs`,
  `crates/api-http/src/routes/agent_tool_calls.rs`, `crates/infra-ir/src/agent.rs` — tests: an eligible
  `operator` without the reveal grant on a `pii` transcript with the default `decisionFields` is refused
  and the approval stays pending; the same operator decides when the author narrowed `decisionFields` to
  an `internal` field they can read, the decision recording the masked fields; with the grant the
  default decision applies; rights revoked between opening the view and deciding are refused; the view's
  body holds no length, digest or prefix of a masked value (a canary of each length from 1 to 64 bytes);
  the race tests of Task 025-033 still pick exactly one outcome.
- [ ] T025-097 [US1] Pins recorded at publication and current restrictions: publication writes
  `version_agent_pins` (provider revision per agent node, MCP-server revision and snapshot per `mcp`
  tool); admission copies them into `run_resource_bindings` and `run_mcp_snapshot_pins` and refuses a
  run whose current ceilings or allowlists no longer admit an agent's transcript level or model
  (`AI_DESTINATION_RESTRICTED`); both resolve phases recheck the same and refuse before dispatch
  (`CLASSIFICATION_EGRESS_REFUSED`, `AI_CAPABILITY_UNAVAILABLE`) — FR-025-002, 019, 035 —
  `crates/application/src/` publication and admission paths, `crates/application/src/agent_capability.rs`,
  `crates/infra-postgres/src/agents.rs` — integration tests: a provider lowered to `public` by a new
  revision without republication refuses the next admission and the next model call of a running agent;
  an MCP server lowered the same way refuses the next call; a model removed from the current revision's
  allowlist refuses the next call; a raised ceiling does not widen a published version; an `mcp` tool
  published `effect: "read"` and reclassified `write` by a new MCP-server revision refuses the next
  admission (`AI_DESTINATION_RESTRICTED`) and the next call before dispatch; the `secret:use` check at
  delivery uses the secrets of the recorded revision.
- [ ] T025-098 [US8] No model-written or provider-written string on run-read surfaces: undeclared tool
  requests recorded as `UNKNOWN_TOOL` plus call index in `agent_tool_calls`, `AgentProgress`, the
  journal (`agent.tool_refused` and the others), the stream and spans; the provider's tool-use id kept
  only in the transcript; the reported model reduced to `model_substituted`; the provider request id
  kept on spans only when it matches `^[A-Za-z0-9._:-]{1,128}$` — FR-025-011, 022, 037, 041 —
  `crates/application/src/agents/tools.rs`, `crates/application/src/telemetry.rs`, `apps/agent-runner/`
  (spans and stream) — tests: the dev provider scenario `undeclared-tools` (256 requests named after a
  canary) leaves the canary only in the transcript, never in the run graph read with `run:read`, the
  journal, the stream's `internal` fields, spans, logs or audit; a provider reporting another model sets
  `modelSubstituted` and its string appears nowhere but the transcript.
- [ ] T025-099 [US6] Remote callers bounded: `mcp_agent_runs_per_caller` (default 4, ceiling 64) counted
  in the MCP admission transaction per credential identity for targets that can contain agents
  (`QUOTA_EXCEEDED` beyond it); an exposure's optional `maxDailyTokens` charged by every model-call
  reservation of a run it admitted and of its descendants, through the usage leases of Task 025-086
  under the exposure's `budget_scope` — FR-025-016, 033 — `crates/application/src/mcp_calls.rs`,
  `crates/infra-postgres/src/ai_usage_leases.rs`, `crates/domain/src/quota.rs` — tests: a fifth live
  agent run of one scoped token is refused while another token of the same namespace is admitted; an
  exposure at its daily token budget fails the next reservation `AI_BUDGET_EXHAUSTED` before dispatch
  while internal agents of the namespace keep running; the exposure scope's invariant (folded plus
  leased equals the sum of its runs' attempts) holds.
- [ ] T025-100 [US1] Entra client credentials through design 047's broker (Owner decision
  2026-11-27-400, Decision 025-017): `azure-entra-client-credentials` accepted at save once the broker
  exists (`AI_CREDENTIAL_MODE_UNAVAILABLE` until then); the client secret is a slot the broker reads and
  `ResolveModelCall` never returns; the broker runs the client-credentials grant against the recorded
  `entraAuthority` only and returns the current access token, which `ResolveModelCall` delivers to the
  lease-owning runner under the third declared set, never as a first-set secret, with the publisher's and
  the initiator's `resource:use` checked (Decision 025-020); the adapter sends it as a bearer token —
  FR-025-003 — `crates/application/src/agent_capability.rs`, `crates/infra-ai/src/azure.rs`, design
  047's broker port — tests: the resolve response for an Entra revision carries an access token and no
  client-secret slot; a token request to a host other than the recorded authority is refused; a revoked
  `secret:use` refuses the next resolution; the canary battery of Task 025-042 extended to the client
  secret (it reaches no runner, transcript, log, span or audit).
  Owner-gate: OD-2026-11-27-400
- [ ] T025-101 [US1] AWS access keys through design 047's broker (Owner decision 2026-11-27-401,
  Decision 025-017; waits for Owner question 025-002, Decision 025-020): `aws-access-key` accepted at
  save once the broker exists and the owner's answer admits a broker-derived bearer credential under the
  third declared set (until then it stays refused, Owner decision 2026-11-27-480); the broker holds the
  long-lived key pair, calls AWS STS (`AssumeRole` with the recorded `aws_role_arn`, `GetSessionToken`
  without one; `aws_session_seconds` 900 by default, 900 to 3,600), mints from that session an Amazon
  Bedrock short-term API key whose lifetime does not exceed the session's, and `ResolveModelCall`
  delivers only that key; the runner uses the bearer path of Task 025-011 and holds no SigV4 signer —
  FR-025-003 — `crates/application/src/agent_capability.rs`, `crates/infra-ai/src/bedrock.rs`, design
  047's broker port — tests: the resolve response for an AWS revision carries one bearer key with its
  expiry and neither the long-lived secret access key nor a session secret key (the key carries the
  session's `X-Amz-Security-Token` inside its presigned query, which is useless without the secret); a key
  past its expiry is never delivered; a session longer than 3,600 seconds is refused at save; the
  runner's dependency tree holds no SigV4 signer (an architecture check); the canary battery extended
  to the long-lived and the session secrets.
  Owner-gate: the owner's answer to Owner question 025-002 (a row of `docs/owner-decisions.md`, not yet
  recorded); Owner decision 2026-11-27-480 keeps the mode refused until then
- [ ] T025-102 [US7] Pinned MCP tool classifications (Owner decision 2026-11-27-402, Decision 025-018):
  the MCP-server revision's `toolClassifications` (tool name to `read` or `write`, at most 512, a tool
  not named is `write`) saved by the `ai:manage` holder and immutable with the revision; publication
  allows an `mcp` tool `effect: "read"` without an approval policy only when the pinned revision
  classifies it `read` (`WRITE_TOOL_APPROVAL_REQUIRED` otherwise); a refresh and a publication that find
  an annotation disagreeing with the pinned classification write `ai.mcp.tool_classification_mismatch`
  (server, revision, snapshot, tool name; never the description) — FR-025-010, 034, 035 —
  `crates/infra-ir/src/agent.rs`, `crates/application/src/mcp_servers.rs`,
  `crates/api-http/src/routes/mcp_servers.rs` — tests: the test MCP server's `delete_records` stating
  `readOnlyHint: true` on an unclassified tool is refused as `read` and audited; the same tool classified
  `read` by the administrator is allowed and the mismatch of a later `destructiveHint: true` refresh is
  audited without changing the published version; a new revision that reclassifies the tool `write`
  refuses the next admission and the next call of a published `read` declaration without republication
  (Task 025-097), while a revision that reclassifies a tool `read` widens nothing published as `write`;
  a classification carries the description and input-schema digests it was given for, and a snapshot
  whose digests differ (the server changed the tool) makes the tool unclassified, so `write` (Decision
  025-020).
- [ ] T025-103 [US3] Escalation and `NO_DECIDER_WITH_CONTEXT` (Owner decision 2026-11-27-403, Decision
  025-019): the context check at the opening transaction, after every `APPROVAL_INPUT_WITHHELD` refusal
  and in the approval sweeper every 60 seconds, under the run row then the request row; escalation to the
  `escalation` route (`escalated_at` set once, `ai.approval_escalated`,
  `agent_approvals_escalated_total`); `no_decider_reason = NO_DECIDER_WITH_CONTEXT` with the gauge
  `agent_approvals_without_decider` and the alert `AgentApprovalWithoutDecider` (its promtool cases);
  the expiry of a marked request ending through `onExpire` with that reason, audited
  `ai.approval_no_decider`; the Studio approvals inbox shows the escalated and the undecidable states —
  FR-025-017, 041 — `crates/application/src/agents/approvals.rs`,
  `crates/infra-postgres/src/primitives.rs` (the sweeper query), `infra/observability/alerts/` — tests: a
  `pii` agent whose eligible deciders all lack the reveal grant is escalated at open; a route whose
  members also lack it marks the request and, at the deadline, the `refuse` or `fail` outcome records
  `NO_DECIDER_WITH_CONTEXT`; a reveal grant given to a route member clears the mark at the next sweep and
  that member decides; the only grant holder losing the grant leads to escalation within one sweep;
  decision versus escalation and versus expiry pick one outcome over 50 rounds with held locks, never
  sleeps.

## Dependencies and order

Phase 1 (including T025-073, T025-082, T025-083, T025-087 and T025-095) blocks all stories; T025-093,
T025-094, T025-096, T025-097, T025-098, T025-099, T025-102 and T025-103 close with the stories they
name (US05, US06, US03, US01, US08, US06, US07, US03) and before the gates of Phase 10; T025-103 needs
T025-032's columns and T025-096's view. US01 (Phase 2)
blocks US02–US09. US03, US04 and US05 depend on US02's tool planning (T025-025, T025-026). US06 needs
Phase 1 and the admission path only; US07 needs the runner (T025-016, T025-017, T025-091). The provider
and MCP-server resource tasks (T025-006, T025-022, T025-024, T025-052) need 021's resource core; until
then T025-092's test provider stands in. US08 streaming needs feature 026; T025-028 needs feature 021's
operations; script and operation exposures need feature 022; T025-058 waits for design 047's broker
(constitution 5.1.0 is adopted, pull request 286), as does T025-100 (the Entra broker route, Decision
025-017); T025-101 (the AWS route) also waits for the owner's answer to Owner question 025-002
(Decision 025-020); T025-090 needs design 040 built. Phase 10 closes after every story
it covers.

## Build slices

The tasks are cut into slices that two builders can run in parallel, in two lanes that never touch the
same crate or the same migration file at the same time. Lane A is the control-plane side: domain types,
the IR schema and codec, publication checks, storage and migrations, the engine transactions, the MCP
server and the Studio. Lane B is the executor side: the egress reader, the provider adapters, the
development provider, the agent runner and its gRPC handlers, the SDKs and the deployment. A slice
merges as one pull request with its verification; a slice that names a dependency in the other lane
starts only after that slice has merged.

### Slice 0 — contracts and fences (one builder, before the lanes split)

- Tasks: 025-001, 025-002, 025-073, 025-087.
- Touches: `specs/001-platform-foundation/contracts/` (workflow schema, OpenAPI, `worker.proto` and its
  baseline, `model-turn.schema.json`), `sdks/SPEC.md` (the register row of the minor and section 2),
  `sdks/error-catalog.json` and the six embedded copies, `crates/api-http/src/error.rs`,
  `scripts/check-architecture.py` and its tests, `tests/Cargo.toml` (the five targets declared once
  here) and `tests/contract/ai-agents-mcp/errors.rs`.
- Depends on: nothing; it fixes the worker-protocol minor number at its merge (the next free one).
- Verification: CI `contracts` lint and compatibility, `cargo test -p sagat-tests --test
  contract_ai_agents_mcp`, `python3 scripts/check-architecture.py` and `scripts/tests/`, each SDK's
  catalog-copy test, `cargo fmt --check`, clippy.

### Lane A — control plane

- **Slice A1, domain model and IR.** Tasks 025-003, 025-004, 025-005, 025-083, 025-009, 025-037,
  025-039 (publication part), 025-079 (when 031 exists), 025-093 and 025-094 (publication parts),
  025-095. Touches `crates/domain` (`workflow.rs`,
  `agent`, `ai_provider.rs`, `mcp.rs`, `identity.rs`, `quota.rs`, `database.rs`, `binding.rs`),
  `crates/infra-ir`, and the test file `crates/application/tests/managed_kind_sites.rs`. Depends on
  slice 0. Verification: domain unit and property tests, publication fixtures,
  `managed_kind_sites`, the permission catalog test.
- **Slice A2, storage and registry.** Tasks 025-007, 025-008, 025-082, 025-086, 025-032, 025-092.
  Touches `crates/infra-postgres` (every migration of this feature is written in lane A),
  `crates/application/src/instances.rs`, `tests/support/`. Depends on A1. Verification: migration up and
  down, the runtime-role sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`), the storage-classification manifest
  check, the registry test, the usage-lease invariant test, feature 014's approval suites
  (`integration_workflow_primitives`).
- **Slice A3, engine and transactions.** Tasks 025-010, 025-017, 025-018, 025-019, 025-020, 025-025,
  025-026, 025-027, 025-029, 025-030, 025-031, 025-033, 025-036, 025-038, 025-078, 025-084, 025-085,
  025-093 and 025-094 (child admission parts), 025-096, 025-097, 025-098 (server part), 025-103.
  Touches `crates/application`, `crates/infra-postgres/src/` (stores only), `crates/scheduler`,
  `crates/domain/src/execution/reset.rs`, and `apps/execution-engine` (its composition root, until the
  order point below hands it to lane B). Depends on A2. Verification: `integration_ai_agents_mcp` and
  `fault_ai_agents_mcp` (tools, sagas, races, approvals) driven by a test harness that plays the runner
  over the application ports, `integration_claim_transition_efficiency`, `integration_step_logs`,
  `fault_schedules_bulk_operations` (slot order), the reset-rule table test.
- **Slice A4, providers and data protection.** Tasks 025-006, 025-022, 025-080, 025-040, 025-041,
  025-034, 025-043, 025-062. Touches `crates/infra-postgres` (021 type registrations),
  `crates/application`, `crates/api-http`. Depends on A3 and on feature 021's resource core having
  merged (Owner decision 2026-11-26-122). Verification: HTTP tests, the transcript rights matrix, the
  `pii` egress tests, the classification battery, retention tests.
- **Slice A5, MCP server.** Tasks 025-044, 025-045, 025-046, 025-047 (server part), 025-048, 025-049,
  025-050, 025-074, 025-081, 025-089, 025-094 (exposure and call parts), 025-099. Touches `crates/api-mcp` (new), `crates/application/src/mcp_*`,
  `crates/api-http` (authentication and routes), `apps/control-plane`. Depends on A3 and on slice B1's
  Task 025-011 (the MCP codec choice and its pin). Verification: protocol unit tests, security tests,
  `fault_ai_agents_mcp` (`mcp_server.rs`), the run-input gate answers in `enforce` and `report`.
- **Slice A6, MCP client control side.** Tasks 025-052 (without its OAuth part), 025-054, 025-102. Touches
  `crates/application`, `crates/api-http`, `crates/infra-ir`. Depends on A4. Verification: HTTP tests,
  publication pinning tests.
- **Slice A7, Studio.** Tasks 025-024, 025-035, 025-047 (Studio part), 025-060, 025-063. Touches
  `frontend/apps/studio`. Depends on slice 0 (the generated client) and, for the browser journeys, on
  A4 and A5. Verification: component and accessibility tests, `npm run api && npm run check:api`, the
  real-browser journey.
- **Slice A8, telemetry and audit.** Tasks 025-059, 025-064, 025-090. Touches `crates/telemetry`,
  `crates/application/src/telemetry.rs`, `crates/domain/src/audit_ocsf.rs` (design 040). Depends on A3
  and, for Task 025-090, on design 040 being built. Verification: telemetry tests with closed label
  sets, design 040's reference test.

### Lane B — executor

- **Slice B1, egress and adapters.** Tasks 025-011, 025-012, 025-013, 025-014, 025-015. Touches
  `crates/infra-egress`, `crates/infra-ai` (new), the workspace `Cargo.toml` and `Cargo.lock` (only lane
  B adds dependencies; lane A asks lane B for one). Depends on slice 0 (`model-turn.schema.json`).
  Verification: adapter suites against recorded exchanges, SSRF, redirect, slow-drip and oversize
  tests, `cargo deny` and the advisory audit, the runner's `max_redirects = 0` test.
- **Slice B2, development provider.** Task 025-023. Touches `apps/dev-model-provider` (new),
  `infra/docker/compose.ai.yaml`. Depends on B1. Verification: its own tests, the production-image
  check.
- **Slice B3, agent runner and resolve handlers.** Tasks 025-016, 025-091, 025-053, 025-056, 025-057,
  025-061 (when 026 exists), 025-098 (runner part: spans and stream). Touches `apps/agent-runner` (new), `crates/api-grpc`,
  `crates/infra-ai/src/mcp_client.rs`, `apps/execution-engine/src/lib.rs` (after the order point).
  Depends on B1, B2, and on lane A's slices A1 and A3 (order points 2 and 3). Verification: runner
  integration tests (no platform credential in its environment, metadata addresses unreachable), gRPC
  denial tests, the drift test, `fault_ai_agents_mcp` (`mcp_client.rs`).
- **Slice B4, SDKs.** Tasks 025-065, 025-066 (builders and clients), 025-088. Touches `sdks/` only.
  Depends on slice 0 (the IR schema and the catalog). Verification: the per-SDK checks of
  `sdks/AGENTS.md`, unit tests, the byte-identical IR fixture; the conformance run is a final gate.
- **Slice B5, deployment and documentation.** Tasks 025-068, 025-069, 025-071. Touches
  `infra/docker/Dockerfile`, `infra/helm/`, `docs/runbooks/`. Depends on B3. Verification: rendered
  chart tests, the network policy test, the docs contract check.
- **Waiting.** Task 025-058 and the OAuth part of 025-052 wait for constitution 5.1.0, approved by the owner
  (Owner decision 2026-11-25-3), to merge and for design 047's broker (Gap 025-001); when both are there they form one slice of lane B (runner side)
  followed by one of lane A (resource side). Tasks 025-100 and 025-101 (the Entra and AWS broker routes,
  Decision 025-017) wait for the same broker and join that pair of slices: the resolve and adapter parts
  in lane B, the save-time acceptance in lane A.

### Cross-lane order points

1. Slice 0 merges before either lane starts; it fixes the contracts, the minor number, the error
   catalog, the architecture rules and the test targets.
2. Slice A1 (the AI managed kinds, `BindingProfile::Agent`, the reserved activity names and the derived
   ids in `crates/domain`) merges before slice B3 registers the runner.
3. Slice A3 (the application use cases `ResolveModelCall` and `ResolveMcpCall`, the `sagat.ai.*` claim
   path) merges before slice B3 adds the gRPC handlers; at that merge the execution engine's
   composition root (`apps/execution-engine/src/lib.rs`) passes from lane A to lane B, one owner at a time.
4. Slice B1's Task 025-011 (the MCP codec and its workspace pin) merges before slice A5.
5. Migrations are lane A's alone; the runner has no database and lane B never adds a migration file.
6. The workspace `Cargo.toml` and `Cargo.lock` are lane B's alone for new dependencies.
7. The `tests` package is shared by design: its targets are declared in slice 0, and afterwards each
   lane adds files only under its own paths (lane B: `tests/integration/ai-agents-mcp/runner_*.rs`,
   `tests/fault/ai-agents-mcp/mcp_client.rs`, `tests/contract/sdk-conformance/scenarios/agent-*.json`
   and their expected files; lane A: every other file under `ai-agents-mcp/`, `tests/support/` and the
   budget and slot-order tests).
8. The final gates run after both lanes: Tasks 025-021, 025-042, 025-051, 025-055, the six-language
   conformance run of 025-066, 025-067, 025-070, 025-072, 025-075, 025-076 and 025-077; Task 025-028
   when 021 offers its operations.

## Traceability

| Requirement group | Tasks | Success criteria |
|---|---|---|
| FR-025-001..005 | T025-005, 006, 015, 017, 022, 024, 080, 092, 097, 100, 101 | SC-025-001, SC-025-012 |
| FR-025-006..009, 013, 014 | T025-003, 004, 009, 013..021, 082, 083, 085, 091, 095 | SC-025-002, SC-025-003, SC-025-010 |
| FR-025-010..012, 025 | T025-009, 025..030, 093, 094, 096, 098, 102 | SC-025-004, SC-025-007 |
| FR-025-015, 016 | T025-036..039, 085, 086, 099 | SC-025-006 |
| FR-025-017, 018 | T025-031..035, 096, 103 | SC-025-005 |
| FR-025-019..022 | T025-040..043, 069, 075, 077, 084, 093, 097, 098 | SC-025-007, SC-025-008 |
| FR-025-023, 024 | T025-012, 016, 017, 057, 067, 068, 073, 083, 084, 091, 095 | SC-025-009, SC-025-010 |
| FR-025-026..033 | T025-044..051, 074, 076, 081, 089, 094, 099 | SC-025-011 |
| FR-025-034..036 | T025-052..058, 080, 097, 102 | SC-025-012 |
| FR-025-037 | T025-061, 076, 098 | SC-025-013 |
| FR-025-038, 039 | T025-024, 035, 047, 060, 062, 063 | SC-025-001, SC-025-016 |
| FR-025-040, 043 | T025-002, 065, 066, 082, 088 | SC-025-014 |
| FR-025-041, 042 | T025-002, 059, 064, 087, 090, 098, 103 | SC-025-015 |
| FR-025-044 | T025-022, 050, 067 | SC-025-001, SC-025-011 |
| FR-025-045 | T025-023, 068..072 | SC-025-017 |
| FR-025-046, 047 | T025-077..079 | SC-025-018 |
