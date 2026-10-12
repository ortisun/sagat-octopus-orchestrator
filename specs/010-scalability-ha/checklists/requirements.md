# Requirements Quality Checklist — Performance, scalability and high availability

**Purpose:** validate completeness and testability of the specification before development.

- [x] Prioritized P1/P2 user stories have independent Given/When/Then scenarios.
- [x] All FR-010-### are objective and traceable to tasks and tests.
- [x] Errors, limits, timeout, retry, cancellation, concurrency and duplicates were evaluated.
- [x] Tenant boundary, RBAC, secret redaction and applicable egress are specified.
- [x] Versioning, migration and compatibility with live runs are defined.
- [x] The journal and outbox preserve state after a crash, if applicable.
- [x] API/OpenAPI, Proto and AsyncAPI were reviewed for impacts.
- [x] SC-010-### criteria have been validated with metrics or automated scenarios.
- [x] The roadmap, ADRs and traceability match the design.
- [x] Pending risks were recorded before merge.

Notes on the ones that are only partly true (see `research.md`): the feature adds no REST, gRPC or AsyncAPI operation (the contract
is the chart, the configuration and the metrics, checked by `contract_scalability_ha`); the tenant boundary is the scheduler's (no
candidate query returns a payload, no metric carries a tenant, a run or an activity) and the weights are the operator's, not a
tenant's; live runs are not touched (an engine of the previous version works alongside the new ones: D243); the journal and the
outbox are the platform's, divided by shard; **the chart has been rendered and never installed in a cluster (G247), the benchmark is
one machine on one day (G249) and the drill is PostgreSQL only (G250, G251)**; the pending risks are G239–G264.
