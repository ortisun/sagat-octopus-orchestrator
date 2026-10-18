# Reproduce

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216 and SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres (owned disposable PostgreSQL 16.15, fresh database per fixture). Run commands from ordinary-end-result-default.json with SAGAT_TEST_AS_RUNTIME_ROLE unset and ordinary-end-result-runtime.json with SAGAT_TEST_AS_RUNTIME_ROLE=1. The two modes run the same five distinct tests, not ten different tests. Other commands are retained in clippy/fmt/architecture JSON files.

This unit extracts existing behavior and adds a private command field/accessors. It introduces no new collection evaluation or persistence, no native activation, migration, grant, public API, SDK or runtime shape. .1d.0 and .0Q remain unresolved; preexisting collection_end_result REDs are deliberately outside these passing selections.
