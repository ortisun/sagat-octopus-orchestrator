# Agent runner protocol (draft worker-protocol excerpt)

Draft for feature 025. Task 025-002 merges it into `specs/001-platform-foundation/contracts/worker.proto`,
regenerates `worker.baseline.json` and adds the row of its minor to the register of worker-protocol
minors in `sdks/SPEC.md` section 9. Minor 1.7 is feature 041's (step logs, Owner decision 2026-11-15-8);
this contract takes the next free minor of that register when it merges (written `1.N` below; Gap 1280).
It is additive: older workers keep their minor, are never offered `sagat.ai.*` tasks, and ordinary
activity tools of an agent need no protocol change (they are ordinary leases whose `step_id` is a
derived id).

Because `1.N` is above 1.7, an agent runner negotiates 1.7 too. It never calls `AppendStepLogs` and
never sends a `StepLogCarry`; the server sends no `StepLogPolicy` in an agent-runner registration,
refuses an `agent-runner` token at the step-log ingest (design 041, pre-check P1 and authority check T7)
and ignores a carry on a `sagat.ai.*` step (carry check C0). The transcript is the authoritative record
of a model or MCP call (Decision 025-005).

## Identity and claim

- Machine role `agent-runner` holds exactly `worker:execute` and `ai:execute` (never
  `database:execute`) and needs a current namespace binding of `BindingProfile::Agent`
  (`crates/domain/src/binding.rs`). A binding's profile decides what the identity may register for,
  whatever its patterns say, so a binding to `*` still serves only the reserved AI activities. Human
  administrators, ordinary workers, sandbox runners and database and lake runners (profiles `database`
  and `lake`) cannot claim or resolve these tasks; the agent runner advertises only reserved activities.
- Reserved activities, created only by an `agent` node, a provider probe or an MCP refresh. Each is a
  managed kind (`crates/domain/src/database.rs`), so every worker path tells them apart by asking
  `Node::managed_kind` of the node the agent declaration materializes for the stored id (Decision
  025-003), never by the id's suffix or the activity's name:

| Activity | `ManagedKind` | Task | Derived step |
|---|---|---|---|
| `sagat.ai.model` | `AiModel` | one model call | `<node>~m<turn>` |
| `sagat.ai.mcp` | `AiMcp` | one external MCP tool call | `<node>~t<turn>.<call>` of an `mcp` tool |
| `sagat.ai.probe` | `AiProbe` | provider probe (list models) | root node of the private probe definition |
| `sagat.ai.mcp-refresh` | `AiMcpRefresh` | external MCP `tools/list` snapshot | root node of the private refresh definition |

- An ordinary activity or worker that names a `sagat.ai.*` activity is rejected at publication and
  registration (017 reserved-prefix precedent; today `check_node` in `crates/domain/src/workflow.rs`
  reserves only `sagat.database.`, so this feature adds the AI prefix there and in
  `managed_capabilities_allowed`). Recording `ai-agents-v1` is refused while any existing version has an
  ordinary activity under the prefix (Task 025-095). The claim selects an agent-runner lease by the
  node's managed kind as well as by activity name: a node whose `managed_kind` is not the AI kind of the
  activity is never offered to an agent runner, whatever its name. For a `sagat.ai.*` claim only, the claim transaction
  counts the live leases of every `sagat.ai.*` task of the namespace and tenant under their locks
  against `ai_runner_concurrency` and `tenant_ai_runner_concurrency` (no counter, so completion and
  expiry release capacity), and for a `sagat.ai.model` attempt inserts its reservation
  (`agent_call_attempts`), raises the reserved counters of the agent and charges the claiming server
  backend's own usage lease (`ai_usage_leases`, refilled from `ai_usage_daily` only when short;
  Decision 025-010). If the reservation does not fit, the same transaction fails the derived step and
  the agent with `AI_BUDGET_EXHAUSTED` and grants no lease. An ordinary claim runs none of these
  statements (design 038 budget, Decision 025-009).
- Managed tasks carry `input_json = {}`; the request is obtained only through the resolve call.
  Generic `ReadBlob` is refused for these leases except the request and response blobs the resolve
  call names.
