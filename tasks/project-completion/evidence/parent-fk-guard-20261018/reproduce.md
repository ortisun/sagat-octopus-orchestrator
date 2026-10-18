# ADR-0037 controls

Use the repository disposable PostgreSQL fixture with SAGAT_TEST_DATABASE_URL unset and
SAGAT_TEST_AS_RUNTIME_ROLE=1. Locked Rust toolchain. Warm build target can be any writable target.
CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3, RUST_MIN_STACK=16777216 were used.

experiment.json records the exact one-test command, baseline/restored source SHA-256 and exits.
without-parent-guard.diff removes only the early hold_parent call. A try/finally restoration
returned the file exactly to its baseline before the successful control. The omitted-guard
source is never a production candidate. The RED public Unavailable is not an observed SQLSTATE.

ancestry-commands.json records two later positive assertion tests; these were added after the
control, and are not a claimed failing-before test. The only final production source difference
is a comment referencing the accepted guard/precondition. Previous whole target/five-SDK results
are in ../child-link-only-20261018/; they are historical supporting evidence, not rerun here.

No payload/secrets, JWT configuration, provider token or external database URL was captured.
