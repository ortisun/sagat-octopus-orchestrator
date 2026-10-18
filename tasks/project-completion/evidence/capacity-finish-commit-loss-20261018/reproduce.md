# B2e.2 actual unresolved COMMIT and response loss

Basea8f07235. Production adapter unchanged. One Rust test contains two independently asserted
server outcomes, in separate disposable PostgreSQL16.15 databases. Native/proxy/direct adapter
pools explicitly authenticate as sagat_app in both wider suite modes. Only group baseline metadata
is owner seeded; actual child creation/finality uses scheduler/worker runtime writers.

Unset SAGAT_TEST_DATABASE_URL; default also unsets SAGAT_TEST_AS_RUNTIME_ROLE, full-runtime sets1.
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; shared warm target optional.
commands.json records exact commands/exits/wall times, source snapshots and raw logs retained.
First compile failed on a wrong nested hint-module path, no test executed (not behavioral RED).
Corrected initial test1/1 in4.84s. Before final startup cleanup, both modes1/1 passed (4.58s/4.75s).
Final reviewed source: default1/1 in4.49s (36.70s command), full-runtime1/1 in4.71s (5.02s command).
Each1/1 executes committed and aborted branches. Strict locked workspace/all-target Clippy8.06s,
final fmt and24-crate architecture pass. Harness3 bounded reviews resolved2 cleanup issues; sequence
review found none. Cleanup cancellation paths are source reviewed, not separately fault injected.

The receipt has a labelled test-only deferred constraint advisory gate, not a retained quota delta.
Observe actual pg_stat_activity query COMMIT and blocking owner PID after the relay forwards Q.
NOWAIT probes show own chain/run/G/shard remain guarded through this actual pending COMMIT.
Close only client reply while keeping upstream alive. The real native call returns typed Unknown
with its original key while PostgreSQL still waits. A distinct actual FINAL peer reaches G and
gets Busy, releases its own chain/run and preserves membership. The same original command instead
waits on the original own-chain prefix; actual blocker observation precedes timeout/cancellation.
That timeout establishes no original rollback: COMMIT and all guards remain pending.

Release gate. The relay withholds/decodes actual C/COMMIT or labelled E23514 and then Z/I. Commit
has one receipt/audit/hint and selected FINISHED/vacant header; abort has zero effects and old state.
Remove the test probe, use a fresh transaction through the distinct direct runtime pool with the
same original command: committed branch replays, aborted branch applies, ending one of each.
Execution/journal/steps/chain/telemetry/retained usage snapshots stay unchanged; actual outbox
validates canonical AsyncAPI. No actual process restart or durable production caller intent is
claimed. Remaining handoff/discovery, same-tenant multi-namespace and broader recovery/release
acceptance stay pending. No production caller, proxy, lifecycle/eligibility, quota, SDK58/fullworkspace
execution/100k/resource/Compose/advisory/nested activation acceptance from these tests.
