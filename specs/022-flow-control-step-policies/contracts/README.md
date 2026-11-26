# Feature 022 — draft contract fragments

DRAFT, not implemented. These files are proposals reviewed with the specification; the canonical
contracts stay in `specs/001-platform-foundation/contracts/` and are changed only at implementation, in
the same delivery as the code and tests (task T022-060), with the compatibility baselines regenerated on
purpose.

| File | Merges into | Change kind |
|---|---|---|
| [openapi-flow-control.yaml](openapi-flow-control.yaml) | `openapi.yaml` | new paths; additive `Run` properties; additive origin and subscription event values |
| [workflow-ir-fragment.schema.json](workflow-ir-fragment.schema.json) | `workflow.schema.json` | additive v1 fields and node kind (ADR-0032) |
| [asyncapi-fragment.yaml](asyncapi-fragment.yaml) | `events.asyncapi.yaml` | new messages; optional properties |
| [journal-events.md](journal-events.md) | journal vocabulary (`crates/domain/src/execution/event.rs`) | new event types |
| [errors.md](errors.md) | `Error.details.reason`, `DefinitionProblem.code`, failure codes | refinements only |
| [conformance.md](conformance.md) | `tests/contract/sdk-conformance` | six scenarios |

**Worker protocol (`worker.proto`): no change.** Cache hits, mocks, early results and permits are decided
by the server; a step waiting for a step permit stays `READY` and is not leased, like a step whose activity
quota is exhausted. No protocol minor is negotiated by this feature.
*Corrected 2026-11-24 (decision D022-220):* a step waiting for a permit, a rate slot or its cache lookup is not `READY`:
it waits outside it (`WAITING`, wait kind `CONCURRENCY`, `RATE_LIMIT` or `CACHE_LOOKUP`; ADR-0089 decision 1, ADR-0135
decision 1), so no claim ever considers it.

**The rest of 022 (2026-11-24, OD-2026-11-24-1).** The fragments now serve release 0.3.0 whole. Changed with the
decisions D022-203 to D022-221: the journal event `step.cache_missed` and its reset rule; the refusal reasons of the
registry capabilities and of a standalone step's business identifier; the interim dependency
`COLLECTION_FINISH_ROUTING`; the standalone request's business-identifier fields; six SDKs in the conformance
scenarios. Everything merges into the canonical contracts with the slice that builds it (task T022-060).

**Release 0.3.0 slice (US02 and US07).** The fragments carry the rate-limit additions next to the
concurrency ones: `rateLimitPolicy` in the IR fragment, `Run.rateLimit` and `GET
/namespaces/{namespaceId}/rate-limit-keys` in the OpenAPI fragment, four `rate_limit.*` journal events,
the reasons and codes marked US07 in `errors.md`, and the scenario `flow-rate-limit`. The worker protocol
is still unchanged: a step waiting for a rate slot is not `READY`, so `PollTasks` never returns it and no
protocol minor is negotiated. The AsyncAPI fragment has no rate-limit message. Only the concurrency and
rate-limit parts merge into the canonical contracts for 0.3.0 (T022-060 narrowed, T022-087).

**Revision after the review of 2026-11-13.** Error rows `FLOW_CONTROL_ACTIVATION_PENDING`,
`FLOW_CONTROL_KEY_UNBOUNDED`, `FLOW_CONTROL_KEY_SECRET`, `FLOW_CONTROL_KEY_PII`, `FLOW_CONTROL_KEY_INVALID`
and the child-node outcomes; the journal event `rate_limit.returned` and step-scope permit events with
`attempt`; in the OpenAPI fragment, `keyLabel` may be `null` (FR-022-057), position and estimate are
computed by the single-run read only, and the `group` filter accepts derived group names. The worker
protocol stays unchanged: a step waiting for either gate is not `READY`.

**Revision of 2026-11-26 after the re-reads (decisions D022-272 to D022-285).** The OpenAPI fragment gains the three
operations of the `RunRecovery` binding (`listRunRecoveryBindings`, `bindRunRecovery`, `unbindRunRecovery`) and the role
value `run-recovery`; `errors.md` gains their rows, the opaque `404` of a bound subject outside its binding as the first
refusal of every route, the continue-as-new refusal of a reset or rerun under a handler tree and the runtime defence's
durable code.
