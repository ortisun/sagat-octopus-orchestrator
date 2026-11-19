# EGM stage 9 of the approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: egm-review (stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents `egm:egm-implementation-readiness`, `egm:egm-codebase-details` and `egm:egm-style` on lines 1110-1486, dispatched directly; recorded condensed, not deduplicated across the three.

## egm-implementation-readiness (Blocker 0, Major 9, Nit 10)

- IR-1 T041-4 breaks the exhaustive match in `crates/infra-postgres/src/quotas.rs:1219-1287` and `tests/integration/security-multitenancy/quotas.rs:507-598`; the `tenant_step_log_bytes` usage arm needs a column T041-5 adds.
- IR-2 T041-5 omits the two accessors, the routing, the `add_delta` argument and the loop filter; the design's "no `SECURITY DEFINER`" contradicts data-model §4.
- IR-3 T041-31's prototype, chunk-row DDL and accounting class are undefined.
- IR-4 The horizon tests cannot get "a container of their own" from `tests/support/mod.rs:120-176`; `bounded.rs:81` is another target.
- IR-5 T041-8's `dropResponse` proxy is in `sdks/rust/conformance`, which `sagat-tests` does not depend on.
- IR-6 T041-9's per-SDK micro-benchmarks and matcher fallback have no owner.
- IR-7 T041-32's selector rule cannot match parts of a file or unnamed SDK paths; `api-grpc/src/service.rs` is missing.
- IR-8 T041-26 names no target and no way to obtain an old API replica.
- IR-9 The control plane never receives `SAGAT_STEP_LOGS_ENABLED` for the script marker.
- Nits: Rust SDK refuses accepted > 6 (`worker.rs:468`); `supports_step_log_protocol` form; callers of `current_worker_authorized`; `take_log_prefix` argument and callers; lock-order test needs later tasks; "within the bound" assertion; missing Owner-gate on T041-23, T041-26; T041-13 test location; T041-22 and T041-25 Files; T041-32 placement.

## egm-codebase-details (Blocker 0, Major 8, Nit 4)

- CD-1 = IR-5 (`tests/integration/database-runner/proxy.rs:21-22` already has a lost-answer proxy in `sagat-tests`).
- CD-2 = IR-4.
- CD-3 = IR-9 (`crates/api-http/src/routes/runner.rs:291` in the control plane).
- CD-4 No task owns the `recordScriptExecution` contract change (openapi, digest, reference, `routes/runner.rs:296-324`).
- CD-5 The oldest-transaction gauge (`operations.rs:425-451`, likely `pg_read_all_stats`) and the purge and marker metrics are in no task's Files.
- CD-6 Files lists omit compile-needed files (Rust SDK `lib.rs:41` and conformance `grpc.rs:6`; `RetainedUsage` at `application/src/quotas.rs:35`; manifest and privilege tests; callers of `current_worker_authorized`; Python `activity.py:229`).
- CD-7 T041-5's source-text test skips `crates/application/src/worker/`.
- CD-8 T041-9 names no entry point to drive SDK workers through the workload (`harness.rs:328` is private).
- Nits: `negotiate_protocol_details` wording; inline-code-sandbox target; `step_log_isolation.rs` in T041-6; T041-32 placement.

## egm-style (Blocker 0, Major 10, Nit 14)

- ST-1 "seen failing for the reason the task states": only T041-14 states one.
- ST-2 "within the bound T041-31 measured" has no number; which T041-31 variant must pass.
- ST-3 T041-11 "(both sources)" against T041-13; the end horizon in T041-11 and T041-12.
- ST-4 T041-12 and T041-30 do not switch their operations to `x-implemented`.
- ST-5 No order between tasks for the lock-order and barrier tests that need T041-10 and T041-30.
- ST-6 T041-30 modifies a runbook T041-25 creates.
- ST-7 = IR-7.
- ST-8 T041-15 to T041-20 have no done condition and no runner work.
- ST-9 T041-9's baseline differs from SC-041-007's; micro-benchmarks unowned.
- ST-10 T041-26 has no expected outcomes.
- Nits: wording of T041-2's condition; "starts" against "merges"; T041-31's held-transaction workload; "four messages"; "deferred answers"; T041-8 Refs; T041-28's "use"; first-failure wording; "the row of section 6"; partial paths; T041-25 quickstart; T041-27 done condition; gap range; formatting.

Counts: Blocker 0, Major 27, Nit 28
