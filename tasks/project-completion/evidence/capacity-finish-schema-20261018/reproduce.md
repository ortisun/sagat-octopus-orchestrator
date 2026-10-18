# Dormant capacity schema evidence

Base0ad46f4b. Real disposable PostgreSQL16.15; SAGAT_TEST_AS_RUNTIME_ROLE=1, unset
SAGAT_TEST_DATABASE_URL. CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216;
shared warm CARGO_TARGET_DIR is optional. Exact commands/exits are in commands.json; uninstrumented
whole-command durations are explicitly null, with build/test times retained in raw logs.

The first positive test failed on missing capacity_groups (42P01), an actual schema-availability
failure, not a behavioral counterexample. The initial expanded test failed compilation because
SQLx0.9 requires audited dynamic SQL; only adjacent hard-coded test assignments/columns enter
AssertSqlSafe. The next run passed7/9 and exposed two probe mistakes: invalid possession hits
consistent CHECK first; INSERT SELECT * hits generated-column rules before runtime privilege
checks. Both probes were corrected. No production schema change was needed for those failures.
Final schema9/9 in18.13s; first separate valid fixture1/1 in1.92s. Classification1/1 in1.79s
(25.86s command). Strict locked workspace/all-target Clippy54.85s, fmt4.23s, architecture24crates.

Schema cases: deferred current-holder ACTIVE FK fails actual COMMIT and preserves original state;
matching header/member/receipt writes commit under actual sagat_app. Narrow privileges verified
through catalog and actual denied statements. Counter/check/immutable-binding and foreign
namespace/tenant-chain probes; immutable Retired/Finished tombstones and append-only receipt
owner triggers; historical receipt holder survives its later Finish, one-Finish-per-member
uniqueness, valid vacant nonholder unchanged epoch and fixed32-byte fingerprints. Empty DOWN/UP
works; retained retired/member/receipt rows refuse DOWN with typed55000 and migration version
unchanged. Concurrent DOWN observes actual pg_blocking_pids wait behind uncommitted writer,
then refuses after that writer commits. No sleeps as ordering authority.

Owner root-chain fixtures establish referential shape only, never actual ChildRoot/finality/fence.
The schema does not reconcile aggregate counts or prove runtime receipt dedup/replay, native
producer, audit atomicity, unknown-COMMIT recovery or production rollout. Permanent member/receipt
chain FKs pin history cleanup, explicitly a C0 gate. No S3, SDK58,100k, fullworkspace test run,
Compose, advisory or general nested execution acceptance is claimed. No external CLI review.
Root implemented and executed tests; fresh Astra/xhigh reviewed actual SQL without concrete issues.
