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

### Completed first-increment broad regression

The complete serial `integration_workflow_primitives` invocation against the first
increment's binary (`5c20d0de` production sources) passed544 tests,0 failures,16 explicitly
ignored,1601.09s test execution. This includes the corrected clock witness. It supersedes
the pending broad-run status above without erasing the earlier two-thread failures.

### Local-control runtime and integrity increments

Scoped boolean evaluation delegates roots unchanged and uses the exact instance's local
environment. Its actual RED was1 pass/1 failure at unsupported scoped evaluation; all11
evaluator tests then passed. The domain control command validates pending state, exact
node/kind/case and every global/ancestor forward fence; its six-case target passes, including
ten malformed pending-metadata variants rejected without a partial state change.

The two public decision tests first failed at the correct unsupported-body boundary, then
passed2/2 in6.31s. The subsequent native suite passed8/8 in29.28s, covering local conditions,
ordered/default switch, expression failure and actual prefix compensation, local recovery,
outbox rollback in bounded/full writes, malformed projection reads and a held-lock pause/CAS
race. An intermediate race fixture expected HTTP200 for resume; the documented operation
returns202. Correcting that assertion completed the lifecycle; it was not a runtime fix.

Adversarial review found two typed-state gaps: three readers accepted malformed scoped controls,
and a routed Pending control could record a choice without its route. The native RED observed
all three readers succeed when all should reject. The shared domain invariant and full/path/
frontier read validation corrected them. A further physical-row counterexample reproduced
orphan `due_at` acceptance in all readers and a prepared native writer returning `Ok(())` over
an orphan reported-lease hash. One shared SQL predicate now checks physical worker/wait/child/
approval/output absence in the existing read projections and both guarded UPDATEs. No extra
per-member/history query, schema, role grant or wire field was added. Final bounded physical-
guard review found no required issue; root reran the actual ten-case native suite:
10 passed,0 failed,0 ignored,56.83s (117.15s including compilation).

The scoped transaction span reuses `scheduler.action` with closed kind `collection_control`
and run/tenant/instance/expected-revision correlation. The real trace canary was observed
failing at the missing-span assertion with the annotations removed, then passed1/1 in2.63s
after restoring them; item and expression canaries are absent from the captured output.

`cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-scheduler --locked`
passed1009 tests,0 failures,0 ignored across77 targets,66.57s including compilation.
Strict workspace/all-target Clippy after the physical correction passed (exit0,83.92s);
its final trace-instrumented follow-up and affected contract/regression gates are in progress.

The common `collection-control-routing` observation ran all five languages; all normalized
outputs were identical (SHA256 `91e94795046ca7e2d647ecdc083f8c1b5f40b704aca0fa93712a0844a1448ceb`).
The only aggregate failures were the two absent new references. Root inspected both complete
workflows/canonical forms, both journals, qualified graphs, all six actual worker calls,
skipped index1 notifications, lifetime12/depth1 and ordered results before recording references.
The subsequent all-five invocation without bless/reuse/archive/language filtering passed
1 aggregate,0 failures,43.02s test execution (70.35s including compilation). The two ordinary
non-ignored registry checks were filtered by `--ignored`, not counted as passes. Registry size
is now62; the full62-scenario release aggregate remains pending.

Current public body capability is activity entry, at most eight executable activity/condition/
switch nodes and one Return. Control entry, waits, children, nested scopes and loop broadening
remain separate increments. All eight canonical tasks remain unchecked; no release metadata,
merge or final release acceptance follows from these focused results.

Final local-control follow-ups: trace-instrumented workspace/all-target Clippy passed
(exit0,93.94s). The affected contract/ordinary registry command passed47 primitive contracts
and2 registry/harness tests; its one expensive all-SDK aggregate remained explicitly ignored
in that ordinary invocation and is represented only by the separate focused all-five result.
Formatting, architecture (24 crates/no cycles) and agent configuration pass. The final
physical-guard adversarial review found no required issue, ending its bounded three-cycle
review. A broader committed-tree regression remains separate from the focused evidence.


### Committed local-control broad regression

