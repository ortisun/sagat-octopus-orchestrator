# Contracts of feature 020 (drafts)

Draft fragments, not canonical. The canonical contracts stay in `specs/001-platform-foundation/contracts/`
and change only when the corresponding capability ships, additively, in the same delivery as its code
and tests (constitution, same-delivery rule). Nothing here is implemented.

| Fragment | Canonical target | Merge task |
|---|---|---|
| [workflow-ir.fragment.schema.json](workflow-ir.fragment.schema.json) | `workflow.schema.json` (additive v1, ADR-0032) | T020-005 |
| [list-query.md](list-query.md) | OpenAPI descriptions of `searchRuns`/`countRuns`, `docs/reference/` | T020-012, T020-016 |
| [openapi.fragment.yaml](openapi.fragment.yaml) | `openapi.yaml` (`/v1`, additive) | T020-012, T020-023, T020-030, T020-042, T020-044 |
| [errors.md](errors.md) | `Error.details` description, `sdks/error-catalog.json` remediation refinements | T020-010, T020-012 |
| [analytics-model.schema.json](analytics-model.schema.json) | new canonical `analytics.schema.json` beside the others | T020-021 |
| [analytics-export-protocol.md](analytics-export-protocol.md) | `worker.proto` (next additive minor) and `worker.baseline.json` | T020-049 |
| [asyncapi.fragment.yaml](asyncapi.fragment.yaml) | `events.asyncapi.yaml` and `events.baseline.json` | T020-038 |
| [sinks.md](sinks.md) | `crates/infra-analytics` adapter contract, `docs/runbooks/analytics.md` | T020-026, T020-036..038 |
| [run-history.schema.json](run-history.schema.json) | new canonical `run-history.schema.json` | T020-042 |
| [run-archive.schema.json](run-archive.schema.json) | new canonical `run-archive.schema.json` | T020-045 |

Compatibility checks that must stay green: the OpenAPI lint and checksum tests, the AsyncAPI
compatibility test (`tests/contract/events-webhooks-messaging/asyncapi/compatibility.rs`), the worker
baseline (`worker.baseline.json`) and the IR fixture corpus of features 002, 014 and 016.
