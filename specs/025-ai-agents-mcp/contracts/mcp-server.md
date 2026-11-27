# MCP server surface (draft)

Implemented by `crates/api-mcp`, mounted by `apps/control-plane` on the same TLS listener as the REST
API. It never runs user code and never calls a model: a `tools/call` admits durable work through the
existing admission transaction and reads committed results. ADR-0060 records the trust boundary.
The endpoint answers `404` for every namespace until the operator records the platform requirement
`mcp-server-v1` in the instance registry (Architecture decision 0082, Decision 025-002).

## Endpoint and discovery

- Endpoint: `POST /v1/namespaces/{namespaceId}/mcp` (Streamable HTTP). The canonical resource URI is
  `<SAGAT_MCP_PUBLIC_BASE>/v1/namespaces/{namespaceId}/mcp`, configured by the operator; a token's
  audience must equal it (RFC 8707). One URI per namespace keeps audience binding and tool lists
  namespace-scoped.
- Protected Resource Metadata (RFC 9728) at
  `GET /.well-known/oauth-protected-resource/v1/namespaces/{namespaceId}/mcp`: `resource`,
  `authorization_servers` (the configured issuer), `scopes_supported` (`mcp:call`),
  `bearer_methods_supported: ["header"]`. Unauthenticated, rate-limited, payload-free, and answered
  identically for every well-formed namespace id (the `resource` value is built from the requested
  id), so it reveals nothing about which namespaces exist; a malformed id is `404` for everyone.
- `GET` on the endpoint returns `405`; there is no server-to-client stream except the response
  stream of a request. `subscriptions/listen` is not offered (`listChanged: false`).
- Request bodies are limited to 256 KiB by default (1 MiB ceiling); larger bodies are `413` before
  parsing.
- A JSON-RPC batch (a body whose top level is an array) is refused with `-32600` (Invalid Request)
  before any member is read or authorized, so every call passes the per-token rate limit on its own.

## Versions

| Client revision | Behaviour |
|---|---|
| `2026-07-28` | Stateless. Each request carries `_meta` `io.modelcontextprotocol/protocolVersion` and client capabilities; `Mcp-Method`/`Mcp-Name` headers must match the body (`HeaderMismatchError` `-32020` otherwise); `server/discover` returns supported versions `["2026-07-28", "2025-11-25"]`, capabilities `{tools: {listChanged: false}, extensions: {"io.modelcontextprotocol/tasks": {}}}` and `serverInfo`. Results carry `resultType`. |
| `2025-11-25` | `initialize` is answered with the same capabilities and no `Mcp-Session-Id` (sessions are optional in that revision); `notifications/initialized` is accepted and ignored; every request is authorized independently. |
| other | `UnsupportedProtocolVersionError` (`-32022`) listing supported versions. |

Not offered: sampling, roots, logging, resources, prompts, completion, elicitation. A request for an
unknown method is JSON-RPC `-32601`.

## Authentication

1. `Authorization: Bearer <token>` only. A token in a query string, a form field or `_meta` is
   rejected (`401`) and the attempt counted in `mcp_token_auth_failures_total{reason="query_token"}`.
2. OAuth access token: JWT from the configured issuer, verified with its JWKS like REST tokens, whose
   `aud` is exactly the namespace's canonical MCP URI (a single string, or an array with that one
   element; any other audience in the array is refused), tenant claim resolving to the namespace's
   tenant, scope containing `mcp:call`. Platform REST tokens (audience `sagat-octopus-orchestrator`)
   are not accepted here, and REST refuses any token whose `aud` contains an MCP URI (T025-074), so
   one token is never replayable across the two surfaces.
3. Scoped MCP token: opaque `smcp_` + 256 random bits, looked up by SHA-256 (no key is involved; 032's
   key inventory records this), bound to tenant,
   namespace, exposures and expiry; the minter's current membership and permissions are re-read on
   every request, and the effective rights are their intersection with the scope.
4. Caller: the credential identity, used for receipts, task handles, rate limits and the per-caller
   agent-run limit: for a scoped token its token identifier (never the minter, so two tokens of one
   minter never share a receipt); for an OAuth access token its issuer and subject.
