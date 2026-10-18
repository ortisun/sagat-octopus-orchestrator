# B1b.2c.1 — Ordinary leaf source in a legacy unchained child

Status: bounded decision reviewed at f7c0ef59; fresh independent Astra/xhigh review found no substantive issue. Root inspected the proposal against current code. Baseline characterization now has actual focused default/runtime-role evidence (checkpoint below); incoming-parent preservation units remain separate. No tests/builds/formatters, agents, commits, repository edits or external writes were performed by this design task. Only this `/tmp` artifact was written.

Initial repository HEAD: `753152f6663ba57af68c1b592661011e5370ab3d`. Final read: `f7c0ef59e972b61dfed93c4e3f98914a77f9e51b`, clean worktree, `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`, branch `feature/016-project-completion`. Root's new 16b/17 results are its evidence, not tests executed here. `.specify/feature.json` is absent; the assigned active feature is 014. Existing branches/worktrees were inventoried before design.

## Decision and exact scope

Choose the current public **ordinary collection child without a chain** as the next source-context proof. Its additional incoming writer surface is concrete and bounded. Do not introduce a generic relationship classifier or new model merely to name it.

Use one root parent with one Child node, and one genuine child Run whose pinned definition satisfies existing `LeafCollectionSuccessShape`/`OrdinaryLeafV1`. Select root collection path `[(lines,0)]` inside that child; a second root collection `later` demonstrates continued child work after selected-source closure. No continuation, children inside bodies, depth-two scoped path, nested controller, effectful selected body or new publication shape.

This is the same certificate's source semantics in an additional execution context. No new profile enum, certificate schema/version, fingerprint, DTO, mapper, port, grant or production caller is proposed. The selected path is independent of whole-child termination and the incoming parent's dependency. Parent settled/cancel flags, detachment or terminality never establish path settlement or capacity authority.

Proposed ledger naming, after root checks concurrent allocation:

- `B1b.2c.1`: current ordinary unchained-child source preservation, remaining open until the bounded incoming-writer proofs below pass.
- **`B1b.2c.1a`: genuine public child source characterization**. This is the next implementation unit. It may close only its own baseline evidence.

## Crucial current behavior

`PgLeafCollectionSuccessStore::apply` (`crates/infra-postgres/src/leaf_certification.rs:57`) locks applicable own chain → exact qualified original Run → own scheduler SHARE/post-wait time, then canonical replay → creation CAS → pinned shape/source proof. It **does not inspect/reject parent_run_id or chain membership**. The current flat boundary limits proven producer preservation; it is not an implemented source-context refusal. A baseline test may pass unchanged. Do not claim this work newly enables child certification or expect a new UnsupportedRunContext source error.

`PgScopedLeafFinishStore` (`scoped_leaf_finish.rs:357`) **does** refuse any parent or chain membership before canonical source/receipt access, including replay. Keep that consumer unchanged. A genuine child certificate cannot authorize scoped Finish.

`LeafCollectionSuccessShape::check` (`domain/src/workflow/collection_shape.rs:41`) checks definition structure only: frozen root collection chain, selected ordinary activity/Return body and no continuation policy/child/nested controllers. Other referenced linear collection bodies may carry their own companions. It cannot identify whether its Run is a child.

Current public `Scheduler::start_child` resolves the published child in the parent's namespace and sets `ParentLink` (`scheduler/src/passes.rs:4500,4620`); `execution.rs:991` inserts the association. `PgRunTx::commit` initializes a chain only for continuation policy (`execution.rs:4246`). Source search found its only production call to `initialize_chain_in_transaction` there. Existing `scoped_leaf_finish_contexts.rs:84` genuinely creates an ordinary unchained collection child but deliberately leaves it uncertified; reuse that creation pattern, not its absent-certificate evidence as proof.

## B1b.2c.1a: smallest first implementation

Use the real disposable HTTP/gRPC/scheduler harness, `.with_children`, existing `LeafCollectionPathIdentity`, `CertifyLeafCollectionSuccess`, `SchedulerAuthority`, `PgLeafCollectionSuccessStore` and exact selected-source oracle. Construct authority from the actual child Run; do not leave a helper accidentally addressing `Flow.run` when that is the parent. Test-only metadata helpers take an explicit Run id, not a new product model.

Required baseline assertions:

