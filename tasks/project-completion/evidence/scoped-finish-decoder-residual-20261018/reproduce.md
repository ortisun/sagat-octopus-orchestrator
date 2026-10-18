# Reproduction and actual results

Source base21139428; only test child registration and cross-kind decoder test added.
No production implementation changed. All1095 frozen executable/contract inputs match
before/after. Disposable PostgreSQL16.15 on loopback65117; each fixture creates its own DB.
Runtime role sagat_app is asserted by fixtures. Environment:

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

Root actually executed all commands: native13/13 in86.09s wall (21.12s harness),
Clippy16.17s, fmt4.22s, architecture0.43s/24crates. Exact commands, UTC and exits are in JSON
records; stdout/stderr retained. No failures in this unit. The exact-fingerprint ChildRoot
row is an owner-only opaque decoder fixture, not a SHA256 collision or native ChildRoot
finality. All CHECK/FK/append-only guards remain intact. Original-key refusal has exact
source/capacity snapshots; fresh key applies once and genuinely scoped receipt replays.

This closes current-schema decoding and metadata qualification, not actual failure injection,
OTLP/redaction, commit ambiguity, release grants, production callers or non-flat sources.
