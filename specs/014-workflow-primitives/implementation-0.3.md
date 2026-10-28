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


## Pure event-wait start command

The new start-only command requires exact nonroot WaitEvent, pristine Pending state and
active global/ancestor forward fences. It accepts the already evaluated correlation result
and explicit time: a key records WaitStarted with the pinned timeout; an expression failure
records the existing local error routing. No clock I/O or native subscription/receipt caller
is introduced. Snapshot CAS, database time/authority and native persistence remain pending.

The explicit unimplemented TDD boundary first compiled and failed1positive test with the named
pending-command error; it was removed before the successful implementation. Focused9/0/0
then passed. The first12-test extension passed11/failed1: its expected journal omitted the
existing empty StepRouted after an unhandled failure. Reading plan_path confirmed that closure
is intentional; the expected journal was corrected, with no production change, and12/0/0
passed1.86s command. The malformed-projection extension passed13/0/0. Final15/0/0 passed
1.98s command, covering ten independent Pending fields, both successful/failed evaluations,
local error recovery/wrong kind, duplicate/paused/stop/drain/cancel/overflow/path guards,
full/path equivalence and varied keys/clocks/member properties. Full-domain and strict-domain
Clippy gates are in progress, with fresh read-only implementation review. All native event
and canonical release tasks remain open; the pure command is not publication authorization.


The final full domain invocation passed574/0failed/0ignored across42target summaries,
38.79s command. Strict domain/all-targets Clippy passed12.08s. Fresh implementation review
found no required issue; formatting, architecture24crates and agent configuration pass.
The native dispatcher and full64 SDK gates remain separate snapshots, not acceptance of a
new scoped wait receiver. This closes the pure start-only command increment.


## Complete64-scenario five-SDK checkpoint

The corrected fixed-source38d7b3f0 invocation completed1aggregate/0failed/0ignored:
64scenarios ×5languages,1396.68s harness/1399.68s command,1399.69s wall/gap0.01s.
Six scenarios are authoring-only;58 produce290normalized observations, including publication
checks. Root independently parsed all290complete observations and compared them to their exact
committed goldens (58 per language), with no difference. The tracked source checkout is clean;
no blessing, language/scenario filter or external database was used. The final retry reused the
five SDKs already prepared unchanged by the initial invocation. Local Node26/Java23 are not
a pinned CI toolchain claim. The prerequisite failure and focused database success remain
recorded separately, not reclassified as passes of the initial aggregate.

Durable evidence: [sdk64-20261027](./evidence/sdk64-20261027/README.md), actual command/log,
manifest and290comparison hashes. Comparison manifest SHA256:
f325991582864acea425e4f1ab4f7e571d2f011d84806acc17125673832b6bcd.
The gate covers migration22 and recorded-event replay at38d7b3f0, not later root dispatch
candidate changes or pure start/timeout commands. Final-tree/native receiver/upgrade/resource
and release gates remain open. Its owned temporary checkout is retired after evidence capture.


## Pure event-wait timeout command

The exact full/path command compares explicit time with the recorded event deadline, derives
handled from the pinned onTimeout and preserves typed WAIT_TIMED_OUT/local onError semantics.
Pause permits an outcome but defers routing; stop/drain/cancel/stale outcome or malformed metadata
refuses before mutation. Start and timeout share only their scheduler-owned metadata invariant.
This is pure preparation, with no native producer/receiver or publication activation.

The explicit pending-command TDD boundary failed1focused case before implementation. The first
attempted edit did not apply because its replacement script missed a formatted block; a follow-up
run on unchanged source still failed the same case (15passed/1failed). That attempt is retained,
not described as an implemented failure. The actual edit then passed16/0/0. Route/pause extension
passed18/0/0 in1.64s command; source/order/kind/binding extension passed21/0/0 in1.12s.
Final focused23/0/0 passed1.20s command, including nine malformed Waiting fields, unchanged
full/path refusals, both pure event/timeout orders, duplicate timeout, paused cold routing,
handled/unhandled/onError and deadline +/-1microsecond properties. The new timeout tests use
a child module; shared fixture restoration changes one row while retaining genuine membership.
Fresh read-only review found no required issue. Full-domain and strict-domain Clippy results
follow separately. These tests do not establish real event/timeout transaction races or native
clock/subscription/receipt authority. All canonical runtime/release tasks stay open.


Full domain passed582/0failed/0ignored across42target summaries,37.29s command.
Strict domain/all-targets Clippy passed11.11s command. Architecture24crates, agent config
and formatting pass. This closes the pure timeout command increment. The retained64-SDK
checkpoint and earlier native root dispatcher gates remain separately sourced evidence.


## Exact-context correlation evaluator preparation

The additive internal evaluator method delegates roots unchanged and refuses scopes by default.
JsonEvaluator explicitly reuses the existing exact-path environment and scalar/key validation.
The initial correlation-only gate passed1root test/failed1scoped test for the real unsupported
compatibility default. The implementation then passed13evaluator tests. The first expanded
suite passed16; full application+IR passed434/0/0 across36target summaries,46.34s command,
and strict application+IR/all-target Clippy passed31.21s. These precede the final test tranche.

Fresh review found2valid coverage gaps: the production override did not exercise the prior
adapter default, and no positive test pinned lazy own input/state/run evaluation without an
iteration input. Both now pass in the18-test evaluator invocation. A further candidate concern
about dynamic missing-key values was tested, but that diagnostic was GREEN1/0 at1.60s:
infra-expr already substitutes <computed> for payload-derived lookup keys. No privacy defect
or new sanitizer is claimed; the regression retains the canonical existing protection. The
misleading red-named temporary log is preserved as an observed pass, not a RED. A final review
found no remaining required issue, including the prior-adapter fixture and lazy context cases.
The complete corrected gate and strict lint results follow separately. This port has no native
start caller/public body capability; it does not prove membership, snapshot CAS or db authority.


The corrected full application+IR gate passed437/0failed/0ignored across36target summaries,
5.13s command, including19evaluator tests. Strict application+IR/all-targets Clippy passed2.09s.
The second bounded review found no remaining required issue. Formatting, architecture24crates
and agent configuration pass. No native scheduler call, subscription, external event authority
or public shape was added. This closes only exact-context evaluator preparation.

## Native scoped event-wait start storage (in progress)

The internal immutable structured-codec fixture now reaches a real Pending wait by collection
admission and scoped worker completion. No published version is rewritten. Both native writers
preserve the exact binding, derive its lookback boundary from the pinned timeout/deadline, and
require acknowledged own-run scheduler authority. Public bodies, discovery and external delivery
remain closed; native timeout persistence and cancellation/drain are separate pending increments.
The contract is [scoped-event-storage.md](./scoped-event-storage.md).

Observed REDs: the initial native start reached the unsupported path-storage event (85.62s
command); the two-case authority gate failed both unsupported start and an actually committed
unfenced EXPRESSION_ERROR start (64.06s command). After implementing clock acknowledgement and
binding storage, the first three cases passed. Expanded event-wait-start-binding-red passed4/
failed1 in12.88s harness/72.23s command: generic StepFailed(WAIT_TIMED_OUT) really committed
without timeout authority. The corrected guard rejects every scoped event-wait StepFailed
except the clocked Pending EXPRESSION_ERROR shape. Its five-case gate passed5/0/0 in12.34s
harness/79.05s command. A fresh bounded authority review found no remaining concrete issue.

event-wait-start-guards passed9/0/0 in31.91s harness/104.16s command. Named cases cover both
writers, missing and successful-then-refused own acknowledgement for both start shapes,
pre-guard clock refusal, checked sibling/lookback/worker absence, cold full replay, five
independent orphan fields across full/path/frontier readers, and actual journal-trigger failure
with unchanged run/steps/scopes/members/usage/retention/payloads/audit/outbox/telemetry. Only the
new start store is explicitly sagat_app in that invocation; the final service-wide runtime-role
profile is separate. No timer or event outcome race is inferred from these start tests.

