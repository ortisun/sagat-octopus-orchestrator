# Contracts of feature 031 (drafts)

Nothing here is deployed or canonical. The canonical contracts stay in `specs/001-platform-foundation/contracts/` and are
changed only by the merge tasks of [tasks.md](../tasks.md) in the implementation delivery, with their compatibility
baselines regenerated deliberately and reviewed.

| Fragment | Canonical target | Merge task |
|---|---|---|
| [envelope.md](envelope.md), [envelope.schema.json](envelope.schema.json) | new `specs/001-platform-foundation/contracts/payload-envelope.schema.json`; referenced from `openapi.yaml` and `sdks/SPEC.md` | T031-005 |
| [encoded-field-registry.md](encoded-field-registry.md) | stays in this feature as the cross-draft registry (coordinator C7); each draft cites it, and its rows move into `docs/reference/workflow-document.md` when merged | T031-020 |
| [workflow-encoding.md](workflow-encoding.md) | `workflow.schema.json` (description of the annotation on the carriers; no structural change), `docs/reference/workflow-document.md` | T031-006 |
| [worker-protocol.md](worker-protocol.md) | `worker.proto`, `worker.baseline.json` | T031-007 |
| [openapi-fragment.yaml](openapi-fragment.yaml) | `openapi.yaml`, `frontend/apps/studio/src/api/schema.d.ts` | T031-008 |
| [errors.md](errors.md) | `openapi.yaml` (`details.reason`), `sdks/error-catalog.json` and the five embedded copies | T031-009 |
| [sdk-codec.md](sdk-codec.md) | `sdks/SPEC.md` (new section), `tests/contract/sdk-conformance/scenarios/` | T031-033 |
| [codec-server.openapi.yaml](codec-server.openapi.yaml), [codec-assertion.md](codec-assertion.md) | new `specs/001-platform-foundation/contracts/codec-server.openapi.yaml`; `docs/guides/client-side-encryption.md` | T031-050 |

`events.asyncapi.yaml` does not change: outgoing webhooks and NATS hints carry no payload values, and an event document
that wakes a wait carries the event data as it was published (envelopes included, if the publisher encoded them).

Compatibility rules applied: IR v1 grows by an annotation (ADR-0032); OpenAPI `/v1` grows by operations, optional fields
and enum values, and the new disclosure node kind is returned only on opt-in (`includeEnvelopes`); the worker protocol
grows by optional fields in a new minor; the envelope is a new, versioned format whose v1 (with its `ctx` binding) is
never changed.
