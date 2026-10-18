# Dormant immutable leaf certification schema

Base9ffedfd4. Source hashes and exact commands/statuses/elapsed times retained. Cargo --locked;
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. SAGAT_TEST_DATABASE_URL unset;
only runtime.txt sets SAGAT_TEST_AS_RUNTIME_ROLE=1. Each schema test also explicitly inspects or
uses actual sagat_app privileges where relevant. PostgreSQL16 is real disposable test infrastructure.
No Bao/S3/SDK/broker/process restart execution is established by this unit.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification::
-- --nocapture --test-threads=1, then repeat with SAGAT_TEST_AS_RUNTIME_ROLE=1. Default5/5
42.23s (includes build), runtime5/5 10.26s. Earlier absence42P01 and first4/4 logs remain. The first
source snapshot preceded an ordering-only cleanup of classification entries; final checks use
unchanged entry content with existing manifest ordering preserved. The fifth test and expanded
source/identity probes were added before source-second and the5/5 runs.

The direct-binary chain regression RED ran independently while Clippy compiled, so it did not
contend on the Cargo build cache. Exact binary/source hashes and invocation are in
chain-fixture-red-command.json, raw failure in chain-fixture-red.txt. Both post-fix chain tests,
SQL/trigger runtime privilege audit, strict workspace Clippy/fmt/architecture are recorded.
Native Finish25/25, D298 maintenance1/1, classification1/1 and append-only privilege1/1 (12grant
cases) precede only the isolated chain helper fix; their production sources remain identical.
source-before-chain-fixture and source-final keep that boundary explicit. No optional retesting
of unchanged production behavior is represented as fresh execution after that fixture change.

Snapshots and populated source rows are owner-only schema fixtures, deliberately not actual
successful execution. Runtime SELECT only cannot certify. Direct DELETE checks identify the
exact rejected source and verify row counts after rollback, including replica bypass. TRUNCATE
CASCADE can fail at a descendant; exact catalog kind/function/ALWAYS checks bind every own guard.
A separate rollback-only owner probe disables just the scope's parent DELETE guard so normal
FK cascading reaches a retained child guard; it does not relax production retention.
Actual source pin/closure, UPDATE preservation, audit creation, uncertain-COMMIT/restart, scoped
Finish, old-writer/C0 and nesting/100k acceptance remain open. No full workspace test claim.
