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
- [ ] T034-003 Generate/check canonical worker protocol and implement public Tokio host/activity
  registration, task context, heartbeat/report/fencing/cancellation and poll/stream negotiation.
- [ ] T034-004 Implement secret/blob/scoped/compensation/reconciliation capability lifetimes and
  safe observability; prove failures against the real server and deterministic transport barriers.
- [x] T034-005 Implement typed declarative builders and canonical serialization through the
  existing shared authoring scenarios/goldens; reject the same invalid documents.
  Evidence: `public_builders_match_existing_shared_programs_and_reviewed_goldens` → all 60
  shared authoring fixtures, canonical texts and errors match; `rust-sdk-builder-final-tests`
  → 102 passed; strict Clippy → exit 0. Runtime conformance remains T034-006.
- [ ] T034-006 Add Rust conformance runner and execute the common runtime/fault/golden scenarios
  with real PostgreSQL, preserving existing five SDKs and server semantics.
- [ ] T034-007 Add package/clean-consumer/CI/version/runtime-matrix/readme examples; run format,
  clippy/tests/conformance/package and record actual evidence before acceptance or release.