5. Failures: missing, malformed, expired, revoked or wrong-audience token → `401` with
   `WWW-Authenticate: Bearer resource_metadata="<PRM URL>", scope="mcp:call"`; valid token lacking
   `mcp:call` → `403` with `error="insufficient_scope"`. Responses are identical whatever the reason,
   so they are not an existence oracle. The received token is never forwarded to a target, worker
   or provider.

## Methods

| Method | Behaviour |
|---|---|
| `server/discover` | Above. No authorization needed beyond a valid token; reveals no tools. |
| `tools/list` | Exposures that are `ENABLED`, in the token's scope and callable by the caller now (`mcp:call`, `run:start` on the target, rights of `maxResultClassification`, and, when feature 029 is enabled, visibility of the exposure's target decided by the 029 access evaluator `sagat_domain::access::evaluate`), ordered by tool name, paginated (50 per page), `ttlMs: 60000`, `cacheScope: "private"`. Each tool: `name`, `title`, `description`, `inputSchema` (from the target), `outputSchema` (the exposure's `resultSchema`), `annotations` (`readOnlyHint` false unless declared, `idempotentHint` false), `_meta.sagat-octopus-orchestrator.io/exposureRevision`. |
| `tools/call` | Check only the JSON-RPC shape (`arguments` is an object within the request bound); authorize as for `tools/list` (an exposure the caller cannot call is answered exactly like an unknown tool, `-32602`, with an `mcp.call_denied` audit), always before any receipt is read; refuse an unattended target version (an agent write tool acknowledged `approval: "none"` in it or in any child workflow it can start, `UNATTENDED_WRITE_TOOL`, answered as a tool error); consult the receipt for `_meta.sagat-octopus-orchestrator.io/idempotencyKey` (1–128 printable ASCII), keyed by the caller (the credential identity) and matched on the exposure as well as the key: same caller, exposure, key and argument digest → same run, whose result is disclosed only through this call's exposure and its current revision's projection; changed arguments, or the same key on another exposure → tool error `IDEMPOTENCY_KEY_REUSED`; count the caller's live runs whose target version can contain agents against `mcp_agent_runs_per_caller` (tool error `QUOTA_EXCEEDED` beyond it); pass the arguments, as the run input, through the run-input gate of design 043 (a new row of its table: a `CheckedRunInput` against the pinned target version's `inputSchema`, admitted through the existing `create`; Decision 025-006); admit a run of the target pinned by the exposure's version policy, marked `remote_root` (so every child admission under it refuses an unattended child version), charged to the exposure's `maxDailyTokens` when set, with the receipt and the `mcp.tool_called` audit in the admission transaction; wait up to `waitMs` (default 10 s, ceiling 30 s; feature-026 `executeRun` semantics, FR-026-007 and FR-026-009, when available). Before any result leaves, re-check the credential (not revoked, not expired, the minter's or subject's current membership), `mcp:call`, the token scope, the exposure's state and the result-level rights; a failed check returns no result and is answered as an unknown tool with an `mcp.call_denied` audit (the run continues). Terminal and succeeded → `structuredContent` is the validated projection and one `text` block with its canonical JSON; audit `mcp.result_disclosed`. Terminal and failed → `isError: true` with the run's failure code only. Not terminal → a task when the client declared `io.modelcontextprotocol/tasks`, otherwise `structuredContent: {"status": "RUNNING", "runId": "..."}`. |
| `tasks/get` | Task handle bound to tenant, namespace, caller (the credential identity) and run; every call re-checks the token, `mcp:call`, the token scope, the exposure's state and the result-level rights (a handle is a name, not a capability; a failed check answers like an unknown task); status `working`, `completed` (with the projected result, audited), `failed` (failure code), `cancelled`. Approval waits stay `working`; `input_required` is never used. |
| `tasks/cancel` | Same re-checks as `tasks/get`, plus `run:operate`; requests run cancellation through the existing operation path; cancellation is not rollback. |

## Run-input gate

`tools/call` is a row of design 043's run-input gate, like every other way a run is created; this
adapter has no validator of its own.

