# Tasks — 017 database connections

Status: All tasks implemented and locally verified on 2026-10-17 after user approval on 2026-10-16. Integrated with the feature-014 base of `develop` on 2026-10-24: every tick below was kept only after its named suites were rerun on the merged tree (see "Integration into develop" in verification.md); where a suite or constant changed, the dated note says so. Evidence and explicit deployment limits are in verification.md and traceability.md.

## A. Contracts and provider foundation

- [x] T017-001 Record approved architecture, dependency scope, role matrix and finite ceilings. FR-017-001..022. Files: spec, plan, tasks, ADR and tracking documents.
- [x] T017-002 Define async provider port, scoped provider references and safe typed failures; redaction/path tests. FR-017-005,006,008,020,021.
- [x] T017-003 Implement bounded OpenBao KV v2 HTTP adapter with token-file workload auth, TLS, CAS and receipt reads; real OpenBao integration tests. FR-017-005..008,020,022.
- [x] T017-004 Add provider intent/version metadata and migration with tenant FKs, audit and quotas. FR-017-007,009,019,021.
- [x] T017-005 Implement idempotent provisioning/rotation/reconciliation and fence legacy migration. FR-017-004..009. Evidence: real OpenBao durable intents/replay/migration plus bounded operator cleanup, lost retirement acknowledgment, stale cleanup lease, delayed CAS fencing and runtime-role current/grace/live/manual-review retention. Production scheduling/restore are covered by T017-023/024.
- [x] T017-006 Wire async resolution into existing secret consumers, respecting current authorization, lease and output scanning. FR-017-008,009,019. Evidence: real-service ordinary resolve/complete/fail allow heartbeat/cancellation during blocked provider reads, reject changed snapshots, replay durable reports during outage, and preserve secret-bearing compensation/reconciliation. Mixed local/OpenBao HMAC migration fails closed during outages; legacy secret/webhook/saga regressions pass.

## B. Reusable connection catalog

- [x] T017-007 Domain connection/operation/limits and validation tests. FR-017-001..003,010,011,021.
- [x] T017-008 Persist immutable revisions, tombstones, current state and run bindings, with tenant/retention constraints. FR-017-001..003,019. Evidence: immutable revision/run pins, tombstones, bounded catalog/revisions, final deletion at the revision ceiling and current-policy count tests.
- [x] T017-009 Add catalog authorization/service, safe audits and current secret checks; real store tests. FR-017-001..004,019,020. Evidence: current roles/ACLs, cross-tenant refusal, audited mutations, policy revocation fences and real PostgreSQL/OpenBao coverage.
- [x] T017-010 Extend canonical OpenAPI/error/capability contracts and management routes; request limits/idempotency/current-role tests. FR-017-001..010,019,021. Evidence: management/probe/ad hoc/result contracts, PostgreSQL driver discovery, quota ranges/scopes, generated Studio parity and real runtime-role HTTP checks.

## C. Durable managed execution

