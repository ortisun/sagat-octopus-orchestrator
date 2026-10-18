# Real collection finalization baselines

Production base `b32cea59`; only two test files changed. Owned PostgreSQL16.15 on loopback65117.
Environment: `CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target`,
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=3`, `RUST_MIN_STACK=16777216`,
`SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres`, runtime flag unset.
Execute each exact test name from its JSON command with `cargo test --locked -p sagat-tests
--test integration_workflow_primitives <name> -- --exact --nocapture --test-threads=1`.

- End semantics matrix: actual first omitted-output case loses the owner aggregate: None versus
  required [{amount:17}]. Exactly1 executed test failed,81.58s including compilation/2.97s harness.
  Literal-null, expression-error and oversized-array cases are later rows and have not executed yet.
  The oversized expression uses a two-element array; string concatenation would instead hit the
  evaluator's64KiB construction bound and is not a valid256KiB payload-limit fixture.
- Two actual empty root collections after16 real gRPC prefix outputs totaling4MiB-2: current Run
  incorrectly SUCCEEDED rather than FAILED/RUN_DATA_LIMIT. Exactly1 executed test failed,
  9.19s wall/7.29s harness. Later ledger/source/audit/journal/End assertions have not executed yet.

All1118 native inputs remain unchanged before/after both genuine failures. There are no fixture
compile or initialization failures in this selection. Both tests use real public publication,
start, worker reports and ordinary scheduler calls; SQL observes only committed results.

The second-root guard/test unit underwent one fresh bounded review, None, sealed patch
`969d410e9122f4215b4b8855abf8479ed0f373cc5b8a1f6ccfdbb55b0813ffbb`.
The End matrix C1 review identified the invalid concatenation fixture and root applied the
recommended array expression before running this baseline. These tests do not establish native
command replay, uncertain COMMIT, process restart, child fanout or earlier separate collection
expression-context acceptance. The existing scalar17 test reads its current collection owner,
not a prior distinct collection.