Fresh binding review exposed missing namespace/UUID checks in the read-only paths. The new
all_readers_refuse_a_same_tenant_wrong_namespace_or_invented_step_uuid test failed for both
independent CHECK-valid corruptions: all three readers accepted each one. The gate failed1
in3.04s harness/62.48s command. The shared identity validation correction and complete focused
runtime-role gate are in progress. A failed patch application changed no file and ran no gate;
the subsequent exact patch applied. All eight canonical tasks remain unchecked.

The corrected service-wide sagat_app gate passed10/0/0 in36.01s harness/103.82s command.
The second binding review found no remaining issue in namespace/UUID selection and comparison.
The final expanded native start gate passed11/0/0 in40.06s harness/97.72s command: six independent
physical changes after preparation (key/type/since/routed/kind/attempt), in both writers and with
an independently unchanged Run revision, are refused without overwriting the changed source.
Full domain passed583/0/0 across42target summaries in39.47s command; strict workspace/all-targets
Clippy passed130.39s. Canonical primitive contracts passed49/0/0 in0.60s harness/3.02s command,
including refusal of both unproven public event-wait body shapes. Timer regression passed13/0/0
in57.79s harness/57.81s command under the complete runtime-role profile. The direct harness is
the unchanged executable compiled by the final11-test gate, SHA256
2ea0af83a7e7bde04c22ccd57a51c05c24a0728016d4eceafadbd8ae041ca54a.
Control, common fence and root-dispatch regressions are separately completing before commit.

The remaining regressions pass under sagat_app: controls23/0/0 in84.56s harness/84.57s
command, common bounded/full fences10/0/0 in34.02s/34.04s (including actual held-lock expiry
and parent-creation refusal), root dispatch24/0/0 in59.84s/90.47s. Both direct native filters
use the same SHA256-pinned11-test executable, not a subsequently rebuilt test target.
Formatting, diff checks, architecture24crates and agent configuration pass. This accepts only
native start storage; no canonical task, public event body, native timeout/receiver/drain,
mixed-version cutover or release is accepted. The original frozen timer and full64 SDK gates
remain independently sourced earlier checkpoints.

## Native scoped event-wait timeout storage

Two real runtime-role positive cases failed at the existing unsupported native event boundary:
full handled and bounded unhandled timeout,0passed/2failed,4.66s harness/57.65s command.
Both sources had actual1ms starts and an observed PostgreSQL due-deadline predicate, without
aging the projection away from its journal. Adding WaitTimedOut to the two native whitelists
uses the already reviewed own-shard clock/read-set and binding cleanup. The complete start+
timeout gate then passed13/0/0 in39.98s harness/99.29s command. No public body, scoped event
receiver, scheduler producer/discovery or drain coordinator is enabled by this change.

The expanded runtime-role invocation passed19/0/0 in65.94s harness/116.47s command:
handled/unhandled in both writers, local onError, scan/subscription cleanup, sibling/worker
absence, cold structured-codec replay, early/wrong-outcome refusal, pause outcomes, actual
audited Resume followed by explicit cold path routing once, and late projection-trigger abort
after journal insertion with complete transaction snapshot equality. Automatic scheduler
resume/discovery and event/cancel races are not inferred. Bounded read-only mechanism review
found no substantive issue; this is storage-only acceptance.

The binding extension passed20/0/0 in72.95s harness/192.06s command, including three owner
changes without a Run revision in each writer: correlation key, event type and a coherent
deadline/since shift of1microsecond. All are refused without repairing the changed source;
restoring the exact source replays. This invocation waited on the shared Cargo build lock.
Strict infra-postgres+tests/all-target Clippy passed72.50s command. Domain583 and start/root/
control/timer/fence regressions remain the separately recorded unchanged-logic checkpoint.

Self-review of the negative matrix found a coverage overlap: the due/no-authority case also
had the wrong handled flag. The test now uses the valid pinned handled outcome when own
authority is absent, independently isolating that refusal. The corrected focused case passed
1/0/0 in4.77s harness/56.62s command. No production defect/change was needed. The20-case
binary was already executing before this test-only correction/rebuild; its unchanged production
mechanism results and the corrected one-case result are separate invocations, not21distinct
cases of a single final-source suite. Final test-target lint follows separately. All eight
canonical runtime/release tasks remain open.

The corrected final integration test target passes strict Clippy in43.13s command, including
its shared build-lock wait. Formatting, diff, architecture24crates and agent configuration
checks pass. This accepts only native timeout storage. Cancellation/drain is the next required
bounded lifecycle increment; receiver, scheduler activation and release remain closed.

## Scoped event-wait collection drain (in progress)

The new [bounded drain contract](./scoped-event-drain.md) covers the reducer, both scheduler
eligibility paths and the independent native coordinator. Native user cancellation first
failed1 in2.91s harness/55.41s command: the run remained CANCELLING on its workerless event
suspension. The first domain invocation failed compilation because the new test module needed
an explicit path, not because behavior was disproved. After correcting only that path, the
focused domain gate passed2/failed1 in0.98s command: a healthy waiting member could not drain,
while malformed waiting fields and the existing cancellation regression passed.

The intrinsic suspension predicate, pinned scheduler checks and native exact locked-row update
then passed26domain cases in4.01s command. The first native correction attempt failed compilation
for a missing OffsetDateTime import (17.02s command); the corrected native case passed1/0/0
in2.59s harness/62.79s command. A fresh implementation review found a valid domain/native
mismatch: failed drain could still manufacture generic failure from a waiting entry. The new
focused case failed1 in1.18s command before adding the pre-mutation refusal. The corrected
focused domain gate passed27; an additional positive case preserves Pending-entry failure.
The third bounded review found no remaining issue in that correction.

The first extended native gate passed1/failed2 in9.34s harness/80.82s command: the direct-port
fixtures had not actually started collection drain, so even their positive control returned
InvalidTransition. These failures are fixture setup errors, not production or malformed-row
acceptance evidence. The fixture now runs the real fenced coordinator DrainStarted transition
before preparing the direct request. Corrected native cases passed3/0/0 in10.72s harness/
62.12s command: full/frontier progress and normalized drain journal parity, independent writer
refusal of worker/type/since/namespace/UUID corruption, unchanged state on refusal, stable
duplicate receipt, and an actual post-cleanup journal fault rolling back binding/counters/
outbox before a successful same-receipt retry. All native services use sagat_app.

Full domain regression, held-Run timeout/cancel ordering and real sibling-expression failure
drain are being completed. No canonical task or public event body/delivery/producer is enabled.

Full domain passed587/0/0 across42target summaries in36.59s command. The six-case native
cancellation gate passed6/0/0 in19.14s harness/69.30s command: both actual held-Run timeout/
cancel orders (including queued public cancellation), actual member-specific correlation
failure and full/frontier drain, plus earlier parity/source/rollback cases. No sleeps order
these races; exact backend identity and pg_blocking_pids prove the wait edges inside bounded
stages. The complete same-source native lifecycle filter passed26/0/0 in97.65s harness/
97.66s command. Strict workspace/all-target Clippy passed90.15s.

Unchanged direct-harness regressions passed under sagat_app: timers13/0/0 in52.91s harness/
52.93s command, controls23/0/0 in84.31s/84.32s, and native primitive transactions15/0/0
in30.48s/30.49s (including stale/expired authority and capacity/storage regressions). All four
direct filters used executable SHA256
f4c923fd963889ba4bd509c09e04aced2b6628c507dadcc53caa9265e0bc857e;
none was launched from a subsequently replaced executable.