The b4dbb28d tree completed the serial native command
`cargo test -p sagat-tests --test integration_workflow_primitives --test fault_workflow_primitives --locked`:
**92 fault tests passed,0failed,0ignored in346.13s** and **555 integration tests
passed,0failed,16ignored in1694.80s**. Total command2073.75s including compilation.
This is the committed local-decision tree, before control-entry admission; it is not a
full regression of the next schema/profile. No ignored test was changed.

## Third runtime increment: scheduler-owned collection entry

CLAIM: Activity and scheduler-owned entry have distinct durable dispatch obligations.
A Condition/Switch entry charges one real logical instance and its complete reservations,
with no fake activity/outbox identity; revision, shard, tenant and quota remain checked in
the existing admission transaction. WHY THIS MATTERS: conflating scheduler work with a
worker task can stall cold recovery, leak a slot or permit an incorrect rollback.

`CollectionEntryDispatch` and general member admission retain the activity constructor/port
signatures. PostgreSQL validates the exact pinned body entry, records Activity as READY
with its actual release/outbox, or Scheduler as pristine PENDING with one admission event.
The latter's append-only marker is independent retained evidence; it is not a second source
of execution state. Full, exact-path and frontier readers check the dispatch projection.
The old activity-only port refuses Scheduler intent.

All local refs and worktree migration files were inventoried before allocating
20261221000000; the previous maximum was20261220000000. The additive migration backfills
ACTIVITY, closes nullable activity/outbox combinations by dispatch CHECK, and leaves
table-level runtime privileges intact. Down locks runs→memberships→journal and refuses
retained Scheduler membership OR immutable admission evidence, including completed runs.
No applied migration was rewritten. The new bounded column is classified protected_system.

Observed focused results, separate invocations:

- `control-entry-public-red`:2failed at unsupported publication, correct behavioral RED.
- `control-entry-first-runtime`:2passed,0failed,5.67s. Pure controls create zero worker
  leases/task.ready rows; switch entry leases one selected real activity with exact budget.
- First native3-case invocation was stopped after a refused SQLx migration left its session
  advisory lock held and a subsequent migration blocked. The queued expanded invocation was
  also stopped. No pass is claimed for either. Dedicated acquired migrator connections now
  close after refusal, with15s bounded waits. `control-entry-native-bounded`:5passed,0failed,
  12.80s, including rollback, replay, capacity/cold discovery, cancel and schema backfill.
- Review reproduced two CHECK-valid dispatch-masquerading REDs: the full reader accepted it
  and downgrade succeeded incorrectly (`control-entry-disguise-red`:2failed,4.37s).
  Pinned dispatch read validation and the immutable journal marker/down predicate correct them.
- `control-entry-evidence-green`:19passed,1failed,86.85s. The failure was an invalid test
  expectation that public quota0 is accepted; API returned422. Existing domain/SQL contracts
  require iteration_concurrency>=1. This increment preserves that policy; dormant nested
  zero-room authority gates remain separate. No public quota product semantics changed.
- `control-entry-focused-final`:21passed,0failed,0ignored,86.01s, including all three
  corruption readers, immutable-evidence downgrade, cancellation, exact journal/outbox rollback,
  cold cached-store/scheduler recovery through reduced capacity and frozen shape exclusion.
- `control-entry-native-regression`:23passed,0failed,0ignored,92.52s. Adds current
  tenant/shard/dispatch/item/reservation refusal snapshots and reruns the genuine activity-chain
  certification/replay after nullability. Both control kinds fail pure-domain frozen shape
  checking and native certification under owner/runtime access without mutations.
- `sdk-control-entry-observation`:all five actual runtime observations identical SHA256
  88b0089b4cb3d73cf0e9853dcc7bc013c231b7630b3ac97ca5c7ab83610624f1; aggregate failed
  only for two missing reference files. After reading complete observations and canonical
  documents, references were recorded without a bless switch. `sdk-control-entry-verified`:
  1aggregate passed,0failed,30.52s. The pure condition Run finishes before startWorker; both
  Runs yield ordered[1,-1], and the only worker call is switch index0/control.notify.
