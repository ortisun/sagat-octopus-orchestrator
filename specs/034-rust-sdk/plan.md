# Plan — official Rust SDK

Base origin/develop@70f09435, branch feature/034-rust-sdk. Read spec/tasks before implementation.
Extend feature006 through a separate SDK package, generated canonical worker protocol and existing
common conformance harness. Avoid internal server crate dependencies in the public distribution.
Use Tokio/tonic/reqwest/serde at the wire boundary. Preserve stable errors, fresh credentials,
bounded IO/concurrency, no redirects, TLS verification, exact idempotency and trace propagation.
Keep public values typed while retaining raw JSON for additive server evolution.

Sequence: contract/spec → failing standalone consumer/error/client/protocol tests → first usable
REST slice → worker/heartbeat/report/stream → typed builders/capabilities → common conformance and
fault parity → package/clean consumer → six-language CI/docs/version guidance and review.
The root handles implementation; any reviewer gets a bounded read-only artifact and contract.

Constitution check: I/III/VII unchanged (no runtime/DB/broker authority); II pure SDK adapters and
no server dependency; IV add the sixth official language under the same versioned conformance;
V fresh scoped secrets/TLS/tenant isolation; VI safe trace/metric hooks and canonical errors;
VIII actual tests/faults, no fabricated parity. The compatibility language enumeration amendment
must carry a version bump and Sync Impact Report in its PR; it does not waive acceptance gates.