1. Publish parent and child, start parent publicly, let the real scheduler create the child. Assert actual tenant/namespace/parent node/cancel policy association, parent waiting, and absent child chain membership/incoming chain link. Do not seed a positive source or rewrite ancestry/status.
2. Before selected scope closure, native certification returns `SourceNotClosed` and writes no fact/audit. Complete the actual selected activity through gRPC and run real collection checkpoint/reduction/closure. Certify from its actual own-child revision/fence while `later` remains unfinished. Assert one correct canonical fact/audit and no certifier changes to child execution/source/journal. Preserve all five exact source event references, routed local steps, reserve0 and selected saga absence.
3. Cold pool/cache/store reconstruction with the same qualified identity and fresh child authority returns the original record/audit. Stale own-child authority refuses before replay. Wrong namespace/tenant/index cannot reuse parent/neighbor evidence. Label reconstruction accurately; no OS restart claim.
4. A genuine certificate still does not authorize scoped Finish. Preserve the existing all-child refusal; preferably add the genuine-certificate control in the existing context test module using its registry fixture helper. Denied source/receipt reads can prove context precedence; capacity/header/member/receipt/audit/hint remain unchanged.

The baseline characterizes already reachable code. Write tests first and retain real failures; do not invent RED by falsely expecting source rejection. Include deliberate missing-closure/authority and oracle-detector controls. Any genuine production defect requires its own bounded root-reviewed correction. No parent race, complete child-source preservation, quiescence or new authority is claimed by this baseline.

Suggested module filter: `leaf_certification_native::child_source::`; `main.rs:38` already registers `leaf_certification_native`. Verify default and `SAGAT_TEST_AS_RUNTIME_ROLE=1` using the explicit runtime pool and disposable test-only certificate INSERT grant. Root must rerun the focused gate on the integrated tree and retain actual command/output. No source INSERT or scoped member INSERT release migration follows.

## Bounded additional writer inventory

The current flat mutation-family inventory (`leaf-current-producer-preservation.md`) is a dependency: exact local reports, lifecycle/drain, later work/sagas, ambient binding/dispatch and source retention. It is not newly rerun evidence and does not itself cover incoming parent writers. Under no child chain/no outgoing child definition, the additional current source surface is:

| Current writer | Exact behavior and required subsequent proof |
| --- | --- |
| `start_child` → create/claim_reservation (`passes.rs:4500`; `execution.rs:991,1535`) | Genuine creation sets immutable parent identity in its creation transaction. Current initial parent-shard exception grants no authority over reopened children. Baseline proves this fixture is real. |
| `StepCancelled` → `release_child(true)` → `settle_child` (`execution.rs:4032,5028,5060`) | Parent-owned transaction first tries its matching chain link; for legacy child, UPDATE child `runs` sets `parent_settled=true`, ORs `parent_cancel` for cascading policy and unfinished child. Exact tenant/child/parent predicate. Test actual PROPAGATE and DETACH writers in both child-row lock orders. |
| `StepCancelRequested` → `release_child(false)` (`execution.rs:4047,5082`) | REQUEST_CANCEL reaches the distinct `UPDATE parent_cancel=true`; parent initially still waits and `parent_settled` stays false. Both orders and eventual real settlement must preserve these semantics. All three policies are necessary. |
| `children_to_cancel` → `children_cancel` (`maintenance.rs:569`; `passes.rs:4932`) | Discovery reads flags; actual stop/drain executes under child's own Run/fence. Separately prove actual child cancellation/CAS and selected preservation. A later known effect must reach genuine later saga/compensation without creating selected obligations. |
| `ended_children` → `children_finish` → `finish_with[_result]` → `ChildFinished` → `settle_child(false,true)` (`maintenance.rs:512`; `passes.rs:4766,4793,4846`; `execution.rs:4028,5028`) | Parent applies genuine success/adverse outcome under its authority then marks the exact child notification flag. Test both observable orders, actual parent transition once, source unchanged. Parent result retention is parent-owned. |
| Chain initialization/link/handoff/finalization (`chain_progress.rs:901,1020`) | Current initialization occurs inside creation only for continuation definitions, excluded here. Handoff/finalization require membership. Assert no child segment/link; retain source call-site check. No late-backfill/old-writer or chain-source guarantee follows. |

Searches covered parent_cancel/parent_settled and run/segment/link INSERTs across production PostgreSQL Rust, the exact helper callers and chain initialization call sites. No current runtime reparent or ordinary-child late-chain writer was found. Existing retention guards and absence of cleanup remain unchanged; arbitrary privileged UPDATEs and old binaries are outside current-writer proof. Assertions about ancestry immutability are current-producer preconditions, not new database guarantees.

