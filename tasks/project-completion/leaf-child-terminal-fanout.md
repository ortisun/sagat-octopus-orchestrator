# B1b.2c.1d — Ordinary child terminal fanout preserves its leaf source

Status: bounded decomposition accepted after fresh Astra/xhigh C2 amendment review (None); execution pending; **C1 is actionable and remains an unresolved prerequisite**. The former claim that adding End.outputExpr makes this collection child reach finish_with_result was incorrect. This amendment supersedes artifact SHA `01dd0d27749a4c703223e8953906832886e7ea8c854680f3f9af38139b0bb36f`. Root is the sole repository writer and executor. Implement the fanout unit only after accepted .1b/.1c and the separately reviewed and verified .1d.0 End-result correction below, retaining accepted .1a at `8df0ae90`. This proposal writes only `/tmp/sagat-child-terminal-fanout-decision.md` and supplies no test result.

Read snapshot: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`, branch `feature/016-project-completion`, HEAD `2d86483f03efa683e13df8d78bb1338f5170602f`; worktree clean at the initial and source-hash snapshots. At sealing, root's concurrent .1b work had modified `tests/integration/workflow-primitives/leaf_certification_child.rs` and added its parent-flags module/evidence directory; this design neither wrote nor reviewed those in-progress changes. `.specify/feature.json` is absent; assigned active feature is 014. Repository AGENTS, constitution, complete 014 spec/plan/tasks and `tasks/project-completion/leaf-child-source-preservation.md` were read. Existing branches/worktrees were inventoried. Additional source reads below are targeted, not exhaustive.

Root checkpoint: .1b is accepted at2cdd5aaf; .1c and .1d.0 remain pending. Root confirmed the omitted End fields/result digest. C2 accepts allocation and honest prerequisite boundaries, not the unresolved result behavior or fanout evidence. Reviewed amendment:c28deb0574a377f9fd4d88815b42cd0fe552fcd170df52023981bd779f8a2f8a. Optional external review was declined by the user.

## C1 — required B1b.2c.1d.0 End-result prerequisite

**Decision classification: actionable contract defect, unresolved; not a fixture-only omission.** Proposed local identifier `.1d.0` is subject to root's concurrent allocation check. The fanout success case must not silently switch to no-result success and close the inventory. No current public, genuinely produced result-bearing fixture that also satisfies OrdinaryLeafV1 was found in the bounded current-writer investigation.

### Accepted contract versus actual current path

- The canonical public `end.outputExpr` contract says its JSON value is the run result, kept as the End step output and handed to a parent as `steps.<child>.output`; evaluation failure is EXPRESSION_ERROR and a value above 256 KiB is RUN_DATA_LIMIT (`specs/001-platform-foundation/contracts/workflow.schema.json:684-686`). Owner decision G21/D36 already requires terminal-transaction evaluation, content charging and parent propagation (`docs/owner-decisions.md:14`; `specs/002-durable-execution/research.md:44`). This needs no new product choice about what a run returns. The schema's historical disclosure sentence must be read alongside current ADR-0033/016, not used to bypass their present classified-result policy.
- Public collection conversion permits the final End and its parsed outputExpr (`infra-ir/src/convert.rs:484-486`, `719-740`). The existing public collection fixture itself declares `outputExpr: steps.lines.output` (`tests/integration/workflow-primitives/collections_http.rs:35-59`); its declaration/publication is not evidence that a run result was retained.
- Current collection finalization does not evaluate that End. Scheduler code computes the collection's mapped aggregate and constructs CollectionFinish (`scheduler/src/passes.rs:1529-1591`); the application record carries aggregate content/digest and source digests, with no End evaluation result (`application/src/primitives.rs:538-547`, `624-681`). The PG adapter matches `End { .. }` without its fields (`infra-postgres/src/primitives.rs:2726-2740`), inserts a SUCCEEDED End step without output (`:2795-2816`), appends empty-detail `end.reached` (`:2908-2926`) through a helper that omits payload_digest (`:401-432`), then updates SUCCEEDED/run_version/finished_at without runs.result_digest (`:468-490`, `:2948-2955`). Retaining the collection aggregate in iteration_scopes is not retaining the workflow result.
- The actual downstream read joins payloads exclusively through `runs.result_digest` (`maintenance.rs:492-506`). With no result digest, succeeded-child fanout takes `Ok(None) -> finish_with`, not finish_with_result (`scheduler/src/passes.rs:4893-4908`). Adding outputExpr to the earlier proposed fixture does not change this fact.

### Why there is no compatible fixture workaround

The current shape predicate permits ordinary root activities **only before** collections, then a root collection chain, then the final End; it rejects post-collection activities, conditions/other controllers, additional root branches and continuation (`domain/src/workflow/collection_shape.rs:53-85`). The public collection conversion enforces that same boundary (`convert.rs:719-740`). Every successful terminal path of that shape therefore reaches End through the collection finalizer above, including zero-item or multiple-collection variants and End expressions selecting input or prefix output. Changing the expression cannot route around the missing End evaluation.

New child creation does not seed a result digest (`infra-postgres/src/execution.rs:990-1017`). Source search of current PostgreSQL `result_digest =` writers found the normal run-result update in `execution.rs:4115-4125`, plus collection/loop scope result fields which are different records. That normal writer derives its result from evaluated EndReached output; the full Engine's Evaluation::Output path calls the existing evaluator and applies output limits (`application/src/execution.rs:1535-1570`). The genuine result-bearing child fixtures in `tests/contract/durable-execution/grpc/run_result.rs:11-24`, `146-175` use a linear activity graph, so they do not satisfy this selected collection-source shape. A post-collection activity to reach that engine path would change the accepted public/source shape and cannot be this unit's workaround.

This is a source-backed reachability conclusion for the current public producers, not a test execution or a statement about arbitrary privileged writes. Never seed result_digest, step output or event payloads; never rewrite the child's definition/status/ancestry; never fake run_result or call finish_with_result directly in a positive fixture. Existing no-result success characterization remains valid only for its actual scope.

### Smallest separately reviewable defect boundary

Open `.1d.0` as **restore the already contracted End result at the existing public root collection-chain terminal boundary**. First retain a real RED regression. Separately decide the minimal correction and prove it before the .1d fanout matrix runs. Do not choose or implement the internal fix inside this fanout decision. In particular, this note does not choose between using existing Engine result machinery and extending a collection transition; those alternatives have different transaction/context implications that require the bounded prerequisite decision.

The required first regression, tentatively `collection_child_end_output_reaches_its_real_parent`, must use public publication/start and real child creation, selected lines/0 completion, actual later worker completion and real collection closure. Declare final `outputExpr: steps.later.output` and use a small known actual later aggregate. Before fanout, prove that the aggregate/checkpoint exists and that the selected lines source remains closed; then assert all of the following from real persisted state:

1. The child is terminal with **Some(exact expected JSON)** as its real run result, not merely SUCCEEDED. End step output, EndReached payload_digest and runs.result_digest name the same actual retained bytes, correctly scoped to tenant/namespace/Run.
2. Real scheduler fanout retains those same bytes/digest as the parent's child step output and its single child.finished payload. `verify_run` agrees for child and parent; selected source identity/events remain untouched.
3. Public result reads, if used, go through existing classification; a test's owner read for storage verification is not public disclosure evidence.

On current source the expected failure is missing child result/end payload, with `run_result=None` and no result-bearing parent fanout. **This is expected RED from source inspection; no RED command has been executed by this design task.** Root must record the actual failing assertion/output before a correction. Do not change the assertion to expect None, mask the missing digest with COALESCE to a collection aggregate, or accept a direct port-seeded positive result. The normal no-outputExpr path is a compatibility control, not a substitute for the required result case.

The separately accepted defect decision must resolve this bounded examination checklist before choosing code changes:

| Concern | Existing authority/contracts to examine; acceptance boundary |
| --- | --- |
| Public/internal contract | G21/D36 and schema above; existing application Evaluator::run_result (`execution.rs:401-410`) and CollectionFinish/CollectionTransition. Preserve current public syntax, ordinary shape and source certificate profile. Pin and qualify the actual End node/version; choose no new public API or fixture-only production hook. |
| Evaluator and expression failure | Existing JsonEvaluator::run_result (`infra-ir/src/evaluator.rs:201-210`) and Engine::Evaluation::Output (`application/src/execution.rs:1535-1570`). Determine how the End sees the **just-finished** collection aggregate together with correct immutable input/preceding outputs; preserve expression limits, deterministic JSON and EXPRESSION_ERROR. A payload copy alone cannot implement arbitrary existing outputExpr. Add actual failing-expression evidence; do not mark End/run SUCCEEDED before evaluation has succeeded. |
| Transaction/source authority | Existing collection own chain/run/fence, expected revision and finish replay checks, journal sequence and source proofs. Decide how collection finish, evaluated End output, End/Run terminal events, step projection, result pointer, accounting, audit and outbox remain an atomic documented transition. Preserve exact selected-source facts/canonical certificate through success/failure, stale revision, duplicate and rollback. No parent mutation lock is introduced into the child transaction. |
| Storage and quotas | Existing payload retention/digest validation, step_outputs and run_events.payload_digest, runs.result_digest, same-tenant FK, per-payload 256 KiB, run logical/retained 4 MiB and tenant content accounting. Examine `execution.rs:3044-3100`, `3864-3898`, `4115-4125` and current collection retention/accounting. Specify failures for oversized result, exhausted child retained/content quota and rollback without double charge; parent retained-room/fallback remains its existing later transaction (`passes.rs:4793-4842`). No bypass using deduplicated bytes or aggregate retention. |
| Classification/security | Current ADR-0033 and feature016 result/step-output selection; `tests/integration/payload-classification/result.rs:11` proves the result reader uses runs.result_digest and workflow output schema. Preserve workflow output classification, parent child-node output classification, unmarked/undeclared pii and never-secret disclosure, tenant boundaries and pinned schema selection. Examine existing schema-validation behavior instead of assuming that general result evaluation validates outputSchema. No new grant, new endpoint or wider disclosure. |
| Telemetry/audit/errors | Existing EndReached/RunFinished journal projection, collection mark_telemetry_pending (`primitives.rs:2954`) and finite scheduler outcomes, source certifier audit, current typed result/size/content errors. Result bytes stay out of logs/spans/outbox/audit; journal references remain digests. Identify the appropriate existing success/failure observability assertions; do not invent a fanout audit or add high-cardinality metric labels. |
| Existing migration and compatibility | `20261110000000_run_result.up.sql` already has the nullable 32-byte digest and exact tenant/payload FK; normal Engine retains results under existing runtime privileges. First examine whether these and existing output/journal storage suffice; no new migration/grant is assumed or authorized here. Retained old collection runs may already be terminal without a result: do not rewrite terminal history or backfill fabricated output. Old pinned outputExpr versions reaching End after the correction may newly evaluate/fail, as the existing G21 compatibility note already recognizes. The separate decision must document supported active-run behavior, retained-history handling and compatible rollback; classify any newly discovered migration/ADR need before proceeding. |

Minimum subsequent proof is focused success/no-expression/failing-expression/size-or-accounting refusal plus exact atomic rollback/replay for whichever transaction changes, and the directly affected current classification/result regressions. The separate correction plan must choose the concrete limits and fault gates from its actual change; this document does not declare those gates passed or reopen the whole engine protocol. No new standalone generalized End framework, source profile or declaration shape is authorized.

### Bounded prerequisite increments and gate

- `.1d.0` decision freeze: new defect decision document, 014 spec, 014 plan, project todo, main leaf-child-source-preservation inventory (five files). Name the current collection-specific G21 gap; do not erase earlier valid linear result evidence.
- RED characterization: existing child fixture registration/helper file, one new focused child End-result regression module, defect document, one evidence file, todo (five). This can reuse genuine public creation but must not touch the root's frozen .1b execution files while that gate runs; root serializes ownership.
- Separately reviewed correction: the defect decision first selects the smallest affected internal files and splits work into increments of at most five (at most three production files plus regression/evidence is one possible bound, not a preselected implementation). Each additional contract/negative/fault/classification increment also remains at most five. No partially verified correction closes .1d.0 or starts .1d acceptance.
- Acceptance: defect document, focused evidence manifest, relevant existing G21 documentation, project todo, 014 validation report (five). Root reruns the genuine public regression plus the selected necessary gates on the integrated tree and records failures/outputs; only that evidence can close the prerequisite.

**Dependency remains `.1a + accepted .1b + accepted .1c + accepted .1d.0 -> .1d fanout matrix`.** The six fanout fixture executions below are retained as a pending design, with genuine result-bearing success conditional on that correction; prerequisite regression counts are separate. C1 is not resolved by this amendment. No product/API/profile/schema/grant/capacity/ADR change or production implementation is performed or authorized by this design artifact.

## Decision

After the required .1d.0 correction has been reviewed and verified, prove one ordinary unchained child whose `lines/0` source is genuinely closed while `later/0/work` is leased and unfinished. Separately establish actual child termination, then race the real parent fanout transaction with the native certifier. Use three bounded scenarios, each in both observed orders: six fresh public-runtime fixtures, three named tests. Do not multiply every outcome by both source modes.

1. Actual child SUCCEEDED: first certification versus fanout in both orders.
2. Actual child FAILED from an ordinary later activity: first certification versus fanout in both orders.
3. REQUEST_CANCEL parent still awaiting its child: genuine own-child cancellation/drain reaches CANCELLED; original canonical replay versus eventual fanout in both orders.

All six end with cold original-identity replay and exact parent-once assertions. The third scenario explicitly owns the eventual REQUEST_CANCEL settlement obligation in the main inventory: .1b's flag races stop before that settlement, and .1c's PROPAGATE cancellation/compensation cannot close it. The third scenario uses an ordinary effect-free later body; real later effect/compensation remains .1c's responsibility.

The native source-certification path is already reachable, but C1 blocks the required result-bearing collection-child success proof until .1d.0 is accepted. The rest of this document defines pending preservation evidence. `PgLeafCollectionSuccessStore` has no parent/chain refusal. `PgScopedLeafFinishStore` still refuses all parent/chain contexts before canonical reads or replay. No profile, schema, API, role/grant, production caller, capacity authority, public publication capability or ADR change is proposed. Reuse only the existing disposable source INSERT fixture grant from .1a; introduce no additional privilege. No scoped capacity mutation is involved.

## Exact transaction authorities and source references

| Path | Current implementation and authority | What it establishes |
| --- | --- | --- |
| Discover ended ordinary children | `crates/infra-postgres/src/maintenance.rs:509-566`: plain SELECT, parent-shard partition, exact stored parent association, `parent_settled=false`, `finished_at IS NOT NULL`. | Candidate discovery only; no child Run or parent mutation lock and no source authority. |
| Map terminal outcome | `crates/scheduler/src/passes.rs:4846-4928`: SUCCEEDED maps to success, FAILED to CHILD_FAILED, CANCELLED to CHILD_CANCELLED. Success reads the already committed child result via `maintenance.rs:492-506`; other terminal statuses have their existing mappings. | Read-only terminal outcome/result selection. Do not call this a locked child-source proof. |
| Apply to parent | `passes.rs:4766-4842`, `3014-3074`, `3534-3561`: `finish_with[_result]` calls `apply[_if]` addressed to the **parent**. `RunStore::begin(..., Lock::Wait)` acquires applicable parent chain then the parent row (`execution.rs:1130-1143`, `169-265`); `view.holds` checks the **parent** scheduler authority. | Parent-owned reducer/transaction. `finish_with_result` checks parent retained room and retains the output in the parent; parent quota failure uses its existing RUN_DATA_LIMIT fallback. It does not confer child scheduler authority. |
| Deduplicate the logical notification | `domain/src/execution/command.rs:1995-2019` and `state.rs:3775-3796`: `ChildFinished` requires this exact parent step to be WAITING on this exact child. | A stale rediscovery fails the current parent transition after acquiring the parent lock; it cannot add a second parent `child.finished`. This is not a child-version CAS. |
| Persist parent notification and child flag | `execution.rs:3919-3990`, `4028-4029`, `4998-5052`: the parent appends `child.finished`; `settle_child(child,false,true)` first tries its exact matching chain link. With no link, it executes `UPDATE runs SET parent_settled=true, parent_cancel=(parent_cancel OR ($4 AND finished_at IS NULL)) WHERE tenant_id=$1 AND id=$2 AND parent_run_id=$3`. Here `$4=false`. | The genuine competing child-row writer is this **parent-owned UPDATE**. It takes a child row lock compatible with PostgreSQL's actual UPDATE behavior but conflicting with the certifier's FOR UPDATE. It changes only notification flags; no child run_version, child execution status, child journal or child own-scheduler check. `parent_cancel` is preserved. |
| Commit parent effects | `application/src/execution.rs:1357-1403`, `execution.rs:4110-4157`, `5189-5207`, scheduler `passes.rs:3073`: parent reducer, outputs, journal, step projection, version/status, child flag and parent run.finished outbox commit together. | One parent settlement with its durable notification. Counts concern this database transition, not exactly-once external delivery. Do not invent a separate fanout audit action; source certification has its own actual audit, and REQUEST_CANCEL retains its command audit. |
| Certify/replay child source | `leaf_certification.rs:52-135`, `199-217`, `264-307`: applicable **child** membership chain, exact tenant/namespace/original-child Run FOR UPDATE, child's own current scheduler SHARE/post-wait check, canonical lookup, then creation CAS on a miss, then source proof and fact/audit. | Current original-child authority always precedes replay. Existing record wins over stale expected creation revision, but never over a stale scheduler fence. First creation uses the actual child run_version. No parent/link/capacity lock. |
| Actual child terminalization | Worker report through `scope_flow.rs:323-393`; real `Scheduler::advance_collection[_from_run]` through `passes.rs:629-662`, `989-1047` and existing collection ports. | Later report/checkpoint/closure/drain changes the child's own journal/revision under worker/child scheduler authority. This is distinct from the later parent flag UPDATE. |

The current ordinary legacy lock edge is parent Run -> child flag UPDATE. Certification takes child Run and never parent. Do not add child -> parent locking, treat the fanout as a child RunStore begin, or import the chain-link protocol into this fixture. ADR-0030 §19 (`docs/adr/ADR-0030-structured-instances-and-continuation-chains.md:158-165`) explicitly retains this legacy protocol separately. No complete lock graph claim follows.

## Common fixture and terminal boundary

Reuse `OrdinaryChildSourceBoundary` and `ordinary_child_source_boundary` (`leaf_certification_child.rs:50-60`, `296-366`) after .1b/.1c's accepted helper changes. Preserve actual public parent/child publication, parent `/runs` start, deterministic production child creation, pinned definitions, exact namespace/tenant association, and absent `chain_segments`, `execution_chains` and `child_chain_links`. Source remains exactly depth-one `lines/0`; no child in a body, continuation or nested controller.

Keep the baseline source command `c0` captured **after** the genuine `later` lease. Its revision must not accidentally be the pre-lease revision. Keep both real Run IDs; the source identity and scheduler authority always address child C, while fanout and the REQUEST_CANCEL HTTP operation address parent P. Do not infer authority from `Flow.run` without asserting its child identity.

A local terminalization helper must stop before parent notification. For success/failure, use the real later gRPC report, then bounded calls to the existing exact `advance_collection`/`advance_collection_from_run` for the root owner `later` until the child really ends. Do not call `Flow::drive()` over that boundary: `Scheduler::tick` runs collection progress then children_finish in the same tick (`passes.rs:600-615`) and could consume the fanout being tested. Assert at the terminal boundary:

- Child's actual terminal status, non-null finished_at, one actual child run.finished, and later activity/collection terminal events that explain it.
- Child revision `vT > c0.expected_run_revision`; exact selected-source rows and five referenced source events remain unchanged.
- Parent remains awaiting this exact child, child `parent_settled=false`, parent child.finished count zero.
- Parent cancel flag is false for normal success/failure, true for REQUEST_CANCEL. Child ancestry and absence of all chain associations remain unchanged.

If the existing exact collection progress methods cannot establish a clean terminal-before-fanout boundary, report that fixture gap. Do not seed finished_at/status/flags, fabricate `ChildFinished`, add a production pass entry point, or manually emulate the scheduler transaction. A fixture-only parent-transaction barrier around an actual scheduler tick is a fallback only after the root reviews the exact observed wait; it is not needed by this design's direct existing progress methods.

The pending success scenario must exercise `finish_with_result` using the exact public collection-child End-result path first proved by .1d.0: final `outputExpr: steps.later.output`, real later completion, and observed retained child result before fanout. At the present snapshot this fixture instead has no run result and reaches finish_with; that is the actionable C1 defect, not a positive result case. Preserve the result-byte/digest and parent `kid` output/journal assertions after the correction; never silently weaken them to no-result success. Ordinary later failure exercises finish_with and retains its actual LATER_DECLINED event/code. The accepted baseline characterizes only no-result success. Parent result-quota rejection stays outside this fanout matrix; the prerequisite separately examines the result/accounting correction's necessary limit/error gates.

## Six concrete fixture executions

| Named test (proposed) | Policy and real production path | Native operation raced with fanout | Expected parent outcome |
| --- | --- | --- | --- |
| `native_child_success_fanout_preserves_creation_in_both_orders` | PROPAGATE unchanged; complete later gRPC activity, actual checkpoint/collection closure/end; child SUCCEEDED with small result. | No prior certificate. After terminalization, c0 with fresh own authority must return StaleRunRevision{current:vT} and write no fact/audit. Race a newly discovered cT with exact revision vT. | `kid` SUCCEEDED/routed, result bytes/digest retained exactly once, parent SUCCEEDED. |
| `native_child_later_failure_fanout_preserves_creation_in_both_orders` | PROPAGATE unchanged; fail later activity through gRPC with nonretryable LATER_DECLINED, real drain/finalization; child FAILED. | Same missing-record stale-c0 refusal then fresh cT creation race. | `kid` FAILED with CHILD_FAILED; parent FAILED. Do not substitute the child's original LATER_DECLINED for the mapped parent failure code. |
| `request_cancel_eventual_child_settlement_preserves_original_replay_in_both_orders` | REQUEST_CANCEL published on parent; first certify c0 while later leased; real parent HTTP CANCEL; real child own cancellation/drain to CANCELLED. | Original qualified c0 identity and stale original expected revision, with renewed/discovered current **child** scheduler authority. Canonical replay must return the complete original record/audit. | Parent stays CANCELLING until fanout; then `kid` FAILED with CHILD_CANCELLED and no longer waiting/cancel_requested; parent CANCELLED, with one child.finished. |

Each test loops `SourceFirst` and `FanoutFirst` over fresh fixtures. This explicitly proves both first-certification flag orders for normal success/failure and both canonical-replay flag orders for REQUEST_CANCEL. Every scenario also cold-replays after fanout. It does not claim the larger eight-or-twelve-way Cartesian product, nor need one to identify these two distinct adapter branches.

The old-c0 refusal in the first two tests is caused by **genuine own-child terminalization**. The later parent flag UPDATE must leave `vT` unchanged: cT succeeds in either fanout order. It would be an incorrect test to expect StaleRunRevision merely because parent_settled changed.

For REQUEST_CANCEL, take the native original record before the real parent CANCEL. The HTTP202 must leave P CANCELLING, its `kid` WAITING/CHILD with cancel_requested=true, C unfinished, parent_cancel=true, parent_settled=false. The initial cancel transaction alone does not settle C. Run the existing scheduler tick which reaches children_cancel after children_finish, assert the genuine child own stop_reason=CANCELLED and its own journal/revision changed. Drive only the later collection through its existing drain methods; observe the actual leased later member's cancel request before reporting a known, nonambiguous stop through the real worker (`try_fail(...,"WORKER_STOPPED",false)`). `command.rs:1012-1038` records StepCancelled when the held attempt was asked to stop. Continue actual collection drain to C CANCELLED. There must be no selected-source obligation and no invented effect/compensation. P must still be CANCELLING and waiting on C, with zero child.finished, when the fanout race begins. Do not claim that the existing ready-child REQUEST_CANCEL test (`durable-execution/children.rs:398-422`) proves this leased scoped-child case.

## Observable barriers, with no assumed FIFO

Use separate real pools/connections and bounded deadline polling of `pg_stat_activity` plus `pg_blocking_pids`. Check `current_user=sagat_app` for native and production scheduler actions. Owner connections may install/remove narrowly qualified fixture triggers and hold locks in the disposable database. Locally poll the actual futures to make progress; no sleeps, elapsed-time ordering, unqualified waiter counts, or assumptions from the order futures were created. Record blocker/waiter backend PIDs and query class in assertion failures.

### SourceFirst: first certification (success/failure)

1. Install the existing deferred first-source COMMIT probe (`leaf_certification_commit_loss.rs:11-27`) for the sole fixture certificate; hold its advisory gate.
2. Start native certification cT on a uniquely named one-connection runtime pool. Observe its actual query `COMMIT`, blocked by the known gate PID. It has completed source proof and uncommitted fact/audit insertion. Check the exact child Run and own scheduler guards with the existing NOWAIT probes (`:37-61`). External readers still see zero fact/audit.
3. Start an actual scheduler tick F1. Observe F1 blocked on the **production `UPDATE runs SET parent_settled=true,...`**, with the certifier PID as blocker. This is the child-row competitor. It is not `SELECT ... FROM runs FOR UPDATE` for C; that query would identify the wrong phase/writer. The parent row is already held by F1 and its parent authority has already passed. An exact parent FOR UPDATE NOWAIT probe must fail, corroborating that target.
4. Start a second actual tick F2 while F1 is held. Observe F2 waiting on the parent Run begin query behind F1. Because first fanout remains uncommitted, both discovered the genuine ended, unsettled child. This is a forced stale candidate, not merely a later empty discovery pass.
5. Release the source COMMIT gate. Await native first creation, F1 and F2. Assert successful native creation and exactly one total children_finished, zero failed actions, then the durable once oracles below. No assertion relies on which future gets polled first after gate release.

### SourceFirst: canonical replay (REQUEST_CANCEL)

A replay writes no new certificate row; the INSERT COMMIT trigger cannot pause it. Do not pretend that it can.

1. Before starting replay, hold `LOCK TABLE leaf_collection_success_certifications IN ACCESS EXCLUSIVE MODE` on an owner transaction in this one disposable fixture database. Name the native replay connection uniquely. Start native replay with original c0 expected revision and current own-child authority.
2. Observe that exact backend blocked on the real canonical SELECT from `read_canonical_leaf_success` (`leaf_certification.rs:273`), with the table-gate PID as blocker. Its child Run and own scheduler guards have already been acquired; confirm them. This is a canonical-read barrier, **not a replay COMMIT barrier**.
3. Start F1; observe its exact legacy child-flag UPDATE waiting on the replay backend. The fanout path does not access the certificate table. Preserve expected relationships and child execution from independent reads; do not try to read the locked certificate table from a verification query while the gate is held. Do not add a supposed stale-fanout F2 witness here: the REQUEST_CANCEL parent is CANCELLING, and a new tick first visits stopped_runs.
4. Release the table gate, await actual replay and F1, then run a fresh quiet tick and assert the complete original record and exactly one parent settlement. This observes replay's child guard before fanout can update, with that guard retained by the native transaction until completion (`leaf_certification.rs:200-215`). It supplies no separately observed read-only COMMIT-pausing claim or new proxy/process test.

This table gate avoids blocking the child scheduler shard exclusively: parent and child can naturally share a shard, and both need compatible scheduler SHARE checks. No random distinct-shard assumption is allowed.

### FanoutFirst: all three scenarios

1. Install a fixture-only deferred constraint trigger on the exact parent's `run_events` INSERT where event_type=`child.finished`, node=`kid`, tenant/run match P. It runs only for that real production event and checks, inside the committing transaction, the event's exact child identity/outcome, C's real terminal status, parent_settled=true, preserved parent_cancel, and the expected parent step/status. Then it waits on a held advisory gate. Use fixed fixture UUIDs/known values in safely constructed DDL, not product payload input.
2. Start actual scheduler tick F1. Observe its backend in actual `COMMIT`, blocked by that gate PID. This means the real parent reducer and exact child flag UPDATE have already run. External readers still see the pre-fanout parent and parent_settled=false. Independently probe C with `FOR UPDATE NOWAIT` and P with `FOR UPDATE NOWAIT` to confirm both conflicts. Do **not** use a child KEY SHARE probe to assert a non-key flag UPDATE conflicts: that is not the lock property being tested.
3. Start native cT creation or original c0 replay. Observe its exact qualified `SELECT workflow_version_id,run_version,trace_id FROM runs ... FOR UPDATE` waiting on F1's PID. In the normal success/failure cases, start F2 and observe its parent Run begin wait on F1; confirm this fixture has no due parent work before children_finish, so that wait follows its real ended-child discovery. These are two observed waits on two actual target rows held by the same parent transaction; there is no FIFO claim between them. In REQUEST_CANCEL, omit concurrent F2 and use subsequent quiet rediscovery: a new tick could block in stopped_runs before discovering the ended child.
4. Release the parent COMMIT gate. Await all actual futures. F1 commits its one parent transition; native certifies/replays with the expected immutable source. In normal cases F2 restores the now-settled parent and skips the stale child-finish transition. Assert one total children_finished and no failed action, then a quiet rediscovery tick in every scenario. Child vT remains exactly unchanged across this fanout transaction.

This gate may reuse .1b's accepted narrowly qualified barrier infrastructure if available. It must be qualified to the terminal notification event and actual parent COMMIT, not a parent CANCEL trigger. An actual gate failure is a test failure to diagnose, never a reason to seed final state or silently fall back to unobserved ordering.

## Oracles and exact-once meaning

Capture three distinct observations before and after actual terminalization and before and after the fanout race:

1. **Immutable selected source:** existing selected-source snapshot plus full rows of the five exact event references; actual definition version/pin; scope/body/Return identity and payload digests; selected owner/local routed terminal rows; zero remaining reservation and absent selected saga/entries/derived obligations. Compare complete canonical record fields, including certified revision/time/audit ID, when it already exists. Check the exact one source fact and truthful audit.
2. **Child execution:** separate child run fields excluding only legitimate notification flags, its exact journal, all child steps, later scope/drain state, and result digest. These must visibly change for genuine terminalization; after the terminal boundary they must remain unchanged across fanout and certification. `ordinary_child_source_snapshot.run_and_journal` currently includes all Run columns (`child.rs:671-680`), so whole-field equality across parent_settled change is wrong; add a local comparison excluding parent_settled/parent_cancel only or compare the relevant typed fields and journal separately. Keep ancestry, status, finished_at, run_version and result under equality. Do not exclude broad unspecified mutable fields.
3. **Parent relationship/execution:** exact P/C association, old/new flags, parent status/revision/stop reason, complete kid step including waiting/cancel_requested/child ID, full parent journal, result payload/digest where present, run.finished outbox and command audit. These must show the actual settlement, not merely equality of the source snapshot.

For each fixture assert:

- Exactly one P `child.finished` with exact root node/instance, child ID and mapped outcome, exactly one P `run.finished`, and exactly one corresponding P run.finished outbox. Parent/certifier transitions must not append to C's terminal journal.
- Exactly one parent notification flag transition from false to true; parent_cancel is unchanged (false for normal cases, true for REQUEST_CANCEL). Use the before/after snapshots plus the forced duplicate's refusal. Do not create a new mutable database counter or pretend unchanged duplicate assignments are a business transition.
- In normal success/failure cases, across F1/F2 total `children_finished==1` and both reports `failed==0`. The stale duplicate may be reported skipped. In REQUEST_CANCEL, F1 alone reports children_finished=1 and failed=0. Every case's fresh subsequent quiet tick has children_finished=0 and creates no new parent events/output/flag changes. Counts are exact scoped IDs, not global database counts.
- Normal success/failure cases prove the existing parent state machine prevents duplicate transition even when both discovery calls saw the child before the first commit. REQUEST_CANCEL proves its distinct eventual settlement and quiet rediscovery, not a forced stale-candidate duplicate under that policy. `maintenance.rs:397-421` selects its CANCELLING parent for stopped_runs, and `passes.rs:611` executes that before children_finish at :614; an observed parent Run wait alone must not be mislabeled as its child-finish transaction. No new pass-level scheduler hook is justified for duplicating the already tested common finish_child guard.
- `verify_run` succeeds for C and P after both futures settle; selected-source and canonical-count checks also pass.
- Close the native pool/cache/store, revoke the existing disposable source INSERT grant, create a fresh runtime pool/cache/store, renew/discover **child** authority, and replay the original qualified command (old expected revision retained). Assert complete original record/audit, counts (1,1), and no new source, child execution or parent changes. This is cold reconstruction, not an OS restart.

A stale-fence-before-replay control already belongs to accepted .1a and must remain in regression. One can reuse its explicit helper while asserting own-child authority in this unit; no parent fence substitute is accepted. Parent notification flags and terminality never authorize a capacity transition or establish selected settlement by themselves.

## Fixture helpers and increments (five files maximum)

Keep helpers test-only and local to `leaf_certification_child.rs` plus one nested module `leaf_certification_child_terminal.rs`. Reuse .1b's accepted policy parameter, exact waiter observation and identity helpers rather than refactoring shared `scope_flow`. Necessary additions are:

- A small fixture option for the successful child's existing end outputExpr; public creation remains genuine. Keep names within the existing public Slug bound.
- Exact later terminalization without a full tick swallowing fanout; REQUEST_CANCEL setup using parent HTTP CANCEL and real own-child drain/report.
- Qualified parent-event deferred COMMIT gate, a replay canonical-read table gate, and target/backend witnesses for fanout UPDATE and duplicate parent begin. Reuse native source COMMIT helper.
- Separate child execution, selected-source event and parent-once snapshots/assertions. Generalize the baseline success-only parent assertion locally where appropriate.

Proposed serialized increments:

1. **Freeze decision after recording prerequisite:** project decision document, `leaf-child-source-preservation.md`, project todo, 014 spec/plan pointers if needed (at most five). Record that REQUEST_CANCEL eventual outcome is allocated here. No implementation or checkmark follows from design review.
2. **Success and later failure, only after .1d.0 acceptance:** child fixture module, new terminal submodule, decision document, one evidence file, project todo (five). Write named tests first and retain real outcomes. Production code may already pass; do not invent RED by expecting a nonexistent UnsupportedRunContext or stale revision from flags.
3. **REQUEST_CANCEL eventual settlement:** same two test modules, decision document, one evidence file, todo (five). Required before .1d closes. This is not an optional future descendant and does not close .1c's effect/compensation work.
4. **Acceptance bookkeeping:** main source-preservation inventory, decision, todo, 014 validation report, evidence manifest (five). Root reruns integrated focused gate itself. Only .1d may close from this unit; .1 umbrella requires all other accepted dependent writer units and their evidence.

Any discovered production defect needs its own bounded root-reviewed correction and genuine failing test before implementation. No permission/owner/product choice is currently required to execute this accepted-path proof after prerequisites. No cross-agent repository ownership is proposed.

## Planned verification and explicit remaining limits

Proposed filter: `leaf_certification_native::child_source::terminal_fanout::`. Root chooses its own private external target directory, checks `df -h`, respects at most two heavy builds, and runs the three named tests in default and runtime-role suite modes on the integrated tree. All actual native/scheduler adapters still use the runtime role regardless of suite mode:

```sh
CARGO_TARGET_DIR=<root-owned-private-target> RUST_TEST_THREADS=2 cargo test -p sagat-tests --test integration_workflow_primitives --locked leaf_certification_native::child_source::terminal_fanout:: -- --nocapture
CARGO_TARGET_DIR=<root-owned-private-target> SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=2 cargo test -p sagat-tests --test integration_workflow_primitives --locked leaf_certification_native::child_source::terminal_fanout:: -- --nocapture
```

Then rerun the child_source regression including accepted .1a/.1b/.1c and genuine-certificate Scoped Finish refusal, plus the existing flat leaf tests directly affected by reused helpers. Run repository formatting, focused strict integration Clippy and architecture/config checks required by the delivery; retain commands, exact named results, durations, source hashes and original failed attempts. Do not report the six fixture executions as six independent Rust tests if the implementation uses three tests looping two orders. No new golden, schema, SDK or production activation change is expected. Broaden tests only for a concrete changed dependency or required gate.

All execution evidence is pending. C1 is an unresolved prerequisite: the proposed outputExpr fixture is currently missing retained End output/run result, and .1d.0 must first produce actual RED and reviewed correction evidence. The exact REQUEST_CANCEL leased-scope drain boundary must also pass real public/runtime checks; static inspection is not evidence that either ran. No rollback/unknown-COMMIT crash matrix is added by this unit, and a paused native or parent COMMIT is not a crash. Existing recovery evidence stays separately scoped. This design covers normal result-bearing success, ordinary later failure and REQUEST_CANCEL -> actual child CANCELLED. It does not claim result-quota failure, timeout/compensated/partial/compensation-failed fanout, child chains, detached-descendant finality, effectful selected sources, old writers/backfill, retention rollout, nested execution, capacity acquisition/vacating, or full resource/release gates.

The existing source certifier's lack of context rejection is characterization, not general nesting activation. All Scoped Finish parent/chain refusals (`scoped_leaf_finish.rs:355-367`) remain, including when a genuine original child certificate exists. Canonical T014-014/015/016/017/040/042/043/044 and broader B1/B2/C0/C1/C2 remain open.

## Read snapshot hashes

Hashes identify read code, not executed test evidence; line references above are at HEAD `2d86483f03efa683e13df8d78bb1338f5170602f`.

| File | SHA-256 |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `0981e53290d2599919b2b0028ceac85a04d1ec0a8e9b94c5f52d98793b808a2b` |
| `specs/014-workflow-primitives/plan.md` | `33580b895a52c01113eacaf9503b95b88d5cb7b0204136fae62206c2c62ecd91` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `tasks/project-completion/leaf-child-source-preservation.md` | `74c39f356896ec202d36e4442e7150773d551f490beb14f4c25a6870d86fc197` |
| `crates/scheduler/src/passes.rs` | `31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/maintenance.rs` | `ce11e42dba0f37de24d56631ebce14c61af00f6ed303b8d2689fa5eed0e3f1b5` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `tests/integration/workflow-primitives/leaf_certification_child.rs` | `c797a0e6eb95e1db90ca93bf56137f408990c6bc4f1e07f664e1059ee3d532fd` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` | `679089900793c3e1949c834747aea66900f419f0116a508db61350b6cbb574c7` |
| `tests/integration/workflow-primitives/leaf_certification_commit_loss.rs` | `ab20f174accfc5d0db192ce64b2bc29b5fc81c572bfbfc2a5119a6155b4a2e8b` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |
| `docs/adr/ADR-0030-structured-instances-and-continuation-chains.md` | `004bdbfdf8b1616852189cec826a0cfaecd68cd287c404cc210d5979cae6852f` |

