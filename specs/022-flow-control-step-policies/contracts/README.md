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