After those filters completed, the shared test snapshot additionally captured the tenant
iteration counter. Its strengthened late journal-abort test passed1/0/0 in2.55s harness/
53.73s command, with unchanged tenant active capacity as well as the previous observations.
The final independent native authority/revision case and corrected test-target lint are now
running. This test-only strengthening is separate from the preceding26-case invocation.

The final cancellation profile passed7/0/0 in21.89s harness/97.83s command (including Cargo
lock wait), with explicit native SchedulerFenceLost and RevisionConflict refusals, unchanged
state/capacity on each refusal and a current-source positive control. All seven use the expanded
tenant-iteration snapshot. Final test-target Clippy passed26.93s; formatting, diff, architecture
24crates and agent configuration pass. This accepts collection event-wait drain, not loop event
drain, receiver/discovery/producer/public body, nested capacity or release acceptance. The
preceding26-case and final7-case runs are separate, not33distinct cases of one suite.

## Pure scoped event satisfaction and canonical window

The explicit pending-command TDD boundary first passed2existing cases/failed2new positives
in4.01s command. Implementing full/path matching and the checked canonical lower bound then
passed30focused cases. A new typed EventNotMatching refusal has only the exact instance, not
event values; its test first failed1in4.01s because ordinary mismatch was incorrectly treated
as Invariant. The corrected gate passed31. The expanded fixture initially failed compilation
for calling paused instead of is_paused (0.30s); fixing that test call passed37/0/0 in1.09s.

The commands check the pinned type/key, inclusive received_at window and exact live member,
allow paused outcome/deferred route and preserve first-commit behavior after the deadline.
The native window adapter now reuses scoped_event_wait_since; root legacy windows stay separate.
Before review corrections, combined domain+application+IR passed1033/0/0 across78target
summaries in127.96s command; native lifecycle passed27/0/0 in96.51s harness/164.97s command
with that canonical window. These invocations are not acceptance of the later pure corrections.

Fresh adversarial review found2real issues: a duplicate pre-existing successor could make
settlement fail after mutating the wait, and root/unadmitted full candidates produced Invariant
where the bounded command produced UnknownInstance. The two added regressions genuinely
failed0pass/2fail in1.17s command. Full now checks root/existence before projection; both
commands apply and settle on a scratch state, assigning it only after complete success.
The path copy is bounded; the full counterpart intentionally copies the whole restored model.
The corrected focused gate passes39/0/0 in4.11s command. Second review found no remaining issue.

Final combined domain+application+IR passes1035/0/0 across78target summaries in65.44s command;
strict workspace/all-target Clippy passes106.70s. Primitive contracts pass49/0/0 in0.59s
harness/21.91s command, including continued public event-body refusal. Formatting, diff,
architecture24crates and agent configuration pass. No pending stub remains. This is matching/
window preparation only: current ingress/own-Run receipt, retained classified payload/quota,
scoped delivery/discovery, native WaitSatisfied persistence, scheduler producer, old-dispatcher
cutover and final release gates remain open. No canonical task is checked by this increment.

## Shared canonical event document

Application dispatch now owns the production JSON event-document encoder; JsonEvaluator delegates
without changing its extension interface. This prepares authoritative native value retention
without an adapter-to-adapter dependency or a second envelope encoder. Exact bytes, nullable
fields and the legacy invalid-JSON→null fallback remain unchanged; this factory is not ingress
validation. The future scoped receiver must reject invalid current inbox data independently.

The new factory stub first failed2cases while the existing evaluator's exact-byte baseline
passed1 (5.96s command). Reusing the old encoder then passed all3cases in6.04s command. Read-only
compatibility review found no concrete issue. Full application+IR passed440/0/0 across38target
summaries in71.33s command; current root dispatch passed24/0/0 in57.61s harness/105.00s command
under sagat_app. Strict workspace/all-target Clippy passed82.80s; formatting, architecture,
agent configuration and diff checks pass. No new dependency, schema, receipt, current-ingress
guarantee, scoped delivery/discovery or public body is introduced; canonical tasks stay open.


## Native scoped event receipt and value checkpoint (2026-10-27)

The internal full/exact-path ports stage one current inbox admission receipt, returning its
canonical document. Inputs are expectations, not authority: current own Run entry and prepared
revision, tenant/namespace, canonical UUID, pinned wait, complete captured/current subscription,
all canonical admission fields (including occurred_at and received_at), stored admission
fingerprint and JSON object shape are checked. Entity order is Run → inbox → step → output
ledger. The receipt uses wait-instance:{run}/{stepUuid} and existing nullable identity columns.
A conflict validates all exact metadata and returns no value; malformed metadata is corruption.
The conflict SELECT is plain because sagat_app lacks receipt UPDATE permission. It authorizes
no outcome; the parent inbox stays locked. No grants or migration changed.

WaitSatisfied and its one exact canonical output require the private unconsumed witness.
Native output quota charging, journal payload association and physical normal Succeeded shape
are integrated; handled timeout stays output-free. Generic completions/resolutions cannot
manufacture this value. Commit refuses orphan receipts and any ignored failed persistence,
including failure after successful consumption and failure before staging hidden by a later
successful legacy retry. Persistence arms before validation/await and complete success restores
the previous failure bit, preserving prior failure history. Non-receipt legacy retry behavior
is unchanged. The detailed protocol is scoped-event-receipt.md.

Real behavioral REDs precede the fixes: the initial two native receiver cases failed at the
unsupported port (4.65s harness/76.72s command); orphan receipt committed (0/1,2.40s/73.35s);
a second operation after an unmatched first candidate was accepted (0/1,2.31s/61.20s). Bounded
staging review corrected occurred_at coverage, repeated-operation tracking, conflict identity
and inbox/step lock order. Its final read-only pass found no issue within that scope.

Four native cases then passed13.64s/72.55s. Additional rejection coverage first passed8/failed1
(33.86s/83.85s): the late-error fixture stored the wrong request fingerprint, so replay correctly
refused IdempotencyKeyReused; this was a fixture error, not behavior proof. With that fixed,
another gate passed9/failed2(41.71s/91.55s): a dotted test source violated Slug and the conflict
FOR SHARE lacked runtime-role permission. The slug fixture and the unnecessary row lock were
corrected without widening grants. The subsequent13cases passed61.80s/122.51s.

Separate witness review found two ignored-error sequences. The post-consumption late conflict
first failed0/1(2.36s/54.40s); the pre-staging failure → successful retry → receipt sequence
first failed0/1(2.27s/54.78s). Both now retain failure history. The final bounded review found no
remaining witness/ignored-error issue. An earlier invocation accidentally selected zero tests
after a failed test-edit script (50.51s); it is invalid RED evidence and is not counted. The
corrected14-case gate passed63.61s/122.15s. These are separate invocations of overlapping cases,
not cumulative counts. All native gates use actual EventService admission/claim, real scoped
activity completion and current sagat_app transactions in the immutable structured-codec fixture.

The actual disclosure service reads the retained wait.satisfied sequence with immutable current
classification rules1 and the exact body outputSchema. Both writers prove secret/PII withheld
and public-container disclosure, with no event value in journal detail/audit reason/outbox.
These remain internal fixtures, not public-body or released-profile activation evidence.

Initial strict Clippy failed on two new expect calls(45.49s); typed corrupt errors replace them.
Final workspace/all-target Clippy passes132.03s. Architecture and agent configuration pass.
The broader lifecycle, pause/quota and root-dispatch gates are recorded below after completion.
Held event/timeout/cancel and ingress-cleanup races, additional current-admission corruption,
production receiver/discovery/start/timeout callers, public bodies, actual released-v0.2 rollout,
nesting/capacity and final native/SDK/resource/release acceptance remain pending. None of the
eight canonical tasks is closed by this checkpoint.

