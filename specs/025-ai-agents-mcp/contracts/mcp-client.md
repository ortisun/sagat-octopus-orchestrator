# MCP client behaviour (draft)

The client lives in `crates/infra-ai` (`mcp_client`) and runs only inside `apps/agent-runner`, for
`sagat.ai.mcp` and `sagat.ai.mcp-refresh` tasks. The control plane and scheduler never open a
connection to an external MCP server. ADR-0060 records the trust boundary.

## Registration and refresh

- An external server resource names an HTTPS URL (no userinfo, no query string carrying
  credentials), a protocol preference (`2026-07-28` first, `2025-11-25` fallback), a required
  `maxArgumentClassification` (`public`, `internal` or `pii`: the most sensitive data the model may send
  to this third party as arguments), an authentication mode and limits. Saving makes no network call.
- Refresh is a durable private run (origin `MCP_TOOL_REFRESH`): the runner calls `server/discover`
  (2026-07-28) or `initialize` (2025-11-25, then discards any session id after the refresh), then
  `tools/list` with pagination, bounded to 1,024 tools, 16 KiB per description and 64 KiB per input
  schema, with no redirect followed. The server stores an immutable snapshot with SHA-256 digests of each canonicalized
  description, input schema and output schema, and moves the resource's current snapshot pointer. The
  refresh is a private run created by `start_private`, so it passes row 13 of the design 043 run-input
  gate. For OAuth servers (once enabled, Gap 025-001) the refresh also reads Protected Resource Metadata and authorization server metadata
  and stores the discovered issuer and token endpoint on the snapshot; if they differ from the recorded
  ones it writes `mcp.oauth_metadata_changed` and marks the server so that `ResolveMcpCall` refuses
  (`AI_MCP_AUTH`) until an `ai:manage` holder saves a revision confirming the new values.
- A tool whose input schema is outside the closed vocabulary (`agentClosedSchema`, the restriction of
  the design 043 subset plus closure that the 043 validator compiles) is stored with
  `closedInput: false` and cannot be allowed in an agent; a tool with an invalid `x-mcp-header`
  annotation is excluded, as the specification requires.

## Use in an agent

- An `mcp` tool names the server resource and one snapshot tool. Publication pins the server's
  current revision and snapshot for that node by recording both in the published version
  (`version_agent_pins`); admission copies that record into `run_resource_bindings` and
  `run_mcp_snapshot_pins`, so a refresh after publication changes neither the descriptions and schemas
  the author reviewed nor the digests the drift check compares against. Publication copies the tool's
  input schema into the declaration it validates against, and requires `effect` (default `write`;
  a `write` tool needs `approval` or `approval: "none"`; `effect: "read"` without an approval policy is
  accepted only when the pinned snapshot's annotations state `readOnlyHint: true` and not
  `destructiveHint: true`, and the untrusted annotations never loosen the rule further). At admission
  and at every resolution the transcript level is checked again against the stricter of the pinned and
  the current server revision's `maxArgumentClassification`, so a ceiling lowered after publication
  applies before the next call. Publication refuses the tool when the server
  revision's `maxArgumentClassification` is below the agent's transcript level
  (`MCP_ARGUMENT_CLASSIFICATION_EXCEEDED`): everything that can enter the transcript can be copied
  into arguments by the model. MCP-argument canaries (T025-042) prove nothing above the ceiling is sent.
- Before each call, the runner fetches the tool's current definition (a `tools/list` page lookup by
  name, cached only within the attempt) and compares its digests with the pinned ones. Any
  difference, or a missing tool, fails the call with `AI_TOOL_CHANGED` without sending it; the model
  receives the fixed reason and an `mcp.tool_changed` audit is written.
- Arguments are the model's tool input validated by the server against the pinned closed schema.
  Requests carry `_meta` `traceparent` and, when the server's tool declares support through a
  documented convention, the platform idempotency key; otherwise the call is at-least-once with no
  deduplication and the documentation says so.

## Authentication

| Mode | Behaviour |
|---|---|
| `none` | No credential; allowed only when the operator enables it for the namespace. |
| `bearer` | Static token from an OpenBao-backed secret, sent only in `Authorization`, never in a URL. The secret is a declared secret of the task definition (the pinned server revision), delivered under Principle V's first declared set (agent-runner-protocol.md, "Credential delivery"). |
| `oauth-client-credentials` | **Waits for constitution 5.1.0 (its third declared set of Principle V, approved by the owner in Owner decision 2026-11-25-3) to merge and for design 047's broker (Gap 025-001); refused at save with `MCP_AUTH_MODE_UNAVAILABLE` until then.** OAuth MCP servers will reach access tokens only through design 047's token broker; the text that follows is the intended behaviour, to be reconciled with that broker. Client id plus a client secret from OpenBao. The issuer and the HTTPS token endpoint are recorded in the server revision, confirmed by the saving `ai:manage` holder (a refresh proposes discovered values; it never replaces them). The client secret is sent only to the recorded token endpoint; the runner performs no discovery at call time. The token request carries `resource` (RFC 8707) and the declared scopes. Tokens are fetched per attempt and dropped with it; credentials are keyed by the recorded issuer and never used with another authorization server. |

User-delegated authorization code flows, dynamic client registration and Client ID Metadata
Documents are not used by the client in this feature (G1288).

## Results and failures

| Situation | Outcome |
|---|---|
| `resultType: "complete"`, `isError: false` | `content` text blocks and `structuredContent` kept (bounded, default 1 MiB, ceiling 4 MiB); image, audio and embedded resources dropped with `truncated: true`; scanned for the attempt's resolved secrets |
| `isError: true` | returned to the model as an error result with the fixed reason `MCP_TOOL_ERROR`; the server's text (bounded to 4 KiB, secret-scanned, marked untrusted) has no carrier and counts as `pii`, so it is the result's content only when the agent's transcript level is already `pii`, otherwise the reason goes alone (Decision 025-008) |
| `resultType: "input_required"` or a task handle | `MCP_UNSUPPORTED_RESULT`; returned to the model; nothing is answered back to the server |
| JSON-RPC error | `AI_MCP_PROTOCOL_ERROR` (non-retryable for `-32602`/`-32601`, retryable for `-32603`) |
| `401`/`403` | `AI_MCP_AUTH` (non-retryable) |
| Connection refused, DNS failure, TLS failure before the request was sent | retryable `AI_MCP_UNAVAILABLE` |
| Timeout or broken stream after the request was sent | `OUTCOME_UNKNOWN` for `write` tools (manual review); retryable for `read` tools |
| Egress denied, or any `3xx` answer (no redirect is followed, `max_redirects = 0`, the refresh included) | `AI_DESTINATION_DENIED`, audited `egress.denied`; the bearer header never reaches the redirect target |

## Untrusted content

Tool titles, descriptions, annotations and results are third-party content that reaches the model.
They can carry instructions ("prompt injection"). The platform does not detect that (G1286); it
bounds the consequence: the model can request only declared tools, inputs are schema-validated,
sensitive tools need approval, budgets cap the loop, egress limits destinations, and descriptions
are pinned at publication so an author reviews what the model will read.
