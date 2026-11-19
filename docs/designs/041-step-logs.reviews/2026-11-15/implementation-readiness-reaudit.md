# EGM implementation-readiness re-audit of design 041 (revision of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 plus the uncommitted revision of 2026-11-15 (committed with this report)
Reviewer: egm-implementation-readiness
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Target: DetailedRange, from `## Detailed Implementation Plan` (line 999) to the end, with lines 1-998 read as the specification
Standard: `implementation-readiness-standard.md`
Note: one fresh `egm:egm-implementation-readiness` agent, the one re-audit the EGM cap allows; given the design, `specs/041-step-logs/{contracts,data-model,research}.md`, the ADRs and the previous `implementation-readiness.md`, not `resolution.md`. Its report is recorded here, condensed. The author's answer to each open finding is in `resolution.md` section 5.

Verdict of the reviewer: not ready; no Blocker; of the 40 earlier entries 30 answered, 10 open (5 Major, 5 Nit); 13 new.

## Earlier entries still open

- 4 (Major): the battery fails on an operation that is neither implemented nor in `not_built` (`battery.rs:584-604`); T041-3 does not add the four operations there and the route tasks do not remove them.
- 5 (Major): T041-4's migration must restate `quota_limits_tenant_scoped` (`20270108000000_pending_resets.up.sql:29-30`); "0 accepted as `validate` accepts the exception of `EventRetentionSeconds`" misreads `quota.rs:547, 569`; no mechanism for the minimums.
- 14 (Major): T041-9 measures "through the RPC" in `scalability-ha`, which has no gRPC path.
- 16 (Nit): T041-28 omits the capability enum (`crates/application/src/instances.rs:129`) and which binaries announce it.
- 20 (Major): step ops and behaviours run in each SDK's conformance runner, none of which is in a task; whether `stepLogs` is an SDK call or raw HTTP is open; until then a scenario fails because the op is unknown.
- 23 (Major): the new selector rule misses `service.rs` and the read routes; the constitution's CI gate names the suites a pull request runs "and only those" (`docs/constitution.md:221-225`), so a new code-to-suite rule needs an owner gate; `scripts/tests/test_select_test_targets.py` is not listed.
- 26 (Nit): scenarios 4 and 5 state no failing reason.
- 30 (Nit): `sdks/SPEC.md` has two `## 10.` headings; the minor register belongs in §9 (versioning), not §7 (workflow builders).
- 35 (Nit): T041-6 and T041-8 are still dense blocks.
- 37 (Nit): "state O041-001's default first" is still there.

## New

- N1 (Major): T041-3, T041-11, T041-12, T041-29 and T041-30 change the generated `docs/reference/api.md` (`tests/contract/developer-experience/reference.rs:14-27, 48`), which only T041-25 regenerates.
- N2 (Major): T041-5's source-text test forbids `step_log` in modules where T041-6 and T041-7 add `supports_step_log_protocol` and the policy.
- N3 (Major): `current_worker_role` collides with `the_shared_secrets_lock_has_one_caller_and_the_scoped_path_never_upgrades` (`quotas.rs:1323-1344`).
- N4 (Major): `SAGAT_STEP_LOGS_READ_ENABLED` is parsed by no task (`apps/control-plane/src/config.rs:601`, `cli.rs:192`, `tests/config.rs:991-1006` are the pattern).
- N5 (Major): "the fault hooks of T041-7" do not exist; no fault-point facility exists in the crates.
- N6 (Major): the FR-041-022 predicate has no named home shared by T041-13 and T041-29.
- N7 (Major): T041-3 freezes the wire without waiting for O041-014 and O041-018.
- N8 (Major): T041-6's tests cannot obtain a 1.7 session before T041-7 (`worker.rs:69, 82-90`).
- N9 (Major): T041-31's merged state is undefined (a driver against tables no migration creates).
- N10 (Nit): `docs/releases/0.3.0.md` does not exist (`unreleased.md` does).
- N11 (Nit): `sdks/rust/build.rs` and its conformance copy derive `Debug` for the request.
- N12 (Nit): `WorkerConfig.java:55` is `builder()` (the `Builder` is at 142); T041-6 omits `workers.rs:199-206`; T041-7 omits `apps/execution-engine/tests/config.rs`.
- N13 (Nit): the OpenAPI `Quota` enums have two owners (T041-3 and T041-4).
- N14 (Nit): `AppendStepLogsRequest.task_id` and `instance` are not used in step 2.

Counts: Blocker 0, Major 14, Nit 10
