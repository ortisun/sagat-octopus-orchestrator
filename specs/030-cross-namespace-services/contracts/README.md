# Contracts — feature 030 (draft fragments)

These files are proposals for review. None is canonical and none is implemented. At implementation
time, tasks T030-005..008 merge them into the canonical contracts and their checks; until then the
canonical files are unchanged.

| Fragment | Merge target | Check that must pass after the merge |
|---|---|---|
| [service.schema.json](service.schema.json) | new `specs/001-platform-foundation/contracts/service.schema.json` | schema compiles; example documents validate; semantic rules of FR-030-002 tested in `crates/infra-ir` and `crates/infra-schema` |
| [workflow-service-call.schema.json](workflow-service-call.schema.json) | `$defs.serviceCall` and one `$defs.node.oneOf` branch in `specs/001-platform-foundation/contracts/workflow.schema.json` | the existing fixture corpus still validates unchanged (ADR-0032 §2); new shape tests; not added to `$defs.bodyNode` |
| [openapi-service-operations.yaml](openapi-service-operations.yaml) | paths and schemas in `specs/001-platform-foundation/contracts/openapi.yaml` (`x-proposed: '030'`, then `x-implemented: '030'`) | OpenAPI lint; REST contract tests; generated Studio client parity; cross-tenant battery driven by the document |
| [asyncapi-service-operations.yaml](asyncapi-service-operations.yaml) | channels, operations and messages in `specs/001-platform-foundation/contracts/events.asyncapi.yaml` (minor version bump) | `tests/contract/events-webhooks-messaging/asyncapi` compatibility with a deliberately regenerated `events.baseline.json`; conformance of the published hints |
| [operation-messages.md](operation-messages.md) | internal; summarized in `docs/architecture/execution-semantics.md` | fault suites of `tests/fault/service-operations/` |
| [errors.md](errors.md) | the `Error.details` description in `openapi.yaml`; `sdks/SPEC.md` §2 table of step failure codes | `sdks/error-catalog.json` unchanged (no new `Error.code`); five SDKs surface the codes |
| [conformance.md](conformance.md) | `tests/contract/sdk-conformance/scenarios/` and `expected/` | `cargo test -p sagat-tests --test sdk_conformance -- --ignored` with all five languages |

Registrations in other drafts' registries — 031's encoded-field registry (refusal of encoded values
at `serviceCall.input`, the operation schemas, handler admission and result return, D1661), 018's reset-rule registry, 020's content-store registry, 028's
time-class catalog, 029's list of permission-adding features and the consolidated FR-016-002
amendment — are made by T030-067..070 together with the code they describe.

Unchanged by this feature: `specs/001-platform-foundation/contracts/worker.proto` and
`worker.baseline.json` (no worker protocol change, D1508). `schema.sql` is an illustrative model
(its README says so); the migrations of [../data-model.md](../data-model.md) are the authority, and
any illustrative update of `schema.sql` is left to the implementation review.
