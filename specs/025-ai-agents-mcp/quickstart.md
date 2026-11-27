# Validation guide: AI agents and MCP

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decisions 0059 and 0060 accepted (delegate, Owner decisions 2026-11-26-123
and 2026-11-26-124); not started. Revised 2026-11-27 (round 3) for Decisions 025-017 to 025-019.

Nothing in this guide runs today. It is the target demo of the Definition of Done: T025-072 makes
`cargo test -p sagat-tests --test e2e_ai_agents_mcp --locked quickstart::` execute exactly these
steps against the real control plane, engine, agent runner, PostgreSQL, OpenBao (development mode)
and `apps/dev-model-provider`. Until that target passes, every step below is a plan, not evidence.

The demo uses no real model provider and no paid API. `apps/dev-model-provider` is development-only
(`SAGAT_ENV=dev`), is not in the production image, answers from scripted scenarios selected by the
request's model id (`claude-sonnet-5-5` and friends are accepted only as names), and hosts a test MCP
server. Model ids are those of the provider; the platform checks them only against the allowlist.

## Prerequisites

```sh
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.database.yaml \
  -f infra/docker/compose.ai.yaml up -d   # Task 025-068; OpenBao from compose.database.yaml
control-plane platform-requirement activate ai-agents-v1 mcp-server-v1   # operator command, schema-owning role (Architecture decision 0082)
export SAGAT=https://127.0.0.1:8443 NS=<namespace-uuid>
export TOKEN=$(curl -s -X POST http://127.0.0.1:9100/token -H 'Content-Type: application/json' \
  -d '{"subject": "alice", "tenant": "demo"}' | jq -r .access_token)   # dev-idp; alice is an admin member
```

`compose.ai.yaml` (planned) is layered on `compose.database.yaml`, which already runs the project's
OpenBao (prepare its private files and scoped tokens with `docs/runbooks/openbao.md` first); it adds
only `dev-model-provider` and `agent-runner`, with the runner's egress limited to the engine and the dev
provider. It starts no second OpenBao. The requirement command above is illustrative: its exact name is
the instance registry's operator command (Task 025-082).

## 1. Register a provider (US01)

Create the API-key secret through the existing secret routes (OpenBao-backed, feature 017); the
administrator who saves the provider needs `ai:manage` and `secret:use` on it, and so does every
publisher of an agent that pins the provider (Requirement 025-003). Then:

```sh
curl -s -X PUT "$SAGAT/v1/namespaces/$NS/ai-providers/anthropic-dev" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{
  "expectedRevision": 0, "requestId": "5f0b7b2e-0000-4000-8000-000000000001",
  "kind": "anthropic",
  "settings": {"baseUrl": "https://dev-model-provider:9443", "promptCaching": true},
  "credential": {"mode": "api-key", "secrets": {"apiKey": "anthropic-dev-key"}},
  "models": [
    {"id": "claude-sonnet-5-5", "maxOutputTokens": 65536, "capabilities": {"nativeStructuredOutput": true, "parallelToolCalls": true, "streaming": true, "effort": true},
     "prices": {"currency": "USD", "inputPerMTokMicros": "2000000", "outputPerMTokMicros": "10000000"}},
    {"id": "claude-haiku-5-5", "capabilities": {"nativeStructuredOutput": true}}
  ],
  "maxPromptClassification": "internal"}'
curl -s -X POST "$SAGAT/v1/namespaces/$NS/ai-providers/anthropic-dev/probe" -H "Authorization: Bearer $TOKEN" -H 'Idempotency-Key: probe-1'
```

Expected: revision 1, `ACTIVE`, no credential value in the response; the probe is a run with origin
`AI_PROVIDER_PROBE` that succeeds with the model list and the tested revision. The prices are demo
values, not real prices. Saving the same provider with `"credential": {"mode":
"azure-entra-client-credentials", ...}` or `aws-access-key` is refused with
`AI_CREDENTIAL_MODE_UNAVAILABLE` until design 047's broker exists (Gap 025-001); once it does, the
runner receives only an Entra access token from it, never the client secret, and `aws-access-key`
stays refused until the owner answers Owner question 025-002 (Decision 025-020; Decision
025-017).

## 2. Publish and run an agent (US01–US04)

Publish the example of `contracts/agent-node.schema.json` (provider `anthropic-dev`, tools
`lookup_order` and `issue_refund`), with `input.ticket` classified `internal` in the workflow
`inputSchema`. Start the demo activity worker (`orders.lookup`, `payments.refund`,
`payments.refundStatus`) from the TypeScript SDK, then start a run whose ticket names an order.

Expected journal sequence (scenario `refund-with-approval` of the dev provider):
`agent.started` → turn 1 (`tool_use`: `lookup_order`) → `<node>~t1.1` leased by the SDK worker with key
`agent:<run>:triage:1:1` → turn 2 (`tool_use`: `issue_refund`) → approval opened for
`<node>~t2.1`, run `WAITING` → decide:

