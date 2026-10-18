# Immutable-old cancellation before original successful report

D2 only. Frozen6c44a0e4 produces a genuine original c10/0/reserve lease after ten historical aggregates: L=2,100,000,V=2,100,020,E=4,200,020. Current public CANCEL owns the first stop before normal scheduler draining. Current public success uses the same original capability/request, succeeds without retaining output, and remains eligible for genuine compensation.

## Actual execution

Root C2 cancellation-before-report passes four separate old/current phases in198.77s: owner producer11.68s/current consumer3.00s, runtime producer11.79s/current consumer3.07s. Actual pool roles postgres/postgres and sagat_app/sagat_app. No registration, binding/grant write, lease replacement or live expiry rewrite occurs on upgrade. The old process exits/reaps before the current process; both binaries are built before the old lease is created.

Public CANCEL records exactly one original CANCELLED/null stop, original actor/reason/trace and complete independently verified operation receipt/fingerprint. Normal tick starts the supported CANCELLED drain without replacing that stop or creating a data-limit drain audit. The original lease remains live and reservations held through another waiting tick. Original successful report returns SUCCEEDED/CANCELLING, adds no refused payload/reference/logical charge/report outbox, and stores the exact original report receipt/ACK. Both report and operation replays preserve their durable receipt identity; operation replay correctly reflects current run state. Exactly one truthful successful-output-refusal audit is allowed after success, with no other or NULL-reason data-limit audit at waiting/replay/final boundaries.

Normal scheduling offers the real line.release compensation with original target and idempotency key. It cannot finalize while that task is leased. Real completion precedes final CANCELLED with null code, original cancelled scope/saga cause and DONE/COMPENSATED saga. Cold journal verification, immutable old source and all exact receipt/head oracles pass. This is engine compensation, not external-provider undo.

Shared default report-before-drain regression passes its four phases in173.73s (owner11.05/2.36s, runtime11.64/2.40s). Shared drain-before-report regression passes its four phases in154.46s (owner6.14/1.51s, runtime6.14/1.47s). These are three invocations and12 separate phase passes, not12 distinct test cases. Each invocation has1125 identical native input hashes before/after. Exact command records, raw private-safe phase logs and old/current manifests are retained; credential-bearing receipts are never copied and are removed by the coordinator.

## Review and gates

Fresh bounded decision review: None. Three independent C1 code units reviewed operation receipt, cancelled drain/report, and compensation/coordinator. The latter two identified the same incomplete audit oracle; classified actionable and fixed once. Fresh C2 audit-only delta review: None. Original draft/notes and exact C1-to-C2 delta are retained. No runtime RED is invented for the static finding. GPT-5.6 Sol drafted; fresh GPT-6 Astra reviewed; root alone applied and executed. Previously declined optional external review was not repeated.

Final strict integration-target Clippy passes33.54s, fmt7.03s, architecture0.97s; shell syntax passes. C1 preflight Clippy16.37s is separate and has no runtime claim. Source/doc whitespace is checked separately from raw patches/logs. scripts/check-agent-config.py does not exist in this checkout and was not executed. Source snapshots/index and frozen manifests bind the executed C2.

## Reproduce / scope

Use the repository-declared disposable PostgreSQL with pg_stat_statements/max_connections300, absolute coordinated CARGO_TARGET_DIR, SAGAT_TEST_DATABASE_URL and SAGAT_Q10C_OWNED_POSTGRES=1. Run bash scripts/test-legacy-collection-inflight-report-v1.sh --case cancellation-before-report; default and --case drain-before-report are independent fresh-DB regressions. Command JSON files retain exact invocations and timestamps.

Only ordinary inline, no-declared-secret E>limit cancellation precedence is accepted. Under-limit increment, Applied, output-dependent old compensation, OS-process death immediately after ACK, uncertain COMMIT, external effects, general nesting and rollout remain separate. No production, migration, grant or public shape changed. Full Q10c and all eight open canonical014 tasks remain open.
