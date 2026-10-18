# Source refusal tests, not producer preservation

Base f19d6d22. Production certifier and schema privileges unchanged; application edit only corrects
stale port documentation. Exact commands/statuses/times in commands.json, tested sources in
source-*.json. Shared target:
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target;
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216.
SAGAT_TEST_DATABASE_URL and SAGAT_TEST_AS_RUNTIME_ROLE unset. Flow explicitly runs actual HTTP/
gRPC/native stores as sagat_app even without the suite switch. Real disposable PostgreSQL16.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::refusals:: -- --nocapture --test-threads=1. Final6/6 72.30s includes
38.72s compilation and31.37s harness. Tests contain four step cases and two reference/digest cases
in loops, plus reservation, identity/missing/CAS, unsupported shape and actual scheduler takeover.
Schema-valid owner mutations are synthetic refusal fixtures; no production guard/constraint was
disabled and no actual producer reachability is inferred. No payload values are transferred by
the fingerprint queries. The moved row is independently observed by its stable step_id.

First3/3 64.62s and expanded10/10 80.51s precede oracle corrections. Final.txt6/6 67.14s follows
cycle1 corrections; head-journal.txt6/6 72.30s also includes cycle2's run/journal/per-call witness.
Earlier passing logs are preserved rather than represented as final-source execution. There is
no implementation RED claim: this increment adds coverage around committed native logic.

Strict targeted integration Clippy8.54s, fmt4.17s and architecture24crates0.32s passed on final
source. The previous commit has the full workspace all-target Clippy and runtime privilege gate;
those production sources remain unchanged. No optional repeat is represented as fresh execution.
Three fresh Astra/xhigh review cycles corrected concrete oracle gaps and then found no issue.
Sol/high supplied read-only fixture research; root alone wrote code and ran tests.

No Bao/S3/SDK/broker/process restart, producer races or full nested capacity acceptance follows.
Released INSERT and production callers remain gated; canonical feature014 tasks remain open.