```sh
curl -s "$SAGAT/v1/runs/$RUN/approvals" -H "Authorization: Bearer $OPERATOR_TOKEN"
curl -s "$SAGAT/v1/approvals/$APPROVAL/agent-tool-call" -H "Authorization: Bearer $OPERATOR_TOKEN"
curl -s -X POST "$SAGAT/v1/approvals/$APPROVAL/decisions" -H "Authorization: Bearer $OPERATOR_TOKEN" \
  -H 'Content-Type: application/json' -d '{"decisionKey": "d-1", "decision": "APPROVE"}'
```

→ `<node>~t2.1` runs → turn 3 final answer validated → `steps.triage.output.result` is
`{"category": "billing", "priority": 2}` and `usage` shows tokens and an estimated cost in USD. A
`run:read`-only user sees turns, tool names and usage in the run graph, never values; the transcript
endpoint returns it only to a holder of `payload:read-internal` and the reveal grant, with a
`payload.pii_revealed` audit per read.

## 3. Failure scenarios

| Scenario | How | Expected |
|---|---|---|
| Runner crash after the provider answered | dev provider scenario `answer-then-hold` and `kill -9` of the agent runner before `CompleteStep`; restart | turn 1 has two attempts and one committed result; both reservations charged; no duplicate tool call |
| Budget exhaustion | scenario `endless-tools` with `maxTurns: 3` | turn 4 never dispatched; `AI_BUDGET_EXHAUSTED` with `details.budget = turns`; `onError` route taken |
| Lost broker hint | stop NATS during turn 2, restart it | runner polls; the outbox re-emits; one turn 2 |
| Approval expiry | approval `timeoutMs: 5000`, no decision | `onExpire: fail` → `AI_TOOL_DENIED`; the losing late decision is `409 ALREADY_DECIDED` |
| Classification refusal | change the provider to `maxPromptClassification: public` and republish | publication `422` with `PROMPT_CLASSIFICATION_EXCEEDED` and the pointer of `context.ticket` |
| Ceiling lowered after publication | keep the published version; save a new provider revision with `maxPromptClassification: public` (no republication); start a run, and separately let a run already waiting on approval reach turn 3 | the new run is refused at admission (`409 AI_DESTINATION_RESTRICTED`); the waiting run's next model call fails `CLASSIFICATION_EGRESS_REFUSED` before dispatch and the dev provider receives nothing |
| Blind approval | decide the `issue_refund` approval as a member without `payload:read-internal` | the tool-call view shows the decision context, the redacted transcript outline and every input field masked by path and level only, with `decidable: false`; the decision is `403 APPROVAL_INPUT_WITHHELD`, nothing is applied, `ai.approval_input_withheld` is audited |
| Masked decision declared | with a context field annotated `pii` (so the transcript level is `pii`), republish with `issue_refund`'s approval `decisionFields: []` (an `activity` tool, which may run with `approval: "none"`); decide as an `operator` without the reveal grant | the view masks `orderId` and `amountCents` by path and level only; `decidable: true`; the decision applies and records the masked fields; the same declaration on an `operation` tool is refused at publication `DECISION_FIELDS_REQUIRED` |
| Narrowed approval offered remotely | the version of the "Masked decision declared" row (`issue_refund` with `decisionFields: []`); save an MCP exposure of it, and call a parent exposed workflow that starts it as a child | the exposure save is `422 UNATTENDED_WRITE_TOOL`; the child admission under the MCP-admitted parent creates no child and the tool call fails `UNATTENDED_WRITE_TOOL`; the same versions run normally when started by a person |
| No decider with context | a `pii` transcript, the default `decisionFields`, an `escalation` route to the `admin` role, nobody holding the reveal grant; approval `timeoutMs: 120000` | at open, `ai.approval_escalated`; within 60 seconds the request is marked `NO_DECIDER_WITH_CONTEXT` and `agent_approvals_without_decider` is 1; at the deadline `onExpire` applies with the reason `NO_DECIDER_WITH_CONTEXT`, audited `ai.approval_no_decider`; granting the reveal grant to an `admin` before the deadline clears the mark at the next sweep and that admin decides |
| Redirected probe | dev provider scenario `redirect-probe` (`302` to another allowed host) | the probe fails `AI_DESTINATION_DENIED`, `egress.denied` is audited, and the other host receives no request and no credential |
| Undeclared tools | scenario `undeclared-tools` (256 requests named after a canary string) | 256 refusal results; the run graph, journal and spans show `UNKNOWN_TOOL` and call indexes only; the canary appears only in the transcript |
| Publisher loses the secret | revoke the publisher's `secret:use` on `anthropic-dev-key`, start a run | the model call is refused at resolution (`AI_CAPABILITY_UNAVAILABLE`), no credential leaves OpenBao, the denial is audited |
| Secret canary | an activity tool echoing a secret it was given | `SECRET_IN_OUTPUT`; the canary appears in no prompt, transcript, log, span, audit or response |
| Data ceilings | annotate `result.priority` as `public`; then lower the MCP server to `public` | publication `422` with `CLASSIFICATION_LOOSENED`, then `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED` |
| Unattended write | mark `issue_refund` `approval: "none"` and try to expose the workflow; then expose a parent workflow that starts it as a pinned child, and one that starts it under an active-version policy and call it over MCP | exposure `422` `UNATTENDED_WRITE_TOOL` for the workflow and for the pinned parent; for the active-version parent the call is admitted and the child's admission is refused `UNATTENDED_WRITE_TOOL`, so no refund runs |
| Provider rate limit | scenario `rate-limited` (`429` with `retry-after: 1`, then success) | two attempts, backoff honoured, success |

