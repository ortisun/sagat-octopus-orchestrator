# Completion work

- [x] Reconcile stale gap records with integrated implementation and test evidence.
- [x] T001-036: Generate a distinct server request id; return it on success and errors, correlate logs, update OpenAPI and verify repeated inbound trace ids do not reuse it.
- [x] T001-037: Add optional idempotency keys to workspace/namespace creation; prove replay, mismatched payload, concurrency, audit atomicity and tenant isolation.
- [x] T010-037: Supervise all critical engine loops; prove unexpected exit/panic stops service and requested shutdown stays clean.
- [x] T010-038: Execute complete product Helm deployment and KEDA scale/recovery in a disposable cluster, record reproducible evidence.
- [x] Classify remaining requirements from the independent architectural audit and implement the approved V1 slices.
- [x] Run integrated gates and independent consequential-change review; see the recorded results below. Final PR CI remains the merge gate.
- [x] Prepare release integration with the verified completion changes in [PR #22](https://github.com/ortisun/sagat-octopus-orchestrator/pull/22), targeting `develop`; readiness follows final CI.

## Remaining written requirements (independent architectural audit)

The checklist below tracks the additional review findings and their completion evidence.
Accepted limits (offline dependency catalog, no physical exactly-once, destructive retention
without a policy) are not silently expanded or reclassified as implemented.

- [x] FR-002-008: lease/tenant-scoped blob references, server inputs/results and five-SDK conformance; carefully port the existing draft without breaking protocol 1.2.
- [x] FR-009-006: exact storage/log-byte, retention-ceiling and event-rate admission quotas with concurrent/crash tests; final activation migration passed enabled-by-default durable/security/script suites (176/95/54) and independent review.
- [x] FR-010-002: per-activity concurrency quotas with fair retry and tenant isolation.
- [x] FR-004-004: OIDC/mTLS webhook authentication profiles with replay/SSRF/failure tests.
- [x] FR-004-005: durable outgoing NATS/Kafka/SQS deliveries with stable idempotency keys, ACK ambiguity and DLQ/redrive tests.
- [x] NFR-006-OBS: real opt-in OTel integration for all five SDKs, redacted failures, trace propagation and unavailable-exporter tests.
- [x] FR-006-008: reproducible package artifacts and install smoke tests; registry publication remains conditional on namespace and credentials (MIT selected by owner).
- [x] FR-007-002: schema-aware workflow input/output editing.
- [x] FR-007-005: authorized, audited `structure-only-v1` payload inspection through REST and Studio; real PostgreSQL/gRPC/blob E2E 2/2 and keyboard Chromium journey 1/1 passed. All values and keys remain hidden.
- [x] FR-005-003: isolated Go and Java build/runtime runners with signed artifacts and conformance/fault tests.
- [ ] SC-008-001 / SC-012-001: agree the numeric UX target and record real human evaluation; never substitute an automated timing claim.
- [x] Owner selected MIT; apply it to the repository and distributable SDK packages.
- [x] Record the owner's explicit ratification of constitution v1.0.0 on 2026-10-11.

## Latest verification checkpoints

- `ee2063a`: output blobs, bounded poll serialization and exact activity admission; per-feature task files contain real PostgreSQL/gRPC evidence.
- `4aeb618`: Go/Java artifact-v3 runners; native AMD64 and ARM64 CI jobs passed on run 38200759888. Full native fault checks are now included in both jobs.
- `64f1532`: schema viewer mutation regression and Clippy assertion corrected; full Studio unit suite subsequently passed 1035/1035.
- Working webhook profiles: real PostgreSQL ingress 26/26, migration suite 5/5, HTTP contract 2/2, HTTP unit/real OIDC suite 8/8 and scoped Clippy pass; independent review subsequently closed as recorded below.

- Native CI run 38201472580: all five Go/Java production and resource tests passed on both AMD64 and ARM64.
- Working integrated server: all workspace targets compile; generated Studio OpenAPI types and typecheck pass; HTTP shutdown fault suite 9/9 and real TLS/OIDC suite 8/8 pass.
- Activity concurrency: deterministic ABBA/fence regressions 5/5 and existing concurrency suite 12/12 pass, including capped-backlog candidate starvation.
- Outbound brokers: five real broker/fault/boundary/rollback tests pass; final read-only review found no outstanding blocker.
- Human acceptance protocol and blank results are in `docs/validation/`; no human outcome is claimed.

- Webhook security review closed: admission rechecks the endpoint under lock after verification; pause/replace/delete races 3/3 and full ingress 29/29 pass, with scoped Clippy.

- SDK blob/input completion: real five-language round trip and early-response refusal pass; all package clean-consumer checks pass. Mixed protocol 1.2/1.3 admission tests pass 3/3 and rollback tests 2/2.

- Latest Studio integration: 1,045 unit tests, lint, typecheck and production build passed; generated OpenAPI types and reference documentation refreshed.
- Durable quota lock measurement: 2,000 committed operations across hot single-tenant and two-tenant cases, exact byte accounting; report `tests/performance/reports/durable-quotas.md`.

- Final five-SDK verification: 18 common scenarios × five languages (90 executions), E2E 6/6 and injected-fault suite 9/9 pass, including protocol 1.3 and older-minor compatibility.
- SDK distribution CI [38210316160](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38210316160) passes after the Go telemetry-test formatting correction.

- Completion regressions: Go/C# formatting and C# span-finalization test ordering corrected; saga query-plan fixture now supplies protocol minor 1.3 (74/74 integration tests pass). New REST blob/payload scenarios and genuine leased-script provenance pass all three contract targets (2/1/1).
- Studio initial run viewport now accounts for the visible canvas area; 3 geometry tests and unchanged light/dark/forced-color browser assertions pass. Linux CI [38211400544](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38211400544) passes the full Studio job, native AMD64/ARM64 runners, compose smoke and scale/recovery checks.
- Local fresh-container resource profile is recorded in `docs/test-strategy.md`; two test threads avoid unrelated fixture pool exhaustion without reducing concurrency within a test.

- Workspace validation with the documented local profile exercised 2,224 runnable tests: 2,223 passed and one telemetry fixture failed before correction; 47 opt-in tests/benchmarks were ignored by this command. The fixture stopped its exporter before the required request span closed. After correcting that ordering, the complete affected target passes 26/26 both with two threads and default parallelism, and its focused regression passes 1/1. Production telemetry is unchanged. The earlier unrestricted run and all failure logs are preserved locally; no failing run is reported as an all-green run.
- Final full-workspace Clippy with warnings denied and formatting pass; the corrected telemetry target also passes scoped Clippy. Five-language opt-in SDK conformance/E2E/fault and native architecture gates run separately in CI; standalone performance benchmarks retain their explicit opt-in profile.
- Independent consequential review is closed for activated durable byte quotas, runtime recovery, blob deletion claims, retention/provenance and audited structural payload inspection. Remaining human acceptance is the unchecked SC item above; registry publication still requires its namespace/credentials.