Final checkpoint verification on the completed typed-error source passes native lifecycle43/0/0
(186.67s harness/248.91s command), including the16receipt cases, current deferred retained-quota
rollback in both writers, and paused delivery → explicit cold resume → duplicate with one
receipt/value/outcome/route and exactly one logical byte charge. No injected lost network
acknowledgement is claimed by that cold duplicate check. Root dispatch passes24/0/0
(65.38s/135.51s), contracts49/0/0(0.60s/2.81s), and strict Clippy132.03s. Separate Cargo
invocations retain fixed sources; tests added only after a prior single-target binary was
already executing belong to their later invocations. Formatting/diff/architecture/agent checks
pass. Native executable SHA256: a88a6c3432f45db3e5cf85910c56440057c983f9c364d6cec3f298c484cedbc3.


## Native scoped event outcome/cleanup ordering (2026-10-27)

On unchanged production source80ae824a, actual held-Run tests prove both commit orders for
scoped event versus due timeout and cancellation. Event-first cancellation queues the actual
public audited RunOperation; cancellation-first stages the real system stop command under own
Run/shard authority and proves the waiting ingress refuses current stop state. The actual
collection scheduler finishes both cancelled drains. Exact PID+backend_start and
pg_blocking_pids show the contender waiting on the holder's Run lock; a bounded join releases
only after observing the real edge. No sleep races, old-process or public body proof is claimed.

Actual purge_events also queues on the receiver's locked inbox row until canonical value commit.
The purge then removes inbox and FK-cascaded receipts while the retained run value and journal
replay remain valid. A separate cleanup-first committed control leaves both native writers with
no value for the stale claim. A prepared bounded snapshot additionally waits on a held timeout
and must fail the changed Run revision before any receipt. Six original order cases pass15.06s
harness/65.49s command, then all seven pass16.58s/66.49s. These are overlapping invocations.
The prepared-path assertion is tightened to the typed RevisionConflict and verified separately
below. Test-target strict Clippy passes36.05s, then16.29s after the seventh case.

The new current-row corruption test initially failed during fixture setup because PostgreSQL
already enforces inbox_events_payload_limit (23514),2.35s/51.73s. This is not a behavioral RED
or a production size-check defect. The corrected test asserts that existing database boundary
and exercises CHECK-valid wrong fingerprints, scalar and malformed JSON in both native writers,
then validates the restored real admission. It passes1/0/0 in2.40s/3.06s; no duplicate production
bound or migration is added. Native receipt/outcome/cleanup ordering now has this focused proof;
actual production callers, timeout discovery/public activation, released-v0.2 rollout and the
remaining canonical SDK/nesting/capacity/resource/fault/release gates remain open.

The tightened prepared-path case passes1/0/0 in2.45s harness/52.89s command, requiring
RevisionConflict exactly. It overlaps the seven-case gate and is not an eighth race. Only
tests/helper visibility and evidence changed after80ae824a; production behavior is unchanged.


## Collection scheduler event subscription and deferred local route (2026-10-27)

A private collection_event_waits pass connects the full/frontier collection scheduler to
accepted exact-instance start and deferred-route commands. The bounded caller evaluates the
captured instance correlation before begin_path revision fencing and obtains current own
scheduler authority/post-lock database time before writing. The full oracle uses its owned
Run/data and the same evaluator/clock acknowledgements. One member decision commits per
transition, with no worker capabilities/hints or output payload for a subscription. Existing
journal/projection/telemetry/tenant quota behavior remains native; both persist and COMMIT
quota refusal end the failed transaction before reopening the original scope/transition drain.

The scheduler start case first failed Waiting versus BodyProgress (0/1,2.33s harness/52.67s
command); the implementation then passed1/0/0 in4.73s/60.87s, covering both modes and two
same-name member keys/windows with one start each. Review found deferred Failed/onError
outcomes were preempted by fail-fast classification. The new paused-timeout case genuinely
failed at the paused scheduler call with Primitive(InvalidTransition)(0/1,2.47s/53.31s); a
same-binary diagnostic backtrace run(2.55s/2.56s) located scheduler.rs:50 after the actual
paused timeout, not fixture setup. Both schedulers now defer only valid unrouted scoped wait
failures with a declared onError while no Run stop is committed. Domain failure classification
is unchanged; local routing must commit before the failure becomes handled. Other active
failures and committed stop/drain retain precedence. Corrected2cases pass9.74s/64.10s.

The second review raised singleton authority, but the enclosing collection entry methods
already reject Hold::Nothing/Lost with FenceUnavailable before reaching this private pass
(passes.rs advance_collection[_from_run]_in_view). The missing entry-contract context was
corrected; no duplicate authority mechanism or production fix was added. The new both-mode
no-placement control passes on the unchanged implementation(4.56s/56.21s); despite its
historical log filename event-scheduler-singleton-red, this is NOT RED evidence.

Expanded8caller cases pass36.04s harness/89.58s command under sagat_app. They cover exact
single-start keys, paused Failed/onError routing, existing no-placement fencing, paused pending
start/resume, committed cancellation before start, actual correlation-expression error/local
recovery without value/worker capability, a real closed current content quota at COMMIT and
an explicitly injected real-SQL typed persistence-time quota fault with ample actual quota.
The two quota cases prove no rejected wait.started/binding and exactly one fresh quota drain
using the original scope/revision; the injected case is not described as an actual policy
refusal. Final bounded review found no remaining issue in start/deferred-onError scope.

Strict workspace/all-target Clippy passes63.81s. Final native lifecycle59/0/0 passes229.10s
harness/229.84s command; collection controls23/0/0 pass86.53s/86.54s; contracts49/0/0 pass
0.68s/5.27s and retain public scoped-event-body refusal. Architecture and agent configuration
pass. Timer regression and final source hash are recorded below on completion. The detailed
bounded contract is scoped-event-scheduling.md. Timeout discovery/processing, external receiver,
public activation, actual released-v0.2 rollout, remaining nested/capacity/SDK/fault/resource
and canonical release gates remain open; none of the eight canonical tasks closes here.

Timer regression passes13/0/0 in49.53s harness/49.54s command. Native fixed executable SHA256: 8c9297513bede69ee66a2fc9854a1efe391ae8c4580e2a7f7e3d12f07ad85123. Formatting and diff checks pass.


## Qualified automatic event deadlines (2026-10-27)

Maintenance discovery now returns complete instance paths for due event waits; selection keys
include that identity. Root processing retains its legacy command. Scoped processing requires
an owned placement and uses bounded path entry, current revision, own Run/shard acknowledgement
and post-lock database clock before the accepted native timeout. Discovery grants no authority.
Both collection readers defer valid unrouted Failed waits while no Run stop is committed,
including waits without onError. Pause holds routing; after resume, a route without onError
still leaves the domain failure unhandled for the next normal fail-fast decision.

Genuine REDs: qualified discovery returned0 instead of2 (0/1,2.40s harness/54.79s command);
paused Failed without onError raised PrimitiveInvalidTransition (0/1,2.74s/62.13s); the combined
three-case gate failed all3 (6.88s/57.60s), including the actual tick returning0 timeouts instead
of2. Query, consumer and deferral correction pass3/0/0 (9.44s/68.06s). The expanded gate had
5pass/1fail (19.20s/69.68s): its whole-run success assumption ignored the fixture's legitimate
root activity a after the collection. A metadata-only diagnostic rerun failed1 (3.12s/54.19s)
and identified that activity READY with the collection already finished. The policy fixture is
now collection → end before immutable insertion; no production End change was needed. Its
three-policy aggregate passes1/0/0 (8.84s/59.31s). This fixture error is not a behavioral RED.

