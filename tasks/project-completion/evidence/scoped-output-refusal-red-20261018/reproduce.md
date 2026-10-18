# Known-success scoped output refusal: actual baseline failures

Production base: `52d2c5723b7c1d98fff2ef8aa5c9f5137585206a`. Only test code was added.
The PostgreSQL server was the owned PostgreSQL 16.15 container on loopback port 65117.
`Flow` always uses the real `sagat_app` execution role, even with
`SAGAT_TEST_AS_RUNTIME_ROLE` unset. These are live HTTP/gRPC/scheduler tests.

Environment:

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
unset SAGAT_TEST_AS_RUNTIME_ROLE
cargo test --locked -p sagat-tests --test integration_workflow_primitives \
  byte_ceilings::a_known_successful_seventeenth_member_is_preserved_without_its_refused_output \
  -- --exact --nocapture --test-threads=1
cargo test --locked -p sagat-tests --test integration_workflow_primitives \
  byte_ceilings::an_applied_lookup_preserves_both_real_successes_without_the_substitute_output \
  -- --exact --nocapture --test-threads=1
```

Each scenario creates an actual root collection with 17 sequential members. The first 16
successful activity outputs are exactly 250,000 JSON bytes each. The seventeenth successful
report would cross the 4 MiB run retention limit while remaining below the per-report limit.
The Applied scenario first reports the actual forward outcome unknown, then reports the genuine
reconciliation verdict through the worker API.

Both tests execute and fail on the public acknowledgement: actual `FAILED`, required `SUCCEEDED`.
The forward failure took 61.41 s including compilation; the Applied failure took 43.45 s including
recompilation. Each log reports exactly one failed test, zero passed. Their subsequent assertions
for atomic receipt/audit/lease, replay and real reverse compensation have **not yet executed**.
They are acceptance obligations for the correction, not established guarantees.

The initial Applied attempt failed earlier because its generated workflow slug exceeded 63
characters. That fixture error is retained separately as `applied-fixture-name-error.*`; it is
not evidence of the output-refusal defect. The fixture prefix was shortened, then the genuine
failure above was observed. Initial and final native-source snapshots each preserve 1,117 inputs
within their respective runs. The forward baseline belongs to the initial snapshots; the final
snapshots cover the corrected Applied fixture. No production source changed between them.

`Flow.verify()` in these tests reconstructs the journal; it does not restart an OS process.
Historical missing-owner accounting, provider secrets, interruption and uncertain-COMMIT cases
remain separate acceptance gates.
