# Dormant native ordinary-leaf certifier

Base c43e8aab. Exact commands, exit codes, elapsed seconds and raw logs are in commands.json.
source-*.json identify tested Rust sources. Cargo uses --locked and the shared warm target:
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. SAGAT_TEST_DATABASE_URL is unset.
Only entries marked runtime_role_suite set SAGAT_TEST_AS_RUNTIME_ROLE=1. Flow always starts its
HTTP/gRPC services explicitly as sagat_app, independently of that suite switch. PostgreSQL16 is
real disposable infrastructure. No Bao/S3/broker/SDK/process-restart result follows from this unit.

Reproduce with cargo test --locked -p sagat-tests --test integration_workflow_primitives
leaf_certification_native:: -- --nocapture --test-threads=1, then repeat in runtime-role mode.
The native happy case creates source through actual HTTP publication/start, scheduler and gRPC
worker completion. Its disposable owner explicitly grants the dormant INSERT. It tests refusal
before closure, audit failure after an uncommitted canonical insert, atomic rollback/retry,
metadata-only preservation, stale-CAS canonical replay and replay after a later collection fails.
The extra timestamp fixture is owner-written canonical metadata solely to test decoder refusal;
it is not proof of successful execution or a legitimate audit record.

Retained failures are real and intentionally not rewritten: missing-adapter.txt is a compile-time
absence failure; guard-log-red.txt is a real PostgreSQL diagnostic leak; canonical-time-red.txt
and scheduler-expiry-red.txt reproduce the SQLx timestamp panic. The scheduler RED used the old
test name and did not reach its initial proposed fence-refusal assertion; final behavior preserves
large live TTLs by comparing expiry in PostgreSQL after locking. No upper TTL policy was added.
The BC canonical date in the initial RED fixture was not reached; final supported BC positive
control and far-future negative controls are identified by the final source snapshot.

The first shared-guard regression filter selected0tests, so it provides no passing regression
evidence. It remains visible in commands.json. Use capacity_finish:: for the real regression.
The SQL privilege audit records a narrow dormant INSERT exception with its release gate; schema
privileges themselves remain SELECT-only. This is deliberate dormancy, not a production grant.

Producer preservation in both transaction orders, bounded source corruption/missing cases,
uncertain COMMIT with fresh reconciliation/restart and closed observations remain pending.
No canonical feature014 checkbox, full B1/B2, source UPDATE immutability, C0, nesting or100k
acceptance follows from this first native increment. The adapter has no production caller.

Final results: native4/4 default55.80s and runtime12.29s; actual capacity_finish:: regression25/25
72.92s; SQL/trigger runtime privilege audit1/1 27.55s; strict workspace all-target Clippy56.17s,
fmt4.21s and architecture24crates0.33s. Whole-command times include compilation where applicable.
The native source proof received one fresh Astra/xhigh review with no finding; driver review
closed after3cycles and the reproduced fixes. No external CLI review ran.
