# ADR-0060: The MCP server exposes only declared tools through a classified result projection, and the MCP client calls only pinned tools of registered servers

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** Applies the ADR-0031 declared-disclosure pattern, ADR-0033 levels and the
`pii`-egress rule proposed with 020's O1081 to a new surface; refines ADR-0026 (a new inbound credential profile for MCP) and ADR-0035 (egress and
credential rules for a third managed executor, via ADR-0059). Supersedes none.
**Enforced by:** planned, not yet existing: MCP conformance tests with independent client libraries at
revisions 2026-07-28 and 2025-11-25 and the security battery under
`tests/{contract,integration}/ai-agents-mcp/` named in `specs/025-ai-agents-mcp/tasks.md` (default-empty
list, audience binding, query-token refusal, foreign-exposure opacity, projection classification,
snapshot drift refusal, egress denial, single-audience tokens, `tasks/get` re-authorization, argument
ceilings and canaries, a changed token endpoint); the restricted-crate → allowed-dependents map in
`scripts/check-architecture.py` (task T025-073: the control plane does not
depend on `crates/infra-ai`). Until those exist this is review only.

## Context

The Model Context Protocol lets a model client list and call tools on a server, and lets an agent use
tools published by others. Both directions cross a trust boundary that is easy to get wrong and hard
to change once clients depend on it:

- A tool result is handed to a model, which is a disclosure to whatever the client does with it.
  ADR-0031 already decided that results leave only by declaration; ADR-0033 decides per field who may
  see a value.
- The MCP authorization specification (2026-07-28) makes a server an OAuth 2.1 resource server that
  MUST validate the token audience (RFC 8707), MUST publish Protected Resource Metadata (RFC 9728),
  MUST NOT accept or pass through other tokens, and forbids tokens in query strings. Some products put
  a long-lived token in the server URL.
- External MCP servers are third parties: their tool descriptions reach the model and can carry
  instructions, and a tool definition can change after an author reviewed it.
- MCP has no idempotency field and, since 2026-07-28, no session; a client re-issues a broken request.

## Decision

1. **Nothing is exposed by default, and nothing unattended is exposed.** The MCP server MUST list and
   call only exposures: namespace resources with immutable revisions that name one target (a published
   workflow; script and managed-operation targets only through feature 022 standalone steps), its input
   schema, a closed result projection with `disclosure: "declared-mcp-result-v1"` and a
   `maxResultClassification` (`public`, `internal` or `pii`). Saving an exposure requires `mcp:expose`,
   the right to start the target and the rights of the declared level. An exposure MUST NOT target a
   version containing an agent `write` tool acknowledged `approval: "none"`, checked when the exposure
   is saved and again when a call resolves the version (feature 026 public links apply the same rule).
2. **A result leaves only through its projection.** The projection MUST be evaluated over the committed
   run result without secret capability, validated against the closed schema, contain no field above
   the declared level and no `secret` field, and be returned only to a caller holding the rights of
   that level; otherwise the call is refused before admission. Every disclosed result is audited.