- The exposure's `inputSchema` is the target version's, which lies inside the 043 subset
  (`specs/043-run-input-validation/contracts/input-schema-subset.md`); an exposure whose target version
  is outside it cannot be saved (`RUN_INPUT_SCHEMA_UNUSABLE`), and an `ACTIVE`-policy exposure whose
  active version is outside it answers the tool error below.
- `RUN_INPUT_INVALID` → JSON-RPC `-32602` with `data: {"rule": ..., "field": ...}`, the gate's
  value-free rule and field; never a value or an excerpt.
- `RUN_INPUT_SCHEMA_UNUSABLE` → a tool error with that reason.
- A transient refusal (no permit within 5 s, the backstop, an interrupted check) → a retryable
  JSON-RPC `-32603` with a trace id; nothing was judged and nothing was admitted.
- `report` mode: the gate runs every check, counts and audits a final refusal and creates the run
  anyway (design 043, FR-043-013), so the MCP caller sees an ordinary call; the operator sees the
  refusal in the gate's metric and audit.
- The receipt is consulted after authorization and before the gate, so a retry by the same caller on
  the same exposure with the same key and arguments returns the run of the first call without a second
  check.

## Result disclosure

- The projection is evaluated by the existing bounded evaluator over the committed run result
  (`result`) with no secret capability; its output must validate against the closed `resultSchema`
  (≤ 256 KiB serialized) before it is returned. Validation failure is a tool error
  `RESULT_NOT_DISCLOSABLE` without excerpts.
- Every projected field's effective classification must be ≤ `maxResultClassification`; `secret`
  never. The effective classification is the one the target version recorded at publication (its
  `effective_output_levels`): the stricter of the workflow `outputSchema` annotation (ADR-0033) and the
  level the field inherits from the agent outputs and child outputs `end.outputExpr` draws it from
  (spec Requirement 025-019, the `written_level` rule), so a model-written value annotated `public`
  below its agent's transcript level never passes. This is checked when the exposure is saved and again
  at call time against the version the call resolves; a run whose result does not validate against its
  carrier is not disclosed.
- A caller without the rights of the declared level is refused before admission (US06 scenario 4).
- Arguments and results never enter logs, spans, metrics or audit reasons; audits record exposure,
  revision, run id, outcome and digests.

## Error mapping

| Situation | MCP response |
|---|---|
| JSON-RPC batch (array body) | JSON-RPC `-32600`, nothing read or admitted |
| Unknown, disabled, out-of-scope or foreign exposure | JSON-RPC `-32602` "Unknown tool" (identical for all four) |
| Credential or rights lost during the bounded wait | JSON-RPC `-32602` "Unknown tool", no result; audited `mcp.call_denied` |
| Same idempotency key on another exposure, or other arguments | tool error `IDEMPOTENCY_KEY_REUSED` |
| Arguments refused by the run-input gate (`RUN_INPUT_INVALID`) | `-32602` with the value-free rule and field |
| Target version's schema unusable (`RUN_INPUT_SCHEMA_UNUSABLE`) | tool error with the reason |
| Run-input gate busy (no permit, backstop, interrupted) | JSON-RPC `-32603`, retryable, trace id only |
| Target version has an unattended write tool | tool error `UNATTENDED_WRITE_TOOL` |
| Caller lacks `run:start` or the result-level rights | JSON-RPC `-32602` "Unknown tool", identical to an absent tool; audited `mcp.call_denied` |
| Quota or rate limit (`QUOTA_EXCEEDED`, `RATE_LIMITED`) | tool error with the reason and `retryAfterMs` in `_meta` |
| Platform unavailable | JSON-RPC `-32603` with a trace id only |
| Target run failed | tool error with the failure code |

## Limits

Per token 10 calls/s with burst 20 (operator-adjustable), the namespace `api_requests_per_second`
bucket also applies; at most 16 concurrent waiting calls per namespace by default (64 ceiling); a
call beyond them is answered immediately as a task or run handle instead of waiting. A caller may hold
at most `mcp_agent_runs_per_caller` live runs whose target version can contain agents (default 4,
ceiling 64; tool error `QUOTA_EXCEEDED` beyond it), and an exposure's optional `maxDailyTokens` bounds
the model tokens all runs it admitted, and their descendants, spend per UTC day, so one remote client
cannot use up the namespace's daily AI budget.
