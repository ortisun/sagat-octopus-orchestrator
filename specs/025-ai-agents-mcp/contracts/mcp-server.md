# MCP server surface (draft)

Implemented by `crates/api-mcp`, mounted by `apps/control-plane` on the same TLS listener as the REST
API. It never runs user code and never calls a model: a `tools/call` admits durable work through the
existing admission transaction and reads committed results. ADR-0060 records the trust boundary.

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
4. Failures: missing, malformed, expired, revoked or wrong-audience token → `401` with
   `WWW-Authenticate: Bearer resource_metadata="<PRM URL>", scope="mcp:call"`; valid token lacking
   `mcp:call` → `403` with `error="insufficient_scope"`. Responses are identical whatever the reason,
   so they are not an existence oracle. The received token is never forwarded to a target, worker
   or provider.

## Methods

| Method | Behaviour |
|---|---|
| `server/discover` | Above. No authorization needed beyond a valid token; reveals no tools. |
| `tools/list` | Exposures that are `ENABLED`, in the token's scope and callable by the caller now (`mcp:call`, `run:start` on the target, rights of `maxResultClassification`, and, when feature 029 is enabled, visibility of the exposure's target decided by the 029 access evaluator `sagat_domain::access::evaluate`), ordered by tool name, paginated (50 per page), `ttlMs: 60000`, `cacheScope: "private"`. Each tool: `name`, `title`, `description`, `inputSchema` (from the target), `outputSchema` (the exposure's `resultSchema`), `annotations` (`readOnlyHint` false unless declared, `idempotentHint` false), `_meta.sagat-octopus-orchestrator.io/exposureRevision`. |
| `tools/call` | Validate arguments against `inputSchema` (`-32602` with a bounded pointer list, no values); authorize as for `tools/list` (an exposure the caller cannot call is answered exactly like an unknown tool, `-32602`, with an `mcp.call_denied` audit); refuse a target version with an unattended write tool (`UNATTENDED_WRITE_TOOL`, answered as a tool error); consult the receipt for `_meta.sagat-octopus-orchestrator.io/idempotencyKey` (1–128 printable ASCII; same key and argument digest → same run; changed arguments → tool error `IDEMPOTENCY_KEY_REUSED`); admit a run of the target pinned by the exposure's version policy, with the receipt and the `mcp.tool_called` audit in the admission transaction; wait up to `waitMs` (default 10 s, ceiling 30 s; feature-026 `executeRun` semantics, FR-026-007 and FR-026-009, when available). Terminal and succeeded → `structuredContent` is the validated projection and one `text` block with its canonical JSON; audit `mcp.result_disclosed`. Terminal and failed → `isError: true` with the run's failure code only. Not terminal → a task when the client declared `io.modelcontextprotocol/tasks`, otherwise `structuredContent: {"status": "RUNNING", "runId": "..."}`. |
| `tasks/get` | Task handle bound to tenant, namespace, caller and run; every call re-checks the token, `mcp:call`, the token scope, the exposure's state and the result-level rights (a handle is a name, not a capability; a failed check answers like an unknown task); status `working`, `completed` (with the projected result, audited), `failed` (failure code), `cancelled`. Approval waits stay `working`; `input_required` is never used. |
| `tasks/cancel` | Same re-checks as `tasks/get`, plus `run:operate`; requests run cancellation through the existing operation path; cancellation is not rollback. |

## Result disclosure

- The projection is evaluated by the existing bounded evaluator over the committed run result
  (`result`) with no secret capability; its output must validate against the closed `resultSchema`
  (≤ 256 KiB serialized) before it is returned. Validation failure is a tool error
  `RESULT_NOT_DISCLOSABLE` without excerpts.
- Every field's effective classification (the workflow `outputSchema` carrier, ADR-0033) must be ≤
  `maxResultClassification`; `secret` never. This is checked when the exposure is saved and again
  at call time against the pinned version; a run whose result does not validate against its
  carrier is not disclosed.
- A caller without the rights of the declared level is refused before admission (US06 scenario 4).
- Arguments and results never enter logs, spans, metrics or audit reasons; audits record exposure,
  revision, run id, outcome and digests.

## Error mapping

| Situation | MCP response |
|---|---|
| Unknown, disabled, out-of-scope or foreign exposure | JSON-RPC `-32602` "Unknown tool" (identical for all four) |
| Arguments fail the input schema | `-32602` with bounded pointers |
| Caller lacks `run:start` or the result-level rights | JSON-RPC `-32602` "Unknown tool", identical to an absent tool; audited `mcp.call_denied` |
| Quota or rate limit (`QUOTA_EXCEEDED`, `RATE_LIMITED`) | tool error with the reason and `retryAfterMs` in `_meta` |
| Platform unavailable | JSON-RPC `-32603` with a trace id only |
| Target run failed | tool error with the failure code |

## Limits

Per token 10 calls/s with burst 20 (operator-adjustable), the namespace `api_requests_per_second`
bucket also applies; at most 16 concurrent waiting calls per namespace by default (64 ceiling); a
call beyond them is answered immediately as a task or run handle instead of waiting.
