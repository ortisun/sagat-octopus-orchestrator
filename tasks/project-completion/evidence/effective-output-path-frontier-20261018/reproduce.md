# Q7c effective path and frontier scalar preparation

Run retained JSON with CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216 SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres SAGAT_TEST_AS_RUNTIME_ROLE=1. PostgreSQL16.15. Final1115 source inputs unchanged.

Fresh bounded C1 on b35d8af2ed56b0f39b19506f77acc9576733b66f975bf2090d541daf9b9ddf37 found no required issue. Root preemptively used the same auto-deref form already required by Q5 Clippy before freezing these gates. Strict integration Clippy28.22s, existing runtime byte_ceilings19/19 (87.73s wall/29.73s harness), fmt4.45s and architecture0.41s/24crates pass.

Path scalar and positive-write guard now use shared qualified E=L+V, checked arithmetic, L-only increments and the exact typed quota refusal; zero-delta return stays before inspection. ScopeRun carries E and typed stop code from the existing repeatable-read snapshot before base_data hydrates values. retained_output_bytes uses the same qualified metadata helper in one snapshot and refuses a missing Run rather than returning zero.

These are existing path/frontier regression tests, not acceptance for genuine old E>L in-flight success, snapshot/CAS races, actual legacy overlay or the new scheduler refusal producer. Those focused frozen-producer, application/consumer, rollback and upgrade gates remain pending. Scheduler does not yet consume the new frontier fields; new native drain production remains explicitly unavailable in this revision. This source increment is not the complete deployable bridge.
