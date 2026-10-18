# Actual collection closure and leaf certification

Base 2b113984. Commands, exit codes and wall times are retained in commands.json and gates.json.
Environment: CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. SAGAT_TEST_DATABASE_URL unset.
Flow explicitly uses sagat_app with only a disposable owner grant for dormant INSERT.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::closure:: -- --nocapture --test-threads=1.
Actual1/1 passed in47.33s (compile39.36s; harness5.77s), containing2fresh public HTTP/gRPC flows.
One production scheduler advance checkpoints the completed iteration without finishing its scope.
The direct real finish_collection transaction and native certification queue behind the held Run;
pg_blocking_pids observes both queue directions. Certifier first returns SourceNotClosed with no
metadata/source/run/journal change. Finisher first commits, then the old creation CAS is refused.
Fresh CAS creates exactly one canonical fact and audit. Original finisher descriptor and canonical
record both replay unchanged. Actual execution replay verification passes for each flow.

Coverage-only change around unchanged production logic: no implementation RED is claimed.
This proves only flat successful collection closure, not all producers, chain membership, restart,
SDK/Bao/S3/broker behavior or release INSERT/caller readiness. Parent producer gate remains open.
Strict targeted Clippy, formatting and architecture results are retained in gates.json.