- `clippy-control-entry`:strict workspace/all-targets locked Clippy passed,86s build output.
  fmt,architecture(24crates) and agent configuration also passed. Core/contracts/classification
  gates are running separately and are not yet claimed here.

Adversarial design review produced the actionable masquerading and frozen-profile evidence
requirements, plus same-delivery spec/contract updates. The zero-quota request was a contract
misread, as verified against quota.rs and the actual422. Mixed older-scheduler execution of
new definitions is unsupported: old authority must be fenced/drained before admission, and
released0.2-profile active-run/rollout evidence is required before release or advertising0.3.
A fresh bounded review of the corrected admission/projection/down artifact found no required
issues. Cross-model review remains skipped per the owner's earlier choice.

The registry now contains63scenarios; a focused pass is not full63 acceptance. Timer/wait/child
body progression, nested capacity and workloads remain pending. All eight canonical tasks stay
unchecked and no version/tag/release has been changed.


### Third-increment final focused gates

- Four core crates (`sagat-domain`, `sagat-application`, `sagat-infra-ir`,
  `sagat-scheduler`) passed1009tests,0failed,0ignored across77targets;60.36s command.
- The initial contract command passed18payload-classification tests but failed one stale
  primitive negative fixture (46passed/1failed): it still declared control entry unproven.
  The negative fixture now preserves the independent Return-only admission refusal, and a
  new positive contract checks both reviewed public control-entry canonical documents.
  Corrected command passed18classification +48primitive +2ordinary SDK tests,0failures,
  with the expensive SDK aggregate explicitly ignored;19.41s command. The separate
  all-five new-case verification above supplies its actual focused runtime evidence.
- Runtime-role storage manifest fitness test passed1/1,1.74s;32.02s including compilation.
- Final strict workspace/all-targets Clippy after the contract adjustment passed,1.13s.
  No implementation change followed the23-case native verification or five-SDK pass.

Complete native regression of this new migration profile and the complete63-scenario
aggregate remain pending; the555/92 broad result above belongs to b4dbb28d.


## Fourth increment preparation: recorded scoped timer lifecycle

Public timer bodies remain closed. Three initial domain tests failed for the correct reason:
scoped planning returned UnsupportedScopedEvent(scoped non-activity planning) and recorded
TimerStarted was refused. Shared full/path step lifecycle now records Pending→Waiting(Timer)
and Waiting(Timer)→Succeeded, rejecting duplicate/cancelled fire without mutation. Pending
timers remain idle for a future clocked scheduler start; no domain wall clock was introduced.
The property test varies member identity and explicit deadline offsets, preserving sibling
state and full/path replay equality.

`scoped-timer-domain-green` passed19tests across the four focused targets. After adding the
property test, `scoped-timer-domain-final` (`cargo test -p sagat-domain --locked`) passed
554tests,0failed,0ignored across41targets in16.87s. Strict domain/all-targets Clippy passed
in2.83s build output.
This is recorded lifecycle preparation only: clocked commands, native due-time persistence,
qualified discovery, pause/resume/drain, public runtime and SDK evidence remain pending in
[scoped-timer-runtime.md](./scoped-timer-runtime.md). Canonical tasks stay unchecked.

## Broad control-entry regression: failures retained

`control-entry-63e3f52e-broad` passed92 fault tests (340.91s), then the integration
binary finished554passed/12failed/16ignored (2331.23s). This is a failed gate, not
acceptance. Two failures were stale fixtures: the publication negative still expected
Condition entry to fail, and an owner `jsonb_populate_record` fixture explicitly created
NULL entry_dispatch instead of the legacy ACTIVITY value. Both fixtures were corrected.
The other ten failures concerned scheduler discovery/authority in approval, byte ceilings,
compensation and capacity recovery. Diagnostic assertions now capture only bounded status,
revision, path and shard authority metadata, never inputs or credentials.

`control-entry-failures-diagnosed` passed the exact12-test selection,0failed,86.17s
harness/185.34s command. The ten failures did not reproduce; no production fix for them
is claimed. The original gate's wall interval was46,599.58s versus2,704.86 monotonic
seconds. Suspension/time discontinuity is an observation, not proof of every failure's
cause. Subsequent native gates use child-scoped `caffeinate -i` and record wall and
monotonic durations in stable_gate JSON. No lease TTL or authority rule was relaxed.
A complete stable broad gate on the final tree remains mandatory.

