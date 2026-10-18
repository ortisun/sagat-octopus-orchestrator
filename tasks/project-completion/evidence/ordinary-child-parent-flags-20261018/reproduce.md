# Actual incoming-parent flag preservation

Source base2d86483f (later a28ac3f4 is docs only). Root ran all commands on disposable PostgreSQL16.15, loopback65117; each fixture owns a fresh database, and native/API writers use sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
env -u SAGAT_TEST_AS_RUNTIME_ROLE cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source:: -- --nocapture --test-threads=1
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual focused child suite:4/4 default in53.65s wall/16.71s harness and4/4 runtime-role in17.43s wall/16.78s harness. These are the same four named tests in separate invocations:two existing baseline controls and two new tests executing six policy/order fixtures plus one actual COMMIT fixture. Seven asserted three-PID observations are retained per mode. Strict integration Clippy11.27s, fmt4.33s and architecture0.36s/24 crates passed. All1105 frozen executable/contract inputs match.

Real public parent CANCEL competes with first native certification of a genuine ordinary unchained child under PROPAGATE, DETACH and REQUEST_CANCEL in both observed child-row queue directions. Exact three-PID role/database/query/wait/blocker witnesses, parent Run conflict probes, original post-lease child CAS, policy-specific parent journal/status/step/flags, atomic audit/dedupe/outbox and immutable selected source/full child core/journal/steps/exact later lease are asserted. All six first certifications succeed without changing the captured revision. A seventh fresh fixture pauses the actual certifier COMMIT behind a deferred advisory gate; the real parent UPDATE waits behind that COMMIT while child Run, parent Run and child scheduler SHARE guards remain held. Releasing only the gate commits both. After revoking the disposable source INSERT grant, a cold runtime store/cache returns the exact original complete record/audit and preserves the full snapshot.

Local B1b.2c.1b closes only this incoming legacy flag writer family. No production code, grant, caller, profile, schema or capability changed. Later remains LEASED and unfinished; no own-child cancellation/drain, detached later progress or terminal fanout is claimed. The parent flag writers do not advance child revision/journal/updated_at. B1b.2c.1c/.1d/.1e and the master .1 remain open, as do broader B1/B2/C0, production activation, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044. Initial default invocation passed the two baseline tests and failed both new tests at the shared uppercase dedupe expectation; the six-cell loop then stopped in its first cell. Initial Clippy failed two test-only lints. All original records/source pins are retained separately, and no partial run is combined with the final suite.
