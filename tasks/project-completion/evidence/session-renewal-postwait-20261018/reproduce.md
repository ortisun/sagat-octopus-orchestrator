# Session renewal after unchanged-row wait

Base68857f2e. Use repository disposable PostgreSQL, unset SAGAT_TEST_DATABASE_URL.
Set SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3,
RUST_MIN_STACK=16777216; optional shared warm CARGO_TARGET_DIR. Exact commands/exits/elapsed
wall times in commands.json/regression-commands.json/gates.json; test times in raw logs.

red-source.json/test-red.diff preserve baseline production+new test. RED0/1 (5.10s) actually
renews expired session and leases1 task, no fixture/compile failure. Apply production.diff to
get GREEN1/1 (5.82s), including expired refusal/no-effects and fresh registration success.
The runtime flag selects World store pool and actual waiter role is checked. A three-second
persisted TTL crosses expiry while another transaction locks but never changes the row.
Postlock DB clock is sampled in renewal TX; original UPDATE predicate remains an additional
check. Missing/expired returns without renewal; record parse and renewal commit together.

Security126/12681.09s, Clippy80.91s, fmt/architecture/diff passed. No old TTL policy or wire
change. Claim-room/database session checks remain separate and pending. B0/group/full-release/
SDK58/resource gates remain open; no canonical closure. No telemetrycounter assertion.
