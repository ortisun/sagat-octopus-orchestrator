# Direct Update storage serialization through HTTP

Use repository disposable PostgreSQL with SAGAT_TEST_DATABASE_URL unset,
SAGAT_TEST_AS_RUNTIME_ROLE=1; locked toolchain. Build settings CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3, RUST_MIN_STACK=16777216; warm target optional.

commands.json and regression-commands.json contain exact commands/exits/times. RED 0/1 timed
out observing the original chain wait, using a successor whose ID differs from the chain.
The new test uses existing trusted state/chain fixture data and the real HTTP mutation/replay.
A database-only receipt trigger exposes lock lifetime, then releases the request to commit.
No fake runtime handoff/publication support is inferred from fixture seeding.

HTTP10 and operations26 passed on unchanged production before the final one-line SHARE probe
refinement. Final focused test verifies that stronger probe; regression-test-source.json records
the earlier oracle source SHA, test-red.diff/test-final.diff retain both sources. No golden or
SDK/runner source changed. No full workspace, five-SDK, Compose/advisory or nesting claim.
