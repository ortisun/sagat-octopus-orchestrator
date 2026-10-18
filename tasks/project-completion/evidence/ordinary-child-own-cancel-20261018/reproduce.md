# Reproduce

Use the repository-owned disposable PostgreSQL 16.15 server. This run used SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres and a separate fresh database per fixture. Runtime pools execute as sagat_app, including default mode.

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216.

Run the exact commands in same-commit-compensation-default.json with SAGAT_TEST_AS_RUNTIME_ROLE unset, then runtime-own-cancel.json with SAGAT_TEST_AS_RUNTIME_ROLE=1. Each invocation selects the same two distinct own_cancel tests (not four distinct scenarios). Both emit two three-PID COMMIT witnesses. Final 1108 input hashes equal before/after; early compile/oracle failures have separate pins.

initial-default: compile failure, no tests. amended-default: incorrect filter, zero tests, not acceptance. corrected-filter-default: actual two tests failed on event-name oracle. completed-event-default: two tests failed on invented compensation transaction boundary. diagnose-suffix-default: one selected test failed and exposed actual two-event suffix. All retained honestly; final passes follow explicit fixes, not unexplained retries. The independent collection_end_result tests remain genuine RED and are not included in these passing selections.
