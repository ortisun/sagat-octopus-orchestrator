# Delivery-1 review corrections — 2026-11-01

Scope: T037-007/008 scoped forward retry guards and T037-009 successful-output refusal.
No commit, merge, public runtime expansion or provider qualification was performed.
The root agent owns final whole-tree verification and task closure.

## Changes

- Scoped forward Retry refuses a global stop and checks every active ancestor, including a
  draining outer scope whose inner collection is still open. Full and bounded-path commands
  produce the same typed refusal without changing their states. Compensation retries retain
  their existing known-outcome behavior.
- A successful root worker report that exceeds the logical run-output ceiling now enters
  consultation/review with `OUTCOME_UNKNOWN`. The scoped fallback does the same when the existing
  atomic known-success refusal path does not apply. This classification occurs at the successful
  report source; `RUN_DATA_LIMIT` remains a known failure for engine-owned/pre-execution limits.
- The flat collection's existing atomic stop plus known-success record remains unchanged.
- A root output-limit report at full tenant storage now also retains `OUTCOME_UNKNOWN`: the
  payload-free review fits, so it no longer creates a growing error route whose persistence would
  trigger the separate `STORAGE_QUOTA_EXCEEDED` fallback.

## Environment and commands

Feature worktree: `feature/037-compensation-safety`, base
`6b6f0fa4bb7b622c960e9d149a4b78785ad0647b`, uncommitted composed tree. Initial disk check:
97 GiB available. Rust 1.98.0. One Cargo command at a time; the root agent coordinated the shared
private target. PostgreSQL tests use repository disposable fixtures and the runtime-role setting.

```sh
export CARGO_TARGET_DIR=/var/folders/60/bf6rmfzx6zq67nrbzb3m0p480000gn/T/sagat-037-target-q0fwz2gu
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUST_TEST_THREADS=2
export DOCKER_HOST=unix:///Users/marcelo/.docker/run/docker.sock SAGAT_TEST_AS_RUNTIME_ROLE=1

cargo test --locked --offline -p sagat-domain --test scoped_compensation stopped_scoped_forward_retry_is_refused_before_native_drain
cargo test --locked --offline -p sagat-domain --test bounded_path_planning nested_forward_retry_is_refused_when_an_outer_scope_is_draining
cargo test --locked --offline -p sagat-tests --test integration_durable_execution run_output_limit_
cargo test --locked --offline -p sagat-tests --test integration_workflow_primitives loop_successful_output_over_run_limit_preserves_review

# Controls executed against that freshly compiled binary, without another Cargo build:
"$CARGO_TARGET_DIR/debug/deps/integration_workflow_primitives-bec365aba8b73d01" internal_input_limit
"$CARGO_TARGET_DIR/debug/deps/integration_workflow_primitives-bec365aba8b73d01" byte_ceilings::a_known_successful_seventeenth_member_is_preserved_without_its_refused_output --exact
```

## Observed evidence

| Check | RED | GREEN |
|---|---|---|
| Global-stop scoped Retry | `d1-stop-retry-red.txt`: accepted Retry instead of typed refusal | `d1-stop-retry-green.txt`: 1 passed; cancellation/deadline × collection/loop, full/path |
| Outer-scope drain Retry | `d1-nested-stop-retry-red.txt`: accepted Retry under draining outer scope | `d1-nested-stop-retry-green.txt`: 1 passed, full/path and unchanged states |
| Root output limit | `d1-output-limit-red.txt`: `Failed` instead of `Waiting` | `d1-output-limit-green.txt`: 2 passed, open/full tenant storage, no error route, direct Retry refusal and receipt replay |
| Loop output limit | `d1-scoped-output-limit-red.txt`: loop `Failed` instead of `Waiting` | `d1-scoped-output-limit-green.txt`: 1 passed, 17 leases, 4,000,000 retained bytes, no defensive compensation/error route, review and receipt replay |
| Internal input limits | Existing controls | `d1-internal-limit-controls.txt`: 2 passed, root/scoped known `RUN_DATA_LIMIT`, no external lease |
| Flat collection known success | Existing control | `d1-known-success-control.txt`: 1 passed, known success/receipt/replay/drain/compensation preserved |

Total targeted GREEN results: **8 passed**, no failed or ignored tests in those commands.
`rustfmt --edition 2024 --check` on the seven owned Rust files and `git diff --check` both exited 0.
Compilation retained the preexisting vendor `rskafka` dead-code warnings; these are not claimed
as a clean workspace Clippy gate.

