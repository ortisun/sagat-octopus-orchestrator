# Native stale report lock orders

Base aab93b70. Exact command/status/time in commands.json, tested source hashes in source-first.json.
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. SAGAT_TEST_DATABASE_URL and
SAGAT_TEST_AS_RUNTIME_ROLE unset; Flow explicitly starts actual HTTP/gRPC/native stores as
sagat_app. The disposable owner grants dormant INSERT only in this database.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::late_reports:: -- --nocapture --test-threads=1. First1/1 80.15s
includes compilation65s and actual harness12.15s. The single test contains4fresh Flow scenarios.
Complete and fail reports both use keys new relative to the original completion. Each leads once
and follows once in the actual original-Run lock queue, observed through pg_blocking_pids and
exact query fragments. No producer bypassed the held Run. Both futures complete after release.
Every late report returns Aborted/STALE_ATTEMPT. Canonical first/replay behavior and exact counts,
selected-source/run/journal preservation, and real execution replay verification are asserted.

This is new race coverage around unchanged production logic, not an implementation RED claim.
No Bao/S3/SDK/broker/process restart or other producer preservation is established. Parent
B1b.2a.1d.3b and canonical feature014 tasks remain open; release INSERT/callers remain gated.

Strict targeted integration Clippy15.45s, fmt4.77s and architecture24crates0.66s passed on the same
tested Rust sources. No optional repeat of the unchanged production workspace/privilege gates
is represented as fresh execution. See prior native increment for those gates.