## Critical transaction and oracle distinction

Certification never locks the parent, an incoming link or capacity G. Its own original child Run and own scheduler guard remain held through fact/audit COMMIT. Legacy parent writers already hold their parent transaction and acquire the exact child's row UPDATE lock; that conflicts with certifier FOR UPDATE. This is ADR-0030 §19's existing legacy protocol. Do not convert it into child-chain locking or add a reverse child→parent acquisition. No whole-platform lock-graph guarantee is inferred.

**Parent flag UPDATEs do not advance the child run_version and do not append its journal.** Parent-first flag mutation therefore need not cause `StaleRunRevision`; both queue orders may certify with the same expected child revision. Prove positive flag/parent-journal changes separately. Actual later own-child stop/drain does advance its journal/revision: a receipt miss with old creation CAS refuses, a fresh CAS can prove the still-closed source, and canonical replay follows current authority but precedes creation CAS.

For the reviewed minimal ordinary-child fixture, public parent CANCEL writes the child flags in the same command transaction before HTTP202; no scheduler tick is required. Race the actual legacy child UPDATE against first native certification by locally polling both operations. The bounded [parent flag contract](leaf-child-parent-flags.md) fixes both observed child-row queue orders and one actual certifier COMMIT guard. Later remains LEASED and unfinished; preserve its exact step/lease and the post-lease command. Use held row/deferred COMMIT barriers plus actual pg_stat_activity/pg_blocking_pids and target witnesses; no sleeps. Distinguish preliminary restore from the writer transaction. Include actual parent UPDATE blocked through certifier COMMIT. Maintain separate oracles for immutable selected source, legitimately mutable child flags/revision/journal, and parent state/journal; a whole-Run equality oracle would be wrong.

## Subsequent bounded subunits, without implementing them now

After baseline acceptance, review/implement one family at a time:

1. Incoming parent flag preservation: PROPAGATE/DETACH/REQUEST_CANCEL, both actual child-row queue directions, real flag effects, COMMIT guard lifetime. Detached later child execution is the separate pending .1e below.
2. Own-child cancellation/drain consequences: actual parent-triggered cancellation under child authority, creation-CAS versus replay distinction, and one known later effect with real later compensation. The reviewed [own-child contract](leaf-child-own-cancel.md) fixes two actual COMMIT orders, then exact later drain/ack/refund, stopping before child completion. Unknown effects/manual review and effectful selected paths remain separate source profiles.
3. Terminal outcome fanout: genuine whole-child success and later failure, actual parent notification/settlement once, both orders and cold original-identity replay. No seeded terminal status as positive proof. The [reviewed conditional fanout contract](leaf-child-terminal-fanout.md) also owns eventual REQUEST_CANCEL settlement, but remains blocked by pending .1d.0: current collection completion discards End.outputExpr and retains no run result. Establish a genuine RED and separately reviewed correction before accepting the result-bearing success fixture; do not seed output or silently downgrade this obligation.
4. Detached later execution (.1e): real DETACH parent CANCEL followed by real completion of the already leased later child activity, actual child progress/source preservation/original replay, unchanged settled parent and no child notification. Stop before whole-child completion; no repeated flag-race matrix.

Only after these proofs and a fresh bounded source review can B1b.2c.1 claim the enumerated current incoming-writer preservation. General B1b.2c remains open. Existing uncertain-COMMIT/OS-reconciler tests remain their own shared transaction evidence; baseline reconstruction must not rebrand them as child engine/original-writer crash evidence.

## Ownership, at most five files per increment

- Decision freeze: new `tasks/project-completion/leaf-child-source-preservation.md`, project `todo.md`, 014 `spec.md`, 014 `plan.md` (4).
- Baseline: `leaf_certification_native.rs` registration, new `leaf_certification_child.rs`, existing `scoped_leaf_finish_contexts.rs` genuine-source refusal control, contract document, one reproduction evidence file (5). Keep helpers in the child test module and reuse existing context helpers; no shared-harness refactor or new API.
- Each later writer subunit: child test module, at most one focused child test submodule, contract document, evidence file, todo update (≤5). Serialize ownership after root's `f7c0ef59` freeze.
- Local acceptance: contract/todo/validation-report/evidence (≤4), with exact executed test names and no canonical runtime closure.

