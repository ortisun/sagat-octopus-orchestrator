# Dormant native Finish observations

Base8c68a16e. Five core files: native wrapper/helper extraction, closed telemetry module/export,
new native tests/registration. No SQL, lock, transaction, caller or activation semantics change.
PostgreSQL16.15 disposable fixtures; explicit sagat_app Finish pool in both suite modes. Other
runtime writers use default versus SAGAT_TEST_AS_RUNTIME_ROLE=1 as recorded. Unset external
SAGAT_TEST_DATABASE_URL; CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. Warm target
optional. commands.json records exact calls/exits/wall times; source snapshots and raw logs kept.

RED missing confirmed transition0vs1:0/1 in3.12s. Initial implementation1/1 in3.21s. Bounded review
found possible fast-server test ordering, fixed with deferred receipt COMMIT gate released only
after reply EOF. Strengthened span outcome counts/transition label checks; default1/1 in3.17s,
all native capacity runtime24/24 in64.70s. Second review found filtered child mutating caller;
new regressionRED0/1 in2.20s. Record on captured exact Finish span, instrument clone. Final source
focused observations default2/2 in5.08s and runtime2/2 in5.38s. The24-test run preceded this exact
handle correction and new regression; do not call it a final25-test run. Contracts9/9 in0.44s,
strict locked workspace/all-target Clippy81.88s, fmt4.08s and24-crate architecture pass.

One current-thread test uses a thread-local detached recorder across actual awaits and existing
collector OTLP/HTTP export. Typed refusal, real held G Busy, actual deferred COMMIT23514 abort,
first acknowledged commit, receipt replay, actual gated COMMIT reply loss+server commitment and
same-command recovery yield returned counts applied1/replayed2/busy1/refused1/storage_error1/
unknown1. Duration histogram counts match. Transition count1 is unlabeled: the unknown committed
receipt does not turn into an acknowledged local transition, and recovery replay adds none.
Spans have exact group-associated outcomes, trace parent, six reference identities, finite
outcome and fixed error status only for storage/unknown. Fixed completion logs contain neither
operation keys nor the supplied secret-shaped stale scheduler identity. Filter regression disables
the Finish INFO child while enabling its traced caller; real closed native pool storage failure
leaves caller outcome/status unchanged, no child export, one returned storage_error metric.

Metrics record returned attempts only; abandonment has no invented outcome. Untraced calls emit
metrics without OTLP export. Future production callers require trace context and durable original
command retention/reconstruction. Atomic receipt/audit/outbox remains authoritative; process
telemetry can be missing. Source inspection of scheduler/outbox.rs confirms generic subject routing
accepts the fixed capacity event, whose metric class remains other; no new broker claim/test.
No production caller/creation/admission/eligibility/scoped-path Finish/retention rollout/SDK58/
100k/resource/fullworkspace tests/Compose/advisory/nested activation acceptance. FullB1/B2 and
canonical tasks stay open even though local dormant B2a-f is complete.
