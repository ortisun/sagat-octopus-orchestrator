# Actual leaf COMMIT outcome reconciliation

Base bd00cb33. Commands/statuses/whole-command times in commands.json; pre/post source hashes in
source-*.json. CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. SAGAT_TEST_DATABASE_URL and
SAGAT_TEST_AS_RUNTIME_ROLE unset. Flow/proxy/direct stores explicitly use actual sagat_app;
disposable owner grants dormant INSERT only for native tests. Real PostgreSQL16.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives commit_loss::
-- --nocapture --test-threads=1. Final2/2 79.49s includes compilation65s and harness11.37s.
Both leaf source and existing capacity Finish consumers use the single shared test wire proxy;
each covers actual committed and23514aborted COMMIT resolutions, lost client replies and fresh
same-identity reconstruction. Native leaf metadata remains invisible while COMMIT is gated.
Its original Run excludes KEY SHARE; scheduler excludes NO KEY UPDATE but permits SHARE.
A blocked reconciler is cancelled without treating that cancellation as original rollback.
After real server C/E and ReadyForQuery idle, the waiter pool closes and a new pool/cache/store
plus fresh revision/authority discovery reconciles original canonical identity/audit or absence.

first.txt81.58s is a real fixture failure: synthetic constraint marker did not match the reused
relay. green.txt1/1 45.22s follows the marker fix. clippy.txt records the duplicate_mod failure;
single module access then replaced duplicate loading without suppressing the lint or changing
wire protocol logic. Final source additionally strengthens lock-mode oracles and creates the
post-resolution recovery pool anew. Previous results are not represented as final-source runs.

Strict targeted integration Clippy17.11s, fmt4.50s and architecture24crates0.40s pass. Three fresh
Astra/xhigh review cycles corrected fixture/oracle claims, then found no bounded issue. Root
alone changed code and executed tests. No production logic, schema permission, SDK/Bao/S3/broker,
OS/server restart, chain-member, other producer or full nesting acceptance follows. The code and
documentation distinguish fresh object/session reconstruction from the pending process restart.
