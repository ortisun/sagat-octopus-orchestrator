# Data model — feature 025

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decisions 0059 and 0060 accepted (delegate, Owner decisions 2026-11-26-123
and 2026-11-26-124); not started. Nothing below exists yet. Revised 2026-11-26 against `develop` at
`3f22b612`.

Every table is tenant-keyed: the primary key starts with `tenant_id`, every foreign key is composite
with `tenant_id` (and `namespace_id` where the row is namespace-scoped), and every statement filters by
the authenticated tenant. Migration versions are allocated at implementation time after the latest
merged migration; down migrations refuse to drop rows that live runs or pins reference. Variable-size
columns are added to `crates/infra-postgres/storage-classification.json` (ADR-0027 classes `content`,
`log`, `protected_system`); the runtime role `sagat_app` gets only the privileges listed. Every
migration of this feature is written by one lane (the control-plane lane of tasks.md, "Build slices").

## 1. States and transitions

**Agent step** (`agent_steps.status`): `RUNNING` (a model call is scheduled or leased) →
`WAITING_TOOLS` (tool calls of the current turn outstanding, including approvals) → `RUNNING` (next
turn) … → `SUCCEEDED` | `FAILED` | `CANCELLED`. Terminal states never change. The owning step in
`steps` keeps its existing states; the agent row adds budgets and counters.

**Turn** (`agent_turns.status`): `SCHEDULED` → `SUCCEEDED` | `FAILED`; attempts are the derived
step's attempts. A turn is `kind = MODEL` or `kind = REPAIR`.

**Tool call** (`agent_tool_calls.status`): `PLANNED` → `WAITING_APPROVAL` → `READY` → `SUCCEEDED` |
`FAILED` | `MANUAL_REVIEW` → (`SUCCEEDED` | `FAILED` after resolution); `PLANNED` → `REFUSED`
(undeclared tool, invalid input, over limit, rejected or expired approval with `refuse`);
`READY`/`WAITING_APPROVAL` → `CANCELLED`; `SUCCEEDED` → `COMPENSATED` | `COMPENSATION_FAILED` in a
saga.

**Resources** (providers, MCP servers, exposures): head status `ACTIVE` | `PAUSED` | `DELETED`
(tombstone, irreversible); revisions are immutable.

**Scoped MCP token**: `ACTIVE` → `REVOKED` | expired (by `expires_at`, database clock).

## 2. Provider and MCP-server resources (on feature 021's resource tables)

Providers and external MCP servers are rows of feature 021's `resources` and `resource_revisions`
under the platform-provided types `llm_anthropic`, `llm_openai_compatible`, `llm_aws_bedrock`,
`llm_azure_openai` and `mcp_server`, whose executor is the agent runner (Owner decision
2026-11-26-122, Decision 1283). 025 creates no catalog tables and there is no second catalog; the
provider and MCP-server tasks start after 021's resource core merges. Until then the engine, runner
and adapter work uses a test provider written through the store port by test support only (never a
table of its own). Run pins use 021's `run_resource_bindings`; credential pins use 021's
`step_resource_credentials`, keyed by attempt and secret slot (below).

The rows below are the settings schema each type declares and the constraints 021's tables must hold
for it; "head" and "revision" are 021's head and revision rows.