- The node the agent declaration materializes for every `sagat.ai.*` kind has an empty `secrets` list.
  The ordinary `ResolveSecrets` call refuses a lease whose node has any managed kind (the managed
  database kinds use `ResolveDatabase` only), `ResolveDatabase` refuses the AI kinds, and
  `ResolveModelCall` and `ResolveMcpCall` each refuse every kind but their own, so the checks below
  cannot be bypassed through another call.

## Credential delivery (Principle V, first declared set)

A model-call or MCP-call task's immutable task definition is the agent node of the published version
plus the provider revision and the MCP-server revision that version pins; a probe or refresh task's is
its private definition, which pins its revision. The logical secrets those pinned revisions name are
the declared secrets of that task definition, so the resolve calls below disclose them under Principle
V's first declared set, ordinary task execution (Decision 025-001):

- only a static API key or bearer token: the provider modes `aws-access-key` and
  `azure-entra-client-credentials`, which would hand the runner a signing key or a client secret, are
  refused at save until design 047's broker issues access tokens (Gap 025-001), and the first set never
  covers their secrets (Owner question 025-001 answered no, Owner decision 2026-11-27-400);
- all or nothing: every slot of the pinned revision, or none;
- only while the caller's current membership, its binding (`BindingProfile::Agent` for the task's
  reserved activity), the pool-delivery policy of every secret and the publisher's `secret:use` on
  every secret (also checked at publication) authorize it; the initiator's `ai:use` is checked too;
- after validating the current tenant and namespace scope, the exact task, instance, attempt and
  generation, the matching capability token and the unexpired lease, and, beyond what the first set
  asks, the recorded lease-owning executor subject and its live session;
- revalidating all of the above and that the selected secret revisions still match after OpenBao I/O,
  and committing the value-free `secret.resolved` audit and a metadata-only pin per secret slot (021's
  `step_resource_credentials`, keyed by slot, Decision 025-014) before any value is returned. A failed
  check or audit refuses disclosure.

Both phases also recheck the current revision's restrictions: the transcript level against the
stricter of the pinned and the current ceiling, and the pinned model against the current allowlist
(spec Requirement 025-002); a failure refuses disclosure with `CLASSIFICATION_EGRESS_REFUSED` or
`AI_CAPABILITY_UNAVAILABLE`.

This grants no general secret read and no `database:execute`. MCP servers authenticated by OAuth, and
the two provider modes above, are outside it: they reach access tokens only through design 047's token
broker, under the third declared set of Principle V that the owner approved for constitution 5.1.0
(Owner decision 2026-11-25-3), merged in pull request 286; `auth_mode` `oauth-client-credentials` is refused at save
until the broker is built (Gap 025-001), and the OAuth fields of `McpEndpoint` below
are reserved for that case. Once the broker exists, the two provider modes are delivered through it
(Decision 025-017): the client secret and the long-lived AWS key pair are read by the broker only; for
`azure-entra-client-credentials` the broker runs the client-credentials grant against the recorded
Entra authority, and for `aws-access-key` it calls AWS STS (`AssumeRole` with the recorded role,
`GetSessionToken` without one; 900 seconds by default, at most 3,600) and mints from that session an
Amazon Bedrock short-term API key; `ResolveModelCall` returns that access token, with its expiry, in the
`bearer` slot, and the runner sends it as a bearer token. The runner makes no token request, holds no
SigV4 signer, and never receives the client secret, the long-lived key or the session's signing key.

## New calls

