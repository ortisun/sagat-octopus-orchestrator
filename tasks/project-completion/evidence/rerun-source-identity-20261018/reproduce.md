# Rerun source identity before admission

Base83010738. Use repository disposable PostgreSQL16.15 and real OpenBao2.7.1 fixtures.
Unset SAGAT_TEST_DATABASE_URL; set SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. Shared warm CARGO_TARGET_DIR optional.
Commands, process exits and wall times: commands.json, gates.json, final-commands.json.
Raw test execution durations differ from command wall times (compilation included in the latter).

compile-error.txt is an initial harness compile failure, not behavioral RED. compile-source.json
and compile-test.diff retain that attempt. The corrected harness with original production in
red-source.json/test-red.diff produces actual RED0/1,8.47s: Secrets retained before source wait;
PostgreSQL deadlocks0→1; rerun succeeds, native terminal pin cleanup returns Unavailable.
A known external active-quota gate orders the actual app-role rerun and cleanup, not stand-in SQL.
The source-blocked public rerun is cancelled before releasing the external source tuple and atomic
run/event/outbox/audit/payload/pin counts remain unchanged before the same-key retry.

production.diff advances exact tenant/source KEY SHARE after receipt replay, before admission;
missing source remains MissingReference and no effects. Cleanup U SKIP LOCKED remains byte-identical.
Original GREEN full connections3/3,27.30s. Final strengthened control compares the new pin's
version/provider-operation to current secret metadata and against original source, without values.
The first stronger-control execution failed on INT4/i64 fixture decoding, not production semantics;
its log/source/diff remain retained. Explicit bigint casts correct only those metadata reads.
Keep final-tested-source.json/test-final.diff and final-commands.json for the actual final result.
Continuation segment rerun regression1/1,1.95s. Full workspace test/SDK58/100k/Compose/advisory/
S3/group eligibility and old-writer acceptance are outside this focused unit, no task closure.

Current root-rerun/child-norerun separation is a writer invariant, not schema/type enforcement.
Identity guards are not source business authority. Final source/ADR review is bounded to this unit;
A/B/C inventories and whole nested-capacity approval are separate. Only root runs tests. Owner's
current-review selection applies, no external review CLI invoked.

Final strengthened full connections3/3,27.36s; strict Clippy2.87s, format4.11s, architecture24.
