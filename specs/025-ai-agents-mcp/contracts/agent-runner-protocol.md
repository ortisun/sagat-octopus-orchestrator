# Agent runner protocol (draft worker-protocol excerpt)

Draft for feature 025. T025-002 merges it into `specs/001-platform-foundation/contracts/worker.proto`
and regenerates `worker.baseline.json`. The minor that carries it is allocated by the coordinator
across features 018–032 (G1280; feature 026 also proposes 1.7 for `AppendStreamEvents`, its G1333);
"1.7" below is provisional and may be the same minor as 026's if they ship together. It is
additive: older workers keep their minor, are never offered `sagat.ai.*` tasks, and ordinary activity
tools of an agent need no protocol change (they are ordinary leases whose `step_id` is a derived id).

## Identity and claim

- Machine role `agent-runner` holds exactly `worker:execute` and `ai:execute` and needs a current
  namespace binding for the reserved activities. Human administrators, ordinary workers, sandbox
  runners and database runners cannot claim or resolve these tasks, including through a wildcard
  binding; the agent runner advertises only reserved activities.
- Reserved activities, created only by an `agent` node, a provider probe or an MCP refresh:

| Activity | Task | Derived step |
|---|---|---|
| `sagat.ai.model` | one model call | `<node>~m<turn>` |
| `sagat.ai.mcp` | one external MCP tool call | `<node>~t<turn>.<call>` |
| `sagat.ai.probe` | provider probe (list models) | root node of the private probe definition |
| `sagat.ai.mcp-refresh` | external MCP `tools/list` snapshot | root node of the private refresh definition |

- An ordinary activity or worker that names a `sagat.ai.*` activity is rejected at publication and
  registration (017 reserved-prefix precedent). The claim transaction counts the live leases of
  `sagat.ai.*` tasks of the namespace and tenant under their locks against `ai_model_concurrency` and
  `tenant_ai_model_concurrency` (no counter, so completion and expiry release capacity), and for a
  `sagat.ai.model` attempt inserts its reservation (`agent_call_attempts`) and raises the reserved
  counters of the agent and the namespace's daily row. If the reservation does not fit, the same
  transaction fails the derived step and the agent with `AI_BUDGET_EXHAUSTED` and grants no lease.
- Managed tasks carry `input_json = {}`; the request is obtained only through the resolve call.
  Generic `ReadBlob` is refused for these leases except the request and response blobs the resolve
  call names.

## New calls

```proto
service WorkerService {
  // Added in 1.7 (provisional). Only the current agent-runner machine that owns this lease may call.
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
  string auth_mode = 3;              // none | bearer | oauth-client-credentials
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
  `output_blob_ref` above 256 KiB). The server rejects a result whose tool names are not declared,
  whose turn differs, or that contains a value the attempt resolved; it never trusts the runner's
  view of budgets or classification.
- `sagat.ai.probe`: `{"models": ["claude-opus-5-5", ...], "outcome": "OK"}` limited to 256 ids.
- `sagat.ai.mcp`: `{"content": [...], "isError": bool, "truncated": bool}` with text and structured
  content only (D1317).
- `sagat.ai.mcp-refresh`: `{"protocolVersion": "...", "tools": [{name, title, description,
  inputSchema, outputSchemaSha256, annotations}]}`, bounded by the snapshot ceilings, as a blob when
  large; the server computes and stores the digests.
- Failures use `FailStep.error_code` from the closed `AiFailure` vocabulary in `errors.md`;
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
  attempts (an OAuth client-credentials access token for MCP is fetched per attempt, from the
  recorded token endpoint only). Provider adapters use explicit static credentials built from the
  resolved values; SDK default credential chains (environment, profile, instance metadata, workload
  identity) are disabled, and a test proves link-local and metadata addresses are unreachable from
  the runner (T025-016).
- No prompt, response, arguments, credentials, provider messages or tokens in logs, spans or
  metrics. Spans carry `sagat.*` ids, provider kind, model id and provider request id.
- The process has no platform database URL, OpenBao token, script signing key or container engine;
  its egress is default-deny except DNS, the engine, approved provider and MCP destinations, and
  telemetry (Helm network policy, T025-068).
