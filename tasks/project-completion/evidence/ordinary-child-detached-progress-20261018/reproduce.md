# Actual detached-child later progress

Source base2cdd5aaf; concurrent86cd7cdd is documentation only. Root ran these commands on PostgreSQL16.15/loopback65117, disposable fixture database, explicit sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
env -u SAGAT_TEST_AS_RUNTIME_ROLE cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source::detached_progress:: -- --nocapture --test-threads=1
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::child_source::detached_progress:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual focused DETACH later-progress test:1/1 default in38.62s wall/1.94s harness and1/1 runtime-role in2.53s wall/1.91s harness. These are separate invocations of the same named test, not two distinct cases. Strict integration Clippy11.21s, fmt4.27s and architecture0.35s/24 crates passed. All1106 frozen executable/contract inputs match.

After genuine ordinary-child source certification, real public DETACH parent CANCEL commits a terminal parent with parent_settled=true and parent_cancel=false. The original leased later activity then completes through real gRPC, retaining its exact expected output, clearing lease metadata and advancing child revision/steps/journal without stopping or finishing the child. Selected source/certificate stay unchanged. Exact parent Run/steps/journal/audit/outbox snapshot stays unchanged, with no child.finished notification. After the disposable source INSERT grant is revoked, a cold explicit sagat_app store/cache replays the original creation CAS and returns the complete original certificate/audit under current own-child authority, preserving the entire post-progress snapshot.

Local B1b.2c.1e only. No scheduler tick, child termination, terminal result propagation, fanout, own-child cancellation/drain/compensation, race matrix, production code/schema/profile/grant/caller or capacity activation is claimed. B1b.2c.1c/.1d/.1d.0 and master .1 remain open. The separately identified collection End/owner-result retention defect requires genuine RED and a reviewed correction; this test stops before that path. Broader B1/B2/C0, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044 stay open. Prior four-test child-suite results at2cdd5aaf remain separate at1105 inputs; this one-test proof at1106 inputs is not a new five-test aggregate.