- [x] T017-011 Extend canonical IR and typed node metadata; reject reserved activity forgery; shared fixtures. FR-017-010..012. Evidence: infra-ir/database (6), domain/database_connections (4), full domain/IR regression and clippy. Managed leasing is covered by T017-013.
- [x] T017-012 Validate and pin connections in publish/admission, including replay and child/ad hoc paths. FR-017-002,003,015,019. Publication, promotion (including AlreadyActive), direct/child/event starts, reruns, receipt replay and rollback/ACL-race evidence implemented; ad hoc/probe admission, receipt replay, current-role/ACL revocation, private rerun and atomic retention rollback also passed T017-016 integration coverage.
- [x] T017-013 Add dedicated executor role, managed claim/capability fence and lease checks. FR-017-006,008,012,019. Evidence: real PostgreSQL/OpenBao + gRPC (protocol 1.5 when delivered; 1.6 since the 2026-10-24 integration, D600), current authority/session/fence races, immutable attempt pins and audit rollback. Result storage is covered by T017-017.
- [x] T017-014 Implement PostgreSQL adapter, destination pinning/TLS, restricted transactions, typed/bounded results and fixed probe. FR-017-011..014,016,021. Adapter implemented and tested with PostgreSQL/TLS, malicious frame, CIDR/port policy and log canaries; retained fixed probes now pass real isolated-runner success and wrong-credential failure scenarios, with revision-specific durable evidence.
- [x] T017-015 Wire isolated runner process to existing transport, cancellation, heartbeat and safe failure reporting. FR-017-012,018,020. Evidence: separate binary with real PostgreSQL/OpenBao/gRPC, typed/blob/write results, completion receipt replay, heartbeat cancellation, capability/egress denial, shared policy capacity, namespace fairness, OTLP/metrics and TRACE canaries; production engine content guards wired. Deployment network/resource enforcement and additional process-crash/restore scenarios are covered by T017-023/024.
- [x] T017-016 Add retained ad hoc/probe definitions and idempotent durable admission; failure/race tests. FR-017-014,015,018. Evidence: actor-scoped immutable deduplication, reuse at full quota, concurrent receipt replay, changed actor/body/origin conflicts, hidden publication surfaces, current role/ACL/rerun checks, precommit rollback across all records, probe metadata and real two-step input/output execution. See verification.md.
- [x] T017-017 Add audited bounded result disclosure and unknown-write handling; secret canary tests. FR-017-016..020. Evidence: retained-credential inline/blob guards, quota-to-manual-review, real PostgreSQL/OpenBao result reads/revocation/audit/corruption/timeout, native slow-reader deadline and schema parity. Production managed execution is wired by T017-015.

## D. SDK and Studio

- [x] T017-018 Add TypeScript and Go builders and shared conformance scenarios. FR-017-010,016. Evidence: shared typed fixture, 14 invalid authoring cases, SDK regressions, byte-identical canonical documents and real PostgreSQL/OpenBao/runner execution with exact results and downstream consumption.
- [x] T017-019 Add Python, C# and Java builders; execute all five against shared server scenarios. FR-017-010,016. Evidence: all five SDKs passed the database-connections scenario, including expression parameters, inline results, one write and refused DDL; independent assertions guard golden updates. Ordinary worker protocol remains 1.4.
- [x] T017-020 UI catalog and inline/standalone OpenBao secret create/rotate/select, with partial-save recovery. FR-017-001..009.
- [x] T017-021 UI database step editor, probe, ad hoc execution and permission-gated result viewer. FR-017-010,014..017,019.
- [x] T017-022 Real-browser happy/failure verification; accessibility and generated-client checks. SC-017-001,003,004,010.

## E. Delivery evidence

- [x] T017-023 Pinned development/production OpenBao and runner profiles, workload auth/bootstrap/network policies and demo. FR-017-006,013,022. Evidence: built images and full Compose success/outage demo; real three-member TLS Raft, projected-JWT auth/reauth, scoped ACLs and Calico socket refusals; rendered chart contracts. See verification.md.
- [x] T017-024 Fault/recovery/restore/rotation/migration tests and runbooks; update dependent specs and ADRs. SC-017-002,005..011. Evidence: real SIGKILL after target commit, lease expiry/manual review/restart without replay; quiesced PostgreSQL/Raft restore and persistent CAS fence; runtime-role binding-key replacement and prior migration/credential rotation/race tests. Recovery runbook and dependency reconciliation recorded. Final common-engine regression is recorded under T017-025.
- [x] T017-025 Run required regression, architecture, five SDK and browser gates; review diff and record actual evidence/remaining gaps. All requirements. Evidence: complete workspace run plus passing reruns of all five initially failing targets; all-five shared scenarios and SDK unit/fault suites; real browser, built-image recovery, audit/clippy/contracts/architecture and measured phase/resource reports. See the final gate section in verification.md for exact results, initial failures and exclusions.