Only the named /tmp decision artifact was written. No tests, builds, formatters, agents, commits, repository writes or external actions were performed by this design task.


### C1 amendment read hashes

These additional source/contract reads remain at HEAD `2d86483f03efa683e13df8d78bb1338f5170602f`; no test result is attached to them.

| File | SHA-256 |
| --- | --- |
| `crates/infra-postgres/src/primitives.rs` | `0c84e9a5e622e5b70135340de75c69e6e04c3e79ee1caf04cdd704c0eb027b75` |
| `crates/application/src/execution.rs` | `c2efa47b53de70747cdd9aa0dceb57d5eb5334c6b76d8c81c8dff85637fb79c2` |
| `crates/application/src/primitives.rs` | `bd58299fa2104543cb4d84d529e1d105c2ee92254fe76ca444319d3b91a617ea` |
| `crates/infra-ir/src/convert.rs` | `fc55881ad1e9dd13f63f29eeb8f53f1766c918a6fbed0536d8f386ba6256e612` |
| `crates/infra-ir/src/evaluator.rs` | `41cc5d551977fd3f95120153f9bf93c2a956fbeaf668ec5647874c9c4bb3c447` |
| `crates/domain/src/workflow/collection_shape.rs` | `0b883bd651cc4aab7ca8193194f44d9c3f2b617b73e7cdce4eee4707b5f325eb` |
| `specs/001-platform-foundation/contracts/workflow.schema.json` | `a0aabe5817b668d063f5eea28fea32a08e6c26a40c1a9904a827289ce3d49a16` |
| `docs/owner-decisions.md` | `c6524ee5b848eecdd4c0cce71adc55cd70be7008ee98997db83c424b6eae8bb5` |
| `specs/002-durable-execution/research.md` | `564e38d6b5ff032e575312d07e4e5fe652fdd80bb78cf8b058c82a03c92b8bf8` |
| `crates/infra-postgres/migrations/20261110000000_run_result.up.sql` | `1bd2eed14a2c58ecb56aca51137d6b64756feccfece9224736970ca36c373d8c` |
| `tests/contract/durable-execution/grpc/run_result.rs` | `fda88d4d10cdf891aa9a9c4978d5f311b546c2d7b67cfedf10193f88892f4da1` |
| `tests/integration/workflow-primitives/collections_http.rs` | `de21951a111ddcc4653b3bae0688d8a822ebf237f67f567bbfe745c3a9eac549` |
| `tests/integration/payload-classification/result.rs` | `ee5474bd539a1acd91185edc81da24cd6530b71448b79cf71d3e6707ca1a824b` |
| `docs/adr/ADR-0033-payload-values-are-classified-per-field.md` | `17937661322cc4c8bdd6c284285fcd4f497565a9b33f2f771e0011a16fd0e6f9` |
