# Feature 025 contract fragments (draft)

Status: Approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision
2026-11-24-1); Architecture decisions 0059 and 0060 accepted (delegate, Owner decisions 2026-11-26-123
and 2026-11-26-124); not started.

None of these fragments is canonical or implemented. Task 025-002 merges them into the canonical
contracts in `specs/001-platform-foundation/contracts/` in the same delivery as the code, with the
existing lint and compatibility checks (`*.baseline.json`, OpenAPI/proto/IR lint in CI `contracts`).

| Fragment | Merges into | Compatibility rule |
|---|---|---|
| [agent-node.schema.json](agent-node.schema.json) | `workflow.schema.json` (`$defs.agent*`, `node.oneOf`, the twentieth kind) | Additive IR v1 (ADR-0032); publication gated by the instance-registry capability `ai-agents-v1` (Architecture decision 0082, Decision 025-002); existing documents unchanged |
| [model-turn.schema.json](model-turn.schema.json) | new `specs/001-platform-foundation/contracts/model-turn.schema.json` | New document, `schemaVersion: 1`; grows only |
| [ai-api.openapi.yaml](ai-api.openapi.yaml) | `openapi.yaml` under `/v1` | New paths and schemas; `AgentProgress` is an optional field of runtime-graph nodes |
| [agent-runner-protocol.md](agent-runner-protocol.md) | `worker.proto`, `worker.baseline.json`, the register of minors in `sdks/SPEC.md` section 9 | One additive minor, the next free one of the register when it merges (1.7 is feature 041's; G1280); older minors keep their meaning |
| [agent-stream-events.md](agent-stream-events.md) | feature 026's stream contract | Only with feature 026 |
| [mcp-server.md](mcp-server.md) | new `mcp-server.md` reference under the canonical contracts and the runbook | Versioned by MCP revision, independent of `/v1` |
| [mcp-client.md](mcp-client.md) | runbook and adapter documentation | Behavioural contract of the runner |
| [errors.md](errors.md) | `openapi.yaml` `Error.details` description, `crates/api-http/src/error.rs` remediation table, `sdks/error-catalog.json` (`worker`-origin entries) and `sdks/SPEC.md` section 2 | No new REST `Error.code`; reasons and failure codes only grow |
| [conformance.md](conformance.md) | `tests/contract/sdk-conformance/` | Every scenario in all six SDKs, Rust included |

AsyncAPI: no new outbound event is required. Agent progress is visible through the run's journal
events, which are payload-free and already covered by `run.finished` for outbound subscribers. If a
later decision adds an `agent.finished` notification, it follows `docs/event-schema-evolution.md`.

Journal events added (payload-free, typed, replayed by the reducer) and their arm in the reset-rule
registry of feature 018, which is code: the `registry!` macro of `crates/domain/src/execution/reset.rs`
(normative text `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`; coordinator C1,
D1635). Every agent event names the agent node, so a window opened by one is closed only by the
`agent.finished` of the same node.

| Event type | `ResetRule` |
|---|---|
| `agent.started`, `agent.turn_scheduled`, `agent.turn_completed`, `agent.tool_planned`, `agent.tool_refused`, `agent.tool_completed`, `agent.repair_scheduled`, `agent.budget_exhausted` | `Refuse { until: Some("agent.finished") }`: no reset point between the event and the `agent.finished` of the same node is offered; once that window is closed the event is re-emitted like any other |
| `agent.finished` | `Rematerialize`: the reset transaction copies the finished step's agent rows into the new run, sharing payloads, repeating no call and charging no budget or usage lease |

Tool-call approval events (feature 014 event types with owner kind `AGENT_TOOL_CALL`) fall under the
same open-loop refusal. The events carry ids, counters, digests and codes; bodies live in `payloads`
referenced by digest.
