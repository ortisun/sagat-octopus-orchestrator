# Managed report replay: binding before catalog

Base56044f75. Repository disposable PostgreSQL + realOpenBao2.7.1; no mockmanaged lease.
Unset SAGAT_TEST_DATABASE_URL. Set SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; optional shared warm CARGO_TARGET_DIR.
Exact commands/exits/walltimes in commands.json/regression-commands.json/gates.json.

RED aggregate0/1,17.62s: public replay of real managed failure receipt waits on exact Binding S
while already holding namespace Secrets X. No fixture/compile failure or inferredSQLdeadlock.
Apply production.diff: full connections3/3,27.25s. Real known blocker+sagat_app wait, pg_try
Secrets probe succeeds; releasebinding, exactalready-applied receipt/status, unchanged atomic
runversion/events/outbox/audit/pincounts andverify_run. CompletedSession explicitly removed,
then publicregistration+poll claim subsequent managedrun. Rootapplied/tested; agentsread-only.

No new Session/TTL/role/provider/API field or lease policy. Existing receipt fingerprint and
durable attempt owner remain checked. Missing/live andordinary paths unchanged. No provider
I/O/replay effects. No reproduced three-party deadlock, freshfullworkspace/SDK58/Compose/
advisory/resource/group acceptance or canonical checkbox closure claimed.
