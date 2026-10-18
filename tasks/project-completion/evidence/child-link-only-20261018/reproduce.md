# Reproduction

Run in the project-completion worktree with repository disposable PostgreSQL. The shared Cargo
target is build output, not a service from another project.

```sh
unset SAGAT_TEST_DATABASE_URL SAGAT_SDK_BLESS SAGAT_SDK_LANGS SAGAT_SDK_SCENARIO
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
cargo test --locked -p sagat-tests --test fault_workflow_primitives chain_parent_ \
  -- --nocapture --test-threads=3
cargo test --locked -p sagat-tests --test fault_workflow_primitives -- --test-threads=3
cargo test --locked -p sagat-tests --test integration_durable_execution -- --test-threads=3
cargo test --locked -p sagat-tests --test integration_workflow_primitives chain -- --test-threads=3
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

`commands.json` records commands, exits and wall time of the sequential broader run. The focused
RED has three failures before correction; explicit audit count assertions were added afterward
to satisfy the first review finding. GREEN includes them. The production change concerns only
the shared link mutation and legacy fallback; the handoff helper's source comment correction
changes no executable behavior.

Both SDK commands reuse unchanged prepared runners, compare existing goldens and never bless:

```sh
SAGAT_SDK_REUSE=1 SAGAT_SDK_SCENARIO=chain-child cargo test --locked -p sagat-tests \
  --test sdk_conformance every_sdk_conforms_to_every_scenario -- --exact --ignored --nocapture
SAGAT_SDK_REUSE=1 SAGAT_SDK_SCENARIO=chain-cancel-parent cargo test --locked -p sagat-tests \
  --test sdk_conformance every_sdk_conforms_to_every_scenario -- --exact --ignored --nocapture
```

`sdk-commands.json` records exact command wall times (including build-lock wait/compilation),
exit status and normalized-file hashes. Test execution times are distinct in the raw logs.
Only normalized documents are retained; generated runner config files containing credentials
were not read or copied. All five complete documents for each case independently match the
unchanged golden. This is two focused scenarios, not a full58 matrix or release gate.
