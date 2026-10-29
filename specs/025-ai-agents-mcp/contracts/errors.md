# Typed errors (draft)

No new `Error.code` (closed enum: `BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`,
`CONFLICT`, `VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`) and no
new worker trailer code (`sdks/error-catalog.json` is unchanged). New meaning goes into
`details.reason` with a remediation row in `crates/api-http/src/error.rs` (T025-002), and into
durable step/run failure codes, an open set like `EXPRESSION_ERROR`. No message, detail or code
carries a prompt, response, argument, credential, provider or executor message.

## API reasons

| Situation | `code` (HTTP) | `details.reason` | Retryable |
|---|---|---|---|
| Agent capability, provider kind or tool kind not enabled (021, 022, 026 absent) | `VALIDATION_FAILED` (422) | `UNSUPPORTED_NODE` (existing) with `details.dependency` (`RESOURCE_TYPES`, `STANDALONE_STEPS`, `WORKFLOW_STREAMS`) | false |
| Agent in a body or with `chainPolicy` | `VALIDATION_FAILED` (422) | `INVALID_SCOPE` (existing) | false |
| Model not in the provider revision | `VALIDATION_FAILED` (422) | `AI_MODEL_NOT_ALLOWED` | false |
| Cost budget without declared prices in that currency | `VALIDATION_FAILED` (422) | `UNPRICED_MODEL` | false |
| Context or projection references a level above the provider's | `VALIDATION_FAILED` (422) | `PROMPT_CLASSIFICATION_EXCEEDED` with `details.pointer` | false |
| An `mcp` tool's server `maxArgumentClassification` is below the agent's transcript level | `VALIDATION_FAILED` (422) | `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED` with `details.pointer` | false |
| A `result`, approval-view tool input or child input field is annotated below the transcript level | `VALIDATION_FAILED` (422) | `CLASSIFICATION_LOOSENED` (existing, FR-016-004) with `details.pointer` | false |
| A `write` tool without `approval`, or an `operation` tool with `approval: "none"` | `VALIDATION_FAILED` (422) | `WRITE_TOOL_APPROVAL_REQUIRED` | false |
| Exposure (or 026 public link) target version has an unattended write tool | `VALIDATION_FAILED` (422) | `UNATTENDED_WRITE_TOOL` | false |
| OAuth MCP server without recorded issuer and token endpoint | `VALIDATION_FAILED` (422) | `MCP_OAUTH_ENDPOINT_REQUIRED` | false |
| A `pii` ceiling saved by an administrator without the namespace reveal grant | `FORBIDDEN` (403) | `REVEAL_GRANT_REQUIRED` | false |
| Agent context, tool-result projection or exposure projection reads an encoded (feature 031) location; a child tool input or the agent output would place model-written plaintext in an encoded location | `VALIDATION_FAILED` (422) | `ENCODING` with 031's `details.issues[].code` (`ENCODING_READ_BY_EXPRESSION`, `ENCODING_CARRIAGE_MISMATCH`, `ENCODING_NOT_PRODUCIBLE`); no 025-specific code | false |
| Schema not closed or over bounds | `VALIDATION_FAILED` (422) | `SCHEMA_NOT_CLOSED` | false |
| Duplicate or invalid tool name | `VALIDATION_FAILED` (422) | `INVALID_TOOL` | false |
| MCP tool not in the pinned snapshot, or not closed | `VALIDATION_FAILED` (422) | `MCP_TOOL_NOT_ALLOWED` | false |
| Exposure projection level above `maxResultClassification` or `secret` | `VALIDATION_FAILED` (422) | `MCP_RESULT_CLASSIFICATION_EXCEEDED` | false |
| `maxPromptClassification` missing or `secret` | `VALIDATION_FAILED` (422) | `INVALID_DATA_POLICY` | false |
| Credential in settings or URL | `VALIDATION_FAILED` (422) | `CREDENTIAL_IN_SETTINGS` | false |
| Paused provider or MCP server at probe, refresh or admission | `CONFLICT` (409) | `AI_PROVIDER_PAUSED` / `MCP_SERVER_PAUSED` | false |
| Stale `expectedRevision` | `CONFLICT` (409) | `REVISION_CONFLICT` (existing) | false |
| Same request id or idempotency key, different content | `CONFLICT` (409) | `IDEMPOTENCY_KEY_REUSED` (existing) | false |
| Transcript reader lacks a right, or foreign/missing run or node | `NOT_FOUND` (404) | none (opaque), restricted denial audit | false |
| Quota or ceiling (providers, servers, exposures, tokens, concurrency, daily tokens or cost) | `QUOTA_EXCEEDED` (429) | `details.limit` | true |

