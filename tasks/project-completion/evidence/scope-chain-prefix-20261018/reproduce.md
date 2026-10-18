# Collection/loop member-chain before own-run writer entry

Basec65e3354. Disposable repository PostgreSQL; unset SAGAT_TEST_DATABASE_URL and set
SAGAT_TEST_AS_RUNTIME_ROLE=1/CARGO_INCREMENTAL=0/CARGO_BUILD_JOBS=3/
RUST_MIN_STACK=16777216. Commands/exits/wall times in commands.json and
regression-commands.json; shared warm target optional.

Trusted native fixtures: pending collection entry, admitted collection with real cancel then
start-drain, pending loop entry. All set a chain deadline and call the real initializer before
using updated run revision. Stores explicitly use runtime pools; barrier observes sagat_app.
No newly accepted public chain/body composition is implied.

red.txt/test-red.diff/red-source.json: all three actually commit while the chain is held,
0/3 in3.12s; no compile/fixture failure or SQL-deadlock counterexample. production.diff adds
transaction typing and shared member-chain guard before own-run to the three private helpers.
All7 collection/6 loop callers already own begin_safely transactions. green.txt:3/3 in3.04s.

The generic barrier polls the real mutation and an actualpg_blocking_pids chain wait, probes
ownrun NOWAIT while waiting, releases chain, then requires fresh success and exact replay.
This proves the prefix, not separate near-commit lifetime coverage at all three ports. Shared
helper lifetime has separate approval final-audit coverage in priorcommitc65e3354. Atomic
21-table checkpoint remains unchanged on replay; new event/outbox controls assert one write.

Full workflow regression includes the genuine60-second approval deadline case and fresh
10k reference-source materialization, but the distinct long10k completion test remains ignored.
It is not the fullworkspace test suite or acceptance of production/nested100k resource limits.
Clippy compiles workspace/alltargets; tests do not claim SDK58/Compose/advisory/release gates.
Current scope adds a lookup before every primitive run lock; old performance numbers are not
new resource measurements. Complete B0 remains open, including opposite incident/report binding
edges and separate worker-session postwait freshness source/test gaps.
