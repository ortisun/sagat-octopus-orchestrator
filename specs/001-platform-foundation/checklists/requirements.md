# Requirements Quality Checklist — Platform foundation and contracts

**Purpose:** validate completeness and testability of the specification before development.

- [ ] Prioritized P1/P2 user stories have independent Given/When/Then scenarios.
- [ ] All FR-001-### are objective and traceable to tasks and tests.
- [ ] Errors, limits, timeout, retry, cancellation, concurrency and duplicates were evaluated.
- [ ] Tenant boundary, RBAC, secret redaction and applicable egress are specified.
- [ ] Versioning, migration and compatibility with live runs are defined.
- [ ] The journal and outbox preserve state after a crash, if applicable.
- [ ] API/OpenAPI, Proto and AsyncAPI were reviewed for impacts.
- [ ] SC-001-### criteria were validated with metrics or automated scenarios.
- [ ] The roadmap, ADRs and traceability match the design.
- [ ] Pending risks were recorded before merge.