Seven deadline cases cover qualified pages/exclusion, same-name members, cold duplicate ticks,
legacy singleton root compatibility, scoped no-placement/stale-generation refusal, paused
handled/unhandled outcomes and cold resume to whole-run success/failure. The actual consumer
race holds a native canonical event delivery while Scheduler.tick queues at bounded Run entry;
exact backend PID/start and blocking edge release the holder, without sleeps. Event commit
invalidates that captured revision; only the other due member times out. It passes1/0/0
(2.50s/54.39s). Existing native port races alone are not presented as this consumer proof.

A fresh-context bounded review found a draft overstatement about sibling failures; the contract
now distinguishes an unlatched sibling failure from committed global/scope closure. The native
command gains no sibling-read authority. The second review found no remaining issue in this
checkpoint. No delegated test result substitutes for the root gates.

Final fixed-source native lifecycle passes66/0/0 (259.49s harness/260.23s command); root dispatch
24/0/0 (60.83s/109.78s), contracts49/0/0 (0.56s/2.57s), strict workspace/all-target Clippy72.31s,
architecture and agent configuration pass. Formatting and diff checks pass. Native executable
SHA256: bc28712f1f47f94cea4591fa43314c5a199bdc674154549ff59bc757d3405ae8.
Detailed contract: scoped-event-deadlines.md. No grants, migrations, public shape or SDK changed.
External receiver, entry admission, public/released-reader activation, loops/nesting/capacity,
full fault/SDK/resource/release acceptance and all eight canonical tasks remain open.


## Production scoped event receiver and bounded lookback recovery (2026-10-27)

The dispatcher now branches nonroot candidates to begin_current_path, reads no full Run history
and retains only the canonical document returned by the accepted native receipt port. The exact
satisfaction/output/audit/receipt and quota commit together; current logical_output_bytes enforces
the existing Run output ceiling. No scheduler clock or lease is invented for external ingress.
Root delivery retains its legacy transaction and receipt key. WaitingStep.delivery_consumer
shares the legacy root/exact scoped key construction with native receipt and lookback selection.
Direct matching pages retain the complete database tuple. Lookback selection and markers both
require the captured current tenant/namespace/run/path/node/UUID/type/key/since binding.

Two genuine initial REDs returned0 wakes instead of2 paged direct deliveries /1 first lookback
(0pass/2fail,5.11s harness/59.93s command). The first caller correction passes2/0/0 in5.24s/80.65s.
The expanded five-case gate passes4/fails1 (12.94s/67.53s): the quota test's exact snapshot
incorrectly included the intentionally committed inbox retry log charge. Metadata diagnosis
isolates log_bytes 0→58; the corrected assertion independently matches that delta to the exact
inbox last_error byte length and still compares every other retained/execution field. This was
a test expectation error, not an outcome rollback defect.

Fresh adversarial review found three actionable recovery defects: Some(event)+No/Refused was
marked scanned; a failing oldest scoped wait monopolized every bounded page; and non-invariant
pure settlement errors after locked native admission were discarded as races. The seven-case
gate has5pass/2fail (17.99s/73.31s): the fairness RED is genuine, while the first purge race failed
fixture setup because purge correctly refuses PENDING inbox rows. Actual claim+finish before
expiry corrects that fixture. The eight-case gate then has5pass/3 genuine failures
(29.40s/84.45s), reproducing scan closure after observed queued-Run/purge ordering, sibling
starvation and deterministic duplicate-successor route error incorrectly finishing the event.
The corruption case explicitly inserts a premature, structurally valid successor and claims no
normal transition or replay guarantee for that malformed source.

The correction propagates pure errors after native admission, leaves selected-event No/Refused
lookbacks pending, and traverses unscanned rows by a strict global SQL tuple with tenant identity.
A short process-local cursor advances before consumer awaits and wraps once on an empty page;
at most one scan_batch page is processed. Pending rows stay durable; restart resets only fairness
position. This is not a new claim or outcome authority. The reviewer's proposed equal run UUID
cross-tenant skip counterexample was withdrawn after checking the global runs.id primary key;
explicit tenant ordering remains appropriate, but is not recorded as a current defect fix.
Second bounded review finds no remaining issue. Existing user preference keeps external review
skipped; no delegated tests are claimed.

Index-only migration20261223000000 matches the new pending-wait tuple. Every committed branch's
migration names were checked before selecting it. The legacy updated_at index remains; no
column, function or runtime grant changes. A normal CREATE INDEX can block writes while it
builds, documented in scoped-event-receiver.md. The focused real-data test undoes only this
index, compares all retained/execution state and qualified candidates, reapplies it and verifies
replay. It is not a released-v0.2 upgrade proof.

The corrected nine receiver cases pass9/0/0 (21.35s harness/96.38s command), including native
runtime-role direct paging, finished-inbox lookback, exact stale candidate matrix, current COMMIT
quota rollback/retry, paused outcomes/cold routing, fair failed-consumer recovery, held-lock
cleanup recovery, deterministic route-error retry and live-data index roundtrip. Final native,
legacy dispatcher, contracts and migration/privilege gates follow below. Workspace/all-target
Clippy passes99.60s; architecture/agent/format/diff checks pass. Sources remained fixed during
each compilation; the eight-case RED executable was already running before later correction.
No public body guard or worker protocol changed. Event entry admission, public/released-reader
activation, loops/nesting/capacity and final fault/SDK/resource/release acceptance remain open;
none of the eight canonical tasks closes here.


Final receiver checkpoint: fixed executable scoped_event gate passes79/0/0 (346.74s harness/
346.76s command), including the75native lifecycle cases and4qualified discovery/identity cases.
This gate invokes the already-built immutable integration executable directly; its SHA256 is
990a4e56671bcf694e26c2eb7e78b860ad413473192943152bda77e2100f3a26.
Legacy dispatch passes24/0/0 (70.83s/211.27s including Cargo build-lock waiting); contracts49/0/0
(0.63s/2.49s) retain scoped wait publication refusal. General migration checks pass5/0/0
(9.37s/10.28s), separately from the receiver's admitted-wait index roundtrip.

The runtime privilege gate initially passed32/fail1 because its lexical extractor could not
resolve17sites from the earlier shared predicate macros. The separate519dfa82 correction and
its complete RED/review trail are in docs/engineering/sql-privilege-audit.md. No grant/ALLOWED
exception changed. The corrected gate passes42/0/0 (14.30s/43.50s), auditing1090statements in56
files,675distinct needs and638native role probes. Final workspace lint passes29.82s after the
extractor changes. Later changes are documentation only; receiver production sources stayed
unchanged after the nine-case gate and during the complete79-case executable run. Architecture,
agent configuration, formatting and diff checks pass. External delivery is now connected for
these internal pinned fixtures; entry/public/old-reader activation and all canonical remainder
boundaries above still apply.


## Scheduler-owned event entry admission (2026-10-27)

Three existing kind checks now accept a pinned WaitEvent entry as Scheduler dispatch: the
collection scheduler's admission builder, native immutable-definition admission and the shared
cold-reader dispatch validator. Native membership/step creation remains PENDING, with null
activity/outbox, no worker hint or subscription/clock fields and the existing complete body
reservation. Activity dispatch still cannot match a WaitEvent declaration. No migration, grant,
public publication guard or worker protocol changes.

Actual full/frontier collection entry → admission tests first fail2 with InvalidProgress
(5.20s harness/61.50s command), then pass2/0/0 (5.54s/72.55s) after those checks. Each immutable
internal fixture has only WaitEvent → Return and root End: cold full/path reads succeed, two
item-specific subscriptions receive actual EventDispatcher admissions, the whole Run finishes
and no task.ready hint or held iteration slot remains. These are not public parser fixtures.

