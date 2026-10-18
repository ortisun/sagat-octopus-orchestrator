# Actual later work and READY cancellation

Base ce60a328. Commands/status/times in commands.json and gates.json; frozen Rust hashes in
source-first.json and source-final.json. CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; SAGAT_TEST_DATABASE_URL unset.
Flow explicitly uses sagat_app; dormant INSERT granted only in disposable test database.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::later_work:: -- --nocapture --test-threads=1.
Initial1/1 passed50.13s (compile31.47s; harness16.41s). Review then identified the missing actual
READY precondition. Final1/1 passed79.58s (compile61s; harness16.15s) with that additional exact-path
OPEN/admitted/READY/lease-absence assertion. The matrix contains6fresh real public HTTP/gRPC flows:
later scoped completion, later scoped permanent failure and run cancellation with later READY work,
each with producer and certifier leading once in the actual original-Run queue. Real soft blocker
observations establish directions. Certifier first creates; producer first changes the run revision,
old creation CAS is refused, then fresh CAS creates. Selected source remains unchanged throughout.
Actual scheduler checkpoint/closure or drain/saga planning reaches SUCCEEDED/FAILED/CANCELLED;
selected source and its saga absence remain unchanged. Canonical replay retains the original record
under stale creation CAS; exact1fact+1audit and real execution replay verification pass in each flow.

New coverage around unchanged production logic; no implementation RED is claimed. No active-lease
cancellation, chain member, other ambient/oldwriter/C0/full producer/release guarantee follows.
No process restart, SDK/Bao/S3/broker execution is attributed to this increment.
Targeted strict Clippy/fmt/architecture results retained in gates.json.
