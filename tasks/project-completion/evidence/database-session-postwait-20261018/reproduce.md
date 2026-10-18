# Managed owner session SHARE freshness

Base4f060e98. Disposable repository PostgreSQL and actualOpenBao2.7.1, no mocks.
Unset SAGAT_TEST_DATABASE_URL; set SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. Optional warm CARGO_TARGET_DIR. Commands/exits/
walltimes in commands.json/regression-commands.json/gates.json; rawtest times inlogs.

New regression lives inside existing16MiB aggregate fixture after actual managed lease; it
uses publicheartbeat and public same-name reconnect, not forgedmanaged storage. RED0/1
17.20s reached it after earlier aggregate scenarios: accepted=true/lease_extended=true after
actual live app-role SessionSHARE wait crossed unchanged-row expiry. No compile/setup failure.
Applyproduction.diff: completeconnections target3/3,28.78s, no effects on refusedheartbeat,
valid reconnect recovers and all prior cases continue. Independentatomic snapshot excludes
bearers, hashes, credentials and SQL text. This is not credential-disclosure evidence.

Rebuilt isolated runner integration1/1,28.49s, benchmarkcase filtered; not freshperformance
acceptance. Clippy67.59s/fmt/architecture/diffpass. No new fullworkspace/SDK58/Compose/advisory/
resource/nested acceptance or taskclosure. Existing source metric/error/audit mappings retained;
no counter assertion. Complete B0/group graph remains open after these focused expiry checks.
