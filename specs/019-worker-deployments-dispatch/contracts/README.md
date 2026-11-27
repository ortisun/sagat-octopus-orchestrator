# Contracts — feature 019 (draft fragments)

These fragments are proposals. The canonical contracts stay in `specs/001-platform-foundation/contracts/`
(`openapi.yaml`, `worker.proto`, `workflow.schema.json`, `events.asyncapi.yaml` and their baselines); task
T019-003 merges these fragments there in the implementing delivery, with the compatibility checks of
`docs/event-schema-evolution.md`, the worker baseline (`worker.baseline.json`) and the OpenAPI lint. Nothing here
is served by any build.

| File | What it proposes |
|---|---|
| [openapi-fragment.yaml](openapi-fragment.yaml) | REST: deployments, versions, routing, retirement, repin, insights, task heartbeats and completions, push endpoints and signing keys, warm policies, additive `StartRunRequest.dispatch`, `Worker` and `PlatformVersion` fields |
| [worker-protocol.md](worker-protocol.md) | Worker protocol minors `1.N`–`1.N+2`: deployment identity and dispatch facts, checkpoints, `DetachStep` |
| [workflow-ir.fragment.schema.json](workflow-ir.fragment.schema.json) | Additive IR: workflow `dispatch` defaults, node `dispatch`, `asyncCompletion`, `heartbeatSchema`, `continueAsNew.deployment`, `child.dispatch` |
| [push-envelope.md](push-envelope.md) | Push invocation envelope, signature, response contract and verifier vectors |
| [errors-and-events.md](errors-and-events.md) | Error reasons, durable failure codes, journal event additions, AsyncAPI alert kinds, task token format |

Compatibility rules applied: REST `/v1` additive (new optional properties and operations only); protobuf fields
added with new numbers in three consecutive minors, one per wire slice; IR fields optional and gated by enabled capabilities (ADR-0032); AsyncAPI
enumerations only grow.

Corrected 2026-11-24: the three minors are the next free ones of the register in `sdks/SPEC.md` §9 when the contract
merges (1.10, 1.11 and 1.12 at the time of writing); `RegisterWorkerResponse.deployment_version_state` is field 5
(field 4 is design 041's); each capability's surfaces are refused until its fleet requirement is recorded (ADR-0082,
research D019-001).
