# Data model — feature 025

Proposed; nothing exists yet. Every table is tenant-keyed: the primary key starts with `tenant_id`,
every foreign key is composite with `tenant_id` (and `namespace_id` where the row is
namespace-scoped), and every statement filters by the authenticated tenant. Migration versions are
allocated at implementation time after the latest merged migration; down migrations refuse to drop
rows that live runs or pins reference. Variable-size columns are added to
`crates/infra-postgres/storage-classification.json` (ADR-0027 classes `content`, `log`,
`protected_system`); the runtime role `sagat_app` gets only the privileges listed.

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

## 2. Catalog tables

If feature 021's resource model (ADR-0055, proposed) is accepted first and lets a type name the agent
runner as its executor (O1282, G1281), `ai_providers*` and `mcp_servers*` are rows of its `resources`
and `resource_revisions` tables under the platform-provided types `llm_anthropic`,
`llm_openai_compatible`, `llm_aws_bedrock`, `llm_azure_openai` and `mcp_server`; the columns below then
describe each type's settings schema, `run_ai_bindings` becomes `run_resource_bindings` and
`step_ai_credentials` becomes `step_resource_credentials`.

| Table | Key and columns | Constraints, indexes, grants, classification |
|---|---|---|
| `ai_providers` | PK `(tenant_id, provider_id)`; `namespace_id`, `name` (`^[a-z][a-z0-9-]{2,63}$`), `current_revision`, `status`, `created_at`, `updated_at` | UNIQUE `(tenant_id, namespace_id, name)`; deferred FK head → revision; SELECT/INSERT/UPDATE (status, head only); `name` protected (pattern-bounded) |
| `ai_provider_revisions` | PK `(tenant_id, provider_id, revision)`; `namespace_id`, `kind` (`anthropic`, `openai-compatible`, `aws-bedrock`, `azure-openai`), `settings jsonb` (closed per kind), `credential_secrets text[]` (logical names, ≤ 4), `models jsonb` (≤ 64 entries: id, alias, maxOutputTokens, contextTokens, capabilities, prices in integer micro-units per million tokens and ISO 4217 currency), `max_prompt_classification` (`public`/`internal`/`pii`), `limits jsonb`, `actor`, `request_id`, `requested_status`, `created_at` | UNIQUE receipt `(tenant_id, namespace_id, actor, request_id)`; ALWAYS trigger rejects UPDATE/DELETE; SELECT/INSERT; `settings`, `models`, `limits` content |
| `mcp_servers`, `mcp_server_revisions` | same shape; revision: `url` (HTTPS, no userinfo, no query secrets), `auth_mode` (`none`, `bearer`, `oauth-client-credentials`), `credential_secrets text[]`, `protocol_preference` (`2026-07-28`, `2025-11-25`), `max_argument_classification` (`public`, `internal`, `pii`; required, no default), `oauth jsonb` (required for `oauth-client-credentials`: `issuer`, `token_endpoint` (HTTPS), client id, resource, scopes; recorded and confirmed by the saving `ai:manage` holder, never replaced by discovery), `limits jsonb`, `current_snapshot_id` on the head; a refresh stores the issuer and token endpoint it discovered on the snapshot (`discovered_issuer`, `discovered_token_endpoint`) for comparison only | as above; `url`, `oauth`, `limits` content |
| `mcp_tool_snapshots` | PK `(tenant_id, snapshot_id)`; `server_id`, `server_revision`, `protocol_version`, `tool_count`, `snapshot_sha256`, `refreshed_by_run`, `created_at` | immutable; ≤ 1,024 tools; SELECT/INSERT |
| `mcp_snapshot_tools` | PK `(tenant_id, snapshot_id, name)`; `title`, `description` (≤ 16 KiB), `description_sha256`, `input_schema jsonb` (≤ 64 KiB), `input_schema_sha256`, `output_schema_sha256`, `annotations jsonb` (untrusted, ≤ 4 KiB) | immutable; `description`, `input_schema`, `annotations` content |
| `mcp_exposures`, `mcp_exposure_revisions` | head as above with `tool_name` (`^[a-zA-Z0-9_-]{1,64}$`, within MCP's name rules and the tool-name rules of current model APIs; unique per namespace among non-deleted); revision: `target_kind` (`WORKFLOW`; `SCRIPT`, `OPERATION` gated by 022), `target_ref`, `version_policy` (`ACTIVE`, `PINNED`), `pinned_version`, `title`, `description`, `result_projection` (expression ≤ 1 KiB), `result_schema jsonb` (closed, ≤ 64 KiB), `max_result_classification`, `disclosure` (`declared-mcp-result-v1`), `wait_ms` | ALWAYS-immutable revisions; partial UNIQUE `(tenant_id, namespace_id, tool_name) WHERE status <> 'DELETED'`; descriptions, schema, projection content |
| `mcp_tokens` | PK `(tenant_id, token_id)`; `namespace_id`, `token_sha256 bytea(32)`, `minter`, `label` (≤ 64), `exposure_ids uuid[]` (≤ 64), `expires_at`, `revoked_at`, `created_at` | UNIQUE `token_sha256`; CHECK `expires_at - created_at <= 90 days`; UPDATE only `revoked_at`; the plaintext token never stored; rows deleted 90 days after expiry or revocation |

## 3. Run-time tables

| Table | Key and columns | Constraints and notes |
|---|---|---|
| `run_ai_bindings` | PK `(tenant_id, run_id, node, binding_kind, resource_id)`; `namespace_id`, `binding_kind` (`PROVIDER`, `MCP_SERVER`), `revision`, `snapshot_id` | written in the admission transaction; immutable; FKs to run and revision; keeps revisions and snapshots alive while the run exists |
| `agent_steps` | PK `(tenant_id, run_id, node)`; `namespace_id`, `status`, `current_turn`, effective limits (`max_turns`, `max_tool_calls`, `max_input_tokens`, `max_output_tokens`, `max_cost_micros` nullable, `currency` nullable, `deadline`), counters (`turns_started`, `tool_calls_planned`, `input_tokens_settled/reserved`, `output_tokens_settled/reserved`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros_settled/reserved` (reserved = sum of the step's `IN_FLIGHT` attempt reservations; settled includes charged-unknown reservations), `repair_turns_used`, `transcript_bytes`), `failure_code`, `version`, `created_at`, `finished_at` | FK to the owning `steps` row; CHECK settled + reserved ≤ limit for every budget; CHECK counters ≥ 0; CAS on `version`; SELECT/INSERT/UPDATE; all columns protected (bounded numerics, enums) |
| `agent_turns` | PK `(tenant_id, run_id, node, turn)`; `kind`, `derived_node` (`<node>~m<turn>`), `status`, settled usage columns of the committed attempt, `usage_known`, `stop_reason`, `reported_model` (≤ 128), `request_sha256`, `response_ref` (payload digest or blob id), `committed_seq`, `created_at`, `settled_at` | FK to `agent_steps` and to the derived `steps` row; UNIQUE `(tenant_id, run_id, derived_node)`; settled once (trigger) |
| `agent_call_attempts` | PK `(tenant_id, run_id, derived_node, attempt, generation)`; `namespace_id`, `node`, `turn`, `day` (database date at claim), `currency`, `reserved_input_tokens`, `reserved_output_tokens`, `reserved_cost_micros`, `state` (`IN_FLIGHT`, `SETTLED`, `RELEASED`, `CHARGED_UNKNOWN`), reported usage, `created_at`, `closed_at` | inserted in the claim transaction that grants the lease (D1286); closed exactly once by completion (`SETTLED`), a provider error response (`RELEASED`) or a lost response, timeout or expiry (`CHARGED_UNKNOWN`); a trigger keeps `agent_steps` and `ai_usage_daily` reserved and settled counters equal to the sum of these rows; `IN_FLIGHT` rows also bound the request staging blob's lifetime |
| `agent_tool_calls` | PK `(tenant_id, run_id, node, turn, call_index)`; `tool_name`, `kind`, `provider_call_id` (≤ 128), `input_ref`, `status`, `derived_node` (`<node>~t<turn>.<call>`), `approval_request_id`, `idempotency_key`, `result_ref`, `result_truncated`, `effect_outcome` (`NONE`, `APPLIED`, `NOT_APPLIED`, `UNKNOWN`), `child_run_id`, `completion_order`, `completed_seq` | FK to the turn; UNIQUE `(tenant_id, run_id, derived_node)`; UNIQUE `(tenant_id, run_id, node, completion_order)` for reverse-order compensation |
| `agent_transcript_entries` | PK `(tenant_id, run_id, node, entry_seq)`; `entry_kind` (`REQUEST_PREFIX`, `ASSISTANT`, `TOOL_RESULTS`, `REPAIR_FEEDBACK`), `turn`, `payload_ref`, `bytes`, `journal_seq` | immutable; bodies live in `payloads` (content, counted once per stored row) or as blob claim-checks; total bytes ≤ the transcript ceiling, enforced in the appending transaction |
| `step_ai_credentials` | PK `(tenant_id, run_id, node, attempt, generation)`; `namespace_id`, provider operation ref, logical secret name and version, `created_at` | metadata only, no credential bytes; SELECT/INSERT/DELETE, UPDATE rejected (017 `step_database_credentials` precedent) |
| `ai_usage_daily` | PK `(tenant_id, namespace_id, day, currency)`; settled `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros`, and `reserved_input_tokens`, `reserved_output_tokens`, `reserved_cost_micros` | `currency = ''` for unpriced usage; the daily quota check is settled + reserved + the new reservation; updated in claim, completion, failure and expiry transactions and locked last (plan.md lock order); contention measured in T025-070 (split per provider if needed) |
| `mcp_call_receipts` | PK `(tenant_id, namespace_id, caller, idempotency_key)`; `arguments_sha256`, `exposure_id`, `exposure_revision`, `run_id`, `created_at`, `expires_at` | 24-hour window from database time; changed arguments under a live key conflict; rows deleted after expiry |
| `mcp_task_handles` | PK `(tenant_id, handle_id)`; `namespace_id`, `run_id`, `caller`, `exposure_id`, `exposure_revision`, `expires_at` | opaque UUID; bound to caller and tenant; deleted with the run or 7 days after the run is terminal |

## 4. Changes to existing tables

- `steps.node` and every reader of derived ids accept `<node>~m<n>`, `<node>~t<n>.<k>` and, for a tool
  call's compensation and reconciliation, `<node>~t<n>.<k>~c` and `<node>~t<n>.<k>~r`
  (`NodeId::parse_stored`); no other two-suffix form is accepted and forged ids from outside remain
  rejected. An agent node id is at most 54 characters, so the longest derived id (`~t64.256~c`, ten
  characters) stays within the existing 64-character node id bound (`MAX_NODE_ID_LEN` in
  `crates/domain/src/workflow.rs`).
- `steps.node_kind` (closed check added by migration `20261205000000_bounded_execution_discovery`)
  gains `agent` for the owning step; model-call and MCP tool-call derived steps are `activity` rows of
  reserved activities, child tool calls are `child` rows.
- `approval_requests`: `owner_kind text NOT NULL DEFAULT 'NODE'` (`NODE`, `AGENT_TOOL_CALL`); the
  owner-node pattern accepts the tool-call derived id only for `AGENT_TOOL_CALL`; route columns are
  nullable only for that kind; the outcome-consistency check is versioned per kind. Feature 014's
  rows are unchanged (G1289).
- `runs.origin` gains `AI_PROVIDER_PROBE`, `MCP_TOOL_REFRESH`, `AUTHORING_ASSISTANT` (private
  retained definitions, 017 pattern).
- The permission catalog gains six permissions and the built-in roles gain `agent-runner` (D1312).
- `workflow_versions` gains `agent_unattended_writes boolean NOT NULL DEFAULT false`, computed at
  publication (true when any agent `write` tool is acknowledged `approval: "none"`); exposures and
  feature 026 public links refuse versions where it is true (D1296).
- Quota resources gain `ai_model_concurrency`, `tenant_ai_model_concurrency`, `ai_tokens_per_day`,
  `ai_cost_micros_per_day`, `ai_providers`, `mcp_servers`, `mcp_exposures`, `mcp_tokens`. The two
  concurrency resources have no usage counter: usage is the count of live `sagat.ai.*` leases read in
  the claim transaction, so a dead runner's capacity returns when its leases expire.

## 5. Time classes, keys and reset

- Time columns are registered in 028's catalog (`crates/infra-postgres/time-classification.json`):
  `agent_steps.deadline` and tool-approval deadlines are `workflow_due`; `ai_usage_daily.day` (a
  database-date quota window), `agent_call_attempts` times, receipts, task handles, token expiry and
  revocation, catalog and snapshot times are `infrastructure`; turn and tool-call commit times are
  `workflow_record`.
- 032's key inventory: `mcp_tokens` holds SHA-256 digests of 256-bit random tokens, so no key exists;
  the table replicates with the database, and tokens minted in a write window lost by a failover are
  invalid afterwards. OAuth verification uses the configured issuer's JWKS (a cache, not a secret).
- Reset (018): the reset transaction re-materializes a finished agent step by copying its
  `agent_steps`, `agent_turns`, `agent_tool_calls`, `agent_call_attempts` and
  `agent_transcript_entries` rows into the new run (same payload digests, bytes charged to the new
  run); a reset point inside an open loop is refused (D1635).

## 6. Canonical transcript representation

A transcript entry body is canonical JSON validated by `contracts/model-turn.schema.json`: blocks
of type `text`, `tool_request` (`callIndex`, `name`, `providerCallId`, `input`), `tool_result`
(`callIndex`, `isError`, `content`, `truncated`), `opaque` (provider kind, provider block type, the
exact bytes as base64, never interpreted, replayed only to the same provider kind) and
`validation_feedback`. Adapters map provider wire formats to and from this form. The domain holds
these as opaque validated documents with typed headers (turn, call index, kinds, digests); it does
not use `serde_json` for business rules.

## 7. Retention

- The rendered request of an attempt above the inline limit is a staging blob written by the server
  for that attempt only: counted as an outstanding reservation of `tenant_content_bytes` (ADR-0027),
  readable only under the attempt's lease, and deleted when the attempt closes or by the existing
  upload-TTL cleanup; it is never a second retained copy of the transcript.
- Transcripts (`agent_transcript_entries` and their payload rows), tool inputs and results, agent
  outputs and request staging blobs are content stores registered in 020's content-store registry
  (coordinator C8) and purged with the run's content under 020's retention and erasure rules; their
  bytes count toward `tenant_content_bytes`. Customer retention policy is 020's (G69); data sent to
  providers or MCP servers is outside platform deletion (G1285).
- Provider, MCP server and exposure revisions and MCP snapshots referenced by `run_ai_bindings` are
  retained while those runs exist; unreferenced snapshots beyond the newest 10 per server are deleted
  by a bounded maintenance job.
- Receipts expire after 24 hours, task handles 7 days after their run is terminal, revoked or
  expired tokens 90 days after the event. Audit records follow the existing audit retention.
