# Feature 017 delivery traceability

Commands and actual results are in [verification.md](verification.md). Historical checkpoint
notes describe that checkpoint; the final gate section records delivery status.

| Requirement / criterion | Automated evidence | Contract / operational evidence |
|---|---|---|
| FR-017-001..004; SC-017-001,005,008 | `integration_database_connections`: catalog/admission/policies; `e2e_database_connections`: catalog, partial save, probe and roles | Canonical OpenAPI connection/driver routes; immutable revisions and current ACLs in [data model](data-model.md); `database_catalog_writes_total` and safe mutation audit |
| FR-017-005..009; SC-017-002,006,008,011 | `sagat-infra-secrets` OpenBao integration; feature intent/migration/consumer/cleanup/key-rotation scenarios; Kubernetes auth and Compose restore fixtures | [Provider contract](contracts/openbao.md); `secret_provider_requests_total`, request/cleanup duration and audit; [recovery](../../docs/runbooks/database-recovery.md) |
| FR-017-010..012; SC-017-003,005 | IR/domain validation, shared `database-connections` scenario in `sdk_conformance` for all five languages; feature admission/executor/gRPC fixtures | Canonical workflow schema and protocol 1.5; [managed capability](contracts/managed-executor.md); `database_capabilities_total` / `database_capability_seconds` |
| FR-017-011..014; SC-017-009 | `sagat-infra-database` real PostgreSQL/TLS/framing/restricted-role tests; separate runner; actual Calico allow/deny sockets | [PostgreSQL adapter](contracts/postgres-executor.md); bounded network policy, verified TLS, `database_connection_seconds` and execution outcomes |
| FR-017-014..018; SC-017-004,007 | Ad hoc atomic admission/replay/probe fixtures; real browser reload/ambiguous acknowledgement; runner completion replay, cancellation, SIGKILL/restart; common saga/fault suites | Canonical ad hoc/probe/result OpenAPI; ordinary journal/task/outbox transaction; [usage](../../docs/guides/managed-database-executions.md); no automatic unknown-write retry |
| FR-017-016..021; SC-017-005,010 | Inline/blob retained-credential canaries; authorized result/denial/audit/corruption/timeout fixtures; native slow TCP reader; browser exact-number/response bounds; runner TRACE/OTLP canaries | Distinct result-read permission and no-store response; `database_output_scans_total`, `database_result_reads_total` and durations; bounded safe labels, identifiers only in spans/audit |
| FR-017-022; SC-017-011 | Built-image Compose happy/outage/coordinated restore; real three-member TLS Raft/projected JWT renewal and isolation; rendered Helm safety/rollout contracts | [Deployment](../../docs/runbooks/openbao.md); cleanup CronJob and dedicated identities; [recovery](../../docs/runbooks/database-recovery.md) |
| NFR-017-SEC,DUR,COMP,OBS | Tenant/ACL/current-role races; domain/journal assertions; all-five SDK parity; real OTLP exports and absent secret/SQL canaries | No domain adapter dependency; typed errors and audit; current limits at admission/claim/resolve/output |
| NFR-017-PERF | 10 warmups + 200 successful measured runs; standalone native 4 MiB slow-reader memory/permit probe | [Phase latency](../../tests/performance/reports/database-connections.md), [response memory](../../tests/performance/reports/database-result-memory.md); declared environment and exclusions |

## Shared semantics exercised by regression

The complete workspace includes `fault_sagas_compensations`, `integration_sagas_compensations`,
`integration_durable_execution`, `fault_events_webhooks_messaging`, secret/tenant security,
publication/promotion, blob storage/retention and contract suites. These guard the existing
timeout/signal, failed compensation, duplicate/lost delivery and restart semantics reused by
managed steps. The dedicated SIGKILL fixture adds the customer-commit/completion boundary.

## Explicit limits

- PostgreSQL is the only advertised driver. Kafka targets and dynamic account provisioning
  remain outside this feature. General script workers receive no managed database capability.
- Fresh target sessions are bounded by distributed leases and local capacity; no warm credential
  cache or cross-step transaction is promised. The customer must grant a restricted target role.
- Retained outputs are bounded, validated and audited. Blob results are descriptors for downstream
  expressions; they are not silently expanded. The UI exposes no generic payload reader/export.
- Recovery proof uses a coordinated, quiesced same-cluster snapshot. It does not reverse customer
  effects or validate unrelated PITR points, fresh-cluster force restore or loss of unseal keys.
- Production installation still requires the operator's image digests, OIDC issuer, certificates,
  storage class, private routes and backup custody. Local fixtures do not supply production secrets.