## Fixture corrections and limitations

- `d1-nested-stop-retry-fixture.txt` records an initial invalid test definition without the required
  collection successor. The corrected definition then produced the intended RED guard assertion.
- `d1-scoped-output-limit-fixture.txt` records publication rejection of an unsupported loop body
  error branch and a nested collection body. The loop fixture was corrected to the public supported
  sequence, then produced the intended RED result.
- The nested-output experiment in `d1-scoped-output-limit-red.txt` did not reach a worker report:
  trusted structured parsing bypassed public subset validation, but the existing scheduler rejected
  the nested fixture's projection. With the root agent's agreement, that newly added invalid
  integration fixture was removed instead of expanding the public runtime subset. Its failed log
  is retained. Nested output runtime acceptance is not claimed; full/path nested Retry is tested.
- Final broad workspace, SDK, browser and CI gates are the root agent's responsibility. This
  targeted correction does not close US04 or certify a provider.

## Additional E2E convergence checkpoint — T037-014/029/031

The final acceptance review found three historical feature-003 E2E expectations that still
required automatic repetition after a negative lookup or an expired compensation lease. The
parent approved narrowly updating `tests/e2e/sagas-compensations/telemetry.rs`, `us02.rs` and
`us03.rs`, and adding a current-authority control in that suite's historical `us04.rs`. These
files are an explicit validation amendment; no production source or golden was changed.
`us04.rs` names feature 003's operator story, not feature 037's excluded qualification story.

Observed REDs used the already compiled binary containing the new policy; no Cargo invocation
ran alongside the parent's workspace compilation:

```sh
export SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=2
export DOCKER_HOST=unix:///Users/marcelo/.docker/run/docker.sock
SAGAT_REVIEW_E2E=/var/folders/60/bf6rmfzx6zq67nrbzb3m0p480000gn/T/sagat-037-target-q0fwz2gu/debug/deps/e2e_sagas_compensations-d54d29db37bf1729
"$SAGAT_REVIEW_E2E" telemetry::compensations_reconciliations_and_decisions_are_counted_without_naming_anyone --exact
"$SAGAT_REVIEW_E2E" us03::us03_when_the_provider_says_it_did_not_happen_the_payment_is_retried_with_the_same_key --exact
"$SAGAT_REVIEW_E2E" us02::us02_a_worker_that_dies_while_releasing_is_replaced_and_the_provider_sees_one_key --exact
```

Each command failed its obsolete `one_task` assertion (zero tasks, expected one):
`d1-telemetry-expectation-red.txt` (3.20 s, parent execution),
`d1-negative-e2e-expectation-red.txt` (2.86 s), and
`d1-expired-compensation-e2e-expectation-red.txt` (2.94 s).

The corrected tests assert review, an empty worker poll, a single original/compensation lease,
and an explicit outcome-bearing human decision. The two lifecycle cases also refuse direct
Retry and retain the journal; the expired compensation retains stale-report rejection. The
telemetry case now counts `resolve_failed` before its existing compensation/review/skip counters.
Current corrected names:

- `telemetry::compensations_reconciliations_and_decisions_are_counted_without_naming_anyone`
- `us03::us03_a_negative_lookup_preserves_review_until_a_human_resolves_without_a_second_attempt`
- `us02::us02_a_worker_that_dies_while_releasing_requires_review_without_a_second_attempt`

The new control is
`us04::a_revoked_review_authority_is_refused_with_the_same_token_and_preserves_the_obligation`.
It authenticates one administrator token, demotes its real membership to Viewer (403), then
deletes that membership (401), and reuses the same token on the review endpoint after each
change. It checks unchanged review/journal/version/outbox/success audit/idempotency receipt,
an empty worker poll and the retained permission-denial audit. This characterizes an existing
guard; it is not a RED-to-production-fix claim.

`rustfmt --edition 2024` on these four files and `git diff --check` exited 0. GREEN execution
requires the parent's newly compiled binary and remains pending at this checkpoint.

## Durable gRPC convergence — T037-026/031

The parent delegated two further historical contract expectations after the full workspace
observed their REDs in `d1-final-workspace.txt` (lines 2948–2968 at this checkpoint):

- `blobs::rejected_blob_completion_replays_after_unused_blob_collection` expected `FAILED`,
  but the successful report whose descriptor exceeded the run budget correctly held
  `MANUAL_REVIEW`.
