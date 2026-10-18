# Reproduction and actual results

Source base 2426083b. Root actually executed the commands below on disposable PostgreSQL16.15,
loopback65117, each fixture's own database and explicit sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish::shard_expiry:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual focused unchanged-row shard expiry controls 2/2 in44.33s wall (8.99s harness); strict integration Clippy9.31s, fmt4.2s, architecture0.28s/24crates. All1097 frozen executable/contract inputs match.

Two tests use real renewal to shorten the current lease, hold its committed tuple unchanged, observe the native shard-row wait while still live, and prove original Run/G guards retained with NOWAIT probes. Database-clock expiry occurs before releasing that unchanged row. Both first application and exact receipt replay return SchedulerFenceLost with exact source/capacity snapshots and no extra effects. Actual fresh owner/generation changes only renewable command authority; the unchanged logical fingerprint applies once or returns the exact original receipt.

No production implementation changed. Previous complete scoped16/16 result remains separately scoped to2426083b; this run is focused2/2, not an aggregate18-test pass. Remaining15c..e,16/17 and production/C0/nesting/canonical runtime/release gates stay open. No sleep ordering, forced expiry UPDATE during wait, or non-flat source claim.
