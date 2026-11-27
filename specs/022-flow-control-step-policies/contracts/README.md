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

**Merge of the slice (2026-11-26, T022-060 narrowed and T022-087, decision D022-110).** The concurrency and rate-limit
parts of `openapi-flow-control.yaml`, `workflow-ir-fragment.schema.json` and `errors.md` are in the canonical documents.
Where the fragment and the code disagreed the fragment was corrected (the listings' `Limit` parameter and answer
headers, a closed `ConcurrencyKeyPage`, `Run.concurrency` and `Run.rateLimit` absent rather than `null`). The test
`contract_flow_control_step_policies::contracts` fails when the two documents' shapes part again, or when the server
gives a code no canonical document names. The ten journal events are not AsyncAPI messages, so `events.asyncapi.yaml`
and `events.baseline.json` do not change for the slice.