## Fourth increment: clocked scoped timers in progress

Explicit-clock full/path start and fire commands now validate pinned Timer identity,
pristine lifecycle and ancestor/global fences. Accepted due firing while paused records
completion; bounded collection discovery finds succeeded/unrouted members after resume.
Native full/path writers retain waiting_kind/due_at, and qualified due discovery uses
the existing timer index. Scheduler Timer entry emits no task.ready or worker lease.
Public activity→Timer and Timer-entry collection bodies are enabled in this working
increment, pending complete acceptance; event/child/nested execution remains closed.

Evidence retained under `/tmp/sagat-v03-20261026`:

- `scoped-timer-clock-first`:7domain tests passed; deadline, pause/resume, overflow,
  stopped ancestors, member isolation and full/path replay properties.
- The first public test used the wrong duration field and is not a behavioral RED.
  Corrected `scoped-timer-public-corrected-red` failed2tests on unsupported bodies;
  `scoped-timer-public-first-runtime` then passed2tests,14.69s harness.
- `scoped-timer-native-expanded`:5passed,20.22s; pause→cold resume→one route,
  prepared deadline changes and orphan metadata readers included.
- `scoped-timer-atomic`:6passed,56.94s harness/116.13s command, with start/fire
  journal-fault rollback and retry in both full/path models.
- `scoped-timer-races`:7passed/1failed,38.24s. Actual held-lock cancellation-first
  left a WAITING timer and CANCELLING Run. The pure drain reproduction failed with
  IllegalCollectionTransition; corrected pure drain gate passed8tests,0failed.
  Native drain verification is still pending at this checkpoint.
- Fresh bounded native-guard review found unsupported Failed timer acceptance and
  prepared writes silently repairing altered logical fields. `scoped-timer-lifecycle-guards-red`
  reproduced both Failed acceptance defects; its third test lost its database connection
  during migration and is not a valid behavioral result. Docker remained healthy inside
  Colima but its macOS socket forwarding was unavailable. A private SSH socket forwarding
  to the same daemon restored access without restarting or changing the shared VM.
- `scoped-timer-prepared-guards-red` then reproduced all10 prepared start/fire
  cases across both writers: changed choice, routing, settled status or failure fields
  were silently overwritten without a Run revision change (18.22s command).
  Correction restricts the legal timer state matrix and compares original status,
  routed and deadline as well as constant logical/physical invariants in UPDATE.
  Corrected native gate/review and common five-SDK evidence are not yet claimed.

All eight canonical tasks remain unchecked. This checkpoint does not authorize release.

### Timer guard correction and five-SDK focused verification

`scoped-timer-guards-and-drain-green` passed11native tests,0failed/ignored,
81.56s harness/162.70s command, wall-minus-monotonic0. Both cancellation/fire
commit orders now pass, as do all10 prepared-state cases and both unsupported-Failed
read/write regressions. The fixture runs the service through sagat_app. Fresh bounded
correction review found no required issues (static read-only review, no test claim).

`collection-timers` is scenario64. Actual TypeScript/Go/Python/.NET/Java normalized
observations matched SHA25673c3cba3365368fad858d5e0686b9f5d5c4d150f4d96edfe19f410adac30fbc2.
All three Runs returned ordered[1,-1]; Timer/Condition Runs finished before worker startup;
only Activity→Timer emitted two real worker calls. Complete normalized observations and
canonical workflow documents were read before recording both new reference files. Initial
observation failed only for missing references. `sdk-collection-timers-verified` then passed
1aggregate,0failed,49.38s harness/90.82s command, without a bless switch. Local Node26/JDK23
are different from pinned CI Node22/JDK21; no CI-toolchain or complete64-case claim follows.

The final native selection also adds no-item trace canary and frozen OrdinaryLeafV1 refusal
for Timer entry; core, contracts and lint acceptance are separate pending gates.

### Timer core and regression gates

