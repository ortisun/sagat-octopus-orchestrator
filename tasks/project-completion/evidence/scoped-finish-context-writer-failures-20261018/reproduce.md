# Reproduction and actual results

Source base 76df372b. Root actually executed the commands below on disposable PostgreSQL16.15,
loopback65117, each fixture's own database and explicit sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual complete current scoped native family 25/25 in94.56s wall (58.70s harness); strict integration Clippy9.81s, fmt4.22s, architecture0.28s/24crates. All1100 frozen executable/contract inputs match.

Three public-context tests cover ordinary child live/detached, settled FINAL child chain, and an independent parentless historical CONTINUED segment after real handoff; current authority precedes UnsupportedRunContext and actually denied source/receipt SELECT. Three writer tests cover first native certifier versus Finish in both actual Run queue orders, later worker completion/failure versus Finish in both orders, and both distinct genuinely certified member orders with actual fresh discovery/new logical keys after confirmed stale refusals. One atomic test uses four isolated worlds: audit/outbox/receipt BEFORE INSERT and deferred COMMIT 23514, each with a nontransactional owner-only marker proving the exact failure point, exact rollback snapshots and unchanged-command apply/replay. Actual source/certificate and nonselected state is preserved; legitimately changed Run/steps are not asserted unchanged.

Production driver unchanged. Initial actual full run22pass/3fail in92.87s wall/56.83s harness is retained with identical1100 frozen failure inputs and owner-only fixture diagnostics. Two failures were children disabled in the fixture scheduler; fourth atomic workflow name deferred_commit was rejected422. After fixing those fixtures and the reviewed oracles, the complete25-test aggregate passes on the newly frozen tree. No combined partial-pass claim. Only dormant local15 closes;16/17, production/C0/callers/general nesting and canonical runtime/resource/release tasks remain open. Child and chain capacity rows are registry fixtures without canonical child-source certification. Returned canary checks cover sanitized detail/Display/Debug; actual diagnostic export remains17.
