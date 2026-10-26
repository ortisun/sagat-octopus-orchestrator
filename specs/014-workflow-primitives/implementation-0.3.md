# Implementation ledger for release 0.3.0

Owner instruction, 2026-10-26: “Continue com a implementacao do restante agora para a release v0.3”.
Baseline: `develop@70f0943540859a4b581b8a848e7ed3fa53edbf8a`; branch
`feature/014-v0.3-completion`. Preserve sibling worktrees and their uncommitted changes.

The release target is 0.3.0. The eight remaining canonical tasks are T014-014–017,
T014-040, T014-042–044. Their original acceptance criteria remain in force; this
ledger decomposes execution and does not close them. Feature 016 and 017 are
released dependencies, not new product scope. GitHub Releases remains the delivery channel.

## Ordered increments

1. [ ] Finish the existing collection output/result compatibility frontier: genuine old
   Applied report, exact replay, stopped drain and original-forward compensation; current
   authority, terminality and uncertain-commit controls. Review existing unverified work
   before reuse. Verify through public gRPC and real PostgreSQL in owner/runtime profiles.
2. [ ] Complete bounded scoped planning and persistence for broader acyclic bodies. Start
   with one control shape at a time, with failing domain and public runtime tests. Preserve
   local identity, evaluation snapshot/CAS, reserved derived instances, cancellation and
   compensation. Keep publication closed for every shape whose runtime proof is absent.
3. [ ] Complete the accepted capacity-group protocol in bounded transaction families:
   own-member authority, current forward versus reserved drain eligibility, durable
   discovery/parking/transfer, creation/continuation/cancellation and retention. Preserve
   chain-before-own-run entry; never mutate a second existing run. Test the actual lock graph.
4. [ ] Integrate quota projection, mixed-writer rollout and one nested public shape. Prove
   capacity-one progress, policy reduction including zero, descendant drain, detached-child
   finality, waking an ancestor while a child waits, stale transfer epochs and restart.
5. [ ] Extend the common five-SDK fixtures for the accepted shapes; inspect golden changes
   and rerun without blessing. Run old-client/old-worker/active-run upgrade evidence.
6. [ ] Run the complete fault/security matrix and measure the specified 10,000-item
   concurrency 1/16/256, 10,000 suspended approvals and nested 100,000-instance workloads.
   Record environment, failures, queue delay, transactions, memory and journal growth.
7. [ ] Close only evidenced canonical tasks; update contracts, telemetry, runbooks,
   traceability and demos. Run all required integrated gates, then prepare 0.3.0 release
   metadata under Gitflow. A release target does not waive a failed gate.

## Verification discipline

Behavior changes start with a test observed failing for the relevant reason. Races use
held locks/barriers. Tests start their own repository-declared disposable services. Use a
private target directory outside the repository and at most two heavy builds. Each accepted
increment records its command, result and limitations here or in the validation report.
Required source/transaction decisions receive bounded adversarial review; the owner's prior
choice of the current review and smaller decisions remains applicable.

## Initial inventory

- Fresh worktree is based on the merged 0.2.0 backmerge, not the stale local develop checkout.
- `feature/014-bounded-execution` contains uncommitted changes; preserve them.
- `feature/016-project-completion` contains an unverified Applied compatibility test diff;
  review and verify in this worktree before accepting any of it.
- Existing scalar/path stores and dormant capacity/source-certification stores are available.
  Dormant storage is not evidence of enabled nested execution.

Refs: T014-014, T014-015, T014-016, T014-017, T014-040, T014-042, T014-043, T014-044.

## Executed baseline and historical fixture limitation

`CARGO_TARGET_DIR=<private target> CARGO_BUILD_JOBS=2 cargo test -p sagat-domain --locked`
passed **544 tests, zero failures, zero ignored**, across 39 targets on the integrated
baseline. The current integration target compiled successfully with the candidate Applied
test; compilation was not counted as runtime acceptance.

The candidate `applied-report-before-drain` coordinator was actually executed using the
repository Compose PostgreSQL service in private project `sagat-v03-20261026`. Its old
producer passed 1/1; current attachment failed before reporting with
`VersionMissing(20261206000000)`. The immutable unpublished `6c44a0e4` profile has
OpenBao at `20261122000000` and the open-step indexes at `20261206000000`; the released
integrated profile preserves develop's index migration at `20261122000000` and OpenBao
at `20261122000100`. ADR-0036 explicitly excludes rewriting this incompatible history.

The candidate test was not accepted, and no migration ledger, applied migration or runtime
authority was changed to make it pass. The foreign worktree remains untouched. Historical
unpublished-profile tests remain historical evidence, not 0.2.0→0.3.0 upgrade acceptance.
Continue the runtime increments against the integrated profile; supported release upgrade
tests must use the actual released profile. An explicit bridge for unpublished conflicting
histories is not part of this release request.


## First runtime increment: local collection activity recovery

Implementation uses existing addressed worker routing. Publication accepts an acyclic
activity-only body with1..8 activities and one terminal Return. The bounded frontier and
full-run scheduler ignore only a committed local handled failure; the native checkpoint
also proves local closure and native drain revalidates an actual unhandled cause. Unselected
branch reservations retire without refunding lifetime charge. Drain retention uses the
planner's same completed/unknown-effect predicate on actual post-drain local states.
An empty saga can plan and finish atomically; persisted plan/end references identify their
actual journal events instead of the batch head.

Observed native results so far (each separate invocation, not added together):