```proto
service WorkerService {
  // Added in 1.N, the next free minor of the sdks/SPEC.md section 9 register when this merges.
  // Only the current agent-runner machine that owns this lease may call, under the credential
  // delivery rules above (Principle V, first declared set).
  // Two phases: fences in a run transaction closed before OpenBao I/O, then the same fences, the
  // metadata-only credential pin and the secret.resolved audit in a new transaction, before any
  // byte is returned. The response contains no OpenBao routing or token, platform URL or key.
  rpc ResolveModelCall(ResolveSecretsRequest) returns (ResolvedModelCall);
  rpc ResolveMcpCall(ResolveSecretsRequest) returns (ResolvedMcpCall);
  // Token streaming uses feature 026's AppendStreamEvents on the reserved channel
  // sagat.agent.tokens (agent-stream-events.md); no agent-specific streaming rpc is added.
}

enum AiProviderKind {
  AI_PROVIDER_UNSPECIFIED = 0; AI_PROVIDER_ANTHROPIC = 1; AI_PROVIDER_OPENAI_COMPATIBLE = 2;
  AI_PROVIDER_AWS_BEDROCK = 3; AI_PROVIDER_AZURE_OPENAI = 4;
}
enum AiTaskKind {
  AI_TASK_UNSPECIFIED = 0; AI_TASK_MODEL_CALL = 1; AI_TASK_PROBE = 2;
  AI_TASK_MCP_CALL = 3; AI_TASK_MCP_REFRESH = 4;
}
message AiEndpoint {
  AiProviderKind kind = 1;
  string base_url = 2;          // https; pinned revision; checked again by the runner's egress guard
  string region = 3;            // aws-bedrock
  string api_version = 4;       // azure-openai
  string deployment = 5;        // azure-openai deployment of model_id
  bool prompt_caching = 6;
  // Reserved for azure-entra-client-credentials once design 047's broker exists: the Entra
  // authority host recorded in the revision, for the runner's spans only; the broker, never the
  // runner, sends the token request there and the runner receives the access token (Decision 025-017).
  string entra_authority = 7;
}
message AiLimits {
  uint32 attempt_timeout_ms = 1;     // the runner also stops at lease_expires_at
  uint64 max_response_bytes = 2;
  uint32 max_stream_events = 3;
  uint32 max_output_tokens = 4;
}
message ResolvedModelCall {
  AiTaskKind task = 1;               // MODEL_CALL or PROBE
  AiEndpoint endpoint = 2;
  string model_id = 3;
  string effort = 4;                 // empty = provider default
  // Canonical rendered request (instructions, messages, tools, output schema), rendered by the
  // server. Inline up to 256 KiB, otherwise a blob the runner reads under this lease.
  string request_json = 5;
  string request_blob_ref = 6;
  bytes request_sha256 = 7;
  bool native_structured_output = 8;
  bool stream_tokens = 9;
  AiLimits limits = 10;
  repeated ResolvedSecret credentials = 11;   // slot name in ResolvedSecret.name
  int64 lease_expires_at_epoch_ms = 12;
  string namespace_id = 13;          // verified scope for the runner's destination policy
  string tenant_id = 14;             // trusted correlation; never a metric label
  string provider_name = 15;
  uint32 provider_revision = 16;
  uint32 turn = 17;
}
message McpEndpoint {
  string url = 1;
  string protocol_version = 2;       // "2026-07-28" or "2025-11-25"
  string auth_mode = 3;              // none | bearer (oauth-client-credentials reserved, Gap 025-001)
  // Fields 4 to 8 are reserved for OAuth servers and stay empty until design 047's broker is built (the third
  // declared set of constitution 5.1.0, merged in pull request 286, admits them);
  // tokens will then come from that broker.
  string client_id = 4;
  string token_endpoint = 5;         // recorded in the server revision; never discovered at call time
  string resource = 6;               // RFC 8707 resource indicator
  repeated string scopes = 7;
  string issuer = 8;                 // recorded issuer; resolution refuses while a refresh reported a mismatch
}
message ResolvedMcpCall {
  AiTaskKind task = 1;               // MCP_CALL or MCP_REFRESH
  McpEndpoint endpoint = 2;
  string tool_name = 3;
  string arguments_json = 4;         // validated by the server against the pinned closed schema
  bytes expected_description_sha256 = 5;
  bytes expected_input_schema_sha256 = 6;
  bytes expected_output_schema_sha256 = 7;   // empty when the snapshot had none
  string idempotency_key = 8;        // agent:{runId}:{node}:{turn}:{call}; sent in _meta when the server accepts it
  AiLimits limits = 9;
  repeated ResolvedSecret credentials = 10;
  int64 lease_expires_at_epoch_ms = 11;
  string namespace_id = 12;
  string tenant_id = 13;
  string server_name = 14;
  uint32 server_revision = 15;
}
```

Field numbers are draft and fixed only when merged.

## Results

