# EGM stage 9 of the third approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: egm-review (stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents `egm:egm-implementation-readiness`, `egm:egm-codebase-details` and `egm:egm-style` on lines 1270-1818, dispatched in parallel after stage 8, each given the target, its persona and standard and the reading list. Recorded condensed and not deduplicated across the three; no severity was lowered.

## egm-implementation-readiness (Blocker 0, Major 11, Nit 11)

- IR3-1 Major: raising the minor to 7 breaks `tests/e2e/sdk-multilanguage/us04.rs:29`, `tests/e2e/developer-experience/doctor.rs:323` (`crates/api-http/src/routes/identity.rs:68`) and `tests/contract/durable-execution/grpc/protocol_compatibility.rs:131, 138`, which T041-7 does not list.
- IR3-2 Major: CI runs one named promtool file (`ci.yml:617-622`), so T041-25's rule test never runs; `contract_observability_operations` is not in its done condition.
- IR3-3 Major: metric ownership and definitions (`step_log_purge_backlog` undefined; tail metrics owned by no task; which task emits the ingest counters).
- IR3-4 Major: the micro-benchmarks name no harness, invocation or evidence, and the fallback matcher leaves an algorithm or a dependency to the implementer.
- IR3-5 Major: T041-31's rules: p50 or p99 of S, the rate (one or two batches a step), thresholds for the other figures, and the attribution of a failure to O041-020, -021 or -026.
- IR3-6 Major: T041-7's lease-p99 assertion is a timing test in CI.
- IR3-7 Major: T041-8's forced database error has no mechanism, and no support reads the container's log.
- IR3-8 Major: T041-9 cannot use T041-7's tickets (`crates/api-grpc/src/lib.rs:12` keeps `limits` private).
- IR3-9 Major: T041-32's SDK paths are left to the implementer.
- IR3-10 Major: no runner has an "unknown behaviour" refusal (behaviours are ignored, `sdks/typescript/conformance/runner.ts:215-290`); only "unknown step op" exists, with six messages.
- IR3-11 Major: the Studio has no script-execution view; how a marker opens the log is unspecified.
- Nits: the CI job name; Files omissions (`content_usage.rs`, `apps/execution-engine/src/lib.rs`, `apps/control-plane/src/lib.rs`, `RunMode.Operations.cs`, `Operations.java`, a support client for batches); the missing hold after T3; the statement-budget test's file; `sdks/SPEC.md` numbering; the digest value; T041-26(b)'s case names and hand-over; `source = script` before T041-13; goldens; `limit_name()` strings and the spike's fences; untestable phrasing.

## egm-codebase-details (Blocker 1, Major 5, Nit 9)

- CD3-B1 Blocker: T041-25's claim that CI runs the rule tests is false (`ci.yml:617-622` runs `batch_stalled_test.yaml` only); `ci.yml` is not in its Files.
- CD3-1 Major: no `integration` job exists; the suites run in the `rust` job (`ci.yml:179-187`, condition as `:95`).
- CD3-2 Major: the digest test's predicate (`tests/contract/payload-classification/openapi.rs:349-358`) needs both `x-planned` and `x-implemented == "041"`; T041-3 says only "with `041`".
- CD3-3 Major: the migrations' up and down tests have no file or target.
- CD3-4 Major: T041-32's SDK path set is left to the implementer.
- CD3-5 Major: `eventually` gives up after 5 s (`tests/support/mod.rs:388`), while holds last up to 60 s in the same process.
- Nits: the stub tool versions are not named in T041-3; `sdks/SPEC.md` §10 headings and the existing §9 row; T041-2's tick and who commits owner-task edits; T041-13's unnamed `Options` field; where `worker_proxy.rs` is declared; Files omissions; `pytest` against `unittest`; the `SHARED_INPUTS` carve-out changes nothing; T041-6's `PERMISSION_DENIED` before the handler exists.

## egm-style (Blocker 0, Major 10, Nit 18)

- ST3-1 Major: T041-2 and T041-33 change committed files but "have no pull request".
- ST3-2 Major: a test needs a hold after T3, which the five hooks lack.
- ST3-3 Major: T041-6 polls `heldBack` before the read path exists (T041-11).
- ST3-4 Major: T041-10 tests reader behaviour that T041-11 builds later.
- ST3-5 Major: T041-28 lands before T041-11 but tests `may_read`.
- ST3-6 Major: T041-32's path set cannot be determined from the text.
- ST3-7 Major: T041-3's done condition omits its new SDK unit tests and the stub regeneration.
- ST3-8 Major: T041-13's `Options` field against T041-6's `step_logs_enabled`.
- ST3-9 Major: T041-25 cites a release-note line T041-29 never writes.
- ST3-10 Major: the canary in the routes' bodies is scheduled by no task.
- Nits (18): placement of "Waits for"; runtime role on a script check; a truncated test name; the cap not named; the p99 baseline; T041-31's Files; "the two moves" and "the switch"; a verbless sentence; vague descriptions in T041-29; "these pass" for behaviours; two names for the index; `last_sequence` against "highest sequence"; rows without tasks; G041-008's status; what the RPC answers between T041-6 and T041-7; "the server canary"; an ambiguous "which"; the estimate's implied nouns.

Counts: Blocker 1, Major 26, Nit 38
