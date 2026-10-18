# Q2 PostgreSQL reader preparation

Run the commands retained in each JSON from this worktree with CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216 SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres. Default runs unset SAGAT_TEST_AS_RUNTIME_ROLE; runtime runs set it to 1. PostgreSQL 16.15, held loopback fixture.

Existing collection_restore passes 4/4 in each mode; existing scope_compensation_http passes 19/19 in each mode. Strict PostgreSQL library Clippy, formatting and architecture pass. All 1109 source inputs match before/after. Counts describe separate invocations, not unique tests summed across modes.

This increment restores the new reason in full, owned-path and descendant-ancestor readers and checks the existing drain finisher without enabling native production. Three bounded reviews ended with no required issue after the separate ancestor query was corrected. Patch SHA256 54004d809a22b2f1591fa87153170dbc595eb8a03bdae3b0c6c1400e58ddab6b.

These are regressions for existing reasons, not executed new-reason SQL acceptance. New-reason positive/corruption fixtures, additive schema, cold scheduler lifecycle, effective accounting, real producer and actual old-binary upgrade remain pending. Q2 stays unchecked until focused SQL evidence lands. This intermediate source commit is not a deployable bridge A.
