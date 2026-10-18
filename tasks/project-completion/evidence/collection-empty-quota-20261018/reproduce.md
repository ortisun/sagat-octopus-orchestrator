# Reproduce

Set CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216. For PostgreSQL use SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres (owned disposable16.15; each test gets a fresh DB). SAGAT_TEST_AS_RUNTIME_ROLE was unset for the integration RED.

Exact invocations are red-raw-codec.json and red-empty-boundaries-import-fixed.json. The earlier red-empty-boundaries.json is compile failure only. No second mode was run to repeat a proven RED; runtime-role GREEN remains required after correction.