- `protocol::a_heartbeat_of_a_lost_lease_stays_a_stale_attempt_after_another_worker_reports`
  expected a second worker task after an expired original lease; the poll correctly returned none.

The authorized amendment touches only `tests/contract/durable-execution/grpc/blobs.rs` and
`protocol.rs`. The blob case now proves `WAITING`/`MANUAL_REVIEW` with `OUTCOME_UNKNOWN`, then
collects the unused blob and proves that the same report receipt replays with the original command
ID, unchanged journal and no new work. The heartbeat case now resolves the review through the
public human operation and proves `STALE_ATTEMPT` both before and after that decision, with one
original lease and one `step.resolved` event. It introduces no automatic repeat exception.

Disk before build: 83 GiB available. The same private target, two build jobs, two test threads,
runtime-role setting and repository Docker endpoint documented above were used. The parent
confirmed workspace compilation and Clippy had released the target. Exactly one Cargo build:

```sh
cargo test --locked --offline -p sagat-tests --test contract_grpc_durable_execution rejected_blob_completion_replays_after_unused_blob_collection
"$CARGO_TARGET_DIR/debug/deps/contract_grpc_durable_execution-4b3b8f3706410a95" protocol::a_heartbeat_of_a_lost_lease_stays_stale_after_human_resolution_without_another_lease --exact
```

Observed GREENs: `d1-grpc-blob-review-green.txt` — 1 passed, 0 failed/ignored, 4.93 s;
`d1-grpc-heartbeat-review-green.txt` — 1 passed, 0 failed/ignored, 2.38 s.
`rustfmt --edition 2024` on both files and `git diff --check` exited 0. No production source,
golden, receipt implementation or lease-fencing implementation changed. The parent owns the
final composed-tree rerun.

## Durable restart E2E convergence — T037-027/031

The composed workspace recorded three additional obsolete takeover expectations in
`d1-final-workspace.txt` (lines 3340–3389 at this checkpoint). The parent authorized updating
only `tests/e2e/durable-execution/stories.rs`, `stream_recovery.rs` and `engine.rs`:

- `stories::us02_a_dead_workers_step_is_taken_over_and_the_stale_generation_is_dead` expected
  one released retry after expiry; the observed count was zero.
- `stream_recovery::an_abandoned_streamed_lease_expires_after_restart_and_fences_its_late_result`
  expected a new streamed lease; the observed task frame was empty.
- `engine::the_engine_takes_a_dead_workers_step_back_by_itself` waited for a replacement original
  task; the engine correctly retained review and no task appeared within the old 20-second wait.

The corrected names and proofs are:

- `stories::us02_a_dead_workers_step_requires_review_and_the_stale_generation_is_dead`:
  expiry produces idle review, another worker receives nothing, old completion/heartbeat are
  stale and leave the journal unchanged. A human resolves A; the other worker executes only B.
  The journal contains one A lease, zero A worker completions and one A human resolution.
- `stream_recovery::an_abandoned_streamed_lease_reviews_after_restart_and_fences_its_late_result`:
  stop the engine with an abandoned streamed lease, explicitly age that lease while stopped,
  then restart. Only the restarted engine notices expiry. A replacement stream has no task;
  the old result is stale. Human resolution finishes the run with one lease, zero worker
  completions and one human resolution.
- `engine::the_engine_preserves_a_dead_workers_step_for_review_by_itself`: explicitly age an
  abandoned lease and let the real engine loop create review without calling a scheduler tick.
  No replacement lease is available; the stale result is refused; an authorized human resolution
  finishes with the same one-lease/zero-completion/one-resolution history.

The two process tests now use a 120-second lease and explicit expiry setup, avoiding an accidental
one-second lease expiration during unrelated setup. Observation is bounded through the existing
`eventually` helper; no timing-based race setup or new sleep was added. The unchanged streamed
lease-survival test remains in the same complete suite.

Disk before build: 82 GiB available; same private target, two jobs/test threads and runtime-role
environment. Command:

```sh
cargo test --locked --offline -p sagat-tests --test e2e_durable_execution
```

`d1-durable-recovery-e2e-green.txt`: **16 passed, 0 failed, 0 ignored**, 31.49 seconds after one
25.50-second compilation. Formatting and `git diff --check` exited 0. No production or golden
changed; the original restart and stale-attempt protections are still asserted. The parent owns
the final composed-tree gate and task amendments.

## Full workflow-primitives gate — environment correction checkpoint

