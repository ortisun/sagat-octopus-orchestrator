# Dormant scoped Finish hint vocabulary

Basec0b38fbc. Exact commands/exits/times in JSON/logs; four frozen source hashes unchanged.
New tests were installed before canonical schema and naturally failed2/2 on the absent channel.
After the reviewed additive schema, all3 hint tests pass, including old ChildRoot. Actual existing
Finish33/33 passes as sagat_app on disposable PostgreSQL16.15; all20 old baseline entries remain
semantically unchanged. No SAGAT_UPDATE_BASELINE/bless/regeneration ran; the one addition was
hand-authored and reviewed. No existing SDK golden changed.

```sh
unset SAGAT_UPDATE_BASELINE
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives hint_contract:: -- --nocapture
cargo test --locked -p sagat-tests --test contract_asyncapi_events_webhooks_messaging -- --nocapture
npx --yes --ignore-scripts @asyncapi/cli@3.6.0 validate specs/001-platform-foundation/contracts/events.asyncapi.yaml
cargo test --locked -p sagat-tests --test integration_workflow_primitives capacity_finish:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives --test contract_asyncapi_events_webhooks_messaging -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

One bounded fresh Astra review: None. External review previously declined, no external CLI.
The document is valid with0errors/0warnings/1version informational notice. Synthetic scoped
payload tests do not prove an actual scoped producer/audit/receipt, permission or execution
eligibility. Native scoped tests and activation gates remain separate.