3. **Tokens are audience-bound and travel in the header.** The server MUST accept only bearer tokens in
   `Authorization` that are either OAuth access tokens whose only audience is the operator-configured
   canonical MCP URI of the namespace, or platform-minted scoped MCP tokens (SHA-256 of 256 random
   bits, no key, recorded as such in 032's key inventory; expiring at most 90 days, revocable, limited
   to listed exposures, effective rights = the minter's current rights ∩ scope). REST MUST refuse any
   token whose audience contains an MCP URI. The server MUST publish Protected Resource Metadata,
   answered identically for every well-formed namespace id, answer `401` or `403 insufficient_scope`
   with `WWW-Authenticate`, refuse tokens in a query string, and never forward a received token.
4. **A call is a durable admission; a task handle is a name.** `tools/call` MUST admit the target
   through the existing admission transaction (receipt, pins, audit, outbox) and nothing else; it never
   runs code. An optional `_meta` key `sagat-octopus-orchestrator.io/idempotencyKey` deduplicates;
   without it every call is a new run, and the documentation says so. A run that outlives the bounded
   wait is returned as a task (tasks extension) or a run handle; `tasks/get` and `tasks/cancel` MUST
   re-check the token, `mcp:call`, token scope, exposure state and result-level rights on every call.
5. **The surface is minimal and stateless.** Revision 2026-07-28 is served, and 2025-11-25 clients are
   served without sessions. Only `server/discover`, `tools/list`, `tools/call` and the tasks extension
   are offered; `tools/list` returns only what the caller may call (decided additionally by the 029
   access evaluator when 029 is enabled), deterministically ordered and privately cacheable; a foreign,
   unknown, out-of-scope or uncallable tool is answered exactly like a missing one.
6. **The client calls only what was reviewed, and sends only what the server may receive.** An agent
   MUST use an external MCP tool only through a registered server resource and an immutable tool
   snapshot pinned at publication; before each call the runner compares the server's current definition
   with the pinned digests and refuses a changed tool (`AI_TOOL_CHANGED`). A server revision MUST
   declare `maxArgumentClassification`; publication MUST refuse an `mcp` tool whose server ceiling is
   below the agent's transcript level (ADR-0059 decision 7), because the model can copy anything in its
   transcript into arguments. A `pii` ceiling follows the uniform rule proposed with 020's O1081 (set by
   a reveal-grant holder, rechecked per call, revocation pauses the server, each call audited).
   Descriptions, annotations and results are untrusted content; results are bounded, text or structured
   only, and scanned for the attempt's resolved secrets.
7. **The client runs only in the agent runner, through the egress guard, and sends its secret only to a
   recorded token endpoint.** Connections use the guarded egress client and the namespace egress
   policy (private destinations only through the operator-approved runner policy); credentials (static
   bearer or OAuth client credentials with a resource indicator) are resolved per attempt from OpenBao
   and dropped with it. For OAuth, the issuer and token endpoint are recorded in the server revision by
   an `ai:manage` holder; the runner never discovers them at call time, and a refresh that finds
   different advertised metadata audits it and blocks calls until the revision is updated.
   User-delegated authorization, `input_required` and client-side tasks are not supported until a
   later decision.

## Considered options

1. **Declared exposures and pinned snapshots (chosen)** — explicit, auditable disclosure and review;
   matches ADR-0031 and the MCP authorization rules.
2. **Expose every published workflow automatically** — no setup, but discloses results nobody declared
   and turns every publication into an MCP change.
3. **Token in the server URL** — works with clients lacking OAuth, but leaks through logs, proxies and
   history and violates the MCP authorization specification.
4. **Call external tools live from their current definition** — always up to date, but lets a server
   change a tool after review and silently change what the model reads and sends.

## Consequences

- Administrators must create an exposure and a projection for each tool; model clients without OAuth
  need a scoped token, which is a bearer credential to protect.
- A client that retries without an idempotency key starts a second run; this is documented, not
  hidden.
- External tool updates require a refresh and a republication before agents use them.
- Every external MCP server must declare an argument ceiling; an agent whose transcript level is `pii`
  can use only servers declared `pii` by a reveal-grant holder.
- A workflow with an unattended write tool cannot be offered to MCP clients at all; authors who want
  that must add an approval policy.
- Narrower audiences than namespace-wide `mcp:call` plus token scope need custom roles or folders
  (feature 029).
- Rollback: disable the MCP endpoint and exposures; existing runs started through MCP are ordinary runs
  and are unaffected.

## Evidence

None yet; this is a proposal. Required: the MCP conformance and security tests listed in
[tasks.md](../../specs/025-ai-agents-mcp/tasks.md), the contracts
[mcp-server.md](../../specs/025-ai-agents-mcp/contracts/mcp-server.md) and
[mcp-client.md](../../specs/025-ai-agents-mcp/contracts/mcp-client.md), and decisions D1289,
D1296, D1301–D1306, D1316, D1317, D1637 and D1638 in [research.md](../../specs/025-ai-agents-mcp/research.md).
