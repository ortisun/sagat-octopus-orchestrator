# Full/exact-path claim session capacity after wait

Base875ff086. Repository disposable PostgreSQL; unset SAGAT_TEST_DATABASE_URL.
Set SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3,
RUST_MIN_STACK=16777216; optional shared warm CARGO_TARGET_DIR. Commands/exits/wall times in
commands.json/regression-commands.json/gates.json; raw test durations in logs.

RED0/2 in5.58s: both native ports return8 after an unchanged-row wait crosses persisted session
expiry. No fixture/compile failure. GREEN2/2 in7.02s: actual known-blocker sagat_app wait while
live; rollback only after database expiry; zero room and unchanged21-table execution snapshot
plus unchanged expiry. Fresh public registration supplies8; old authority stays0; verify_run.
The native ports intentionally bypass publicpoll's earlier renewal to test claim-time authority.
No claim of publicexpired-session leasing is made from this capacity-only counterexample.

Shared private transaction-typed helper preserves tenant/ns/id/hash/subject filters, current
count/saturation and each caller's prefix. Root has no binding guard, scoped has one. Session
row remains on owning TX through count/lease commit, no renewal. Time is sampled after session
lock, not continuously through commit. Managed-database SHARE and complete B0/group are pending.
No fresh allworkspace/SDK58/Compose/advisory/resource/nesting acceptance or checkbox closure.
No telemetry counter assertions.
