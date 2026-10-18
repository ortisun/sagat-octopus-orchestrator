# Reproduction and actual results

Source base 753152f6. Root actually executed the commands below on disposable PostgreSQL16.15,
loopback65117, each fixture's own database and explicit sagat_app.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_AS_RUNTIME_ROLE=1
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives leaf_certification_native::scoped_finish::observations:: -- --nocapture --test-threads=1
cargo clippy --locked -p sagat-tests --test integration_workflow_primitives -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

Actual final strengthened observation controls 3/3 in6.35s wall (6.07s harness); strict integration Clippy10.95s, fmt4.19s, architecture0.35s/24crates. All1103 frozen executable/contract inputs match.

The actual process matrix kills and reaps a genuinely blocked reconciliation OS child while the original uncertain writer remains alive, observes actual committed/23514-aborted server resolution, changes scheduler authority through real release/acquire, and starts another OS process from the byte-identical eleven-field complete original command. The replacement discovers only scheduler owner/generation, preserves original key/CAS and returns the pre-captured original IDs/time/counters or applies once after confirmed abort; subsequent replay preserves exact capacity/source snapshots. Three current-thread observation tests exercise all six returned outcomes, actual OTLP parent/status/exact identity attributes, fixed log sequence, closed metric series/labels, raw diagnostic/source canary redaction, filtered parent safety, and abandoned-call zero returned metrics/logs/effects with exact five identity values. Only acknowledged first apply increments the transition counter.

No production driver changed (832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780). Native source+scoped baseline51passed/2private probes ignored in direct enumeration,170.51s wall/134.30s harness, is retained at its exact1103 frozen inputs. Both private probes actually run through their parent matrices:8 OS children total,4 killed while blocked and4 successful replacements. Final sources differ from that baseline only by abandoned-span identity assertions; focused final3/3 verifies those assertions. This is not a new aggregate54-test pass. Final-tree regressions:capacity/legacy/schema/privileges/downgrade33/33 in79.36s wall/42.91s harness, hint3/3 in0.30s, AsyncAPI4/4 in22.42s. Earlier72.99s full invocation was administratively SIGINT-superseded for review-oracle corrections; it is retained as incomplete, not a test assertion failure or completed aggregate. This proves reconciliation-client process death, not original writer/control-plane/engine restart. General B1b.2b/B1/B2/C0, source/scoped release grants, caller intent, nesting, resource ceilings and canonical runtime/release tasks remain open.
