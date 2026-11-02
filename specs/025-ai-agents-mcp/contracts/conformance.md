# Five-SDK conformance scenarios (draft)

Scenarios live in `tests/contract/sdk-conformance/scenarios/` with expected observations in
`tests/contract/sdk-conformance/expected/`, run by the existing harness for TypeScript, Go, Python,
C# and Java against the real engine, a real agent runner and `apps/dev-model-provider` scripted
responses. A subset of languages never closes a scenario. Normalization reuses
`tests/contract/sdk-conformance/normalize.rs`; decimal strings stay strings.

| Scenario file | Builds / does | Expected observation (all five identical) |
|---|---|---|
| `agent-builders.json` | builds the example agent of `agent-node.schema.json` with all tool kinds available to SDKs | byte-identical canonical IR; invalid authoring cases (duplicate tool name, open schema, `secret` context, cost without currency, `stream` without capability) refused locally or by publication with the same reason |
| `agent-structured-output.json` | publishes and runs a one-turn agent; dev provider returns a valid JSON answer | run `SUCCEEDED`; `steps.triage.output.result` equal; usage counts equal |
| `agent-repair.json` | first answer invalid, second valid | two turns, one `REPAIR`; result equal |
| `agent-tools-parallel.json` | dev provider requests two activity tools; each SDK's worker serves them | both tool steps `<node>~t1.1`, `<node>~t1.2` carry `agent:{run}:{node}:1:{k}` keys; results return in request order; second turn final |
| `agent-tool-invalid.json` | dev provider requests an undeclared tool and an invalid input | no worker lease for them; error results; `maxToolCalls` charged |
| `agent-approval.json` | write tool with approval; SDK client lists approvals, reads the tool-call view, decides | approve path runs the tool; reject with `refuse` returns a refusal result; duplicate decision key replays; initiator excluded |
| `agent-budget.json` | scripted endless tool loop with `maxTurns: 3` | `AI_BUDGET_EXHAUSTED`, `details.budget = turns`, routed to `onError` |
| `agent-transcript.json` | reads the transcript with and without each of `run:read`, `payload:read-internal` and the reveal grant | exact page with all three; opaque `NOT_FOUND` otherwise; `Cache-Control: private, no-store`; one `payload.pii_revealed` audit per read |
| `agent-classification.json` | publishes agents whose `result` or approval-view input is annotated below the transcript level, an `mcp` tool on a server below it, and a `write` tool without approval | refused with `CLASSIFICATION_LOOSENED`, `MCP_ARGUMENT_CLASSIFICATION_EXCEEDED`, `WRITE_TOOL_APPROVAL_REQUIRED`; an `approval: "none"` version cannot be exposed (`UNATTENDED_WRITE_TOOL`) |
| `agent-retry-budget.json` | dev provider loses the first response of turn 1; budget fits exactly one more reservation | the retry runs and the turn commits; a second loss fails with `AI_BUDGET_EXHAUSTED`; settled + reserved never exceeds the limit |
| `agent-stream-multiturn.json` | 16-turn agent with `stream.tokens: true` (only with feature 026) | every turn has `text_delta` events and a `turn_end`; reserved-channel totals within the declared budget; author channels unaffected; a superseded attempt is marked |
| `agent-cancel.json` | cancels during a pending approval | approval `CANCELLED`, agent `CANCELLED`, no further lease |
| `agent-errors.json` | each `AiFailure` code the dev provider can trigger | SDK error/failure objects expose the same code, `retryable` and trace id |

The MCP server is exercised by an independent MCP client library (not by the five platform SDKs,
which are not MCP clients): `tests/contract/ai-agents-mcp/mcp_server/` runs the official TypeScript
and Python MCP SDKs as clients at revisions `2026-07-28` and `2025-11-25`, pinned by lockfile, in
the test environment only.