The root's independent fresh review covers this decision, then the first baseline implementation; it is not a fourth whole-protocol review. Do not wait for a new owner/product choice: existing all-open-tasks/decomposition authorization and ADR dependencies cover this proof.

## Exact ledger accounting at f7c0ef59

Canonical unchecked 014 tasks remain **T014-014,015,016,017,040,042,043,044**. This unit closes none. T014-004/038/041 stay checked. Feature016 T016-001 and the historical top-level project categories are outside this acceptance scope.

Open capacity umbrellas, exhaustively from current todo:

| Item(s) | Remaining reason |
| --- | --- |
| B1, B1b, B1b.2, B1b.2c | Full current-member authority, broader relationship closure and quiescence/transition rules. |
| B1c; B1d; B1e | Forward versus drain/reduction/policy0; discovery/offer/parking/fairness; creation/join/continuation/cancellation/cleanup. |
| B2 | Remaining lifecycle including vacant acquisition; dormant ChildRoot Finish is its restricted completed slice only. |
| C0; C1; C2 | Old writers/backfill/retention/quota projection, mixed-version/rollback, accepted low-capacity nested progress, original counterexamples and full resources. |
| B1b.2a.1, B1b.2a.1d, B1b.2a.1d.3, B1b.2a.1d.3d | Overall source-writer/release gates remain explicitly open. Their closed flat proof/recovery/observation subitems must not acquire child/chain or activation authority. Observation portion of .3d is already closed by .3d.1; do not repeat it or equate it with release INSERT. If bookkeeping later separates proof from activation, close only the explicitly narrowed proof. |
| B1b.2b | Fixed flat scoped-consumer evidence does not close broader relationship/source context, retention rollout or production caller authority. |

B1b.2b.16, .16b and .17 were pending initially and are now checked at `f7c0ef59`; final source read confirms that ledger change. Root's recovery/observation gates close those locals only. Preserve already closed .15/.16a and flat source .3b/.3c/.3d.1. No new test acceptance is supplied here.

The nine unchecked historical project summary rows also stay unchanged: 017 reconciliation; 016 domain/contracts/publication; disclosure/grants; inbox/dead-letter reveal; SDK authoring/runtime; CLI/Studio; accepted014 runtime/large workloads; integrated release gates; final ledger/evidence/commit delivery. They are not nine newly discovered defects.

## Dependency and release boundary

ADR-0030 already approves existing 002/003/010 identity, child policies, collections/effects, own scheduler authority and separate legacy/chain protocols; ADR-0032 controls capability rollout. ADR-0031 is unchanged: only bounded certification metadata is involved. No ADR divergence is proposed.

Unsettled: authorizing child scoped Finish, Vacate quiescence, Acquire eligibility, retained relationship/continuation/detached-descendant rules, broader controller/effectful source closure, C0, production durable intent/grants, nested progress and resource acceptance. Changing cancellation semantics or taking another existing Run mutation lock would exceed this contract. Passing this baseline proves a genuine source can be certified in the stated child context; it does not prove complete child relationship preservation or any capacity transition.

## Read provenance

Mandatory AGENTS/constitution, 014 spec/plan/tasks, ADR0030/31/32 and project todo/plan were read. Source files below were read at cited functions/excerpts or source searches, not all exhaustively. Hashes identify complete files at the final read snapshot; they are not test evidence. No repository content was written.

Final hash snapshot HEAD: `f7c0ef59e972b61dfed93c4e3f98914a77f9e51b`. Some files were read by targeted searches/excerpts only, as stated above.

