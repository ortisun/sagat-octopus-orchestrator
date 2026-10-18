# Fixed V1 leaf certification application boundary

Base55c00d8c. Exact commands, statuses and elapsed wall time are in commands.json. Cargo uses
--locked, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3, RUST_MIN_STACK=16777216 and the shared warm
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target.
SAGAT_TEST_DATABASE_URL is unset. No database/Bao/S3/SDK/process-restart execution occurs.

Run cargo test --locked -p sagat-application --test leaf_certification -- --nocapture, then
cargo clippy --locked -p sagat-application --all-targets -- -D warnings, cargo fmt --all --
--check and python3 scripts/check-architecture.py. Final7/7 (5.01s command), Clippy6.76s,
fmt4.10s, architecture24crates0.32s. Module exports plus port/tests are the only core changes.

The canonical identity retains tenant/namespace/original-run/depth1path; caller flags/source facts
are absent from command. Creation revisions and six stored counters use positive signed-journal
range. Sequence ordering and exact return path are structural only; tests deliberately allow
checkpoint and aggregate digests to differ and certification revision to exceed source routing.
Schema/adapter/source/pin/authority/CAS/replay/retention/unknown-COMMIT/producer proof remain open.
The privacy canary proves ordinary Display/Debug redaction, not arbitrary source-chain formatting.