The expanded gate initially passes5/fails2 (22.18s/76.16s). Its paused/zero controls incorrectly
expected Waiting instead of the existing direct-command typed refusals: Primitive(InvalidTransition)
and Primitive(AtCapacity). The scheduler's real discovery excludes paused runs; its sweep treats
capacity as skipped. Corrected tests require the exact errors, unchanged execution state and an
actual tick with failed=0. No production error mapping changes and no behavioral RED is claimed
for those assertion mistakes. Seven cases then pass7/0/0 (26.64s/81.05s), including pause/resume,
current explicit zero preserving admitted progress while blocking the next member, pending-entry
cancellation retiring its return reservation, full native admission journal-fault rollback/retry
and CHECK-valid worker-dispatch corruption rejected by full/path/frontier readers.

A fresh bounded reviewer finds no issue in the three production checks and runs no tests.
A separate later cold-start case recreates cache/store with one connection, proves durable
collection_candidates discovery and uses actual scheduler ticks to start both waits without
worker hints. It passes1/0/0 (2.46s/60.18s). It was added only after the prior fixed native
executable was already running; it belongs to this later invocation and is not counted as part
of an87-case aggregate. Receiver code is unchanged except test-helper visibility.

Workspace/all-target Clippy passes76.45s; control regression23/0/0 passes97.74s/97.75s and
contracts49/0/0 pass0.58s/67.26s. The completed86-case event executable and final targeted lint
are recorded below on completion. The later source change to BodyProgress is documentation only.
The complete bounded contract is scoped-event-entry.md. Public event bodies/entries, actual
released-v0.2 cutover, loops/nesting/capacity, final SDK/fault/resource/release gates and all
eight canonical tasks remain open.

The completed fixed event executable passes86/0/0 (338.78s harness/338.79s command), using
SHA256 d1df40a9992d1c9bf3241c732caddafc518ee6aa3c29affed513b9eaba27ebab. Its source includes
the seven entry cases. The later cold-only test passes separately and its rebuilt executable
has SHA2564a638270b16cfe1887aff2b97e94c0c86ee86ead0d6a5bc013707ddb0984cbac; no87-case aggregate
was run or claimed. No production behavior changed between those invocations. Final targeted
Clippy for the enlarged integration target passes30.65s after the cold test and doc comment;
architecture/agent/format/diff checks pass. Public activation and the eight canonical tasks
remain pending.

## Actual released-v0.2 active-data profile — 2026-10-27

The [released profile contract](./released-v02-profile.md) now has an executable coordinator
and a reproducible source-verifying runner. It archives actual v0.2.0 commit
bfda342073db71e84f30bdc79ccf3fbc97a7153c; 649 production/configuration files are checked against
that immutable source, with exactly three declared test overlays. Old/current targets are
distinct. Copied executable hashes, current tracked/untracked pre/post-build input manifest,
schema boundary, real output and phase timings are retained in
[released-v02-20261027](./evidence/released-v02-20261027/README.md).

The released public producer completes reserve through worker1.5, leaves charge READY/unleased
and starts a legacy root event wait. After old-process exit, the current coordinator attaches
the exact owned database and applies normal migrations20261220000000→20261223000000 without
checksum or missing-version exceptions. It checks unchanged subscription metadata and both
old runs before progressing them. Explicit database-clock expiry of the stopped scheduler is
followed by actual Keeper takeover: relevant shard ownership, live expiry and higher generation
are asserted. Current runtime-role worker/scheduler/event services finish both runs. Another
process of the exact released executable then verifies both through REST and native replay,
including immutable definition/journal/captured/completed prefixes and the original wait UUID.
Legacy receipt identity remains wait:{run}/payment. The fixture owns every required service.

Observed setup failures are separate from behavior REDs: the initial test overlay had three
UUID serialization errors (35.32s), corrected with to_string without changing release dependencies.
An initially shared Cargo target then caused old/current same-version dependency collisions:
the old build appeared successful (47.88s), but the current build failed with163 errors (16.44s).
That old executable/manifest was invalidated, not accepted as released evidence. A fresh marked
release-only target built successfully in309.17s; source mtimes, not contents, were refreshed
before rebuilding the current tree. The earlier functional preflight passed separately
(161.52s outer,137.56s current build,20.84s command/18.66s coordinator). It predates final input
provenance hardening and is retained under its own filenames.

Bounded review found missing immutable root-wait coverage, unasserted stopped-owner takeover,
and incomplete overlay provenance; all were corrected. The next review found symlink-following
and unbound current pre/post-build provenance; lstat-based regular-tree validation and a full
tracked/untracked input snapshot plus HEAD comparison address these. The third review found
no remaining actionable issue within this contract; it ran no tests. Eight Python verifier
controls pass (0.082s), including changed production/registration, undeclared files, root/file/
directory symlinks and changed untracked inputs. No production behavior was changed in this
checkpoint, and setup/fixture failures are not reported as behavioral REDs.

The final runner passes exit0 in325.75s: old isolated cached build0.59s; clean no-debug current
build313.03s; upgrade command9.66s. The coordinator passes1/0/0 in8.42s, containing old producer
1/0/0 in3.86s and old reader1/0/0 in0.30s. These nested phases are not added into an aggregate.
Actual compiled parent33ab9da5 plus source digest
f2d70eacd5e7d3d69284072429c6e84d7938c313f03069ced2f23f3fbc7aa216 identifies the dirty fixture
input. Old executable SHA2568d88d23e68ba9a1dfd59cc88b8d54eb75c9a6ea1e9b3efaec5d0fe6af42da15f;
current aec73604a56fe0e49166f0c9534a38c17357c901fd4a832ea19adacb8b318c37.

This is actual released-source runtime compatibility, not a redistributed binary asset or
a live old-owner race. Old in-flight lease reports, new scoped-event bodies in old readers,
actual old-dispatcher stop/join/credential fencing, public activation, five-SDK parity,
general loops/nesting/capacity and final fault/resource/release gates remain open. None of
the eight canonical tasks closes here.

Strict integration-target Clippy passes108.51s after the complete profile. Format,
architecture24-crate, agent configuration and whitespace checks pass. This test-only increment
does not require rerunning the unchanged86-case production event gate as a new claim.


## Actual released worker reports its old in-flight lease — 2026-10-27

[The bounded in-flight contract](./released-v02-inflight-profile.md) now has a separate ignored
coordinator and released-source phase. The exact old generated gRPC1.5 client holds the original
scoped task capability/request in memory across normal schema upgrade and reports it twice to
current runtime-role services. Safely inherited anonymous Unix sockets carry capped length-prefixed
async frames, including a fresh current-provider JWT for the same existing subject/tenant. No
authentication/task/session tokens or report payloads enter witness files or diagnostics.

The old server explicitly grants a one-hour lease. Before migration and GO, the coordinator
matches old physical identity/path/attempt/generation/token commitment/wire expiry and requires
more than600s margin above the entire240s scenario. It asserts schema20→23, one native completed
step with the original reported token hash/fence and cleared live lease fields, exact retained
output, one canonical scoped/fingerprinted receipt and one logical-byte charge. First ACK has
already_applied=false, replay true with equal command/status fields. Old replay and immutable
completed/captured/journal prefixes survive the current write. The old worker exits before
explicit stopped-owner expiry/current Keeper takeover; current scheduler must perform work
and settle within32ticks before terminal native replay. The final run requires2working ticks.

The first runtime invocation fails0pass/1fail in5.99s (121.27s runner; old/current builds51.48s/
59.98s), while its nested actual old client passes2.41s. The fixture incorrectly compared
command_dedupe.resource_id to ACK command_id: persisted STEP_STATUS|RUN_STATUS|VERSION differs
from run:version. Correcting the exact scope/resource assertion needs no production change.
The corrected preflight passes1/0/0 in5.12s (61.14s runner, old phase1.99s), but its isolated
coordinatorSha256 field still names the active-data coordinator and is not final provenance
acceptance. All source hashes were separately retained; results are not combined.