| Read/source-searched file | SHA-256 |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `5ee76107121bb0b7f4b2bea38a430cfe9ff8cd07d87ce6c24fd6fb4db5e258f0` |
| `specs/014-workflow-primitives/plan.md` | `df76cdf7764ebe2f6d16293fb115205bd51466cd9671a657aeca5f3705137dfc` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `docs/adr/ADR-0030-structured-instances-and-continuation-chains.md` | `004bdbfdf8b1616852189cec826a0cfaecd68cd287c404cc210d5979cae6852f` |
| `docs/adr/ADR-0031-declared-operation-result-disclosure.md` | `afd5abf1e0a220e732b6ffece6f5dba84d6fbd41b7ebd11e693ab4d23213c347` |
| `docs/adr/ADR-0032-additive-workflow-ir.md` | `87cbaeae372df88ed0e3aff1ca48baae6b6bbd4e2408d7ddd003ab92c9f8797d` |
| `docs/owner-decisions.md` | `c6524ee5b848eecdd4c0cce71adc55cd70be7008ee98997db83c424b6eae8bb5` |
| `tasks/project-completion/todo.md` | `aa2987b6992ccc3e96a96d4916a1ae12e9517ad35b53dbd79f2dfccdfdc19fee` |
| `tasks/project-completion/plan.md` | `9aa906fe974d44ecdf0ac242ad28441ebe0dfa2a30c72d8df5414b107506de35` |
| `tasks/project-completion/nested-capacity-review.md` | `8eac76a12963a6a50dc3d929fb6c4cdb4e036431d687a29af98d12af2533032d` |
| `tasks/project-completion/capacity-path-settlement.md` | `1605a2c72da25a9c941b552b9ea018e3f074804043e8bb62c16abe2b37116e46` |
| `tasks/project-completion/capacity-member-finality.md` | `c454b22bbef82c706d086f16b4eb668866906256ef41b23b2dba20472a008877` |
| `tasks/project-completion/capacity-transaction-entry.md` | `2d11d9838090c31d6e60659ed6e8b493c3a1ebae130a4756960bea554d8a27c8` |
| `tasks/project-completion/leaf-source-retention-inventory.md` | `81ce099662e07b2236d953f015b969ff2f47f2a08115aa0bd0353f3c6da3cad4` |
| `tasks/project-completion/leaf-current-producer-preservation.md` | `716870a727903c24c02f50675129f994193af0d3ab39832851013896f30e122f` |
| `tasks/project-completion/leaf-certification-transaction.md` | `965c9df4778ed94ae3de0701b65c252444addd175c80eb1791e1fd4aca9d62db` |
| `tasks/project-completion/scoped-leaf-finish-flat-context.md` | `640d40ec1b1589233ab6e9adfa3a247f736db505d5a9af867188cf829d22befa` |
| `tasks/project-completion/scoped-leaf-finish-binding.md` | `7541a4c5612cd77c3c6c2f8f44b7b8c124a9ffbd61f9848a8f4cd3e5578d4dd2` |
| `crates/domain/src/execution/capacity.rs` | `3d4329ae341da7ebfd355fb1a1a2b2c147474ace63f04e4a0b0e509394374fb9` |
| `crates/domain/src/execution/continuation.rs` | `c88cac77906bd408e68df88bc5400f83a3d6f5ec5f8a1ac662f69f82c491419a` |
| `crates/domain/src/execution/path_state.rs` | `da02a6dd7841057e1171a50e693c881c7ab82028255305f72efb9ec9c08c3ccc` |
| `crates/domain/src/execution/plan.rs` | `d9c1c67ab83990be5cb89b9e7f1604b29b767a947917a94b5247522403091879` |
| `crates/domain/src/execution/state.rs` | `e802c45c2bdbad9da159c6f60ad28ab6406c3fa04ce17b473ba7cb63f71af0e5` |
| `crates/domain/src/workflow.rs` | `4334a0181a5fc5d2b0a97f5c0da25573851df8a03b9612876c2fb4650dda76e3` |
| `crates/domain/src/workflow/collection_shape.rs` | `0b883bd651cc4aab7ca8193194f44d9c3f2b617b73e7cdce4eee4707b5f325eb` |
| `crates/application/src/scoped_leaf_finish.rs` | `f389cf29a48684f24fa76d9cd4da567b1bea6a81ae52bc1f0a7a06ce0038e2e3` |
| `crates/application/src/execution.rs` | `c2efa47b53de70747cdd9aa0dceb57d5eb5334c6b76d8c81c8dff85637fb79c2` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/chain_progress.rs` | `0f25b60c6dd4db6ff78c25651f357f4df8dced94549d21878eeb9e4f58688439` |
| `crates/infra-postgres/src/maintenance.rs` | `ce11e42dba0f37de24d56631ebce14c61af00f6ed303b8d2689fa5eed0e3f1b5` |
| `crates/infra-postgres/migrations/20261009000100_execution.up.sql` | `d51cf9fd30c804fc87b0f3463114addd1dff6cf2edfcbc9d759f84a592126aca` |
| `crates/scheduler/src/passes.rs` | `31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d` |
| `crates/scheduler/src/lib.rs` | `fb8e5d28e5cf4b78b79d244f1e8452f7989831395a83cd992782486ed3fca6b9` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |
| `tests/integration/workflow-primitives/main.rs` | `188a4bb0062a181c765d7235a5106fe5eaed65d20d53c8d87bd63e2d6d292018` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` | `468e0505b935e3696e5ee0aa28d32f51d338e23eda5241b0d5bb7361b454f14d` |
| `tests/integration/workflow-primitives/leaf_certification_later_work.rs` | `e00184bf3cd342965923aeeb9574ab4b69c57e69c4f67d1952ca07991a5bad1c` |
| `tests/integration/workflow-primitives/leaf_certification_binding.rs` | `b8d30c2e7bf1087127b950213d61f5bf0e3f3e0274bcc0c6c7909c8e1422c16a` |
| `tests/integration/workflow-primitives/leaf_certification_saga.rs` | `2ebfd384293d235a1b719f2b6fa11e4640715c02e5b948c30fe6e2cc9c6a933b` |
| `tests/integration/workflow-primitives/leaf_certification_process.rs` | `65e1222b205b288e2194249668c993b14f5400b61f3150448e85180b4c14fe33` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` | `8783891258f4b6cc60c04caff77213bfbf529fd42ce016a6224c3654e4ebd8e9` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_contexts.rs` | `7222e366728f2fcb5ca1381186e6e830c2aa96ddf018131c97d13acb66af2326` |


