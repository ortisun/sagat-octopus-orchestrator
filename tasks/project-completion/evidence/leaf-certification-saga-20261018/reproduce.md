# Later effectful saga planning and leaf certification

Base8da036eb. Exact commands/status/time in commands.json; tested source-first/final hashes map to
first/final raw logs. CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; SAGAT_TEST_DATABASE_URL unset.
Actual Flow/native transactions use sagat_app. Only disposable DB owner grants dormant INSERT and
installs the test-only deferred gates. No production behavior or schema changes.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::saga:: -- --nocapture --test-threads=1.
First1/1 passed73.79s (compile59.71s; harness11.98s), before correcting the preliminary-restore oracle.
Final1/1 passed49.49s (compile34.51s; harness12.86s) with both actual writing-transaction barriers.
One matrix contains4fresh real HTTP/gRPC flows (failure/READYcancel times2writer orders).

Selected lines work is ordinary and closed. Later line.charge has compensateWith line.refund,
reconcileWith line.lookup and highRisk; it completes before successor line.notify becomes READY.
Actual permanent failure or READY cancellation, then scheduler drain, reaches preplanning.
Planner-first pauses inside actual saga-header deferred COMMIT; certifier waits on its exact Run,
planner commits and old creation CAS is refused; fresh CAS certifies alongside another owner's saga.
Certifier-first queues preliminary scheduler restore before certification on held Run, then lets
certification reach actual deferred COMMIT after restore releases. Only the planner's second Run
acquisition is then observed blocked by the committing certifier. This proves the writing entry,
not only read restoration. Both writers retain exclusive Run and shared scheduler fence at gates.

Actual later saga header/COMPENSATE entry/scoped planning event are positively asserted, then the
real line.refund compensation task is delivered and completed. Terminal run is COMPENSATED(fail)
or CANCELLED(cancel), later saga DONE/COMPENSATED. Selected source and its exact saga absence remain
unchanged throughout; original canonical record replays and exact1fact/audit plus execution replay
verification pass. Strict targeted Clippy/fmt/architecture commands retained in commands.json.

Coverage-only: no implementation RED claim. No unknown-effect reconciliation, active-lease cancel,
other ambient/chain/C0/source UPDATE immutability or release readiness follows. No SDK/Bao/S3/broker
or process restart is attributed to this unit.
