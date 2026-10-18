# First dormant scoped native Finish

Source base4cb02846. See exact command JSON/logs and1091 before/after input hashes.
Natural RED: tests import absent PgScopedLeafFinishStore before module/export. The reviewed
store is then integrated, formatted, and the three static oracle findings corrected.
Four actual native tests pass; no synthetic/mocked transaction claim. Positive source comes
from real public Flow + native certifier, source INSERT revoked before Finish. Capacity identities
and nonselected neighbors are labelled owner fixtures, never production enrollment/source proof.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-infra-postgres --all-targets -- -D warnings
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Equivalent dedicated disposable PostgreSQL16.15 required; World creates isolated databases.
Four scoped receipt columns are granted only inside fixture DB; source INSERT absent at Finish.
No released grants, callers or runtime activation. Two implementation slices independently
reviewed None. Test review C1 three actionable findings fixed; C2 None. External model review
already declined; no external model-review CLI invoked. Full decoder/qualification/metadata-only/
context/barrier/rollback/unknown/process/OTLP evidence remains explicitly pending.
