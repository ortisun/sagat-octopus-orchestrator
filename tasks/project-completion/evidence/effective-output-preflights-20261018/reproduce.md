# Q6b full preflights

Run retained JSON commands with CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216 SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres. Default unsets SAGAT_TEST_AS_RUNTIME_ROLE; runtime sets1. PostgreSQL16.15. All1115 inputs unchanged.

Existing real gRPC result tests pass5/5 in each separate mode (49.94s/8.96s and9.24s/8.94s wall/harness). Strict all-target application Clippy8.47s, fmt4.38s and architecture0.34s/24crates pass. No new mirror arithmetic/getter test was introduced.

The four existing preflights now use RunTx::logical_output_bytes rather than hydrating full data merely to sum outputs: ordinary completion, Applied substitute, run signal and chain signal. Saturating u64 conversion/addition/multiplication preserves refusal semantics on overflow; existing managed-write OUTCOME_UNKNOWN and ordinary RUN_DATA_LIMIT branches remain. Root inspected the mechanical consumer replacement against approved scalar semantics. The Q5 PostgreSQL override is already installed. The action rustdoc now describes actual output refusal even when a prior stop wins; no runtime audit behavior changed in that wording fix.

These are existing gRPC result regressions, not direct new legacy-signal/chain acceptance or actual old E>L worker/compensation upgrade proof. Frozen-producer, exceptional scoped known-success application, matching scheduler/native producer, fault and final integration gates remain pending. Source is an intermediate feature increment, not a deployable complete bridge A/B.
