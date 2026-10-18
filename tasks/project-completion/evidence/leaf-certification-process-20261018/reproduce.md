# Actual native reconciliation client process death and restart

Base758ab955. Exact commands/status/time in commands.json and gates.json. Source hashes in source-*.json.
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; SAGAT_TEST_DATABASE_URL unset.
Flow and child connections run as actual sagat_app; only disposable test DB receives dormant INSERT.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::process::killed_native -- --nocapture --test-threads=1.
First compile failed33.02s: UUID has no Serialize feature. Converted fixture metadata UUIDs to strings;
no production behavior bug/RED is claimed. Green1/1 in40.65s (compile33.14s; harness5.35s).
One matrix contains2fresh public HTTP/gRPC source flows and4actual re-executed OS child processes.
The child-only ignored test is explicitly invoked by the parent with --exact/--ignored; it requires
private parent configuration and intentionally fails without it. It creates no database/container.

Original native writer sends actual COMMIT through existing one-shot relay into a deferred gate;
client receives Unknown while actual backend remains executing. Child discovers current metadata
from retained5field identity file and blocks in the native exact Run SELECT behind that backend.
Parent kills/reaps the child, proves original Run/scheduler locks survive and no result/metadata
appears, then releases the original gate and observes actual commit or23514abort and idle server.
Old child backend disappears naturally; no pg_terminate_backend masks client-process cleanup.
Actual ShardStore release/acquire changes scheduler owner/generation. New OS child/new runtime,
pool/cache/store reads the same identity file and discovers the new authority and current revision.
Committed fact replays exact original audit ID; absent aborted fact creates from full bounded source
proof. Canonical audit ID and1fact+1audit counts match, source/run/journal stay unchanged, actual
execution replay verifies. No credential appears in child arguments, metadata files or output.

This establishes native reconciliation-client process recovery, not original-writer process crash,
control-plane/execution-engine restart, chain membership, other producer barriers, SDK/Bao/S3/broker
behavior or release readiness. Targeted strict Clippy/fmt/architecture are retained in gates.json.
