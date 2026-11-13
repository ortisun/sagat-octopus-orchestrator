# Official Rust SDK — release0.3

Owner request2026-10-27: “Também senti falta do SDK para Rust”. Continue the active0.3 scope;
existing Redis033 and workflow-primitives014 work remains authorized and independent.
Dependencies006/001/002/003/004/014/016/017 reuse their current public contracts, not server internals.

US01: a Rust application starts, reads and commands workflows through an async typed REST client.
US02: a Tokio process registers typed activities and executes leased tasks through the canonical
worker gRPC protocol, including streaming, heartbeat, cancellation, compensation/reconciliation,
secret and blob capabilities, without implementing an orchestration runtime.
US03: Rust developers author the same declarative workflows and get the same canonical errors and
conformance outcomes as the existing five SDKs.

- FR-034-001: add an independent distributable Rust crate under sdks/rust with MIT licensing,
  locked dependencies, canonical embedded protocol/error catalog and a clean consumer example.
- FR-034-002: implement the operations and security/idempotency/retry/serialization contract in
  sdks/SPEC.md, with typed values retaining unknown/raw response fields; no redirects or token leaks.
- FR-034-003: expose an idiomatic Tokio WorkerHost/activity registry and ActivityContext, sharing
  the existing gRPC negotiation, lease/task identity, scoped path/capability, heartbeat/report,
  stream/poll/fallback, safe cancellation, secret lifetime and blob semantics.
- FR-034-004: provide side-effect-free typed workflow constructors for the current common IR,
  including existing primitives, operation/state/chain and database-connection capabilities.
  The server remains the execution authority; no direct PostgreSQL/NATS/Redis task consumption.
- FR-034-005: add Rust to the existing common scenario/fault/golden harness. No forked scenarios,
  echoed expected results or relaxed existing assertions; only genuinely unsupported server features
  retain their current shared release gates.
- FR-034-006: expose safe trace/metric hooks, fresh token providers, typed closed errors and tests
  for secrets/TLS/tenant boundaries, lost responses, stale leases, shutdown and backpressure.
- FR-034-007: update SDK version/package/CI/docs/constitution guidance in the same delivery.
  Keep historical five-language evidence intact; new six-language acceptance needs actual results.

SC-034-001: real Rust REST/gRPC/worker scenarios match the common reviewed goldens on real PG.
SC-034-002: protocol/catalog drift, canonical builders, error/security and fault gates pass.
SC-034-003: cargo fmt/clippy/tests/package plus a clean consumer pass with named command outputs;
SDK release artifacts include the Rust crate. Registry publication needs its own configured namespace.
SC-034-004: existing five-language conformance remains supported without server semantic changes.