| Type | Settings and fields | Constraints, classification |
|---|---|---|
| `llm_*` (providers) | head: `name` (`^[a-z][a-z0-9-]{2,63}$`), `status`; revision: `kind` (`anthropic`, `openai-compatible`, `aws-bedrock`, `azure-openai`), settings (closed per kind: HTTPS base URL, region, API version, deployment map, prompt caching), credential secrets (slot name → logical secret name, at most 4), `models` (≤ 64 entries: id, alias, maxOutputTokens, contextTokens, capabilities, prices in integer micro-units per million tokens and ISO 4217 currency), `max_prompt_classification` (`EgressCeiling`: `public`/`internal`/`pii`; required at the API), limits, `actor`, `request_id` | 021's revision immutability and receipt rules; unknown settings and credentials in settings or URLs refused; settings, models and limits are `content` |
| `mcp_server` | revision: `url` (HTTPS, no userinfo, no query secrets), `auth_mode` (`none`, `bearer`; `oauth-client-credentials` refused at save until Gap 025-001 closes), credential secrets, `protocol_preference` (`2026-07-28`, `2025-11-25`), `max_argument_classification` (`EgressCeiling`, required at the API), `oauth` (for the OAuth mode once enabled: `issuer`, `token_endpoint` (HTTPS), client id, resource, scopes; recorded and confirmed by the saving `ai:manage` holder, never replaced by discovery), limits; head: `current_snapshot_id` (025's pointer, below) | as above; `url`, `oauth`, limits are `content` |

025's own catalog tables:

| Table | Key and columns | Constraints, indexes, grants, classification |
|---|---|---|
| `mcp_server_heads` | PK `(tenant_id, resource_id)`; `namespace_id`, `current_snapshot_id`, `oauth_metadata_mismatch` | FK to 021's resource head; SELECT/INSERT/UPDATE (pointer and flag only); protected |
| `mcp_tool_snapshots` | PK `(tenant_id, snapshot_id)`; `resource_id`, `resource_revision`, `protocol_version`, `tool_count`, `snapshot_sha256`, `discovered_issuer`, `discovered_token_endpoint` (OAuth servers only, for comparison), `refreshed_by_run`, `created_at` | immutable; ≤ 1,024 tools; SELECT/INSERT |
| `mcp_snapshot_tools` | PK `(tenant_id, snapshot_id, name)`; `title`, `description` (≤ 16 KiB), `description_sha256`, `input_schema jsonb` (≤ 64 KiB), `input_schema_sha256`, `output_schema_sha256`, `closed_input`, `annotations jsonb` (untrusted, ≤ 4 KiB) | immutable; `description`, `input_schema`, `annotations` content |
| `mcp_exposures`, `mcp_exposure_revisions` | head with `tool_name` (`^[a-zA-Z0-9_-]{1,64}$`, within MCP's name rules and the tool-name rules of current model APIs; unique per namespace among non-deleted), `status`; revision: `target_kind` (`WORKFLOW`; `SCRIPT`, `OPERATION` gated by 022), `target_ref`, `version_policy` (`ACTIVE`, `PINNED`), `pinned_version`, `title`, `description`, `result_projection` (expression ≤ 1 KiB), `result_schema jsonb` (closed, ≤ 64 KiB), `max_result_classification`, `disclosure` (`declared-mcp-result-v1`), `wait_ms`, `actor`, `request_id` | ALWAYS-immutable revisions; partial UNIQUE `(tenant_id, namespace_id, tool_name) WHERE status <> 'DELETED'`; descriptions, schema, projection content |
| `mcp_tokens` | PK `(tenant_id, token_id)`; `namespace_id`, `token_sha256 bytea(32)`, `minter`, `label` (≤ 64), `exposure_ids uuid[]` (≤ 64), `expires_at`, `revoked_at`, `created_at` | UNIQUE `token_sha256`; CHECK `expires_at - created_at <= 90 days`; UPDATE only `revoked_at`; the plaintext token never stored; rows deleted 90 days after expiry or revocation |

## 3. Run-time tables

| Table | Key and columns | Constraints and notes |
|---|---|---|
| `run_mcp_snapshot_pins` | PK `(tenant_id, run_id, node, resource_id)`; `namespace_id`, `snapshot_id` | written in the admission transaction beside 021's `run_resource_bindings` rows of the agent node's provider and MCP-server revisions; immutable; keeps snapshots alive while the run exists |
| `agent_steps` | PK `(tenant_id, run_id, node)`; `namespace_id`, `status`, `current_turn`, effective limits (`max_turns`, `max_tool_calls`, `max_parallel_tool_calls`, `max_repair_turns`, `max_input_tokens`, `max_output_tokens`, `max_cost_micros` nullable, `currency` nullable, `deadline`), counters (`turns_started`, `tool_calls_planned`, `input_tokens_settled/reserved`, `output_tokens_settled/reserved`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros_settled/reserved` (reserved = sum of the step's `IN_FLIGHT` attempt reservations; settled includes charged-unknown reservations), `repair_turns_used`, `transcript_bytes`), `failure_code`, `version`, `created_at`, `finished_at` | FK to the owning `steps` row; CHECK settled + reserved ≤ limit for every budget; CHECK counters ≥ 0; CAS on `version`; a trigger keeps the reserved and settled counters equal to the sum of the step's `agent_call_attempts` rows; SELECT/INSERT/UPDATE; all columns protected (bounded numerics, enums) |
| `agent_turns` | PK `(tenant_id, run_id, node, turn)`; `kind`, `derived_node` (`<node>~m<turn>`), `status`, settled usage columns of the committed attempt, `usage_known`, `stop_reason`, `reported_model` (≤ 128), `request_sha256`, `response_ref` (payload digest or blob id), `committed_seq`, `created_at`, `settled_at` | FK to `agent_steps` and to the derived `steps` row; UNIQUE `(tenant_id, run_id, derived_node)`; settled once (trigger) |
| `agent_call_attempts` | PK `(tenant_id, run_id, derived_node, attempt, generation)`; `namespace_id`, `node`, `turn`, `day` (database date at claim), `currency`, `reserved_input_tokens`, `reserved_output_tokens`, `reserved_cost_micros`, `state` (`IN_FLIGHT`, `SETTLED`, `RELEASED`, `CHARGED_UNKNOWN`), reported usage, `lease_writer` (the server backend whose usage lease was charged), `created_at`, `closed_at` | created by Task 025-007; inserted in the claim transaction that grants the lease (D1286); closed exactly once by completion (`SETTLED`), a provider error response (`RELEASED`) or a lost response, timeout or expiry (`CHARGED_UNKNOWN`); `IN_FLIGHT` rows also bound the request staging blob's lifetime |
| `agent_tool_calls` | PK `(tenant_id, run_id, node, turn, call_index)`; `tool_name`, `kind`, `provider_call_id` (≤ 128), `input_ref`, `status`, `derived_node` (`<node>~t<turn>.<call>`), `approval_request_id`, `idempotency_key`, `result_ref`, `result_truncated`, `effect_outcome` (`NONE`, `APPLIED`, `NOT_APPLIED`, `UNKNOWN`), `child_run_id`, `completion_order`, `completed_seq` | FK to the turn; UNIQUE `(tenant_id, run_id, derived_node)`; UNIQUE `(tenant_id, run_id, node, completion_order)` for reverse-order compensation; `kind` is what maps the derived id to its managed kind (section 4) |
| `agent_transcript_entries` | PK `(tenant_id, run_id, node, entry_seq)`; `entry_kind` (`REQUEST_PREFIX`, `ASSISTANT`, `TOOL_RESULTS`, `REPAIR_FEEDBACK`), `turn`, `payload_ref`, `bytes`, `journal_seq` | immutable; bodies live in `payloads` (content, counted once per stored row) or as blob claim-checks; total bytes ≤ the transcript ceiling, enforced in the appending transaction |
| credential pins (021's `step_resource_credentials`) | PK `(tenant_id, run_id, node, attempt, generation, secret_slot)`; `namespace_id`, resource and revision, logical secret name and version, `created_at` | metadata only, no credential bytes; SELECT/INSERT/DELETE, UPDATE rejected (017 `step_database_credentials` precedent). The slot is part of the key so an attempt that resolves several secrets (an access key pair, an Entra client id and secret) pins each, as migration `20270110000000_step_credential_roles` did for `step_database_credentials.secret_role` (Decision 035-139); 025 adds the slot to 021's table if 021 has not (Decision 025-014) |
| `ai_usage_daily` | PK `(tenant_id, namespace_id, day, currency)`; folded settled `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros`, and folded `reserved_input_tokens`, `reserved_output_tokens`, `reserved_cost_micros` | `currency = ''` for unpriced usage; never written at every commit: only a lease refill or a fold of the purge pass writes it (Decision 025-010, Architecture decision 0097); lock slot 8b of Architecture decision 0050, always last |
| `ai_usage_leases` | PK `(tenant_id, namespace_id, day, currency, writer)`; `writer` = `pg_backend_pid()` of the charging server backend, `granted_tokens`, `used_tokens`, `granted_cost_micros`, `used_cost_micros`, `charged_at` | CHECK 0 ≤ used ≤ granted; a claim's reservation, a settlement and a release move only the writer's own lease; a refill locks the `ai_usage_daily` row, then the writer's own lease `NOWAIT`; the purge pass folds and deletes leases idle for 10 minutes or whose backend has ended; the daily quota check is folded usage plus every lease's granted room, exact (Architecture decision 0097 decisions 1 to 5); fill factor 50; SELECT/INSERT/UPDATE/DELETE through `SECURITY DEFINER` functions only |
| `mcp_call_receipts` | PK `(tenant_id, namespace_id, caller, idempotency_key)`; `arguments_sha256`, `exposure_id`, `exposure_revision`, `run_id`, `created_at`, `expires_at` | 24-hour window from database time; changed arguments under a live key conflict; rows deleted after expiry |
| `mcp_task_handles` | PK `(tenant_id, handle_id)`; `namespace_id`, `run_id`, `caller`, `exposure_id`, `exposure_revision`, `expires_at` | opaque UUID; bound to caller and tenant; deleted with the run or 7 days after the run is terminal |

The daily invariant (folded usage plus every lease's used room equals the sum of that day's
`agent_call_attempts` reservations and settlements) is checked by an integration test and by the
maintenance pass, not by a trigger on the shared row (Decision 025-010).

## 4. Changes to existing tables and types

- `steps.node` and every reader of derived ids accept, besides a declared id and the existing `~c`
  and `~r` forms (and `~d.<name>` of Architecture decision 0071 decision 5 once feature 035's
  destinations are built), `<agent>~m<n>`, `<agent>~t<n>.<k>`, `<agent>~t<n>.<k>~c` and
  `<agent>~t<n>.<k>~r` (`NodeId::parse_stored`); no other form is accepted and forged ids from outside
  remain rejected. An agent node declares no destinations (Decision 025-004), so an agent node id is at
  most 54 characters and the longest derived id (`~t64.256~c`, ten characters) stays within the
  existing 64-character node id bound (`MAX_NODE_ID_LEN` in `crates/domain/src/workflow.rs`).
- Managed kinds (`crates/domain/src/database.rs`): `ManagedKind` gains `AiModel`, `AiMcp`, `AiProbe`
  and `AiMcpRefresh`, whose `activity()` is `sagat.ai.model`, `sagat.ai.mcp`, `sagat.ai.probe` and
  `sagat.ai.mcp-refresh` and whose `profile()` is the new `BindingProfile::Agent`
  (`crates/domain/src/binding.rs`, stored as `agent`). A derived id maps to a kind only through the
  agent declaration's derived-node function, which builds the `Node` every worker path then asks
  `Node::managed_kind` (Decision 025-003):

  | Stored id | Node it materializes | `managed_kind` |
  |---|---|---|
  | `<agent>~m<turn>` | the model call | `AiModel` |
  | `<agent>~t<turn>.<call>`, tool kind `mcp` (from `agent_tool_calls.kind`) | the MCP call | `AiMcp` |
  | `<agent>~t<turn>.<call>`, tool kind `database` | a feature-017 operation | `Sql` |
  | `<agent>~t<turn>.<call>`, tool kind `activity`, `script`, `child`, `operation` | an ordinary activity, a script, a child or a 021 operation | `None` (their own executors) |
  | `<agent>~t<turn>.<call>~c`, `~r` | the declared compensating or reconciling activity | `None` |
  | root node of a probe or refresh private definition | the probe or refresh | `AiProbe`, `AiMcpRefresh` |

- `steps.node_kind` (closed check added by migration `20261205000000_bounded_execution_discovery`)
  gains `agent` for the owning step; model-call and MCP tool-call derived steps are `activity` rows of
  reserved activities, child tool calls are `child` rows.
- `approval_requests`: `owner_kind text NOT NULL DEFAULT 'NODE'` (`NODE`, `AGENT_TOOL_CALL`); the
  owner-node pattern accepts the tool-call derived id only for `AGENT_TOOL_CALL`; route columns are
  nullable only for that kind; the outcome-consistency check is versioned per kind. Feature 014's
  rows are unchanged (G1289).
- `runs.origin` gains `AI_PROVIDER_PROBE`, `MCP_TOOL_REFRESH`, `AUTHORING_ASSISTANT` (private
  retained definitions, 017 pattern); all three are created by `start_private`
  (`crates/application/src/runs/ad_hoc.rs`) and so pass row 13 of the design 043 run-input gate.
- The instance registry (migration `20270113000000_instance_registry`): the closed CHECKs
  `serving_instances_capabilities_closed` and `platform_requirements_capability_valid` gain
  `ai-agents-v1` and `mcp-server-v1`, and `Capability::ALL` (`crates/application/src/instances.rs`)
  gains `AiAgentsV1` (kinds `CONTROL_PLANE` and `EXECUTION_ENGINE`) and `McpServerV1` (kind
  `CONTROL_PLANE`) (Decision 025-002).
- The permission catalog gains six permissions (`Permission::ALL` grows from 55 to 61) and the built-in
  roles gain `agent-runner`, the eighth (D1312, Owner decision 2026-11-26-121).
- `workflow_versions` gains `agent_unattended_writes boolean NOT NULL DEFAULT false`, computed at
  publication (true when any agent `write` tool is acknowledged `approval: "none"`); exposures and
  feature 026 public links refuse versions where it is true (D1296).
- Quota resources gain `ai_runner_concurrency`, `tenant_ai_runner_concurrency`, `ai_tokens_per_day`,
  `ai_cost_micros_per_day`, `ai_providers`, `mcp_servers`, `mcp_exposures`, `mcp_tokens`. The two
  concurrency resources have no usage counter: usage is the count of live leases of every `sagat.ai.*`
  activity (model calls, MCP calls, probes and refreshes) read in the claim transaction, so a dead
  runner's capacity returns when its leases expire (Decision 025-013).

## 5. Time classes, keys and reset

- Time columns are registered in 028's catalog (`crates/infra-postgres/time-classification.json`,
  to be created by feature 028): `agent_steps.deadline` and tool-approval deadlines are
  `workflow_due`; `ai_usage_daily.day` and `ai_usage_leases.day` (a database-date quota window),
  `agent_call_attempts` times, lease charge times, receipts, task handles, token expiry and revocation,
  catalog and snapshot times are `infrastructure`; turn and tool-call commit times are
  `workflow_record`.
- 032's key inventory: `mcp_tokens` holds SHA-256 digests of 256-bit random tokens, so no key exists;
  the table replicates with the database, and tokens minted in a write window lost by a failover are
  invalid afterwards. OAuth verification uses the configured issuer's JWKS (a cache, not a secret).
- Reset (018, merged): the reset-rule registry is code (`crates/domain/src/execution/reset.rs`). The
  open-loop events are `ResetRule::Refuse { until: Some("agent.finished") }` and `agent.finished` is
  `ResetRule::Rematerialize`; the reset transaction re-materializes a finished agent step by copying
  its `agent_steps`, `agent_turns`, `agent_tool_calls`, `agent_call_attempts` and
  `agent_transcript_entries` rows into the new run (same payload digests, bytes charged to the new
  run, no usage-lease charge); a reset point inside an open loop is not offered (D1635).

## 6. Canonical transcript representation

A transcript entry body is canonical JSON validated by `contracts/model-turn.schema.json`: blocks
of type `text`, `tool_request` (`callIndex`, `name`, `providerCallId`, `input`), `tool_result`
(`callIndex`, `isError`, `content`, `truncated`), `opaque` (provider kind, provider block type, the
exact bytes as base64, never interpreted, replayed only to the same provider kind) and
`validation_feedback` (value-free rule and field pairs of the 043 validator, Decision 025-007).
Adapters map provider wire formats to and from this form. The domain holds these as opaque validated
documents with typed headers (turn, call index, kinds, digests); it does not use `serde_json` for
business rules.

## 7. Retention

- The rendered request of an attempt above the inline limit is a staging blob written by the server
  for that attempt only: counted as an outstanding reservation of `tenant_content_bytes` (ADR-0027),
  readable only under the attempt's lease, and deleted when the attempt closes or by the existing
  upload-TTL cleanup; it is never a second retained copy of the transcript.
- Transcripts (`agent_transcript_entries` and their payload rows), tool inputs and results, agent
  outputs and request staging blobs are content stores registered in 020's content-store registry
  (coordinator C8) and purged with the run's content under 020's retention and erasure rules; their
  bytes count toward `tenant_content_bytes`. Customer retention policy is 020's (G69); data sent to
  providers or MCP servers is outside platform deletion (G1285). Agent-runner steps write no step logs
  (design 041; Decision 025-005).
- Provider and MCP-server revisions referenced by `run_resource_bindings`, exposure revisions, and MCP
  snapshots referenced by `run_mcp_snapshot_pins` are retained while those runs exist; unreferenced
  snapshots beyond the newest 10 per server are deleted by a bounded maintenance job.
- Receipts expire after 24 hours, task handles 7 days after their run is terminal, revoked or
  expired tokens 90 days after the event. Audit records follow the existing audit retention.