The first 772-test invocation inherited `RUST_LOG=warn` from the process environment. This activates
the global logging subscriber in `tests/support/engine.rs:134–137` before OTLP test setup.
`Collector::install` independently requires a process registry at `tests/support/collector.rs:266–267`,
so `capacity_finish::acquire::acquire_exports_closed_outcomes_and_counts_only_acknowledged_first_transitions`
failed with `SetGlobalDefaultError("a global default trace dispatcher has already been set")`.

The diagnostic rerun used the exact already-built binary, not a rebuilt or modified test:
`d1-primitives-acquire-diagnostic.txt` reproduced the failure in 4.17 seconds. With only
`env -u RUST_LOG` changed, `d1-primitives-acquire-no-log.txt` passed in 4.51 seconds. No collector,
production source, assertion or test expectation was changed.

The parent approved stopping only this contaminated suite and retaining its output as
`d1-workflow-primitives-rust-log.txt`. Its Cargo/test PIDs were 54203/54370 (session 45397).
SIGTERM initiated Testcontainers fixture cleanup while the harness briefly continued: the library's
`testcontainers-0.27.3/src/watchdog.rs` handles TERM by stopping/removing registered containers before
the default signal handler. The log then records a cluster of chain tests failing and a database EOF.
SIGKILL ended only test PID 54370; Cargo returned 101. This interrupted 253-result run is not a gate
result. The interrupted tail is not attributed to product regressions without further evidence.

As a diagnostic control, `chain_wait::a_chain_that_waits_in_every_segment_reaches_its_final_outcome`
passed alone both without `RUST_LOG` (`d1-primitives-chain-wait-diagnostic.txt`, 3.18 seconds) and with
the inherited `warn` (`d1-primitives-chain-wait-with-log.txt`, 3.21 seconds). These controls do not
replace the complete run or erase the interrupted log.

The complete gate was restarted without the optional logging override, preserving the workspace
feature selection, runtime role, target and two-thread resource profile:

```sh
env -u RUST_LOG CARGO_TARGET_DIR=/var/folders/60/bf6rmfzx6zq67nrbzb3m0p480000gn/T/sagat-037-target-q0fwz2gu CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUST_TEST_THREADS=2 SAGAT_TEST_AS_RUNTIME_ROLE=1 DOCKER_HOST=unix:///Users/marcelo/.docker/run/docker.sock cargo test --workspace --locked --offline --test integration_workflow_primitives
```

Output: `d1-workflow-primitives-full.txt`; session 57002, initial Cargo/test PIDs 20072/31097.
This clean run completed with **749 passed, 1 failed, 22 ignored**, in 1329.80 seconds. It remains
a recorded failed full run. Its sole failure and subsequent correction are detailed below; this
is not a claim that 772 tests passed or that the full suite was rerun after the fixture correction.

## Scoped blob quota fixture — T037-009/027/031 validation amendment

The sole clean-run failure was
`scoped_blob_completion::a_scoped_blob_quota_recovery_rechecks_lease_expiry`: the public response
was `INTERNAL`, expected `STALE_ATTEMPT`. An isolated diagnostic on the same binary reproduced it
in 14.39 seconds (`d1-primitives-scoped-blob-expiry-diagnostic.txt`), without `RUST_LOG`.

`leases()` in `tests/integration/workflow-primitives/scoped_blob_completion.rs` deliberately revokes
the runtime role's checkpoint-read permission to detect unrelated-history reads on normal blob
reports. The approved exceptional rejected-content fallback restores the full run to stop routing
and preserve the source outcome (`research.md`, delivery-1 quota correction). That artificial
revocation prevented restoration before the fallback could recheck the expired lease.

The parent authorized moving the existing test-only `GRANT SELECT ON iteration_checkpoints` from
the final replay oracle to just before this exceptional report. This restores the fixture's actual
runtime permissions; it does not widen production privileges. No other history-read trap changed.
The real quota lock, clocked expiry, `STALE_ATTEMPT`, blob `UPLOADED`, step `LEASED` and replay
consistency assertions are unchanged. No production source or golden changed for this correction.

Same private target, runtime role and resource settings; one compilation, then direct module run:

```sh
env -u RUST_LOG cargo test --workspace --locked --offline --test integration_workflow_primitives a_scoped_blob_quota_recovery_rechecks_lease_expiry
env -u RUST_LOG "$CARGO_TARGET_DIR/debug/deps/integration_workflow_primitives-93819aef1967810c" scoped_blob_completion::
```