Bounded design review requires explicit lease margin, framed async IPC/joins, replay flag
distinction, durable-only external-effect wording and compiled exact-overlay identity. The
implementation review additionally requires selected coordinator provenance, post-migration
version, positive bounded scheduler progress and durable capability/fingerprint checks. After
adding the self-source assertion, the still-unfixed runner genuinely fails wrong coordinator
hash before PostgreSQL (0pass/1fail,0.00s;82.91s runner including76.98s current build). Selecting
the scenario's case/coordinator/transcript together fixes this tooling provenance RED. The
third review finds no additional concrete defect; reviewers run no tests.

Final runner passes10.47s: cached old/current builds0.88s/0.49s, runtime command7.44s,
coordinator1/0/0 in6.25s with nested actual released client1/0/0 in2.67s. Final active-data
regression passes separately1/0/0 in15.50s (19.74s runner; nested old producer9.41s/reader0.26s).
Strict integration-target Clippy passes18.62s; eight Python source-verifier cases pass0.074s;
format/architecture24crates/agent checks pass. No workspace production code changed.

Exact outputs, failed/corrected/final metadata and normalized-transcript raw checksums are in
[evidence/released-v02-inflight-20261027](./evidence/released-v02-inflight-20261027/README.md).
Compiled parentf251c04e plus source digestd10ba3d21bc2eb0c54f7bdd9011cc260b7d740f4529e50c0f92cfdcfe2f0bedd
identifies the final dirty input; current executable7a1c2be022fe9e30d5f46e2ab7511ce00f7f786c76bcf4be444cdc35a78d2e44,
released executable83ea2ae6b5054bc793211dcf0d76a946d23fcc2b71ae8547e288995cd7081641.

This accepts only a genuine old scoped FORWARD report/replay to current services, with refreshed
authentication and a still-live original lease. It does not accept expired-task behavior,
Applied/reconciliation frontier, old-dispatcher stop/join/credential fencing, public scoped waits,
five-language asset parity, general loops/nesting/capacity or final fault/resource/release gates.
All eight canonical tasks stay open.


## Actual released Applied lookup and post-final reader — 2026-10-27

[released-v02-applied-profile.md](./released-v02-applied-profile.md) extends the source-verified
in-flight proof without changing production behavior. Actual old REST/gRPC/scheduler code
reports OUTCOME_UNKNOWN for charge, then leases the real qualified charge~r reconciliation.
Its original target instance/node/provider key are asserted; the lookup capability/request
stay only in old memory. Current services accept/replay the Applied wrapper across normal
schema20→23 migration with the original live lookup token/fence/expiry margin.

Native checks bind the original forward WAITING/RECONCILIATION/OUTCOME_UNKNOWN, cleared lease,
reported old token/fence/provider-key commitment and exactly one requested journal event.
After reporting both physical rows are explicitly SUCCEEDED with correct original capabilities
and cleared failure/wait/lease fields. Exactly one lookup StepCompleted has no payload; exactly
one total forward ReconciliationConcluded is Applied/Succeeded and carries the nested forward
output digest. There is no synthetic forward StepCompleted. The canonical lookup receipt binds
the full length-prefixed completion fingerprint and ACK/head/status; one total dedupe key and
the immediate pre-scheduler logical-byte delta prove one report transaction/forward-output charge.
Final aggregation may legitimately retain additional bytes afterwards.

Shared ProfileMode::{Forward,AppliedLookup} binds manifest, compiled coordinator, old child,
transcript and result. The old wire ACK explicitly reports RUNNING. After old reporter exit,
current Keeper takeover and2working scheduler ticks finish/replay. A second process of the
same verified old executable reads terminal REST/native state and recomputes the original
cutoff-bound immutable prefix after finalization. The old API is idle and its scheduler never
ticks after the handoff barrier; no mixed-dispatcher activation is inferred.

First weaker baseline passes1/0/0 in14.52s (126.52s runner, nested old reporter9.19s). A
review-driven compile is intentionally interrupted(-2,29.36s) before runtime when root spots
an accidentally contradictory fixture predicate; only the exact fixture condition is corrected.
It is not a pass or behavior RED. The stronger preflight passes1/0/0 in6.19s (84.68s runner,
old reporter2.57s). Three bounded reviews require explicit event/payload/fence/uncertainty/receipt/
accounting assertions, then exact wire status, total conclusions, typed mode and post-final
old-prefix timing. The final review finds no remaining substantive gap and runs no tests.

Final runner passes119.27s: old/current builds53.90s/55.25s, runtime7.63s; coordinator1/0/0
in6.37s contains old reporter1/0/0 in2.38s and terminal old reader1/0/0 in0.21s. Separate
FORWARD regression passes1/0/0 in6.55s (15.77s runner, with new post-final old reader); active-data
regression separately passes1/0/0 in6.75s (11.23s runner). Strict integration lint19.57s, final
Python verifier8cases0.098s and format/architecture24crates/agent checks pass. Nested phases
and separate invocations are not combined into one aggregate.

Exact outputs/identities and the interrupted/preflight/final distinction are retained in
[evidence/released-v02-applied-20261027](./evidence/released-v02-applied-20261027/README.md).
Compiled parenta9e6ee37 plus source digestdf8cd4a1aae0ecb95023a3415307d08cd1bf6301a5d85c8e91e864adfb6716ea
identifies the dirty input; current executable64281b36d2f764f23c256c53d6e268ef4cb17490489770068995b9d6bd27de30,
released executable385eef58c9e19e2ee3e64ea4a49b470ef57e216c1e9a737a7881cb2e9b5e3bc1.

This accepts a genuine released Applied report/replay and post-final old reading. Quota-refusal/
drain/original-forward compensation frontier, expired/stale reports, old-dispatcher credential
fencing/public activation, five SDKs, general loops/nesting/capacity and final resource/release
gates remain open. No canonical task is ticked and no external effect is claimed exactly once.


## Exact selected-test acceptance guard — 2026-10-27

A real negative control injects a nonexistent case into the runner's in-memory scenario mapping
without changing the verified Rust source/build. Before the fix, libtest returns exit0/running0
tests and the runner falsely prints evidence success. The control's required rejection assertion
fails in4.43s. This is a genuine tooling RED, not a platform test or a compatibility pass.
The fixed runner requires the exact selected case marker and final outer1passed/0failed/0ignored
summary before success; nested old phases cannot hide an empty/failed/multiple outer gate.
Unix-only fixtures now refuse unsupported hosts explicitly. The same actual negative control
passes2.82s after rejection. Five focused outcome controls plus eight source controls pass13/0
in0.081s. All actual scenarios pass separately again: active-data1/0/0 in6.47s (9.43s runner),
FORWARD1/0/0 in5.53s (8.31s runner), Applied1/0/0 in5.35s (8.63s runner). Rust inputs remain
unchanged from bd4f611d; no repeated Rust lint claim is needed for this Python-only fix.
[evidence/released-v02-selector-20261027](./evidence/released-v02-selector-20261027/README.md)
retains the fault injection, RED/green and all positive outputs. No canonical task closes.


## Public collection event waits and actual old-dispatcher cutover — 2026-10-27

The public body guard now permits bounded event waits, reusing native admission/start, receipt,
lookback, automatic deadline and stop/drain logic. Initial public RED:3failed at unsupported
publication. An intermediate guard still refused Node.timeout populated from timeoutMs; using
its existing validated event timeout corrected it. Final native public gate passes6/0/0 in16.20s
(67.66s command): delivery/order/receipts, activity lookback, worker-free deadline, cancellation,
original-effect compensation and control→event→control→activity with isolated local output.

