# Native leaf certification observations

Base371848c0. Exact commands/status/wall time in commands.json. Shared warm build cache:
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; SAGAT_TEST_DATABASE_URL unset.
All Flow/native SQL runs as sagat_app with dormant INSERT granted only in disposable databases.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::observations:: -- --nocapture --test-threads=1.
RED40.45s: both tests compiled and failed missing counters (0instead1), not compilation failure.
Initial instrumented run57.95s: filtered-span test passed; main passed actual outcome counts/export
then failed because captured synthetic CREATE FUNCTION SQL contained its driver canary literal.
Moved only fixture DDL before capture; actual native failed COMMIT remains inside observation.
Stronger oracle version2 passed2/2 in40.43s (compile31.76s; harness6.58s), with status.message and
payload canary checks. A review then replaced text log parsing with exact structured event checks.

Final source hashes in source-complete.json correspond to native-regression.txt and subsequent
gates in commands.json. Earlier source-red/green/oracles-v2 map respectively to red/green/final logs.
The final regression selects all leaf_certification_native tests. Its ignored private subprocess
probe is deliberately invoked by the parent process-recovery test with --exact/--ignored; it is
not a missing restart test. No test manufactures a native returned outcome or driver result.

Main observation matrix: invalid revision and actual lost scheduler authority refusals, real deferred
COMMIT23514abort, acknowledged first certification and canonical replay, real dispatched COMMIT
response loss with confirmed server commit, then same original identity recovery replay. Seven
operational spans reach the HTTP OTLP collector with exact per-Run outcomes and parent trace IDs;
metric attempt/histogram counts match. Only acknowledged first commit increments confirmed total;
Unknown/backendcommit and recovery replay never infer another local confirmed transition.
Filtered-span test uses a real closed native pool: storage error still records metric while a
deliberately disabled child leaves caller outcome/status untouched.

Exported source fields are tenant/namespace/originalRun/owner/index plus closed outcome; no status
message, events or links. Unredacted JSON log events from exact native target are exactly7DEBUG
events, fields exactly fixed complete message+closed outcome (no prefix/suffix parser). Live source
output payload, driver diagnostic and secret-shaped scheduler owner markers are absent from raw
log capture, metric render and collected export. Metrics permit only outcome and histogram le;
confirmed counter has no labels. Canonical fact/audit counts stay1+1 per source.

Operational observations do not project workflow journal history, certify other producers/C0,
prove source UPDATE immutability, or enable released INSERT, a production caller or nesting.
No SDK/Bao/S3/broker acceptance is attributed. Review3bounded cycles ended with no concrete finding;
external review was previously declined and no external CLI ran.

Final full native regression:18passed/0failed/1private helper ignored,151.14s (harness86.61s).
Strict workspace/all-target Clippy passed89.00s; formatting and architecture results in commands.json.
