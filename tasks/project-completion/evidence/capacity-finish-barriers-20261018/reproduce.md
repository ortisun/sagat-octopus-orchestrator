# B2e.1 native barriers and confirmed aborts

Base932188b1. No production adapter changed. Real disposable PostgreSQL16.15; unset
SAGAT_TEST_DATABASE_URL. CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216,
optional shared warm target. All native adapters explicitly connect as sagat_app. Execute exact
commands in commands.json; default mode unsets SAGAT_TEST_AS_RUNTIME_ROLE, full-runtime mode sets1.

Initial4/4 (11.47s) used whole-world runtime mode. Review caught reliance on that mode for blocker
filters, corrected by explicit native runtime pool. Corrected default4/4 (13.87s), full-runtime4/4
(14.24s). Final injected-marker assertion rerun atomicity1/1 in both modes (default1.98s,
full-runtime2.02s; raw logs are authoritative). Other three proof bodies were unchanged by that
assertion. Strict locked workspace/all-target Clippy8.01s, final fmt and24-crate architecture pass.

G Busy occurs while an external unchanged header remains locked; fresh NOWAIT probes acquire
own chain/run before that G is released. Header/member and all side effects stay unchanged, then
same-command retry succeeds. Two duplicate commands actually wait on own chain before either
locks Run; recursive blocker observations include soft queue dependencies. Release yields exactly
one apply and one identical replay, with one receipt/audit/hint.

First application and receipt replay each reach an unchanged shard-row wait while DB time is
still before committed expiry. The gate releases only after DB time passes expiry; both refuse
without effects, and renewed current authority permits same-command apply/replay respectively.
Test-only BEFORE audit/outbox/receipt failures and deferred receipt constraint COMMIT23514 failure
all return the injected-marker confirmed storage error, leave no partial header/member/audit/
outbox/receipt, preserve execution/usage snapshots, and allow one fresh same-key success.

No fake retained-usage delta or quota barrier. Actual commit-ack loss, unresolved same-command
reconciliation versus distinct-member Busy, actual handoff/discovery race, same-tenant multiple
namespace group evidence and further fault recovery remain B2e.2/3. No full B2e/B2, canonical task,
SDK58/fullworkspace execution/100k/resource/rollout/nested acceptance is claimed.
