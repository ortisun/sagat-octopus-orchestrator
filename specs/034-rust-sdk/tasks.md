# Tasks — Rust SDK

- [x] T034-001 Record scope, ADR-0070, constitution impact and public package/API contracts.
  Evidence: ADR-0070, Sync Impact Report, spec/plan/contracts; `python3 scripts/check-architecture.py`
  → 24 crates/layers/no cycles; `python3 scripts/check-agent-config.py` → agent configuration ok
  (`rust-sdk-foundation-architecture` / `rust-sdk-foundation-agent-config` receipts).
  Constitutional amendment acceptance still requires its delivery PR; SDK completion is not claimed.
- [x] T034-002 Add failing standalone client/error/security cases and implement typed REST calls,
  idempotency/safe retry, trace/cancellation/pagination, fresh token provider and exact JSON values.
  Evidence: `rust-sdk-boundary-tests` → 107 passed, including 54 REST/catalog cases; strict
  core Clippy exit 0. Shared real-PG run-lifecycle, state-operations, errors and lost-responses
  passed with unchanged goldens. Full matrix acceptance remains T034-006.
- [x] T034-003 Generate/check canonical worker protocol and implement public Tokio host/activity
  registration, task context, heartbeat/report/fencing/cancellation and poll/stream negotiation.
  Evidence: `rust-sdk-complete-shared-suite-final` → all 60 common scenarios passed on real PG;
  `rust-sdk-scoped-flight-key-red`/green barrier regression plus 108 native tests; stream and
  poll transport faults, protocol negotiation, restart, shutdown and cancellation all passed.
- [x] T034-004 Implement secret/blob/scoped/compensation/reconciliation capability lifetimes and
  safe observability; prove failures against the real server and deterministic transport barriers.
  Evidence: all 60 shared real-PG scenarios passed; six SDK user-story tests and nine fault
  tests passed; native lazy-secret/revocation, verified blob EOF and bounded frame tests passed.
  `rust-sdk-grpc-tls-native` passed CA/name/mTLS acceptance and refusal cases, alongside REST TLS tests.
- [x] T034-005 Implement typed declarative builders and canonical serialization through the
  existing shared authoring scenarios/goldens; reject the same invalid documents.
  Evidence: `public_builders_match_existing_shared_programs_and_reviewed_goldens` → all 60
  shared authoring fixtures, canonical texts and errors match; `rust-sdk-builder-final-tests`
  → 102 passed; strict Clippy → exit 0. Runtime conformance remains T034-006.
- [x] T034-006 Add Rust conformance runner and execute the common runtime/fault/golden scenarios
  with real PostgreSQL, preserving existing five SDKs and server semantics.
  Evidence: `rust-sdk-complete-shared-suite-final` exit 0 (213.49 s), all 60 unchanged goldens;
  `rust-sdk-user-stories-and-faults` exit 0 (126.11 s), 6 e2e + 9 fault cases. Existing SDK
  native/type/style gates all passed (`rust-sdk-regression-*`); no backend semantic change.
  The integrated six-language release run remains a release gate, not claimed by this Rust-only receipt.
- [x] T034-007 Add package/clean-consumer/CI/version/runtime-matrix/readme examples; run format,
  clippy/tests/conformance/package and record actual evidence before acceptance or release.

  Evidence: `rust-sdk-committed-artifact-consumer` exit 0 (12.24 s), immutable SDK source
  cefbde24, `.crate` SHA256 `1073709e5a5d93c01e58a1c4c278e0e847e5c62db5e128de5b7120ce5d1c359a`;
  extracted-artifact consumer resolves once and runs again --locked. SDK/runner strict Clippy,
  Rust unit/TLS suites, all 60 Rust common scenarios, e2e/fault suites and existing-five native
  gates passed. CI/package/compatibility docs updated; actionlint/bash syntax/architecture pass.
  SDK dependency audit exit 0 (227 dependencies). Product release integration, its all-six live
  run, feature 014 acceptance and Redis 033 cold-cutover remain separate release work.