Observed results:

- `d1-scoped-blob-expiry-fixture-green.txt`: **1 passed, 0 failed/ignored**, 12.85 seconds after
  a 1 minute 9 second rebuild (including the parent's comment-only application edit).
- `d1-scoped-blob-module-green.txt`: **8 passed, 0 failed/ignored**, 16.44 seconds. This includes
  bounded normal report/replay with history access revoked, cross-instance authority, rollback,
  cancellation, descriptor provenance and the corrected quota-expiry case.
- Formatting and `git diff --check` exited 0. The parent owns independent verification of this
  corrected module; the unchanged 749 cases are retained as the observed broad-run evidence.

The 22 explicit ignored entries are: one opt-in 10,000-iteration end-to-end performance scenario;
19 immutable-old-producer or release-upgrade scenarios requiring their dedicated scripts/artifacts;
and two private subprocess entrypoints. The two private probes are invoked with `--ignored` by
`killed_native_reconciler_restarts_same_identity_after_actual_commit_or_abort` and
`killed_scoped_reconciler_restarts_complete_old_command_after_real_commit_or_abort`, both of which
passed in the broad run. These entries do not establish old/live-version upgrade support or a
performance SLO; delivery 1 remains a fresh coordinated installation, and US04 remains excluded.

## Security integration expectations — T037-027/031 validation amendment

The broad workspace run observed two further obsolete automatic-repeat expectations in
`integration_security_multitenancy`: **124 passed, 2 failed, 0 ignored**, 115.75 seconds.
`d1-final-workspace.txt:8230–8253` records the original failures before these edits:

- `bindings::removing_a_binding_closes_the_session_releases_the_lease_and_stops_the_worker`
  expected a replacement task after unbinding expired the original lease, but received no task.
- `incidents::a_delayed_terminal_report_finds_incident_evidence_after_the_lease_advances`
  expected one retry released after expiry, but received zero.

The parent authorized adjusting only these two test files to policy 037. The renamed bindings
case, `removing_a_binding_closes_the_session_and_preserves_the_revoked_effect_for_review`, retains
the original immediate revocation checks: one session/lease revoked, old session token rejected,
and old heartbeat/completion refused as `NotBound`. It now proves `MANUAL_REVIEW`/`WAITING`, no
new lease, explicit human successful resolution, exactly one original lease and one resolution,
and replay consistency. A separate new run proves that a bound worker still completes authorized
new work, without repeating the unresolved old effect.

The renamed incident case,
`a_delayed_terminal_report_keeps_its_incident_after_review_or_a_known_retry`, covers two distinct
causes for the old lease ending. Expiry preserves manual review with no new task. A reported known
retryable failure preserves the original positive control: a new lease has the same step identity
and higher attempt/generation. In both cases the delayed old-attempt diagnostic retains only
`FORBIDDEN_ADDRESS`, never the raw destination canary, and does not mutate the run journal.
The known retry can finish; expiry remains in review. Both projections replay consistently.
No production source, permission grant or golden changed.

Full target command (same private target and runtime-role resource profile):

```sh
env -u RUST_LOG CARGO_TARGET_DIR=/var/folders/60/bf6rmfzx6zq67nrbzb3m0p480000gn/T/sagat-037-target-q0fwz2gu CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUST_TEST_THREADS=2 SAGAT_TEST_AS_RUNTIME_ROLE=1 DOCKER_HOST=unix:///Users/marcelo/.docker/run/docker.sock cargo test --workspace --locked --offline --test integration_security_multitenancy
```

Output: `d1-security-multitenancy-green.txt`, session 14461, Cargo/test PIDs 79148/95554.
Observed **126 passed, 0 failed, 0 ignored**, 104.57 seconds, after one 26.62-second compilation;
command exit 0. Both renamed regressions passed in this complete target. Formatting and
`git diff --check` exited 0. Target released; the parent owns independent merged-tree verification.

The parent's independent `scalability-ha/fairness.rs` fixture correction was also reviewed against
`apps/control-plane/src/lib.rs::set_scheduling_weight` and
`20261017000000_scheduler_shards.up.sql`: the privileged operator command creates its own store,
while `sagat_app` receives only `SELECT` on `tenant_scheduling`. The fixture uses the owner store
only for that operator command and asserts runtime privileges `(true, false, false, false)`;
existing audit and invalid-input assertions remain. No blocking issue found in this narrow review;
the parent's reported 44-test and workspace doc results are not represented as child executions.
