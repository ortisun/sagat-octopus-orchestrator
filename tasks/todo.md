# Completion work

- [x] Reconcile stale gap records with integrated implementation and test evidence.
- [x] T001-036: Generate a distinct server request id; return it on success and errors, correlate logs, update OpenAPI and verify repeated inbound trace ids do not reuse it.
- [x] T001-037: Add optional idempotency keys to workspace/namespace creation; prove replay, mismatched payload, concurrency, audit atomicity and tenant isolation.
- [x] T010-037: Supervise all critical engine loops; prove unexpected exit/panic stops service and requested shutdown stays clean.
- [x] T010-038: Execute complete product Helm deployment and KEDA scale/recovery in a disposable cluster, record reproducible evidence.
- [ ] Classify remaining requirements from the independent architectural audit and implement the approved V1 slices.
- [ ] Run integrated gates and independent consequential-change review.
- [ ] Prepare release integration with the verified completion changes.

## Remaining written requirements (independent architectural audit)

These remain open even though the historical implementation tasks are checked.
Accepted limits (offline dependency catalog, no physical exactly-once, destructive retention
without a policy) are not silently expanded or reclassified as implemented.

- [x] FR-002-008: lease/tenant-scoped blob references, server inputs/results and five-SDK conformance; carefully port the existing draft without breaking protocol 1.2.
- [ ] FR-009-006: exact storage/log-byte, retention-ceiling and event-rate admission quotas with concurrent/crash tests.
- [x] FR-010-002: per-activity concurrency quotas with fair retry and tenant isolation.
- [x] FR-004-004: OIDC/mTLS webhook authentication profiles with replay/SSRF/failure tests.
- [x] FR-004-005: durable outgoing NATS/Kafka/SQS deliveries with stable idempotency keys, ACK ambiguity and DLQ/redrive tests.
- [x] NFR-006-OBS: real opt-in OTel integration for all five SDKs, redacted failures, trace propagation and unavailable-exporter tests.
- [x] FR-006-008: reproducible package artifacts and install smoke tests; registry publication remains conditional on namespace and credentials (MIT selected by owner).
- [x] FR-007-002: schema-aware workflow input/output editing.
- [ ] FR-007-005: authorized redacted payload inspection through an appropriate read API and Studio view.
- [x] FR-005-003: isolated Go and Java build/runtime runners with signed artifacts and conformance/fault tests.
- [ ] SC-008-001 / SC-012-001: agree the numeric UX target and record real human evaluation; never substitute an automated timing claim.
- [x] Owner selected MIT; apply it to the repository and distributable SDK packages.
- [x] Record the owner's explicit ratification of constitution v1.0.0 on 2026-10-11.

## Latest verification checkpoints

- `ee2063a`: output blobs, bounded poll serialization and exact activity admission; per-feature task files contain real PostgreSQL/gRPC evidence.
- `4aeb618`: Go/Java artifact-v3 runners; native AMD64 and ARM64 CI jobs passed on run 38200759888. Full native fault checks are now included in both jobs.
- `64f1532`: schema viewer mutation regression and Clippy assertion corrected; full Studio unit suite subsequently passed 1035/1035.
- Working webhook profiles: real PostgreSQL ingress 26/26, migration suite 5/5, HTTP contract 2/2, HTTP unit/real OIDC suite 8/8 and scoped Clippy pass; independent review pending.

- Native CI run 38201472580: all five Go/Java production and resource tests passed on both AMD64 and ARM64.
- Working integrated server: all workspace targets compile; generated Studio OpenAPI types and typecheck pass; HTTP shutdown fault suite 9/9 and real TLS/OIDC suite 8/8 pass.
- Activity concurrency: deterministic ABBA/fence regressions 5/5 and existing concurrency suite 12/12 pass, including capped-backlog candidate starvation.
- Outbound brokers: five real broker/fault/boundary/rollback tests pass; final read-only review found no outstanding blocker.
- Human acceptance protocol and blank results are in `docs/validation/`; no human outcome is claimed.

- Webhook security review closed: admission rechecks the endpoint under lock after verification; pause/replace/delete races 3/3 and full ingress 29/29 pass, with scoped Clippy.

- SDK blob/input completion: real five-language round trip and early-response refusal pass; all package clean-consumer checks pass. Mixed protocol 1.2/1.3 admission tests pass 3/3 and rollback tests 2/2.
