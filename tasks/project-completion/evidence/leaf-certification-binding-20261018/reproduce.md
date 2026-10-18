# Actual ambient binding replacement versus leaf certification

Base01210eaf. Commands/status/time in commands.json and gates.json; tested source hashes in
source-first.json. CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; SAGAT_TEST_DATABASE_URL unset.
Flow/certifier use sagat_app. Dormant INSERT is granted only by disposable database owner.

Run cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native::binding:: -- --nocapture --test-threads=1.
Actual1/1 passed46.59s (compile39.26s; harness5.20s), containing2fresh public HTTP/gRPC source flows.
Each leases one later scoped activity. Actual HTTP binding replacement narrows patterns and returns
1session closed/1lease released; exact later step is LEASED with live expiry before and expired
after. The binding writer completes under5s while Run is deliberately held in both interleavings:
certifier blocked at original-Run entry BEFORE source proof, and actual native COMMIT deferred
AFTER proof/canonical+audit inserts. The latter observes actual backend COMMIT behind a test gate
and probes retained exclusive Run/shared scheduler locks before and after binding replacement.
No SQL relay/response loss is used here. Binding is explicitly NOT a Run-serialized producer.
Selected source/Run/journal fingerprints stay unchanged;1canonical fact+1audit commit and original
record replay remain stable, actual execution replay verifies. No manual lease/source mutation.

Coverage-only change; no implementation RED claim. Other ambient writers, chain membership,
all-producer/C0/source UPDATE immutability, process restart, SDK/Bao/S3/broker or released INSERT/
caller readiness are not established. Targeted strict Clippy/fmt/architecture in gates.json.