## 4. MCP server (US06)

```sh
curl -s -X PUT "$SAGAT/v1/namespaces/$NS/mcp-exposures/lookup_order" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"expectedRevision": 0, "requestId": "5f0b7b2e-0000-4000-8000-000000000002",
  "target": {"kind": "WORKFLOW", "ref": "lookup-order"}, "description": "Look up an order by id.",
  "resultProjection": "result.summary", "resultSchema": {"type": "object", "additionalProperties": false,
  "required": ["status"], "properties": {"status": {"type": "string", "maxLength": 32}}},
  "maxResultClassification": "internal", "disclosure": "declared-mcp-result-v1"}'
MCP_TOKEN=$(curl -s -X POST "$SAGAT/v1/namespaces/$NS/mcp-tokens" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: t-1' -d '{"label": "demo", "exposures": ["lookup_order"], "expiresInDays": 1}' | jq -r .token)
```

Then, with the pinned official MCP TypeScript client in `tests/contract/ai-agents-mcp/mcp_server/`:
`server/discover`, `tools/list` (one tool), `tools/call lookup_order {"orderId": "o-1"}` with
`_meta["sagat-octopus-orchestrator.io/idempotencyKey"] = "k-1"` twice (one run, same result),
a call with `?access_token=` in the URL (`401`), a call with a REST token (`401`, wrong audience),
a second scoped token with the same key `k-1` (its own run, never the first token's), the first token
reusing `k-1` on another exposure (`IDEMPOTENCY_KEY_REUSED`), a JSON-RPC batch (`-32600`) and a call
whose token is revoked during the bounded wait (no result, answered as an unknown tool).
The projection selects an existing object (`result.summary`, declared `internal` in the workflow's
`outputSchema`); the bounded evaluator has no object literal (feature 014). Annotating `summary`
`public` while it is drawn from an agent's `internal` result is refused at publication
(`CLASSIFICATION_LOOSENED`), so the exposure can never be saved with `maxResultClassification: public`
over a model-written value. The mint is replayed with `Idempotency-Key: t-1`: the replay returns the
token's metadata without the token.

## 5. MCP client (US07)

Register the dev provider's test MCP server (`https://dev-model-provider:9443/mcp`, mode `bearer`,
`maxArgumentClassification: internal`; mode `oauth-client-credentials` is refused with
`MCP_AUTH_MODE_UNAVAILABLE` until Gap 025-001 closes), refresh it, allow `get_weather` in an agent node with a closed
`outputSchema` annotated `internal` (otherwise its results count as `pii` and the `internal` provider
refuses the agent), publish, and run: the call is leased by the agent
runner and its result returns to the model. Then switch the test server to `weather-v2` (changed input
schema) and run again: the call fails with `AI_TOOL_CHANGED` and the server log shows no `tools/call`.
Refreshing the server after that changes nothing for the published version, whose snapshot identifier
is recorded in it; only a republication picks up the new snapshot. Switch the test server to the
scenario `false-read-only` (its `delete_records` states `readOnlyHint: true`): declaring
`delete_records` `effect: "read"` without an approval policy is refused at publication
(`WRITE_TOOL_APPROVAL_REQUIRED`) because the revision does not classify it `read`, and
`ai.mcp.tool_classification_mismatch` is audited at the refresh and at the publication. Lowering the server's
`maxArgumentClassification` to `public` in a new revision refuses the next call before it is sent
(`CLASSIFICATION_EGRESS_REFUSED`).

## 6. Evidence to record (T025-072)

Commands and outcomes, environment (hardware, versions, compose profile), counts of runs, turns,
attempts and audits per step, the transcript and audit queries used, and every skipped or failed step.
Performance figures go to `tests/performance/reports/ai-agents-mcp.md` with their environment. Do not
report a step as passed without its assertion output.
