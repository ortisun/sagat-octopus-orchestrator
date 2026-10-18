# Native collection data-limit writer and closed observations

Base: `3fbb7b3c` plus the reviewed native C2 tests and finite telemetry increment. The native
producer remains uncalled by the production scheduler in this snapshot. All1118 native inputs
match before/after the corrected tests and subsequent static gates.

## Commands and environment

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
unset SAGAT_TEST_AS_RUNTIME_ROLE
cargo test --locked -p sagat-tests --test integration_workflow_primitives \
  collection_data_limit_native -- --nocapture --test-threads=1
export SAGAT_TEST_AS_RUNTIME_ROLE=1
# Repeat the exact native selection above.
cargo test --locked -p sagat-telemetry --test primitive_vocabulary
cargo test --locked -p sagat-application --test primitive_telemetry
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo clippy --locked -p sagat-telemetry -p sagat-application --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Owned PostgreSQL16.15, container `sagat-project-completion-scoped-schema`, loopback65117.
Native drain/checkpoint calls explicitly use `runtime_pool_with`; thus both environment passes
exercise the actual `sagat_app` native writer. The flag selects other fixture execution-store pools;
it does not make the first native pass a database-owner writer test.

## Actual results

- Native default selection: **4/4**,63.94s wall including49.24s compilation,11.17s harness.
- Runtime flag selection: **4/4**,13.67s wall/12.78s harness.
- Closed metric vocabulary/export: **5/5**,13.04s including compilation/0.20s harness.
- Event-derived span/observation: **6/6**,9.13s including compilation/0.00s harness.
- Strict integration Clippy:29.35s; application/telemetry all-target Clippy:13.99s.
- Formatting:4.66s; architecture:0.51s. All passed.

Native source is created by real HTTP/gRPC/scheduler prefixes, then a test-only typed native
command starts the drain. A real empty aggregate at4MiB produces atomic FAILED/RUN_DATA_LIMIT
stop, DRAINING projection, journal, existing public FAILED hint, fixed audit and telemetry head.
Cold store-object replay retains its original receipt/sequence with exact unchanged durable
snapshots even after isolated budget/checkpoint corruption. Namespace/current fence still precede
receipt replay. Wrong source/owner/namespace/revision/fence and a fitting empty aggregate are
refused without writes. Actual audit/outbox/telemetry trigger failures roll back the complete
native command; a fresh retry commits once.

For the nonempty oversized aggregate, the real public worker produces a256KiB source. The test
then calls **typed native CollectionCheckpoint**, not the scheduler checkpoint producer, because
the current scheduler otherwise refuses the262146-byte aggregate before checkpointing. The
fixture asserts OPEN/no drain, the full genuine checkpoint/Return tuple and source bytes before
checking source contradictions and the accepted native drain. No SQL manufactures that success
or checkpoint. Deliberate checkpoint/ledger corruption is isolated and labelled, not valid history.
The accepted drain is reconstructed through full and exact-path readers. These tests stop at
DRAINING; they do not establish drain completion or compensation.

The initial run was3passed/1failed because a generated workflow name exceeded63characters. It is
retained separately and is not a behavioral counterexample. Shortening that fixture prefix was
the only source change before the4/4 reruns. Original draft C1 and amended C2 underwent bounded
fresh review; C2 found no remaining issue. Patch seal:
`7f4fb3ba088ffa47febbecf6861739760d715e63a31b8ca8bc5951fd0d76cb72`.

Telemetry review found no remaining issue for patch
`1d5feb65c36a659e7449d115ca23a84f4b9ac8bcc777717e65dd06d4aac437f5`.
The real detached recorder exports the closed collection/RUN_DATA_LIMIT series without arbitrary
reason strings; actual RunDataLimit event projection carries no candidate evidence. These tests
do not prove scheduler counter timing, which remains coupled to later Q9 producer acceptance.

## Remaining boundaries

Actual ExistingOutputs legacy E>L and StoppedForRunDataLimit compositions remain pending, including
the second-root legacyControl discovery hint correction. Scheduler/core End and known-success
report producer activation, process restart/uncertain COMMIT, mixed-version rollout, full source
preservation and nested/capacity release acceptance remain separate. No public wire vocabulary,
capability, grant, publication shape or canonical task is enabled by this native test increment.
