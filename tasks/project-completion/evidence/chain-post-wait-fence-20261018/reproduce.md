# Chain scheduler expiry during an unchanged-row wait

Repository disposable PostgreSQL, external SAGAT_TEST_DATABASE_URL unset, runtime option
SAGAT_TEST_AS_RUNTIME_ROLE=1. Build CARGO_INCREMENTAL=0/CARGO_BUILD_JOBS=3/
RUST_MIN_STACK=16777216; warm target optional. Commands/exits/times in commands.json and
regression-commands.json. Tests use real ports, with the evaluated handoff reached through a
published activity/continuation workflow; private legacy/finalization fixtures are explicitly
seeded and their runtime stores now honor execution_pool.

- compile-fixture-error.txt: wrong test-only verify_journal method; corrected to verify_run.
  Compilation failed before tests; this is not behavioral RED. test-compile.diff retains it.
- mixed-slug-fixture.txt: two expired-authority commits, one InvalidSlug fixture setup error.
  test-mixed.diff retains that variant; shorten evaluated fixture label before final RED.
- red.txt/test-red.diff: compatibility overflow passes, all three guarded real mutation ports
  incorrectly commit after expiry. 1 pass/3 failures (6.47s), not three setup failures.
- production.diff: common helper visibility plus chain wrappers retaining transaction typing,
  post-lock comparison and old generation conversion. No DDL, API or capability change.
- green.txt: 4/4,6.70s. Independent persisted checkpoint unchanged on rejection; fresh own
  authority succeeds, exact replay and journal consistency checked.

Barrier commits future expiry, locks but never updates that tuple, observes a blocked scheduler
query while live, waits until database clock crosses persisted expiry, then rolls back the guard.
No manual scheduler timestamp advance or sleep-only race oracle. Scope is expiry while waiting
for the authority lock; no continuous wall-clock-until-commit claim. Historical owner fixture
counts are not evidence all private tests used runtime privileges merely from the env option.
