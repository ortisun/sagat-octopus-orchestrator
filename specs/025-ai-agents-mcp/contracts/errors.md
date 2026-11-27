# Typed errors (draft)

No new REST `Error.code` (closed enum: `BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`,
`CONFLICT`, `VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`). New
meaning goes into `details.reason` with a remediation row in `crates/api-http/src/error.rs` (Task
025-002), and into durable step and run failure codes. The durable failure codes of managed steps (the
`AiFailure` vocabulary below) are added to `sdks/error-catalog.json` as `worker`-origin entries (with
`grpc` status, `retryable` and a remediation, as the Iceberg codes of feature 035 were) and to
`sdks/SPEC.md` section 2, and every SDK's embedded copy is regenerated in the same change (Task
025-087, Decision 1318). Run-time classification refusals reuse the existing
`CLASSIFICATION_EGRESS_REFUSED`. No message, detail or code carries a prompt, response, argument,
credential, provider or executor message.

## API reasons

| Situation | `code` (HTTP) | `details.reason` | Retryable |
|---|---|---|---|
| Agent node while the platform requirement `ai-agents-v1` is not recorded in the instance registry; provider kind or tool kind not available (021, 022, 026 absent) | `VALIDATION_FAILED` (422) | `UNSUPPORTED_NODE` (existing) with `details.dependency` (`ai-agents-v1`, `RESOURCE_TYPES`, `STANDALONE_STEPS`, `WORKFLOW_STREAMS`) | false |
| Agent in a body or with `chainPolicy` | `VALIDATION_FAILED` (422) | `INVALID_SCOPE` (existing) | false |
| Model not in the provider revision | `VALIDATION_FAILED` (422) | `AI_MODEL_NOT_ALLOWED` | false |
| Cost budget without declared prices in that currency | `VALIDATION_FAILED` (422) | `UNPRICED_MODEL` | false |
| Context or projection references a level above the provider's | `VALIDATION_FAILED` (422) | `PROMPT_CLASSIFICATION_EXCEEDED` with `details.pointer` | false |
| An `mcp` tool's server `maxArgumentClassification` is below the agent's transcript level | `VALIDATION_FAILED` (422) | `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED` with `details.pointer` | false |
| A `result`, approval-view tool input or child input field is annotated below the transcript level; a workflow `outputSchema` field, a child node `outputSchema` field or a `child` tool result carrier field is annotated below the level it inherits from an agent output or a child output (Requirement 025-019) | `VALIDATION_FAILED` (422) | `CLASSIFICATION_LOOSENED` (existing, FR-016-004) with `details.pointer` | false |
| A tool input field annotated `secret` | `VALIDATION_FAILED` (422) | `SECRET_TOOL_INPUT` with `details.pointer` | false |
| A `write` tool without `approval`; an `operation` tool or a database `execute` tool with `approval: "none"`, or a database `execute` tool declared `read`; a `child` tool declared `read` without an approval policy; an `mcp` tool declared `read` without an approval policy whose pinned snapshot does not state `readOnlyHint: true` (or states `destructiveHint: true`) | `VALIDATION_FAILED` (422) | `WRITE_TOOL_APPROVAL_REQUIRED` | false |
| Exposure (or 026 public link) target version is unattended: an agent write tool acknowledged `approval: "none"` in it or in a child workflow it can start, at any depth | `VALIDATION_FAILED` (422) | `UNATTENDED_WRITE_TOOL` | false |
| A run start whose pinned provider or MCP server now has a current ceiling below an agent's transcript level, or whose pinned model left the current allowlist | `CONFLICT` (409) | `AI_DESTINATION_RESTRICTED` with `details.pointer` of the agent node | false |
| A tool-call approval decision by a decider to whom a field of the tool input would be withheld | `FORBIDDEN` (403) | `APPROVAL_INPUT_WITHHELD`; nothing applied, audited `ai.approval_input_withheld` | false |
| A provider saved with credential mode `aws-access-key` or `azure-entra-client-credentials` before design 047's broker issues access tokens (Gap 025-001) | `VALIDATION_FAILED` (422) | `AI_CREDENTIAL_MODE_UNAVAILABLE` | false |
| An ordinary activity, worker registration or version naming a `sagat.ai.` activity; recording `ai-agents-v1` while such a version exists | `VALIDATION_FAILED` (422) | `RESERVED_ACTIVITY_NAME` (with the versions listed for the operator command) | false |
| MCP server saved with `auth.mode` `oauth-client-credentials` before constitution 5.1.0 (its third declared set of Principle V, approved by the owner in Owner decision 2026-11-25-3) has merged and design 047's broker is built (Gap 025-001) | `VALIDATION_FAILED` (422) | `MCP_AUTH_MODE_UNAVAILABLE` | false |
| OAuth MCP server without recorded issuer and token endpoint (once the OAuth mode is enabled) | `VALIDATION_FAILED` (422) | `MCP_OAUTH_ENDPOINT_REQUIRED` | false |
| A provider or MCP server referencing a secret the saving administrator may not use (`secret:use` and the secret's access list) | `FORBIDDEN` (403) | `SECRET_NOT_USABLE` | false |
| An exposure whose target version's `inputSchema` is outside the design 043 subset | `VALIDATION_FAILED` (422) | `RUN_INPUT_SCHEMA_UNUSABLE` (existing, design 043) | false |
| A `pii` ceiling saved by an administrator without the namespace reveal grant | `FORBIDDEN` (403) | `REVEAL_GRANT_REQUIRED` | false |
| Agent context, tool-result projection or exposure projection reads an encoded (feature 031) location; a child tool input or the agent output would place model-written plaintext in an encoded location | `VALIDATION_FAILED` (422) | `ENCODING` with 031's `details.issues[].code` (`ENCODING_READ_BY_EXPRESSION`, `ENCODING_CARRIAGE_MISMATCH`, `ENCODING_NOT_PRODUCIBLE`); no 025-specific code | false |
| Schema not closed, over bounds, or outside the restriction of the design 043 subset that `agentClosedSchema` allows | `VALIDATION_FAILED` (422) | `SCHEMA_NOT_CLOSED` | false |
| Duplicate or invalid tool name | `VALIDATION_FAILED` (422) | `INVALID_TOOL` | false |
| MCP tool not in the pinned snapshot, or not closed | `VALIDATION_FAILED` (422) | `MCP_TOOL_NOT_ALLOWED` | false |
| Exposure projection level above `maxResultClassification` or `secret` | `VALIDATION_FAILED` (422) | `MCP_RESULT_CLASSIFICATION_EXCEEDED` | false |
| `maxPromptClassification` missing or `secret` | `VALIDATION_FAILED` (422) | `INVALID_DATA_POLICY` | false |
| Credential in settings or URL | `VALIDATION_FAILED` (422) | `CREDENTIAL_IN_SETTINGS` | false |
| Paused provider or MCP server at probe, refresh or admission | `CONFLICT` (409) | `AI_PROVIDER_PAUSED` / `MCP_SERVER_PAUSED` | false |
| Stale `expectedRevision` | `CONFLICT` (409) | `REVISION_CONFLICT` (existing) | false |
| Same request id or idempotency key, different content (a scoped-token mint replayed with the same key and content returns the token's metadata without the token instead) | `CONFLICT` (409) | `IDEMPOTENCY_KEY_REUSED` (existing) | false |
| Transcript reader lacks a right, or foreign/missing run or node | `NOT_FOUND` (404) | none (opaque), restricted denial audit | false |
| Quota or ceiling (providers, servers, exposures, tokens, `ai_runner_concurrency`, daily tokens or cost) | `QUOTA_EXCEEDED` (429) | `details.limit` | true |

## Durable failure codes (closed `AiFailure` vocabulary)

Recorded on the step (model call, tool call or the agent node) and in the payload-free journal. Each
code is a `worker`-origin entry of `sdks/error-catalog.json` (Task 025-087).

| Code | Where | Retryable by the platform | Meaning |
|---|---|---|---|
| `AI_BUDGET_EXHAUSTED` | agent | no | a reservation did not fit a budget (`details.budget`) |
| `AI_DEADLINE_EXCEEDED` | agent | no | agent wall time elapsed |
| `AI_OUTPUT_INVALID` | agent | no | no valid structured answer within repair turns |
| `AI_OUTPUT_TRUNCATED` | agent | no | `max_tokens` without a complete answer or tool request |
| `AI_MODEL_REFUSED` | agent | no | provider refusal stop reason |
| `AI_TOOL_DENIED` | agent | no | approval rejected or expired with `fail` |
| `CLASSIFICATION_EGRESS_REFUSED` (existing catalog code) | model or MCP call | no | a value above the provider's `maxPromptClassification` or the server's `maxArgumentClassification` (the stricter of the pinned and the current revision's), or one that does not validate against its carrier, would be sent |
| `CLASSIFICATION_LOOSENED` (existing) | child node or `child` tool call | no | a child under an active-version policy whose version records output levels stricter than the parent's carrier; no child created; the model receives `TOOL_FAILED` with this code |
| `UNATTENDED_WRITE_TOOL` | child node or `child` tool call | no | under a run whose root MCP or a 026 public link admitted, the child version is unattended; no child created; the model receives `TOOL_FAILED` with this code |
| `AI_REQUEST_INVALID` | model call | no | provider `400` (other than context size) |
| `AI_CONTEXT_TOO_LARGE` | model call | no | provider `413` or context-length rejection |
| `AI_PROVIDER_AUTH` | model call | no | provider `401`/`403` |
| `AI_PROVIDER_REJECTED` | model call | no | provider spend limit or billing rejection |
| `AI_RATE_LIMITED` | model call | yes, until the retry policy ends | provider `429`; reservation released |
| `AI_PROVIDER_UNAVAILABLE` | model call | yes | `500`, `504`, `529`, or a connection failure before the request was sent; reservation released |
| `AI_RESPONSE_INCOMPLETE` | model call | yes | the stream broke after `200` or the response was lost; reservation kept as charged |
| `AI_CREDENTIAL_UNAVAILABLE` | model or MCP call | yes, bounded | OpenBao unavailable; no fallback |
| `AI_DESTINATION_DENIED` | model or MCP call, probe, refresh | no | egress guard refusal, or any `3xx` answer (no redirect is followed) |
| `AI_CAPABILITY_UNAVAILABLE` | model or MCP call | no | provider or server paused (including by revocation of the reveal grant behind a `pii` ceiling, or the administrator who set it leaving), deleted, the pinned model no longer in the current allowlist, or rights revoked at resolution |
| `AI_RESPONSE_LIMIT` | model or MCP call | no | response bytes or stream events over the limit |
| `AI_TOOL_CHANGED` | MCP call | no | pinned digests differ from the server's current definition |
| `AI_MCP_AUTH` / `AI_MCP_PROTOCOL_ERROR` / `AI_MCP_UNAVAILABLE` | MCP call | no / per JSON-RPC code / yes | see mcp-client.md; `AI_MCP_AUTH` also, for OAuth servers once enabled, when a refresh reported a changed issuer or token endpoint |
| `MCP_UNSUPPORTED_RESULT` | MCP call (returned to the model) | no | `input_required` or task result |
| `OUTCOME_UNKNOWN` (existing, reserved) | MCP call or other `write` tool | no; reconcile or manual review | effect may have happened |
| `SECRET_IN_OUTPUT` (existing) | any tool or model call | no | output contained a value the attempt received, verbatim, JSON-escaped, base64 or hexadecimal (values of at least 8 bytes; Gap 025-004) |
| `CHILD_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE` (existing, design 043) | `child` tool call | no | the child's input was refused by the run-input gate; no child was created; the model receives `TOOL_FAILED` with this code and the value-free rule and field |
| `RUN_DATA_LIMIT` (existing) | agent | no | transcript or retained bytes would exceed the run's bound |
| `WORKER_ERROR` (existing) | any | per report | free text recorded as this code, never as a reserved one |

## Tool errors returned to the model

Fixed reasons, never executor text: `UNKNOWN_TOOL` (also the fixed marker that names an undeclared
request on every run-read surface, in place of the name the model wrote), `INVALID_INPUT` (with the 043 validator's
value-free rule and field only), `LIMIT_REACHED`, `APPROVAL_REJECTED`, `APPROVAL_EXPIRED`,
`TOOL_FAILED` (with the failure code only, and for `CHILD_INPUT_INVALID` the value-free rule and
field), `MCP_UNSUPPORTED_RESULT`, `AI_TOOL_CHANGED` and `MCP_TOOL_ERROR` (see
`model-turn.schema.json` `toolResults`). `MCP_TOOL_ERROR` is an external server's `isError: true`
result: its text, bounded to 4 KiB, secret-scanned and marked untrusted, has no carrier and counts as
`pii`, so it is sent as the result's content only when the agent's transcript level is already `pii`;
otherwise the reason goes alone (Decision 025-008). This is the one place a third party's text reaches
the model as an error; no platform or executor message ever does.

## MCP server tool errors

Returned as tool execution results (`isError: true`) with a fixed `reason` in `structuredContent`
(see mcp-server.md): `IDEMPOTENCY_KEY_REUSED`, `RESULT_NOT_DISCLOSABLE`, `UNATTENDED_WRITE_TOOL`,
`RUN_INPUT_SCHEMA_UNUSABLE`, `QUOTA_EXCEEDED` and `RATE_LIMITED` (with `retryAfterMs` in `_meta`), and
the target run's durable failure code. A caller who may not call an exposure (missing `run:start`,
missing result-level rights, out of scope, foreign, unknown or disabled) always gets JSON-RPC `-32602`
"Unknown tool". Arguments refused by the design 043 run-input gate (`RUN_INPUT_INVALID`) get JSON-RPC
`-32602` with `data: {rule, field}` and never a value; the gate's transient refusals (no permit, the
backstop, an interrupted check) get a retryable JSON-RPC `-32603` with a trace id. A batch is
JSON-RPC `-32600`. A caller whose credential or rights fail the recheck before a result is returned
gets `-32602` "Unknown tool" and no result. Protocol errors use
JSON-RPC codes only (`-32600`, `-32601`, `-32602`, `-32603`, `-32020`, `-32022`).

`tests/contract/ai-agents-mcp/errors.rs` (Task 025-002) reads these tables: every reason has a
remediation row and appears in the OpenAPI `Error.details` description; every `AiFailure` code has a
parse round trip and a `worker`-origin entry in `sdks/error-catalog.json` (Task 025-087).
