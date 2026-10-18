# Reproduction and actual results

Source base c332bffc; add three native entry-barrier tests and child registration only.
PostgreSQL16.15 disposable loopback65117, isolated database per fixture; explicit sagat_app.
All1096 frozen executable/contract inputs match before/after. Root environment:

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual full family16/16:61.22s wall/25.97s harness; strict Clippy9.43s, fmt4.19s,
architecture0.29s/24crates. No failures in this unit. Complete logs/commands/UTC/exits retained.

Three new tests witness actual native Run entry, Busy rollback/released Run while G held,
unchanged-command success; duplicate exact commands through real Run blocker graph produce
one apply/one identical original-result replay; owner-labelled chain context witnesses chain
before Run and UnsupportedRunContext before actually denied source/receipt SELECT (42501),
with stale authority earlier. Source/capacity snapshots and counts assert preservation.
No sleep ordering, process/restart claim, production caller/grant or non-flat source closure.
Only local15a closes; remaining wait, source-writer, fault, recovery and observations stay open.