- `scoped-timer-core`:1017passed/0failed/0ignored across78targets,90.55s command.
- `scoped-timer-contracts`:18classification +48primitive +2ordinary SDK tests passed,
 0failed; the expensive aggregate is explicitly ignored here and independently executed
 for the new scenario above.50.24s command. Registry phase evidence now covers64scenarios.
- First strict workspace Clippy failed filter_map_bool_then in timer read-set capture.
 The equivalent filter/map correction preserves the read-set. `scoped-timer-clippy-corrected`
 passed workspace/all-targets/locked/-D warnings,194.87s including build-lock wait.
 fmt, architecture(24crates) and agent configuration checks passed.
- `scoped-timer-final-native` passed12/failed1: the new frozen-profile Timer fixture
 incorrectly used duration0 (schema minimum1). Correction uses1ms; no schema relaxation.
 `scoped-timer-control-regression` then passed35/0failed/0ignored,345.94s harness,
 including all control-entry/decision regressions, timer transactions and no-item qualified
 start/fire trace canary. The fixture always uses sagat_app; negative source certification
 is separately checked under owner/runtime roles.
- Additional full/frontier timer oracle preparation first failed test INT4/i64 decoding,
 then found unnormalized numeric due_at_us (the event codec stores microseconds, not RFC3339).
 These are test fixture errors, not runtime REDs. The corrected comparison validates the
 closed deadline field and compares exact transitions and all remaining journal values,
 normalizing only generated UUIDs and each execution's explicit database deadline. Its
 final gate and the genuine existing activity certificate regression remain pending here.

No SDK source or runner file differs from the released v0.2.0 tree (`git diff v0.2.0 -- sdks`
empty; working SDK paths clean). This source identity observation is not an actual active-run
upgrade/mixed-authority demonstration. Only additive migration20261221000000 differs from
released0.2 so far; no old applied migration was rewritten. Final integrated/runtime/resource
and release gates remain open; all eight canonical tasks remain unchecked.

The final oracle/positive-source gate (`scoped-timer-native-oracle-final`) passed2tests,
0failed/ignored,10.27s harness: exact full/frontier transitions and qualified journals match
apart from validated database deadlines/generated UUIDs; the genuine activity source still
certifies and replays after later failure. Final strict workspace/all-targets Clippy passed
again after adding the oracle (52.50s including build-lock wait). No production code changed
after the35-test native regression or after the focused five-SDK verification. Full stable
native regression, complete64-scenario, released-profile rollout and workloads remain open.

## Timer broad-gate provenance correction

`timer-19c60a86-broad` passed92fault tests,0failed/ignored,566.81s. Its subsequent
integration binary was replaced by a concurrent build before Cargo launched it: that phase
reported597tests instead of the frozen commit's596. It is not evidence for19c60a86.
The owned mixed-source Cargo/integration processes were terminated (command exit-15,
1023.32s), preserving the log; no completed integration acceptance is claimed. This was an
execution mistake, not a test defect. The replacement gate runs from an owned detached
worktree fixed at19c60a86 (`sagat-octopus-v03-timer-gate-19c60a86`), keeping runtime source
and fixtures fixed. Cargo still uses the same executable filename in the shared private cache;
this replacement launches a single integration target, already executing its frozen binary
before further builds. Its actual header confirms596tests, and a private copy of the linked
executable is retained before additional compilation. `timer-19c60a86-frozen-integration`
is pending. Subsequent final release gates must likewise have fixed input provenance.

## Fifth increment: explicit zero tenant iteration policy (O701)

Integrated the existing capacity-protocol lane's owner decision records with --no-ff,
commit0ff0678e. Its final net diff changes documentation only; no withdrawn pause-based
model was activated. D704 keeps only fresh admission gated; D705 leaves descendant joins
and current possession authority as separately unimplemented obligations.

The bounded zero-policy contract is zero-iteration-policy.md. Real REDs:
`iteration-policy-zero-domain-red` refused0 with OutsideRange (13.43s command);
`iteration-policy-zero-http-red` returned422 instead of200 (5.00s harness/169.03s command).
Validator now accepts tenant0; namespace/ceiling/hard maximum rules stay intact. Additive
migration20261222000000 changes only the existing policy CHECK to0..1e9; down locks the
policy table, refuses retained0 and restores the positive CHECK. Existing native admission
comparison already handles0, so no new scheduler throttle/counter or permission is added.
The id was absent from all local refs/migration paths before allocation; the old migration
was not rewritten. New/old writer rollout order and the ACCESS EXCLUSIVE validation window
are documented explicitly.