- Initial public publication tests:2failed/0passed, correct unsupported-body RED.
- First runtime correction:1passed/1failed at checkpoint; diagnostic exposed its closed-step guard.
- Checkpoint correction:2passed/0failed.
- Baseline native drain guard regression:1failed, actual wrongful applied drain of a handled error.
- Corrected four-case suite:4passed/0failed/0ignored,9.15s; includes full/frontier oracle and lifetime12.
- Unselected compensable branch: genuine reservation RED `(3,1,2)` versus `(3,0,3)`;
  its full lifecycle also exposed the pre-existing empty-saga projection failure.
- Corrected six-case suite:6passed/0failed/0ignored,9.11s. Subsequent assertions add the
  public qualified graph edge and exact native plan/end journal references; final regression pending.

Common SDK case: initial observation executed all five languages and produced identical
complete normalized outputs; the aggregate failed only for the two absent new reference
files. After inspecting canonical workflows, all ten worker calls, ordered results, actual
compensation target, failure cause and peak concurrency, those files were recorded without
blessing. The all-five no-bless verification passed1aggregate/0failed/0ignored in36.39s.
That result precedes the last drain-retention/empty-saga correction; rerun on final sources.

Bounded adversarial reviews found and corrected the checkpoint closure, unused branch
reservations, native failed-drain revalidation, stale negative publication fixtures and
missing full/frontier evidence. Final focused drain-retention review and the separate empty-
saga transaction review found no required issue. Root executes the actual verification gates;
reviewer reports alone are not test evidence. The eight canonical tasks stay unchecked.

### Current-tree gates for the first increment

- `cargo test -p sagat-domain -p sagat-infra-ir --locked`:721 passed,0 failed,0 ignored
  across57 targets (including empty doc-test targets),89.70s including compilation.
- `cargo test -p sagat-tests --test contract_workflow_primitives --locked`:47 passed,
  0 failed,0 ignored,0.59s test execution.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`:exit0,171.25s.
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=1 cargo test -p sagat-tests
  --test integration_workflow_primitives --locked -- collection_recovery::
  refusal_secret_versions::a_scanned_output_refusal_rechecks_the_original_lease_after_path_drop`:
  7 passed,0 failed,0 ignored,553 filtered,34.53s test execution. All six current
  recovery cases include the final graph and exact plan/end-reference assertions.
- Architecture:24 crates, respected layers, no cycles. Agent configuration and formatting pass.
- All five SDK unit/lint command groups from `sdks/AGENTS.md` passed. Python uses the
  existing Python3.12.12 toolchain with the hashed dev lock:264 tests, mypy38 files,
  ruff clean. TypeScript uses Node26.12.0, Go selects its pinned1.25.14 toolchain,
  .NET10.0.401 passes450 tests on net10.0 and builds both SDK target frameworks,
  and Maven uses Java23 with the project's
  release target. These are local results, not evidence of CI's Node22/Java21 jobs.

The full two-thread PostgreSQL invocation completed with541 passed,3 failed,16 ignored
in610.26s. Two migrations failed with PostgreSQL SQLSTATE53100 (`No space left on
device`) in the shared30GiB Docker VM. A separate refusal race incorrectly compared a
database-clock binding timestamp with the host-clock request interval; the host and VM
were observed about325ms apart. `ce4bace4` bounds the binding by PostgreSQL samples while
keeping the API-clock audit's own request interval. The corrected race passes in the
seven-case least-privilege invocation above. The isolated `populated_` migration group
passes4/4 with one outer test thread and fresh harness servers.

A complete serial outer-runner regression and the92-test fault suite are in progress at
this checkpoint. One outer test thread preserves every concurrency/barrier inside each
test and avoids accumulation of hundreds of migrated test databases in a continuously
held shared server. The failed two-thread invocation is retained as failed evidence;
no ignored test was changed and no Docker data from another session was removed.
Final all-five recovery conformance is also in progress. No merge/release gate is inferred.

Subsequent results on the first increment's production sources (`5c20d0de`):
`RUST_TEST_THREADS=1 cargo test -p sagat-tests --test fault_workflow_primitives --locked`
passed92/92,0 ignored,381.83s test execution. The final
`SAGAT_SDK_SCENARIO=collection-local-recovery ... cargo test -p sagat-tests
--test sdk_conformance --locked -- --ignored --nocapture` passed1 aggregate covering
all five languages,0 failures,41.59s test execution, with no bless/reuse/language filter.
The two ordinary non-ignored registry tests are filtered by `--ignored`; this is still
one focused scenario, not a full61-scenario aggregate. Broad serial integration remains
in progress against that increment's binary.

## Next increment: scoped condition/switch evaluation

The first bounded artifact is domain recording/routing only. Tests initially observed
`UnsupportedScopedEvent("condition.evaluated")` and rejection of a closed switch body
as a prematurely ending success path. The shared scoped step reducer now records a
Pending control's outcome, the full and exact-path planners share existing control
successor selection, and switch success branches satisfy the body's Return rule.
Root selection behavior is preserved. Duplicate choices fail without mutation, unrelated
iterations remain untouched, and both selected/default paths match the full-state oracle.

`cargo test -p sagat-domain --test scoped_control_routing --test scoped_reducer --locked`
passes9/9 (2 new choice tests and7 reducer cases). The subsequent full domain invocation
passes546 tests,0 failures,0 ignored,68.41s including compilation. A fresh read-only
adversarial review of this artifact found no required issue; root ran the tests.

This does not activate control nodes: the public parser and native projection guards remain
closed. The separately added two public runtime tests were actually run and fail at the
expected unsupported-body publication boundary (0 passed,2 failed,4.02s). Their completion
requires a scoped evaluator, exact snapshot/revision/fence validation, transactional
condition/switch projection and branch reservation accounting. An evaluator compatibility
default must delegate root calls and refuse unsupported scoped evaluation. No canonical
task closes through this domain foundation.