## Durable failure codes (closed `AiFailure` vocabulary)

Recorded on the step (model call, tool call or the agent node) and in the payload-free journal.

| Code | Where | Retryable by the platform | Meaning |
|---|---|---|---|
| `AI_BUDGET_EXHAUSTED` | agent | no | a reservation did not fit a budget (`details.budget`) |
| `AI_DEADLINE_EXCEEDED` | agent | no | agent wall time elapsed |
| `AI_OUTPUT_INVALID` | agent | no | no valid structured answer within repair turns |
| `AI_OUTPUT_TRUNCATED` | agent | no | `max_tokens` without a complete answer or tool request |
| `AI_MODEL_REFUSED` | agent | no | provider refusal stop reason |
| `AI_TOOL_DENIED` | agent | no | approval rejected or expired with `fail` |
| `AI_PROMPT_CLASSIFICATION` | model call | no | a value above the provider's level would be sent |
| `AI_REQUEST_INVALID` | model call | no | provider `400` (other than context size) |
| `AI_CONTEXT_TOO_LARGE` | model call | no | provider `413` or context-length rejection |
| `AI_PROVIDER_AUTH` | model call | no | provider `401`/`403` |
| `AI_PROVIDER_REJECTED` | model call | no | provider spend limit or billing rejection |
| `AI_RATE_LIMITED` | model call | yes, until the retry policy ends | provider `429`; reservation released |
| `AI_PROVIDER_UNAVAILABLE` | model call | yes | `500`, `504`, `529`, or a connection failure before the request was sent; reservation released |
| `AI_RESPONSE_INCOMPLETE` | model call | yes | the stream broke after `200` or the response was lost; reservation kept as charged |
| `AI_CREDENTIAL_UNAVAILABLE` | model or MCP call | yes, bounded | OpenBao unavailable; no fallback |
| `AI_DESTINATION_DENIED` | model or MCP call | no | egress guard refusal |
| `AI_CAPABILITY_UNAVAILABLE` | model or MCP call | no | provider or server paused (including by revocation of the reveal grant behind a `pii` ceiling), deleted, or rights revoked at resolution |
| `AI_RESPONSE_LIMIT` | model or MCP call | no | response bytes or stream events over the limit |
| `AI_TOOL_CHANGED` | MCP call | no | pinned digests differ from the server's current definition |
| `AI_MCP_AUTH` / `AI_MCP_PROTOCOL_ERROR` / `AI_MCP_UNAVAILABLE` | MCP call | no / per JSON-RPC code / yes | see mcp-client.md; `AI_MCP_AUTH` also when a refresh reported a changed issuer or token endpoint |
| `MCP_UNSUPPORTED_RESULT` | MCP call (returned to the model) | no | `input_required` or task result |
| `OUTCOME_UNKNOWN` (existing, reserved) | MCP call or other `write` tool | no; reconcile or manual review | effect may have happened |
| `SECRET_IN_OUTPUT` (existing) | any tool or model call | no | output contained a value the attempt received |
| `RUN_DATA_LIMIT` (existing) | agent | no | transcript or retained bytes would exceed the run's bound |
| `WORKER_ERROR` (existing) | any | per report | free text recorded as this code, never as a reserved one |

## Tool errors returned to the model

Fixed reasons, never executor text: `UNKNOWN_TOOL`, `INVALID_INPUT`, `LIMIT_REACHED`,
`APPROVAL_REJECTED`, `APPROVAL_EXPIRED`, `TOOL_FAILED` (with the failure code only),
`MCP_UNSUPPORTED_RESULT`, `AI_TOOL_CHANGED` (see `model-turn.schema.json` `toolResults`).

## MCP server tool errors

Returned as tool execution results (`isError: true`) with a fixed `reason` in `structuredContent`
(see mcp-server.md): `IDEMPOTENCY_KEY_REUSED`, `RESULT_NOT_DISCLOSABLE`,
`QUOTA_EXCEEDED` and `RATE_LIMITED` (with `retryAfterMs` in `_meta`), and the target run's durable
failure code. A caller who may not call an exposure (missing `run:start`, missing result-level rights,
out of scope, foreign, unknown or disabled) always gets JSON-RPC `-32602` "Unknown tool". Protocol
errors use JSON-RPC codes only (`-32601`, `-32602`, `-32603`, `-32020`,
`-32022`).

`tests/contract/ai-agents-mcp/errors.rs` (T025-002) reads these tables: every reason has a
remediation row and appears in the OpenAPI `Error.details` description; every `AiFailure` code has a
parse round trip.