- `sagat.ai.model`: `CompleteStep.output_json` is a `model-turn.schema.json` document (or
  `output_blob_ref` above 256 KiB). The server rejects a result whose turn differs or that contains a
  value the attempt resolved (verbatim, JSON-escaped, base64 or hexadecimal, spec Requirement 025-020);
  a tool request whose name is not declared is kept in the transcript as the model wrote it and is
  recorded everywhere else as the marker `UNKNOWN_TOOL` with its call index; the server never trusts the
  runner's view of budgets or classification.
- `sagat.ai.probe`: `{"models": ["claude-opus-5-5", ...], "outcome": "OK"}` limited to 256 ids.
- `sagat.ai.mcp`: `{"content": [...], "isError": bool, "truncated": bool}` with text and structured
  content only (D1317). The server turns `isError: true` into the reason `MCP_TOOL_ERROR` and forwards
  the bounded text to the model only when the agent's transcript level is `pii` (Decision 025-008).
- `sagat.ai.mcp-refresh`: `{"protocolVersion": "...", "tools": [{name, title, description,
  inputSchema, outputSchemaSha256, annotations}]}`, bounded by the snapshot ceilings, as a blob when
  large; the server computes and stores the digests.
- Failures use `FailStep.error_code` from the closed `AiFailure` vocabulary in `errors.md`, whose
  codes are `worker`-origin entries of `sdks/error-catalog.json` (Task 025-087);
  `safe_message` is discarded by the transport. `OUTCOME_UNKNOWN` is reserved for an MCP call whose
  request may have been applied. A model call is never `OUTCOME_UNKNOWN`: it has no external effect
  of its own; a lost response is a retryable failure. The failure code decides the attempt's
  reservation: a provider error response (`AI_RATE_LIMITED`, `AI_PROVIDER_UNAVAILABLE` for `5xx`/`529`
  and failures before the request was sent, `AI_REQUEST_INVALID`, `AI_PROVIDER_AUTH`,
  `AI_PROVIDER_REJECTED`, `AI_CONTEXT_TOO_LARGE`) releases it (G1294); `AI_RESPONSE_INCOMPLETE` (the
  stream broke after `200`, or the response was lost) and lease expiry keep it as charged.

## Runner behaviour

- Heartbeats start before any provider or MCP I/O; `cancel_requested`, lease loss or expiry aborts
  the provider request. Renewing the lease does not extend `attempt_timeout_ms`.
- Each attempt sends its provider or MCP request once. A transport retry of the report reuses the
  same idempotency key and content for up to 30 seconds within the lease; it never re-sends the
  provider request.
- Credentials live in memory for the attempt only; no provider token cache on disk or across
  attempts (OAuth MCP tokens, once enabled, come only from design 047's broker, Gap 025-001).
  Provider adapters use explicit static credentials built from the
  resolved values; SDK default credential chains (environment, profile, instance metadata, workload
  identity) are disabled, and a test proves link-local and metadata addresses are unreachable from
  the runner (T025-016).
- No redirect is followed: the runner's egress client has `max_redirects = 0` for every provider and
  MCP request, the probe and the refresh included, and a `3xx` answer fails the attempt with
  `AI_DESTINATION_DENIED` (audited `egress.denied`), so no credential header reaches a redirect target.
  A test builds the runner's client and asserts the setting, and a probe answered `302` fails (Task
  025-016).
- The namespace egress policy is operator-owned runner configuration, per namespace, reread before
  every attempt (the `apps/database-runner/src/config.rs` precedent); the resolve response's
  `namespace_id` selects the entry, a namespace absent from it is denied, and the server cannot widen it.
- No prompt, response, arguments, credentials, provider messages or tokens in logs, spans or
  metrics. Spans carry `sagat.*` ids, provider kind, the pinned model id, `sagat.ai.model_substituted`
  and the provider request id when it matches `^[A-Za-z0-9._:-]{1,128}$`; a tool name only when it is
  declared, otherwise `UNKNOWN_TOOL`; never the provider's tool-use id or the reported model.
- The process has no platform database URL, OpenBao token, script signing key, database credential
  or container engine (`scripts/check-architecture.py` refuses a container-engine dependency, Task
  025-073);
  its egress is default-deny except DNS, the engine, approved provider and MCP destinations, and
  telemetry (Helm network policy, T025-068).
