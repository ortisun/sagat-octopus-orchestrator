# Actual ordinary-child baseline

Source base9b61b3e4, root execution on disposable PostgreSQL16.15/loopback65117. Each fixture creates its own database. Final source hashes are recorded before/after, including the uncommitted child module.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
env -u SAGAT_TEST_AS_RUNTIME_ROLE cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source:: -- --nocapture --test-threads=1
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual focused child baseline: 2/2 default (40.1s wall) and 2/2 runtime-role (6.58s wall), the same two tests in separate invocations. Strict integration Clippy 39.21s, formatting 4.49s and architecture 0.41s/24 crates passed. All 1104 frozen executable/contract inputs match.

A genuine publicly started parent creates an ordinary unchained child through the production scheduler. Before source closure the native certifier refuses SourceNotClosed; actual worker completion/checkpoint/collection closure creates the source while later child work remains LEASED and unfinished. First native certification retains exact qualified source identity, pinned definition, scope/body/Return/digests/five actual event references and one truthful audit. Cold runtime pool/cache/store replay returns the complete original record. Stale own authority precedes replay; foreign tenant/namespace, missing index and parent identity cannot reuse the child evidence. Actual later work finishes child and parent while selected source/certificate stay unchanged; terminal replay preserves the entire observed snapshot. A second genuine certificate fixture still receives UnsupportedRunContext from Scoped Finish before denied source/receipt reads (actual42501 probes), with no source/execution/capacity effects. Both use explicit sagat_app and a disposable test-only source INSERT grant that is revoked before replay. No production implementation changed.

This closes local B1b.2c.1a only. It characterizes existing source acceptance, not new child support or complete parent-writer preservation. Cold reconstruction is not an OS restart. Parent CANCEL races, child cancellation/drain/later compensation and terminal fanout remain separately pending. All parent/chain Scoped Finish refusals remain. Broader B1/B2/C0, production source/scoped INSERT grants/callers, nested runtime, resource ceilings and canonical T014-014/015/016/017/040/042/043/044 remain open. Earlier compilation is compilation only. Initial runtime0/2 failed at public workflow creation422; shortened test fixture prefixes fixed the public Slug63 mismatch. The retained preformat run passed2/2 in each mode and Clippy/architecture, but formatting rejected one edition2024 trailing comma. Final results above are from the corrected final source, not a combined aggregate.
