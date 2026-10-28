# Tasks — Redis Streams, release0.3.0

- [x] T033-001 Record owner scope, ADR-0069, contract/retention/security boundaries and dependency mapping.
  Verify: spec/plan/ADR agree with selected owner option; no existing feature/id is overwritten.
- [x] T033-002 Add failing internal Redis adapter/configuration cases; implement async XADD publisher
  and typed selectable engine transport. Verify real Redis output, bounds, redaction and NATS regressions.
- [x] T033-003 Add failing Redis callback destination/domain/API/store cases; implement BrokerKind,
  bounded settings, canonical contracts and additive guarded migration. Verify tenant/runtime-role lifecycle.
- [x] T033-004 Extend guarded outbound publishing with current namespace secret/TLS/egress handling.
  Verify real acknowledged delivery, exact metadata/body, denied/invalid/wrong-type failures and no secret logs.
- [x] T033-005 Prove ambiguous ACK duplicate identity, outage/restart, retry/DLQ/redrive, fencing and
  existing HTTP callback behavior using disposable fixtures and deterministic barriers.
- [x] T033-006 Add pinned Compose Redis/AOF/healthcheck, deployment selection and operator examples;
  regenerate/check Studio types and canonical REST/AsyncAPI contracts.
- [ ] T033-007 Run appropriate format/lint/static/contract/integration gates; update validation report,
  traceability, roadmap and release0.3 dependency evidence. Tick only obtained acceptance.


Evidence for T033-001..006: [validation report](validation-report.md), actual gates dated2026-10-27.
T033-002:27 engine configuration cases plus28 Redis cases and111 existing event regressions.
T033-003: public API/schema/tenant/runtime-role lifecycle and guarded migration cases.
T033-004: real TLS/ACL/rotation/DB/pinned-DNS/secret/boot cases in tls.rs/bootstrap.rs.
T033-005: actual lost wire ACK, crash/stale publisher, outage/DEAD/redrive and111 legacy cases.
T033-006: actual Compose Healthy/AOF/loopback proof and teardown;25 Helm contracts; Studio type drift,
typecheck/lint and canonical REST/AsyncAPI contracts.
T033-007: actual old callback writer/reader, login/session revocation, process join before Redis
activation and preserved old rows now pass; six source-bound released profiles, 139 messaging/
Redis regressions and five common scenarios in six SDKs pass on the combined tree. Evidence:
[released-callback-cutover.md](released-callback-cutover.md). The final release-tree gates remain
open; no acceptance is inferred from the default-off flag.
