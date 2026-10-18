# Reproduction and actual results

Source base 80fc3917. Root actually executed the commands below on disposable PostgreSQL16.15,
loopback65117, each fixture's own database and explicit sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish::commit_loss:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual focused COMMIT loss test with both actual server resolutions 1/1 in39.35s wall (4.02s harness); strict integration Clippy9.83s, fmt4.22s, architecture0.28s/24crates. All1101 frozen executable/contract inputs match.

One param-loop test executes actual committed and deferred23514-aborted server outcomes after losing the dispatched COMMIT reply. Actual COMMIT/pg_blocking_pids and NOWAIT probes prove original Run/G/shard guards remain held while unacknowledged. A distinct real public Run reaches G Busy and releases its Run; the same complete original command waits at the original Run. Dropping that claimant does not resolve the unknown original outcome. The relay observes actual C/E plus idle ReadyForQuery after gate release. A fresh runtime pool/store reconciles the unchanged complete command/key/expectations: committed receipt replays its exact original IDs/time/counters without changing its pre-recovery snapshot; aborted original applies once. Subsequent replay preserves the exact resolved snapshot and one receipt/audit/hint set. Genuine source state remains unchanged.

No production driver changed. The earlier focused1/1 pass at46a978de (39.37s/4.00s harness) is retained separately as pre-original-result-control, with1101 identical inputs; final stronger-oracle run is its own actual result. No invented failure or combined aggregate. The previous scoped25/25 aggregate remains separately scoped to80fc3917; this is focused1/1 with two real outcomes, not aggregate26. Process restart16b, actual observations17, production/C0/callers/nesting and canonical runtime/resource/release tasks remain open.
