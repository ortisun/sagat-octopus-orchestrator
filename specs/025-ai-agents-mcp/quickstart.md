# Validation guide: AI agents and MCP

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
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.ai.yaml up -d   # T025-068
export SAGAT=https://127.0.0.1:8443 NS=<namespace-uuid>
export TOKEN=$(curl -s -X POST http://127.0.0.1:9100/token -H 'Content-Type: application/json' \
  -d '{"subject": "alice", "tenant": "demo"}' | jq -r .access_token)   # dev-idp; alice is an admin member
```

`compose.ai.yaml` (planned) adds `dev-model-provider`, `agent-runner` and an OpenBao development
service, with the runner's egress limited to the engine, OpenBao and the dev provider.

## 1. Register a provider (US01)

Create the API-key secret through the existing secret routes (OpenBao-backed, feature 017), then:

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
values, not real prices.

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
| Secret canary | an activity tool echoing a secret it was given | `SECRET_IN_OUTPUT`; the canary appears in no prompt, transcript, log, span, audit or response |
| Data ceilings | annotate `result.priority` as `public`; then lower the MCP server to `public` | publication `422` with `CLASSIFICATION_LOOSENED`, then `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED` |
| Unattended write | mark `issue_refund` `approval: "none"` and try to expose the workflow | exposure `422` `UNATTENDED_WRITE_TOOL` |
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
a call with `?access_token=` in the URL (`401`), and a call with a REST token (`401`, wrong audience).
The projection selects an existing object (`result.summary`, declared `internal` in the workflow's
`outputSchema`); the bounded evaluator has no object literal (feature 014).

## 5. MCP client (US07)

Register the dev provider's test MCP server (`https://dev-model-provider:9443/mcp`, mode `bearer`,
`maxArgumentClassification: internal`), refresh it, allow `get_weather` in an agent node with a closed
`outputSchema` annotated `internal` (otherwise its results count as `pii` and the `internal` provider
refuses the agent), publish, and run: the call is leased by the agent
runner and its result returns to the model. Then switch the test server to `weather-v2` (changed input
schema) and run again: the call fails with `AI_TOOL_CHANGED` and the server log shows no `tools/call`.

## 6. Evidence to record (T025-072)

Commands and outcomes, environment (hardware, versions, compose profile), counts of runs, turns,
attempts and audits per step, the transcript and audit queries used, and every skipped or failed step.
Performance figures go to `tests/performance/reports/ai-agents-mcp.md` with their environment. Do not
report a step as passed without its assertion output.
