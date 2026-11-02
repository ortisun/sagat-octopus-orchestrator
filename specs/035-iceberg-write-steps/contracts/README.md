# Contracts of feature 035 (draft)

Design contracts; nothing here is implemented. At implementation each one is merged into the canonical
contract it extends, in the same delivery as the code (constitution, same-delivery rule).

| File | Extends | Canonical home after merge |
|---|---|---|
| [workflow-lake.schema.json](workflow-lake.schema.json) | workflow IR | `specs/001-platform-foundation/contracts/workflow.schema.json` |
| [connections.md](connections.md) | REST API | `specs/001-platform-foundation/contracts/openapi.yaml` |
| [worker-protocol.md](worker-protocol.md) | worker gRPC protocol | `specs/001-platform-foundation/contracts/worker.proto` |
| [receipts.schema.json](receipts.schema.json) | step outputs | the workflow IR's output schemas and SDK types |
| [errors.md](errors.md) | failure and error codes | `DatabaseFailure`, OpenAPI `Error.code`, `sdks/error-catalog.json` |
| [lake-runtime.md](lake-runtime.md) | runner ↔ Python host | `runners/lake/` (internal protocol `sagat.lake.v1`) |
| [destinations.md](destinations.md) | per-kind write semantics | `crates/infra-database`, `runners/lake/`, ADR-0025 pipeline |

Checks run on 2026-10-27: both JSON Schemas compile with ajv 8.17.1 in strict mode (draft 2020-12), and
fourteen IR samples and five receipt samples (including an injected extra field) are accepted or refused as intended (rerun after review round 1). The ajv used was
installed in a scratch directory, not the repository; the repository's own contract tests run when
these fragments are merged.