`iteration-policy-zero-domain-green` passed12quota unit tests,0failed, plus filtered
zero-test targets;176.54s command. First native attempt passed2/failed1 because the new
fixture incorrectly expected RUNNING where the existing durable collection status is
WAITING. This was corrected without changing runtime status semantics. A subsequent
expanded test compile failed SQLx Acquire's Send lifetime bound inside tokio::spawn;
joining the actual migrator and held-lock release futures avoids that test-only bound.
`iteration-policy-zero-expanded-corrected` passed5native tests,0failed/ignored,44.89s
harness/123.05s command. They include the existing positive/auth/namespace/hard-maximum/audit
regression, admitted timer completion plus blocked next member at0 then resumption at1,
actual absent-row INSERT/source/clear distinction and typed AtCapacity versus PolicyMissing,
CHECK boundaries/down/up and both real writer/down relation-lock orders. The writer uses
sagat_app. Schema-first and fenced-writer rollback requirements are not inferred as actual
released-profile acceptance.

Fresh bounded review requested current runbook consistency, absent-row INSERT/source proof,
actual writer/down barriers and explicit rollback ordering. Those changes are implemented;
correction review and final contracts/lint are pending. Studio API generation and independent
check pass; the inspected generated diff changes only the public quota description. No SDK
protocol/builders, UI layout, nested runtime, version or release changed. Canonical tasks
remain unchecked.

Correction review found the primary runbook quota paragraph still had the positive range,
and correctly distinguished the new HTTP hard-maximum negative from a real operator-ceiling
negative. That paragraph is updated, and the focused HTTP regression now also submits1025
against the default1024 ceiling. Its updated invocation and final correction review remain
pending; the earlier five-test pass does not claim this additional HTTP ceiling coverage.

The updated HTTP ceiling gate passed1test/0failed (5.39s harness/75.79s command),
including1025 versus1024 and the independent1e9+1 hard maximum. Strict workspace/all-targets
Clippy passed138.25s, then41.48s after the added HTTP case. Contracts passed8developer
experience +18classification +48primitives,0failed/ignored,65.46s command. fmt, architecture
(24crates), agent configuration and Studio generation/check pass. Three bounded review cycles
completed: the last found only wording that overstated all reductions as blocking and an
awkward explicit-zero term; both are corrected to D704's exact retained<limit rule. No
required storage/locking/range/permission defect remained in that final review. This closes
only the explicit-zero increment; full fixed-source native/SDK/upgrade/resource and release
gates remain open.

## Sixth increment preparation: recorded scoped event waits

Public event-wait bodies remain closed, enforced by the new contract test
recorded_event_lifecycle_does_not_enable_unproven_collection_wait_delivery for both entry
and activity→wait shapes. The internal typed model can record the shape; this is not a
native subscription, accepted external delivery or timeout producer.

`scoped-event-recorded-red` failed5tests for actual unsupported scoped pending planning/
wait.started. After sharing recorded lifecycle, `scoped-event-timeout-routing-red` passed4
and failed1: a handled timeout incorrectly activated normal next instead of onTimeout.
The shared recorded_successors helper now selects the same recorded timeout/control branches
for root, full and exact-path planning. No root outcome or wire semantics are changed.

`scoped-event-recorded-green` passed7tests,0failed/ignored, including exact sibling identity,
correlation/deadline replay properties, event/timeout mutual exclusion, cancellation, pause
completion and resume routing. `scoped-event-domain-full` passed566tests/0failed/0ignored
across42targets,48.31s command. Strict domain/all-targets Clippy passed15.58s command including
build-lock wait (15.34s build output). A fresh read-only review found no implementation defect and requested
negative source-state coverage; a bounded full/path matrix now rejects duplicate start,
non-Event waiting outcomes, pending outcomes and duplicate cancellation without mutation.
`scoped-event-recorded-final` passed8tests/0failed,2.05s command; this is a separate focused
run, not a claim of567full-suite tests. Final focused-target strict Clippy passed0.93s.
`scoped-event-public-gate` passed1contract test/0failed,29.75s command including compilation.
No public parsing capability was widened; no golden or SDK protocol changed.

