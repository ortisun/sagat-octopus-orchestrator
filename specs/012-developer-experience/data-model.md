# Data model: developer experience

## Ownership and boundaries (T012-003)

This feature introduces no durable business entity. Workflow versions, runs,
journal events, timers and outbox records remain owned by their existing domains.
The CLI uses public REST/gRPC contracts and the platform IR codec; it never writes
PostgreSQL directly and never implements a local reducer or compensation scheduler.

| Type / artifact | Owner | Invariant |
|---|---|---|
| `Env`, invocation and common options | `apps/cli/src/{lib,args}.rs` | Token comes from the environment, never a command argument. Namespace does not replace server tenant authorization. |
| Workflow document | canonical 001 schema / shared IR codec | The same validation semantics apply locally and on the server. |
| Diagnostic | `apps/cli/src/diagnostics.rs` | Stable platform code, location and remediation; docs links use the shared catalog. Unknown remote codes must not invent semantics. |
| Template and mock answers | `apps/cli/src/{new,mocks}.rs` | Local fixtures, not durable state or production credentials. Mocks still run through the real worker protocol. |
| Version response | canonical OpenAPI `/v1/version` | Compatibility comes from the server protocol range, not a guessed release match. |

## Observable transitions

Commands parse configuration, validate input, call an existing service where needed,
and render its result. Only that service commits state, scheduling and outbox atomically.
A client timeout does not prove cancellation or that a remote mutation failed; inspect
server state and reuse supported idempotency keys before retrying external effects.
Published versions remain immutable and compensation/reconciliation follows 003.

Exit statuses already exported by `sagat_cli`: 0 done, 1 failed, 2 usage,
3 pending approval, 4 breaking change refused, 5 differences, 6 wait timeout.
A successful local validation does not establish permissions, available secrets,
script build completion, publication or workflow completion.

## Security, telemetry and persistence

Never persist bearer tokens, payloads or secrets in diagnostics. Server correlation,
typed errors, audit records and OTel remain the source of operational evidence.
Cross-tenant and redaction integration tests are still required before story closure.
No migration is introduced by this foundation; any later storage need requires a
specific ownership decision and additive migration tests.
