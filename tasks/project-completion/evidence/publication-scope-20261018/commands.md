# Reproduction

Run from the project-completion worktree. The PostgreSQL fixture is repository owned/disposable;
the application runtime role is enabled. No external database URL is used. On this host the warm
build directory was shared with the database-connections worktree; it is build output, not a
shared PostgreSQL service.

```sh
unset SAGAT_TEST_DATABASE_URL
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216

cargo test --locked -p sagat-tests --test integration_workflow_primitives \
  publication_rejections -- --nocapture --test-threads=3
cargo test --locked -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-api-http
cargo test --locked -p sagat-tests --test contract_workflow_primitives -- --nocapture
cargo test --locked -p sagat-infra-ir
cargo test --locked -p sagat-tests --test integration_workflow_primitives -- --test-threads=3
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
git diff --check
```

`gate-commands.json` records exact commands, exit codes and elapsed times of the final sequential
contract/IR/integration/Clippy run. The four-package native run predates the final adapter byte
precheck; the final separate IR run covers that change. Counts overlap; do not add them as unique
tests. Neither command executes the complete workspace/Compose/advisory release gate.

The two focused failing expression reproductions used the same environment and integration
target, with exact selectors and `-- --exact --nocapture --test-threads=1`:

- `publication_rejections::an_oversized_multibyte_expression_returns_expression_instead_of_scope`
- `publication_rejections::a_multibyte_expression_above_the_legacy_wrapper_limit_returns_expression`

Stage history (earlier logs intentionally retained): initial mixed oracle4 tests; corrected scope
RED6; scope/root compatibility GREEN7; typed-byte GREEN9; wrapper RED1; final public GREEN10.
The initial mixed oracle's body-end failure was an incorrect expected reason, not a production
schema defect. The corrected scope RED was2 passed/4 failed; both focused expression REDs failed
on their response reason. The three production remedies are in `review.md`.