Actual released dispatcher uses its own inherited runtime login to complete a root wait. The
coordinator proves login/current-connection access before NOLOGIN and session termination,
refuses existing/fresh old connections, acknowledges stop and joins the real old process before
current public publication. Both new waits deliver and old/new native replay pass. Preflight
correctly failed at the closed publication guard; final1/0/0 passes5.36s (31.13s runner), including
one nested old phase1/0/0 in1.39s, not a two-case aggregate. One bounded read-only review found
no substantive issue. This is an explicit operator sequence, not mixed-version safety.

Common collection-events first exposed non-deterministic concurrent journal ordering and a
matching retained event in the deadline fixture. Concurrency1 and distinct deadline keys correct
those fixture assumptions; native tests retain concurrency2. The next all-five observation was
identical and failed only for missing references. Root inspected canonical IR, complete journals,
two actual worker calls, scope outputs/peak1 and native projections before recording references.
The no-bless all-five gate passed1aggregate/0failed in28.12s. Rust must join after feature034
integration. No SDK/server assertions or normalized journal fields were removed.

Details, commands and preserved failures/results: scoped-event-publication.md and its linked
evidence. The eight canonical tasks remain open; this does not accept general loops, approvals,
children/nesting/capacity, the full resource/fault matrix or final integrated release gates.


Final public-event regression: native scoped_event_storage83/0/0 in296.59s, IR188/0/0,
contracts49/0/0 plus SDK registry2/0/1ignored, strict workspace/all-target Clippy exit0 in73.87s,
format/architecture24/agent checks and13Python profile verifiers pass. Initial Clippy rejected
duplicate phase-module loading and err().expect(); one shared module and let-else correct
those test-only issues without changing production behavior. All four actual released profiles
pass again separately on the lint-corrected source: cutover60.90s runner, active-data9.66s,
FORWARD8.43s, Applied8.41s. A missing CARGO_TARGET_DIR in one wrapper was refused before
build/runtime; its0.09s setup failure is not a runtime RED. Exact receipts and source bindings
are in evidence/public-scoped-events-20261027. No canonical or integrated release gate closes.


## Actual released Applied output quota and ordered compensation — 2026-10-27

The real v0.2 writer retains nine completed collection outputs and one reservation, then holds
a genuine Applied lookup report whose output would exceed the run's 4 MiB budget. Normal current
migration/report/replay refuses only that output, preserves known forward success and the exact
receipt, and persists CANCELLING/RUN_DATA_LIMIT before any recovery tick. Current runtime-role
workers compensate charge then reservation with their original provider keys, even after the
namespace admission limit becomes zero. Exact identity/digest/length checks preserve all 19 prior
output sources and the 4,030,018-byte ledger; no held slot or refused payload remains. The current
verifier and actual old terminal reader both accept COMPENSATED.

The final five actual-released profiles pass independently, including ordinary FORWARD/Applied,
active data and dispatcher cutover. Details, reproducible commands, timings, source hashes, review
corrections and original receipts are in [released-v02-output-quota.md](released-v02-output-quota.md).
A stale supplemental result-copy bug is corrected with provenance; new per-invocation result paths
and identity/hash checks prevent reuse. The Python verifier battery passes 16 tests. No production
workflow semantics changed in this increment. Ledger item 1 remains open for the wider authority
and uncertain-commit compositions; none of the eight canonical tasks is marked complete.


## Consolidated release dependency tree — 2026-10-27

The owner's v0.3 scope explicitly includes Redis Streams (033) and the official Rust SDK (034).
Their implementation branches are merged locally into this completion branch for combined
verification, preserving each source commit and historical evidence. The common SDK registry
now includes Rust alongside the five existing languages and the new 014 scenarios. This is an
approved release/test dependency, not a claim that prior isolated runs cover the combined tree.
Next checks must execute the combined Redis/native/contract and six-language scenarios. The
remaining Redis old-callback-writer cold-cutover proof reuses this branch's verified released
source harness; its event-dispatcher cutover is not callback-publisher proof. Canonical feature
014 and final release checkboxes stay open until their own acceptance passes.


## Combined Redis/Rust SDK checkpoint — 2026-10-27

The local 014/033/034 merge is verified with 139 real messaging/Redis tests, 109 native Rust SDK
tests, all five new common scenarios in six languages (unchanged goldens), strict workspace/all-
target Clippy, Studio API/type/lint and contract/static gates. Six actual released-source profiles
pass on one frozen current source manifest, including new callback-publisher stop/join/credential
fencing before explicit Redis activation. The active-data/report schema expectations now include
the exact Redis migration; original integration failures remain preserved. Commands, timings,
source identities and limitations are in ../033-redis-streams/released-callback-cutover.md.
This adds release integration evidence without enabling general nesting, capacity groups or
closing the eight canonical 014 tasks. Full 65-scenario/release-tree CI remains separate.


## B1d.1 bounded read-only capacity discovery — 2026-10-27

Branch feature/014-capacity-discovery is cut from develop and reuses the verified v0.3 integration
baseline, leaving PR56's pending CI head unchanged. The first new capacity runtime component is a
read-only bounded scan of ACTIVE ChildRoot memberships. It preserves tenant/namespace/current-head
identity, advances beyond filtered pages and returns no execution authority. A partial covering
index excludes retained FINISHED membership history from its intended access path. There is no
production caller or group mutation. Exact semantics and limitations: capacity-discovery.md.

Two native REDs preceded implementation; a valid zero-revision chain exposed and corrected a
decoder assumption. Nine discovery tests, the complete43-case capacity regression and42 SQL
privilege checks pass. An overbroad migration-test setup was isolated to its actual scoped schema
without changing its lock/commit/refusal assertions. Strict workspace Clippy and final focused
Clippy, contracts/static gates and all six source-bound actual-released profiles pass. The full
SDK aggregate in the contract-only gate is ignored explicitly. Original failures and all positive
receipts are linked from capacity-discovery.md. Canonical tasks remain open; this closes only the
bounded discovery component, not B1d, group quota/creation/transfer or nested execution.


## Build-dependency security gate — 2026-10-28

After the Redis fixture correction, CI for PR56 (run40686861523,
job122811344560) refused `pulldown-cmark`0.13.4 with
[RUSTSEC-2026-0414](https://github.com/RustSec/advisory-db/blob/main/crates/pulldown-cmark/RUSTSEC-2026-0414.md).
The local `cargo audit --deny warnings` reproduced that exact refusal against
advisory database commit40f5be6fb12ed93fc591bf4ab23b7d793b3f2239.

`cargo update -p pulldown-cmark --precise 0.13.5` changes only that package's
version and checksum in the workspace lockfile. This is the same version and
checksum already present in the merged official Rust SDK lockfile; no package,
feature, manifest pin or advisory ignore is added. It is a transitive build-only
workspace dependency via `api-grpc` → `tonic-prost-build` → `prost-build`.
Pinned `prost-build`0.14.4 parses repository-controlled proto comments with
`Options::all() - Options::ENABLE_SMART_PUNCTUATION`. It is not a runtime Markdown
endpoint.
The [upstream release](https://github.com/pulldown-cmark/pulldown-cmark/releases/tag/v0.13.5)
is the security patch for this existing dependency line.

Both workspace and independent SDK `cargo audit --deny warnings` checks now pass
under the existing audit policy. The gRPC crate tests pass26/26, including
compatibility and scoped-wire checks. Generated worker Rust source before/after is
byte-identical (SHA2563a0fb2b09b974b4c96117f05aa38f89b97fdb84b97d14998c70d7247b56f67ec);
no protocol baseline was blessed or changed. Receipts are preserved in
[evidence/pulldown-audit-20261028](evidence/pulldown-audit-20261028/README.md).
Strict workspace/all-target Clippy also passes (169.35s); formatting, architecture
and agent configuration pass. The new CI run must pass before merge. This build fix closes
neither general nesting nor the remaining canonical release tasks.
Refs: T014-043, T033-007.