## Root integration checkpoint

Fresh bounded decision review returned None. This accepts only the characterization contract,
not an unexecuted child-source guarantee. The first test module may keep its small owner-seeded
capacity registry helper local to avoid widening sibling-private harness interfaces; the genuine
certificate/refusal/denied-read control remains required. Baseline code and evidence are pending.
The earlier external second review was declined by the user; no external model CLI was run.


## Genuine ordinary-child baseline — 2026-10-18

A genuine publicly started parent creates an ordinary unchained child through the production scheduler. Before source closure the native certifier refuses SourceNotClosed; actual worker completion/checkpoint/collection closure creates the source while later child work remains LEASED and unfinished. First native certification retains exact qualified source identity, pinned definition, scope/body/Return/digests/five actual event references and one truthful audit. Cold runtime pool/cache/store replay returns the complete original record. Stale own authority precedes replay; foreign tenant/namespace, missing index and parent identity cannot reuse the child evidence. Actual later work finishes child and parent while selected source/certificate stay unchanged; terminal replay preserves the entire observed snapshot. A second genuine certificate fixture still receives UnsupportedRunContext from Scoped Finish before denied source/receipt reads (actual42501 probes), with no source/execution/capacity effects. Both use explicit sagat_app and a disposable test-only source INSERT grant that is revoked before replay. No production implementation changed.

Actual focused child baseline: 2/2 default (40.1s wall) and 2/2 runtime-role (6.58s wall), the same two tests in separate invocations. Strict integration Clippy 39.21s, formatting 4.49s and architecture 0.41s/24 crates passed. All 1104 frozen executable/contract inputs match.

This closes local B1b.2c.1a only. It characterizes existing source acceptance, not new child support or complete parent-writer preservation. Cold reconstruction is not an OS restart. Parent CANCEL races, child cancellation/drain/later compensation and terminal fanout remain separately pending. All parent/chain Scoped Finish refusals remain. Broader B1/B2/C0, production source/scoped INSERT grants/callers, nested runtime, resource ceilings and canonical T014-014/015/016/017/040/042/043/044 remain open. Earlier compilation is compilation only. Initial runtime0/2 failed at public workflow creation422; shortened test fixture prefixes fixed the public Slug63 mismatch. The retained preformat run passed2/2 in each mode and Clippy/architecture, but formatting rejected one edition2024 trailing comma. Final results above are from the corrected final source, not a combined aggregate.
Evidence: tasks/project-completion/evidence/ordinary-child-source-baseline-20261018/manifest.json.


## Actual incoming-parent flag preservation — 2026-10-18

Real public parent CANCEL competes with first native certification of a genuine ordinary unchained child under PROPAGATE, DETACH and REQUEST_CANCEL in both observed child-row queue directions. Exact three-PID role/database/query/wait/blocker witnesses, parent Run conflict probes, original post-lease child CAS, policy-specific parent journal/status/step/flags, atomic audit/dedupe/outbox and immutable selected source/full child core/journal/steps/exact later lease are asserted. All six first certifications succeed without changing the captured revision. A seventh fresh fixture pauses the actual certifier COMMIT behind a deferred advisory gate; the real parent UPDATE waits behind that COMMIT while child Run, parent Run and child scheduler SHARE guards remain held. Releasing only the gate commits both. After revoking the disposable source INSERT grant, a cold runtime store/cache returns the exact original complete record/audit and preserves the full snapshot.

