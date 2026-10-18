# Approval member-chain before exact-run writer serialization

Based413e5a8, disposable repository PostgreSQL. Unset SAGAT_TEST_DATABASE_URL; set
SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3,
RUST_MIN_STACK=16777216. Exact commands/exits/wall times in commands.json and
regression-commands.json; target reuse optional.

- compile-fixture-error.txt/test-compile.diff/compile-source.json: test tuple mismatch while
  adding explicit route/version oracle. No tests executed, not behavioral RED.
- red.txt/test-red.diff/red-source.json: missing chain wait,0/1 in8.37s. Test timed out at
  pg_blocking_pids chain barrier; it did not reproduce a PostgreSQL deadlock.
- production.diff: exact declared Run Update membership SQL extracted into transaction-typed
  execution::lock_member_chain; caller uses StoreError conversion directly, no empty wrapper.
  Approval common writer helper invokes it before exact ownrun. No new capability or DDL.
- green.txt:1/1 in2.58s; actualapp chainwait with free ownrun, finalaudit gate with exclusive
  chain and ownrun retained55P03, exact decision+route events/version5, receipt/audit/outbox1.
  Current-authority decision replay leaves21-table checkpoint and chainhead unchanged.

Fixture is trusted native storage protocol: existingapproval opened, deadline set+real chain
initializer, revision3; decision4 and route5. No new public entrypoint/body shape is enabled.
Initial public membership is committed with new-run creation; identity is immutable later.
This guard does not authorize live reparent/delayedinitialization or forward an address.
Actual runtime store/waiter evidence is separate from older owner-pool fixture results.
Approval regression skips only the unchanged genuinelydue60second expiry test, whose latest
result is previouscommitd413e5a8, not a fresh result for this tree. Other two postwait tests run.
Counts overlap earlier suites. No full-workspace execution/SDK58/Compose/advisory/resource/
nesting gate or canonical closure. Collection/loop prefixes and complete B0 remain pending.