The next native boundary must replace the dispatcher's run+node-only wait/cursor/delivery
identity with exact member identity while preserving root receipts. Clock/evaluation fences,
physical metadata, event binding/receipt/payload quota+classification atomicity, lookback,
actual event/timeout/cancel races, paused cold recovery and five-SDK runtime acceptance remain
pending in scoped-event-waits.md. All canonical tasks and release gates remain open.

Strict contract-target Clippy also passed35.62s after the publication-gate addition.
The pure event increment introduces no native caller; recorded event replay cannot authorize
an external delivery. The immutable timer integration run remains a separate older-profile
invocation, not acceptance for this later pure event code or the new zero-policy migration.

The next dispatch-identity proposal is recorded in scoped-event-waits.md. Discovery remains
root-only until a qualified receiver exists, and rollout must fence old dispatchers as well as
old schedulers before admitting the new shape. The older matcher has run+node identity, and
external event receipt authority is distinct from scheduler clock authority. No native scoped
publication or receiver is implemented by this proposal.

Before changing dispatch types, event-wait-root-consumer-baseline passed2application tests
that pin the exact persisted root wait and trigger consumer strings plus their metadata.
These are compatibility regression controls for existing active-run receipts; no qualified
consumer implementation is claimed. The subsequent root-only preparation guard below closes that query gap without enabling
a scoped receiver.

Dispatch identity design review requires explicit tenant in scan CAS, both root-only discovery
queries plus marker guard, and a concrete old-dispatcher activation barrier. The first two
are now named in the bounded proposal and require native regression tests. The last remains
an explicit unimplemented SC-014-005 activation blocker: no dispatcher generation exists,
and scheduler fencing/TTL alone cannot prove its termination. Native scoped row creation
and publication stay closed; the identity preparation does not claim mixed-version safety.

## Frozen timer integration completed

`timer-19c60a86-frozen-integration` completed580passed/0failed/16ignored at the
immutable19c60a86 source. Harness2584.38s; command2693.95s monotonic/2693.97s wall,
recorded wall-minus-monotonic0.01s. Together with the separate92/0/0 fault invocation,
this supplies broad timer-profile evidence. The16ignored historical/environment-bound
tests remain explicitly ignored, not passed. This is migration profile through20261221,
not the later zero-policy migration22 or the event replay preparation. The initial mixed-
source integration interruption remains retained and is not reclassified as a pass.
Private linked executable SHA256:
cabdd9ff2acdc27c86b8500a5e938c52eb605136123b589e20f3a0851f76cc7d.

The frontend pipeline on the current branch also exited0: npm ci, npm audit(moderate,
0vulnerabilities), lint, Studio/e2e TypeScript checks, Studio unit tests, production build and
check:api. Node26 reports expected experimental localStorage warnings; this is not a browser
journey or pinned CI Node22 claim. No frontend source behavior changed beyond the generated
quota documentation. Its dependency/build directories remain owned temporary cache until
cleanup; package/lock versions were not changed.


## Root event-dispatch preparation guard

The real PostgreSQL app-role fixture first reproduced two defects: a scoped sentinel consumed
one root page and a root scan marked two same-name scoped siblings (2 genuine REDs). The
three native queries now restrict both path arrays to root, and marker CAS includes namespace
and the exact selected event type/key/since. Root/trigger consumer keys remain unchanged.
The first corrected run then failed only its attempt to DELETE append-only sentinels; tests
now retain them until the owned database teardown. No production trigger/grant was weakened.