Actual focused child suite:4/4 default in53.65s wall/16.71s harness and4/4 runtime-role in17.43s wall/16.78s harness. These are the same four named tests in separate invocations:two existing baseline controls and two new tests executing six policy/order fixtures plus one actual COMMIT fixture. Seven asserted three-PID observations are retained per mode. Strict integration Clippy11.27s, fmt4.33s and architecture0.36s/24 crates passed. All1105 frozen executable/contract inputs match.

Local B1b.2c.1b closes only this incoming legacy flag writer family. No production code, grant, caller, profile, schema or capability changed. Later remains LEASED and unfinished; no own-child cancellation/drain, detached later progress or terminal fanout is claimed. The parent flag writers do not advance child revision/journal/updated_at. B1b.2c.1c/.1d/.1e and the master .1 remain open, as do broader B1/B2/C0, production activation, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044. Initial default invocation passed the two baseline tests and failed both new tests at the shared uppercase dedupe expectation; the six-cell loop then stopped in its first cell. Initial Clippy failed two test-only lints. All original records/source pins are retained separately, and no partial run is combined with the final suite.
Evidence: tasks/project-completion/evidence/ordinary-child-parent-flags-20261018/manifest.json.


## Actual detached-child later progress — 2026-10-18

After genuine ordinary-child source certification, real public DETACH parent CANCEL commits a terminal parent with parent_settled=true and parent_cancel=false. The original leased later activity then completes through real gRPC, retaining its exact expected output, clearing lease metadata and advancing child revision/steps/journal without stopping or finishing the child. Selected source/certificate stay unchanged. Exact parent Run/steps/journal/audit/outbox snapshot stays unchanged, with no child.finished notification. After the disposable source INSERT grant is revoked, a cold explicit sagat_app store/cache replays the original creation CAS and returns the complete original certificate/audit under current own-child authority, preserving the entire post-progress snapshot.

Actual focused DETACH later-progress test:1/1 default in38.62s wall/1.94s harness and1/1 runtime-role in2.53s wall/1.91s harness. These are separate invocations of the same named test, not two distinct cases. Strict integration Clippy11.21s, fmt4.27s and architecture0.35s/24 crates passed. All1106 frozen executable/contract inputs match.
Fresh bounded Astra review:None.

Local B1b.2c.1e only. No scheduler tick, child termination, terminal result propagation, fanout, own-child cancellation/drain/compensation, race matrix, production code/schema/profile/grant/caller or capacity activation is claimed. B1b.2c.1c/.1d/.1d.0 and master .1 remain open. The separately identified collection End/owner-result retention defect requires genuine RED and a reviewed correction; this test stops before that path. Broader B1/B2/C0, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044 stay open. Prior four-test child-suite results at2cdd5aaf remain separate at1105 inputs; this one-test proof at1106 inputs is not a new five-test aggregate.
Evidence: tasks/project-completion/evidence/ordinary-child-detached-progress-20261018/manifest.json.

## .1c acceptance checkpoint (2026-10-18)

Two real PostgreSQL tests pass in default mode (42.77 s wall / 4.74 s harness) and explicit runtime-role mode (5.30 / 4.66); same two names in both, no aggregate four-test claim. Actual three-PID witnesses establish both original-child writer COMMIT orders. After real public PROPAGATE, cancellation, drain and known-effect refund, selected source/certificate/audit remain unchanged and the complete original record replays cold. The child remains unfinished; End/result/fanout remain separate. Strict Clippy passed 11.69 s, fmt 4.36 s, architecture 0.34 s / 24 crates. Final 1108 source inputs unchanged. See evidence/ordinary-child-own-cancel-20261018 manifest/review/reproduce and retained initial failures. B1b.2c.1c closes only; parent umbrella, .1d/.1d.0 and canonical 014 tasks stay open.

Implementation/evidence commit: `de64eafc`. Prior child-suite, detached-progress and this two-test selection are separate source revisions and invocations, not a new combined suite. The independent collection End result and instance-limit RED regressions remain unresolved at `6ef22a85`; these successful cancellation tests stop before that path.
