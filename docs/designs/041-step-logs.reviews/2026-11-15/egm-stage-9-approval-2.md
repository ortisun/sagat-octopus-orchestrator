# EGM stage 9 of the second approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: egm-review (stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: fresh agents `egm:egm-implementation-readiness`, `egm:egm-codebase-details` and `egm:egm-style` on lines 1185-1652, dispatched in parallel after stage 8, each given the target, its persona and standard and the reading list. Recorded condensed and not deduplicated across the three; no severity was lowered.

## egm-implementation-readiness (Blocker 1, Major 12, Nit 11)

- IR2-B1 Blocker: Slice D cannot pass CI: CI plays every scenario in every SDK (`.github/workflows/ci.yml:398-406`, `tests/contract/sdk-conformance/main.rs:14-20`), so T041-14 and each SDK task fail until all six land; a staging mechanism is needed.
- IR2-1 Major: T041-32 listed out of order.
- IR2-2 Major: T041-32's paths miss step-log code outside `step_logs` modules (purge, marker, `take_log_prefix` variant, script routes, migrations, SDK capture points in existing files).
- IR2-3 Major: the ingest service, `StepLogTx`'s methods, the wiring (`apps/execution-engine/src/lib.rs:775`, `tests/support/api.rs:616`, test `Options`) and `may_read`'s transaction type are not designed.
- IR2-4 Major: the upgrade-boundary oracle's files (`tests/integration/workflow-primitives/release_upgrade*.rs`) are in no task; T041-4's migration has no procedure, down guard or tests.
- IR2-5 Major: T041-26's mechanics (`OVERLAYS`, `phase.rs`, the module declaration, how the released replica's address reaches the coordinator; a sandbox runner and an `Options` field for part (a)).
- IR2-6 Major: T041-9 cannot produce the 8-replica row (`platform_with` is one API; scalability-ha calls the application in process).
- IR2-7 Major: T041-31's fan-out workflow, its baseline switch, the MultiXact and per-lock wait methods, the held transaction and the ticket rounding are undefined.
- IR2-8 Major: the wrapper has no hold point for the `lock_secrets` variant, and that variant's blocking set is not "only ingest backends".
- IR2-9 Major: T041-20 omits `sdks/rust/conformance/src/faults/grpc.rs`.
- IR2-10 Major: T041-24's journey location does not match the Playwright harness (`tests/e2e/workflow-studio/studio_journeys.rs:95-227`, CI filter `journeys`).
- IR2-11 Major: Verification Plan rows no task builds (redelivery under a new generation, projection rebuild, run stopping, per-shape scrubber tests, canary in stored rows and `run_events` and `audit_records`, audit failure returns no line, lease p99 with a full Logs class).
- IR2-12 Major: a marker's `level` is not specified, which decides what `minLevel` hides.
- Nits: `register_version` anchor; `worker_pb2.pyi` and the regeneration scripts; the Rust build files need `compile_fds_with_config`; the role matrix test file; the gauge's snapshot type and export files; how 128 is carried and `openbao_secrets.up.sql:57,60`; purge page size and `MaintenanceStore` signatures; guard tests that pass from the start; `sdks/SPEC.md` §9 split; the PostgreSQL 18 CI step; `endpoints.ts`.

## egm-codebase-details (Blocker 0, Major 9, Nit 11)

- CD2-1 Major: T041-32 order (= IR2-1).
- CD2-2 Major: the `take_log_prefix` variant: `scripts.rs:1161-1855` includes `record_execution`'s own call (`:1855`), and section 7 says a new argument.
- CD2-3 Major: T041-29's "route's read transaction" does not exist (`PgScriptStore::execution`, `scripts.rs:1049-1100`); Files short; executions with no run unhandled.
- CD2-4 Major: the switches cannot reach the in-process test platform (`tests/support/api.rs` `Options`, `:226-282`).
- CD2-5 Major: T041-9's setup is undefined (one gRPC server per `TestApi`; workers register at protocol "1").
- CD2-6 Major: T041-7's done condition fails for Rust until T041-20 (`protocol-negotiation.json:61-65`, `sdks/rust/conformance/src/runtime.rs:477`, `sdks/rust/src/worker.rs:468`).
- CD2-7 Major: T041-20 omits the Rust fault proxy `faults/grpc.rs` (= IR2-9).
- CD2-8 Major: every SDK's RPC wrapper and test fake is missing from Slice D (`grpc.ts:70`, `rpc.go`, `_grpc.py`, `WorkerRpc.java`, `WorkerRpc.cs`, `worker_rpc.rs`).
- CD2-9 Major: the Verification Plan's CI does not exist (lints only on a full run, `ci.yml:91-93, 126-128`; no runtime-role step, `ci.yml:179-187`).
- Nits: no prost `Config` in the SDK builds; regeneration commands and `worker_pb2.pyi`; stale anchors; module and overlay declarations; the Slice D command picks no SDK; T041-2 tick; `tests/support/lock_order.rs` not reused; `isolated_server` with `SAGAT_TEST_DATABASE_URL` and its return type; no RPC battery exists; unnamed insert sites and sandbox target; no alert rule test.

## egm-style (Blocker 0, Major 11, Nit 18)

- ST2-1 Major: task order contradicts itself (T041-32; Slice D's parallel exception).
- ST2-2 Major: the test-first rule cannot be met by test-only tasks (T041-8, T041-24, T041-26, T041-28).
- ST2-3 Major: O041-020 and O041-021 have no task that owns them; T041-2's instruction contradicts its done condition.
- ST2-4 Major: R undefined in T041-31.
- ST2-5 Major: bare and partial paths against "relative to the repository root".
- ST2-6 Major: T041-5 and T041-11 done conditions omit or blur what must pass.
- ST2-7 Major: Verification Plan rows with no task (= IR2-11).
- ST2-8 Major: the counting invariant stated with three terms (`last_sequence`, `highest_sequence`, "add up").
- ST2-9 Major: "limit" and "ceiling" used interchangeably although T041-4 separates them.
- ST2-10 Major: "the three repository reviewers" not named.
- ST2-11 Major: T041-9 asks for 500-record batches its workers never send.
- Nits (18): "nothing runs yet"; "at once" in two senses; protocol version wording; undefined terms in the range; done conditions with no command; T041-24's reflow and keyboard journey; duplicated work (§9, the release note); "also as the runtime role" uneven; stale anchors after earlier tasks; vague Files; the gaps paragraph; the estimate; benchmark naming and `step-logs-mixed-version`; "meet"; the protoc version string; the verification row's "and"; "proven by a failing test" for drift findings; T041-2's `Owner-gate`.

Counts: Blocker 1, Major 32, Nit 40
