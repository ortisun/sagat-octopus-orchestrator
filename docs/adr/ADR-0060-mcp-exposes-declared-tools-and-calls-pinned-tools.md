# ADR-0060: The MCP server exposes only declared tools through a classified result projection, and the MCP client calls only pinned tools of registered servers

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal (2026-10-26, revised 2026-10-27 after the independent architecture and
security reviews); accepted on 2026-11-26 by the agent preparing feature 025 for build, under the owner's
delegation (Owner decision 2026-11-24-4), recorded as Owner decision 2026-11-26-124; revised the same day
against `develop` (`3f22b612`) after an independent audit of the feature 025 set; revised on 2026-11-27
after the security review of that date (`specs/025-ai-agents-mcp/reviews/2026-11-27-security.md`,
Decision 025-016), which tightened decisions 1, 2, 4, 6 and 7 and left their direction unchanged.
**Refines / supersedes:** Applies the ADR-0031 declared-disclosure pattern, ADR-0033 levels, the
`EgressCeiling` of Architecture decision 0071 decision 8 and the `pii`-egress rule decided for Owner
question 1081 to a new surface; adds a row to the run-input gate of design 043 (Architecture decisions
0090 and 0092); refines ADR-0026 (a new inbound credential profile for MCP) and ADR-0035 (egress and
credential rules for a third managed executor, via Architecture decision 0059). Supersedes none.
**Enforced by:** planned, not yet existing: MCP conformance tests with independent client libraries at
revisions 2026-07-28 and 2025-11-25 and the security battery under
`tests/{contract,integration}/ai-agents-mcp/` named in `specs/025-ai-agents-mcp/tasks.md` (default-empty
list, audience binding, query-token refusal, foreign-exposure opacity, projection classification,
snapshot drift refusal, egress denial, single-audience tokens, `tasks/get` re-authorization, argument
ceilings and canaries, the run-input gate's answers); the extension of the restricted-dependent rules
of `scripts/check-architecture.py` (Task 025-073: the control plane does not depend on
`crates/infra-ai`, and only it depends on `crates/api-mcp`). Until those exist this is review only.

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
   the right to start the target and the rights of the declared level. An exposure MUST NOT target an
   unattended version: one containing an agent `write` tool acknowledged `approval: "none"`, or able to
   start a child workflow (child node or `child` tool, at any depth, under its version policy) that is
   unattended; checked when the exposure is saved, again when a call resolves the version, and at every
   child admission under a run the MCP server admitted (feature 026 public links apply the same rule).
2. **A result leaves only through its projection.** The projection MUST be evaluated over the committed
   run result without secret capability, validated against the closed schema, contain no field whose
   effective level is above the declared level and no `secret` field, and be returned only to a caller
   holding the rights of that level; otherwise the call is refused before admission. A field's
   effective level is the one the target version recorded at publication: the stricter of its
   annotation and the level it inherits from the agent and child outputs it is drawn from (ADR-0059
   decision 7), so an annotation cannot relabel a model-written value. Every disclosed result is
   audited.
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
   runs code. Its arguments are the run input and MUST pass the run-input gate of design 043 as a row of
   that gate (`CheckedRunInput`, the existing `create`): a refused input is JSON-RPC `-32602` with the
   value-free rule and field, an unusable schema a tool error, a transient refusal a retryable `-32603`,
   and `report` mode admits as it does everywhere else. The MCP server has no validator of its own. An optional `_meta` key `sagat-octopus-orchestrator.io/idempotencyKey` deduplicates;
   without it every call is a new run, and the documentation says so. A receipt belongs to the caller,
   defined as the credential identity (the scoped token's identifier, or the OAuth issuer and subject),
   and to the exposure; it is read only after the call is authorized, the same key on another exposure
   is refused, and a replayed result is disclosed only through the replaying call's exposure. Before a
   result leaves the bounded wait, the credential and rights are checked again. A JSON-RPC batch is
   refused. A run that outlives the bounded
   wait is returned as a task (tasks extension) or a run handle; `tasks/get` and `tasks/cancel` MUST
   re-check the token, `mcp:call`, token scope, exposure state and result-level rights on every call.
   Remote callers are bounded per caller (live agent runs) and per exposure (optional daily tokens).
5. **The surface is minimal and stateless.** Revision 2026-07-28 is served, and 2025-11-25 clients are
   served without sessions. Only `server/discover`, `tools/list`, `tools/call` and the tasks extension
   are offered; `tools/list` returns only what the caller may call (decided additionally by the 029
   access evaluator when 029 is enabled), deterministically ordered and privately cacheable; a foreign,
   unknown, out-of-scope or uncallable tool is answered exactly like a missing one.
6. **The client calls only what was reviewed, and sends only what the server may receive.** An agent
   MUST use an external MCP tool only through a registered server resource and an immutable tool
   snapshot pinned at publication, whose identifier and server revision the published version records
   and admission copies; before each call the runner compares the server's current definition
   with the pinned digests and refuses a changed tool (`AI_TOOL_CHANGED`). A ceiling lowered after
   publication applies at the next admission and resolution. An `mcp` tool may be declared `read`
   without approval only when its pinned snapshot states it read-only; untrusted annotations only ever
   tighten that rule. A server revision MUST
   declare `maxArgumentClassification`; publication MUST refuse an `mcp` tool whose server ceiling is
   below the agent's transcript level (ADR-0059 decision 7), because the model can copy anything in its
   transcript into arguments. The ceiling is the domain's `EgressCeiling`. A `pii` ceiling follows the
   uniform rule decided for Owner question 1081 (Decision 1617 of feature 020, applied by Owner decision
   2026-11-26-120: set by a reveal-grant holder, rechecked per call, revocation pauses the server, each
   call audited, a secret never leaves, default `internal`). Descriptions, annotations and results are
   untrusted content; results are bounded, text or structured only, and scanned for the attempt's
   resolved secrets. A server's error result reaches the model as `MCP_TOOL_ERROR`; its text has no
   carrier, counts as `pii` and is forwarded only when the transcript level is already `pii`.
7. **The client runs only in the agent runner, through the egress guard, with declared secrets only.**
   Connections use the guarded egress client with no redirect followed and the namespace egress policy
   held in operator-owned runner configuration (private destinations only through the operator-approved
   runner policy). A static bearer credential is a declared secret of the
   task definition, because the MCP-server revision that holds it is pinned by the published version;
   it is disclosed under Principle V's first declared set, all or nothing, to the lease-owning agent
   runner, under the caller's current membership, its binding (`BindingProfile::Agent`), the
   pool-delivery policy and the publisher's `secret:use` on it, checked at publication and again at
   delivery, and dropped when the attempt ends. MCP servers authenticated by OAuth reach access tokens
   only through design 047's token broker, under the third declared set of Principle V that the owner
   approved for constitution 5.1.0 (Owner decision 2026-11-25-3); the `oauth-client-credentials` mode is refused at
   save until 5.1.0 merges and the broker is built. When it is enabled, the issuer and token endpoint are recorded in the server revision by an
   `ai:manage` holder, never discovered at call time, and a refresh that finds different advertised
   metadata audits it and blocks calls until the revision is updated. User-delegated authorization,
   `input_required` and client-side tasks are not supported until a later decision.

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

None yet: accepted as a design, not as an implementation. Required before the implementation is
declared done: the MCP conformance and security tests listed in
[tasks.md](../../specs/025-ai-agents-mcp/tasks.md), the contracts
[mcp-server.md](../../specs/025-ai-agents-mcp/contracts/mcp-server.md) and
[mcp-client.md](../../specs/025-ai-agents-mcp/contracts/mcp-client.md), and decisions D1289,
D1296, D1301–D1306, D1316, D1317, D1637, D1638, D025-001, D025-006, D025-008 and D025-016 and gap G025-001 in
[research.md](../../specs/025-ai-agents-mcp/research.md).
