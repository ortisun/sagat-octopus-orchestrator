# Canonical metadata reader extraction

Base2b53a658. See command JSON/logs for exact exits/times. Two new controls first passed on
the unchanged private reader; this behavior-preserving refactor has no artificial RED.
After the exact retained extraction.diff, full native regression21passed/1private probe
ignored directly; parent recovery executes real subprocesses. Hashes before/after match.
Wrong pin/future revision are owner fault fixtures, never a production source proof.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives canonical_metadata_reader_ -- --nocapture --test-threads=1
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Use an equivalent dedicated disposable PostgreSQL16.15; World creates isolated databases.
Both implementation/control reviews None. External review previously declined. No new
source proof, authority, release permission, source producer, scoped Finish adapter or caller.
