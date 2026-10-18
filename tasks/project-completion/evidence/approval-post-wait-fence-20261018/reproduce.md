# Approval scheduler authority expiry after row-lock wait

Baseca6d2ba6. Disposable PostgreSQL, unset SAGAT_TEST_DATABASE_URL, set
SAGAT_TEST_AS_RUNTIME_ROLE=1/CARGO_INCREMENTAL=0/CARGO_BUILD_JOBS=3/
RUST_MIN_STACK=16777216. Exact commands/exits/wall times in commands.json and
regression-commands.json. Shared warm target is optional. No external CLI.

Three focused tests: generation overflow retains Storage(Internal); opening and genuinely due
expiry reject authority that expired during an unchanged scheduler-row SHARE wait. Runtime
stores are explicit and the actual blocking backend must be sagat_app under the runtime option.
Atomic21-table snapshot includes telemetry_pending and approval requests/receipts/timers;
the deliberately changed scheduler lease is excluded.

red.txt: overflow passes, opening and due expiry actually commit under expired authority,
1 pass/2 failures65.16s. test-red.diff and red-source.json retain exact tested variants.
production.diff replaces only approval's private scheduler check with lock-then-clock helper,
retaining own-shard and generation-conversion error precedence. No business timestamp change.

The due test polls the PostgreSQL clock up to75seconds for its60-second fixture deadline,
then invokes the3-second unchanged-tuple expiry barrier. Its actual due timestamp is supplied
to audit and occurred_at. The ordinary eventually helper has a5-second limit and is not used
for this business-deadline wait. Fresh authority succeeds; opening audits once, expiry writes
one audit/event/outbox and FIRED timer. Opening replay retains RuntimeUnavailable unchanged.

New private barriers prove app-role calls, not all historical owner fixtures. Regression filters
skip these3 tests because their exact focused run already covers the60-second barrier; other
approval cases still execute. Counts overlap; no all-workspace execution/SDK58/Compose/
advisory/resource/nesting gate or canonical task closure. Expiry is validated at post-lock
sample, not continuously until commit. Human decide authorization is unchanged.

Final GREEN3/3 in67.95s; approval workflow46/46 in35.54s; approval fault21/21 in29.07s.
