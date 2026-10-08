# Canonical design contracts

- `openapi.yaml`: REST endpoints of the control plane and runs.
- `worker.proto`: gRPC for worker registration, leases, heartbeat, complete/fail.
- `events.asyncapi.yaml`: NATS event envelopes and integrations.
- `workflow.schema.json`: declarative IR validated by JSON Schema 2020-12.
- `schema.sql`: **illustrative SQL model** to guide reviewed migrations; do not use as a production migration without reviewing composite foreign keys/tenant isolation, indexes, partitioning and policies.

## Rules

1. These contracts are design proposals; they do not present themselves as working endpoints.
2. If the contracts change, update all SDKs, conformance tests and `docs/traceability.md`.
3. `workflow.schema.json` validates the basic format; extra semantic validation checks node uniqueness, references, cycles, branches, Saga scopes and fanout limit.
4. `worker.proto` requires mTLS authentication/worker session scoped via interceptor in addition to payload fields.
5. `openapi.yaml` does not allow an arbitrarily supplied tenant_id in the request; authorization resolves the tenant.
