# Scoped Finish source controls

Base7b0ced12; characterization controls for existing dormant implementation, no artificial RED.
Final3/3 actual PG tests use explicit sagat_app, genuine public Flow/native certificates and
original native receipt; controlled wrongpin/future revision/removal are owner-only faults.
Always guards are restored before native calls and per-call snapshots compare before repair.
No fake definition row or definition guard change is needed: certification UUID pin has no FK.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish::source_controls:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Use equivalent disposable PostgreSQL16.15, World-isolated databases. Denied SELECT tables:
iteration_scopes, collection_iterations, iteration_checkpoints, steps, scope_sagas,
scope_saga_entries, run_events, workflow_versions, audit_records, payloads, collection_scope_items.
Each has actual SQLSTATE42501 probe; certificate SELECT/noINSERT and audit INSERT/noSELECT
are asserted, then genuine apply/replay succeeds. This supplements reviewed bounded SQL; it
does not trace all nested SECURITY DEFINER operations. Source/Run/capacity snapshots are owner reads.

The real E0521 helper compilation failure and nine-table intermediate3/3 pass retain their own
logs/hashes. C1 added the two omitted tables, C2None; only final11-table run claims that coverage.
All1092 final before/after source hashes match. No production permission/caller/native semantic
change. Other qualification/receipt/diagnostic/barrier/unknown/process/OTLP gates remain pending.
External model second review was declined earlier; no external model-review CLI invoked.
