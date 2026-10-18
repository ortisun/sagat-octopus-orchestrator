# Exact-path entry serialization

Use repository disposable PostgreSQL with SAGAT_TEST_DATABASE_URL unset and
SAGAT_TEST_AS_RUNTIME_ROLE=1. Build: CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3,
RUST_MIN_STACK=16777216; warm target is optional. commands.json records exact test commands.

- red.txt: original test prototype with directly attached chain metadata, behavioral 0/4.
- test-prototype.diff retains that exact original fixture/oracle, before review refinements.
- red-final-oracle.txt: fixture import compile failure; not behavioral RED.
- red-corrected-fixture.txt: real initializer/deadline plus final oracles, behavioral 0/4 on
  unchanged production. The chain was ignored in both Wait/SkipLocked paths.
- production.diff: only chain-before-run acquisition added to both entries.
- test-final.diff: corrected fixture, actual database barriers and lifetime assertions.
- green-fences.txt: 8/8 bounded fence regressions including all four new cases.
- regression-commands.json records full workflow/Clippy; fmt and architecture are local gates.

No current public chain+collection shape or observed PostgreSQL deadlock is claimed. New chain
lookup is tenant-scoped via the unique run-membership index; no history is restored by it.
