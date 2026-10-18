# B2e.3 exact child authority and namespace proof

Base e77e9497. Production source unchanged. PostgreSQL16.15 disposable databases; native Finish
and handoff ports explicitly connect as sagat_app, including default suite mode. Only dormant G
membership/header baseline is owner seeded; physical child creation, handoff and completion use
real runtime ports. No nested runtime, production creation, eligibility or SDK acceptance.

Unset SAGAT_TEST_DATABASE_URL. Default unsets SAGAT_TEST_AS_RUNTIME_ROLE; full runtime sets1.
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. Shared warm target is optional.
commands.json records exact invocation/exits/wall times. source-final.json records final source.
Initial handoff1/1 in2.23s, initial namespace1/1 in4.23s; no failures in this increment.
Final both tests default2/2 in6.50s, full runtime2/2 in6.65s. The namespace test independently
executes both holder placements in separate databases. Strict locked all-target workspace Clippy
8.12s, fmt4.06s, architecture24crates pass. These are focused tests, not full workspace execution.

Handoff queues first behind an unchanged chain U gate; native Finish reads binding then queues
behind that actual writer. Observed pg_blocking_pids and own Run NOWAIT establish ordering before
release. Actual continuation wins, old head rejects with no capacity effects; actual successor
worker FINAL then settles only current head. Execution snapshot unchanged by Finish.

Two actual physical child chains of one tenant in separate namespaces share one owner-seeded G.
Holder in first namespace vacates on first Finish; holder in second remains held across first
Finish. Last member retires; original first command replays its exact historical result even
when its stored holder is now FINISHED. Wrong namespace is BindingMismatch with zero effects.
Execution/usage snapshots remain unchanged; receipt-linked audit/outbox identities and canonical
actual hints validate. No foreign business run mutation or public namespace API is added.

B2e local closes; B2f/fullB1/B2/C0/durable caller intent/process restart/SDK58/100k/resource/
Compose/advisory/release gates remain open. No production caller or nested activation.