The reviewed two-case gate passed2/0/0 in3.90s harness/75.81s command, with a positive root
marker assertion and independent stale type/key/since cases, foreign tenant/namespace refusal,
unchanged sibling/run/journal/receipt metadata, and the page-before-LIMIT control. A subsequent
SQL wrapper annotation is verified by the final lint/focused gate. Existing event dispatch
regressions passed24/0/0 in66.34s harness/148.35s command, including duplicate, fan-out,
event/timeout, tenant isolation, crash retry and root lookback behavior. Application baseline
pins2 existing consumer/metadata cases. Architecture24crates and agent configuration pass.
No qualified dispatch type, receiver, clock producer or public scoped wait is enabled.

## Full five-SDK gate prerequisite correction

The immutable38d7b3f0 full64 invocation failed before the database scenario could execute:
its required database-runner executable had not been built. The failed gate is retained
(766.76s harness/820.09s command); it is not a64-scenario pass. The CI-declared prerequisite
build now passes52.53s. A private fixed copy of that executable has SHA256
6c32f3dc1d9150d1feaeccde3331bc7dd637a584b0041575f6d559bc274a7a07.
The focused database-connections aggregate passed all five languages without blessing,
1aggregate/0failed,28.67s harness/56.30s command, reusing the same already prepared SDKs.
A complete64-scenario retry is in progress against the same immutable source, same prepared
SDKs and fixed prerequisite. The test executable itself is copied before execution to prevent
replacement by concurrent builds; SHA256
a9f5df00128e7e9314679467e16188ece5b2cf4b27965737833ee8a960aa059f.
No acceptance is inferred before that invocation completes. Later root guards remain outside
this snapshot, so the result cannot replace final-tree release gates.


Strict workspace/all-targets Clippy passed103.06s at the root-guard production diff. The
SQL wrapper annotated fixture then passed2/0/0,4.25s harness/101.50s command. A second
review found the lookback test did not independently prove filtering before LIMIT; scoped
sentinels now sort earlier and lookback uses limit1 with an exact root-node assertion.
This is a test-strengthening correction; no production query changed. Final lookback-strengthened focused gate passed2/0/0,4.27s harness/65.76s command. The separate broader qualified-interface design
keeps scoped delivery construction closed until its native receiver exists.


## Qualified event candidate preparation

The bounded interface design received a fresh read-only review with no required issue. It
retains complete WaitingStep.instance and stored step_id, uses the same SQL tuple for cursor
seek/order, and preserves root-only discovery and delivery. It does not introduce a scoped
consumer or receipt writer: those must arrive together with actual receiver authority.
A root receiver also verifies the candidate UUID against its own pinned version/run/instance;
nonroot candidates are refused before opening the root RunTx. The existing canonical stored
path encoder/parser is reused instead of introducing another mapper or a second identity.

Mechanical interface adaptation preserved the old behavior long enough to observe genuine
failures: event-candidate-root-delivery-red passed2 and failed1 when a scoped candidate made
a legacy root receipt. event-candidate-identity-red passed2 and failed2: root was returned
after a scoped cursor, and wrong UUID/path candidates both marked root ([true,true]). These
are actual behavior failures, not compilation or infrastructure errors. Corrected application
and native gates are recorded below once complete. Public scoped waits, start/timeout clocks,
new receipt/value ownership and the old-dispatcher activation fence remain unimplemented.


Corrected application gate passed4/0/0 in5.25s command. The runtime-role native gate passed
4/0/0 in9.57s harness/87.91s command: exact cursor tuple, root discovery/lookback pages,
current binding markers, physical/logical candidate CAS and closed scoped lookback all pass.
These are candidate/storage preparation tests, not event acceptance by a scoped receiver.
Existing root dispatcher regressions and strict workspace Clippy are rerunning for the new
UUID guard and changed port signature before this increment is accepted. New full64 remains
tied to38d7b3f0; its SDK sources/prerequisites are fixed separately from the working tree.


Final candidate implementation review found no required issue. Existing root dispatch passed
24/0/0 in80.87s harness/120.55s command, including real event/timeout, fan-out and lookback
regressions under sagat_app. Strict workspace/all-targets Clippy passed125.26s command.
Architecture24crates and agent configuration pass. These close the bounded candidate increment,
not its future receiver or any canonical task. Root/trigger receipt construction retains the
legacy columns and keys; nonroot construction remains explicitly refused.
