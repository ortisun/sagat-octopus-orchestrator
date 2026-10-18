# B1b.2c.1c — Own-child cancellation, drain and later compensation

Status: bounded decision accepted after fresh independent Astra/xhigh C1 review (None). Root checked the actual child-cancel discovery, own-child scheduler pass and frozen source-shape predicate. Execution remains pending. The design task performed no tests, builds, formatting, repository edits or implementation draft. Reviewed sealed draft:c67262990515b08acbe2041e269064fcd9b40ec152c41b063786a0af09341322. Optional external review was declined by the user. Read base: `2d86483f03efa683e13df8d78bb1338f5170602f`. `.1a` acceptance at `8df0ae90` is recorded evidence from the root, not a run performed here. `.1b` must pass and its helper ownership be sealed before these behavior tests are integrated.

## Decision and boundary

Add two tests of existing behavior in one genuine ordinary, unchained child: parent PROPAGATE CANCEL → committed incoming flags → actual child scheduler cancellation → exact later-scope drain → actual compensation of a known later effect. Test the two observed commit orders between the own-child stop and first native certification. The selected `lines/0` source remains OrdinaryLeafV1 throughout; the effectful `later/0` body is distinct. Stop with the later saga DONE and the child still unfinished. Terminal child completion and parent fanout remain `.1d`.

This extends preservation evidence for an already accepted source context. `PgLeafCollectionSuccessStore` already accepts a ParentLink; it does not enforce a flat-parent refusal. No new source profile, parent policy, capacity transition, public API, producer, grant or caller is introduced. The separate Scoped Finish parent/chain refusal remains before source/receipt lookup, including replay. ADR0030/31/32 remain unchanged.

Use the existing ordinary child fixture, domain instance/authority/command types, native certificate store and source readers. No DTO/mapping layer. These tests add evidence for the existing child cancellation family; they do not close the complete child-source inventory or any canonical 014 umbrella.

## Genuine precondition and captured boundary

Reuse `leaf_certification_child.rs` at its accepted boundary: public parent/child publication and start, real scheduler child creation, selected `lines/0` ordinary work and Return/checkpoint/closure, then actual later lease. Retain the bootstrap-terminal check, exact ParentLink/root/depth/namespace/policy assertions, absence of chain membership/history/child-chain links, and no outgoing child in the child definition. Parent has only its waiting `kid`; its successor End is unactivated. Use short fixed labels that keep public generated Slugs within 63 characters.

Permit a small test-local document parameter after `.1b` seals its helper API; preserve existing `.1a` behavior. Only the later body changes, to:

```json
{"entrypoint":"work","nodes":[
 {"id":"work","type":"activity","activity":"line.charge","next":"after","compensateWith":"line.refund"},
 {"id":"after","type":"activity","activity":"line.notify","next":"returned"},
 {"id":"returned","type":"return","result":"steps.after.output"}
]}
```

Complete the real leased `later/0/work` through gRPC with a known successful output. Drive its legitimate route before cancellation, then lease exactly `later/0/after`. Require charge SUCCEEDED/routed with its actual output/journal; successor LEASED and unfinished with its exact identity, attempt/generation/token/owner/deadline snapshot; later OPEN, iteration ADMITTED and slot held; no later saga. Selected ordinary source is closed/quiescent, reserve zero, and has no local saga/entries. Capture a new source command `c(r)` **after this second actual lease** and all selected-source projections. The baseline helper's earlier command is not the race command. Initially selected fact/audit counts are zero.

Optionally incorporate the existing-authority negative as a precondition in each test, without adding a scenario: begin the already committed child Run, require `holds_parent_shard` with actual parent lease to return false, and `holds_shard` with the actual child lease to succeed; then roll back. Refusal must also hold when parent and child shard numbers coincide. `execution.rs:3536` limits parent authority to uncommitted zero-version creation; it cannot authorize mutation of this existing child. Reuse real leases, not fabricated fences.

Send **public CANCEL to the parent id**, not `Flow.run` (which is the child), and await 202 before racing the two child operations. `.1b` proves the flag race separately. Public CANCEL persists parent stop/settlement and legacy child flags in this same transaction before response; no scheduler tick is needed (`routes/execution.rs:935`, `runs.rs:1180`, `execution.rs:4028,5028,5060`). Require parent CANCELLED/finished, kid CANCELLED/routed/waiting=NULL with preserved child id, child flags `(parent_cancel,parent_settled)=(true,true)`, and the documented parent suffix. Child own journal/revision remains `r`, stop_reason remains NULL and after lease is unchanged. Capture the post-CANCEL parent snapshot for subsequent preservation checks.

## Actual producer and authority

`maintenance.rs:569` discovers a legacy child only while `parent_cancel AND finished_at IS NULL AND stop_reason IS NULL`. `passes.rs:4932` invokes `children_cancel` → `apply`/`apply_if` (`3534/3014`) → `stop_run(Cancelled)` on that **child Run**. It acquires applicable chain latch (absent here), full child `FOR UPDATE`, restores that child, obtains its own scheduler-shard SHARE guard via `View::hold_for`/`holds` (`449/461`), checks a fresh database clock after any wait, applies and commits child journal/projections/outbox. `execution.rs:520,3523` uses the child's shard and current owner/generation. It does not borrow the parent's scheduler authority.

Native certification (`leaf_certification.rs:57`) uses applicable chain → compact original-child Run `FOR UPDATE` → own scheduler SHARE/post-wait clock → canonical lookup → creation CAS → exact ordinary shape/source validation → fact/audit → COMMIT. Canonical replay precedes creation CAS but still requires current authority. Neither operation may release its original-child lock/own scheduler guard before COMMIT finishes.

Use the real scheduler `tick()` locally; do not expose a new production child-cancel API. Its child-cancel pass runs last (`passes.rs:586–617`). At the captured boundary no earlier pass has eligible work: no pending Return, failed step, admission room, READY hint, retry, due timeout, expired lease, stopped child, new child or terminal child notification. `maintenance.rs:174` therefore does not select the OPEN full later scope. Scope-frontier snapshot reads are read-only; a restore/read alone is not the mutation witness. Assert actual leases remain live and all unexpected mutation counters stay zero.

Discovery caching may cause an empty tick. Use a bounded locally polled producer future that calls real ticks and `yield_now` until exactly one report has `children_cancelled=1`, then returns immediately. Preceding ticks must be mutation-free. Do not use sleeps or assume the first tick acts. Do not tick again after the child-stop report, since a subsequent tick could start drain or finish other work. Use legitimate renewal if necessary, never SQL lease extension. `Flow` is not Send: pin/poll producer, certifier and observer locally rather than spawning them.

## Two deterministic commit orders

Proposed tests in one focused child submodule:

1. `parent_propagate_child_stop_commits_before_first_source_certification`
2. `first_child_source_certification_commits_before_parent_propagate_child_stop`

Each uses a fresh disposable fixture and the test-only source INSERT grant already permitted by the native harness. Both calls must execute as `sagat_app`. Use generated advisory keys and test-only deferred constraint triggers, bounded timeout/yield observation, distinct recorded backend PIDs, same database, actual statement text, Lock waits and `pg_blocking_pids`. Fail if an operation finishes before its required witness. Bind values are not in pg_stat_activity; exact target preconditions, qualified locks, trigger predicates, blockers and positive row changes establish the target.

### A. Actual own-child stop commits first

After parent CANCEL, install a test-only `AFTER INSERT ON run_events DEFERRABLE INITIALLY DEFERRED` constraint trigger whose function waits on the generated advisory key **only** for the fixture's exact tenant, child Run and `event_type='run.stop_requested'`. Other events return NEW. Hold the key in owner transaction G. Poll the real producer until runtime PID S is executing `COMMIT`, waiting on G. The exact event predicate plus COMMIT identifies the child cancellation transaction, not parent cancellation or a preparatory restore.

While S is blocked, start the first certifier using unchanged `c(r)`. Require runtime PID C to wait on S at exactly:

```sql
SELECT workflow_version_id,run_version,trace_id FROM runs
WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE
```

Observe the original child Run guard and own scheduler SHARE still held by S: exact-child `FOR KEY SHARE NOWAIT` →55P03; child's scheduler row `FOR NO KEY UPDATE NOWAIT` →55P03; same shard `FOR SHARE NOWAIT` →success. Use fresh/autocommit connections for failing probes. Independent reads must still show pre-stop child state and no selected fact/audit.

Release only G and locally join both operations. Require actual `children_cancelled=1`, no other mutation family, and exactly one new child `run.stop_requested` (CANCELLED, no code), giving observed revision `r+1`. Assert this event/revision relation from rows, not a guessed stale revision. Child is CANCELLING, unfinished; later remains OPEN/ADMITTED, after still LEASED without cancel_requested; selected source unchanged. First certification must be `StaleRunRevision { current: observed_child_revision }`, with fact/audit still zero. Create a command using that observed revision and valid current own authority; first certification now succeeds with `replayed=false`. Once the canonical record exists, replay both that successful creation command and the original failed `c(r)` identity/revision with current valid authority; both return that same canonical record. Do not substitute SourceNotClosed.

### B. Actual certification commits first

Use the existing deferred certification-insert gate design (`leaf_certification_commit_loss.rs:11`), without its loss proxy or forced abort. Use an isolated one-connection runtime pool/application_name to identify certifier C. Hold key in owner G and poll first certification `c(r)` until C is executing actual COMMIT behind G; fact/audit remain invisible. Perform the same child Run/scheduler guard probes while C holds them.

Poll the real child-cancel producer until S waits on C at the **full original-child Run FOR UPDATE**, whose stable prefix is `SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id,` and suffix is `FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE` (`execution.rs:1613`). Require the suffix; a read-only full snapshot has a similar prefix. Source COMMIT must still be blocked by G while this actual writer waits. There is no separate collection/planner restore to count as this stop's lock witness.

Release G and join. First certification succeeds at certified revision `r`; child stop then has the same one-event positive outcome as A. Replay `c(r)` with current valid authority returns the **same complete original record/audit**, `replayed=true`, despite newer child revision. No second certification or audit is written.

Remove each test-only trigger/function and close dedicated pools after its proof. Retained quota locks, artificial SQL source edits and a substitute RunHold do not establish COMMIT lifetime.

## Shared continuation in both tests: exact later drain and real refund

After A's fresh successful certification or B's original successful certification, advance only the addressed later collection. Do not use broad `Flow::drive` after flags or compensation, since it may pass the intended boundary.

1. `advance_collection(tenant,child,root("later"))` → `DrainStarted`. Assert actual `collection.drain_started` (cancelled) plus `step.cancel_requested` for leased `later/0/after`; later DRAINING, after remains LEASED with cancel_requested=true, iteration still ADMITTED/slot held, no saga. Selected source and canonical certificate unchanged.
2. Repeat exact advance → `Waiting` while after is still leased. No false quiescence, slot release or compensation plan. Then acknowledge this exact task through real gRPC Fail with raw code `CANCELLED`, `retryable=false`, original valid lease/fence. This is the supported cancellation acknowledgment (`worker/scoped_reports.rs:15`, `command.rs:926–1045`): assert addressed step.cancelled, cleared lease, no retry/reconciliation, no activated Return. The known charge remains SUCCEEDED.
3. Exact advance → `Drained { index:0 }`. Assert later iteration DRAINED/slot=false, active_count=0, released forward reservations and retained declared compensation reserve (`kept_reservations`, `primitives.rs:222`: this body has one companion). Selected iteration reserve stays zero. Match each revision increase to actual journal suffix; do not hardcode later counts across distinct transitions.
4. `advance_collection_from_run(child,later)` → `Compensated` indicates planner progress, not completed compensation. The existing `advance_scope_compensation` loops until Idle (`command.rs:1249-1270`): it plans the saga and releases the eligible compensation **in the same commit**. Assert the exact consecutive `scope.compensation_planned` and addressed `step.released(work~c)` suffix, one later/root COMPENSATING saga/cause RUN_CANCELLED/during_stop=true/outcome=NULL, one COMPENSATE entry, and `work~c` READY. No second scheduler call or intermediate durable PENDING state is assumed. Selected lines has no saga/entries.
5. Poll the real worker; require exactly `later/0/work~c`, kind COMPENSATION, activity `line.refund`, and complete through gRPC with a known result. Assert actual completion/routing and intentionally absent retained compensation output. Inspect saga state before every subsequent explicit advance; if needed emit scope.compensated, then stop as soon as phase DONE/outcome COMPENSATED. Do not invoke the subsequent finish_collection_drain branch.
6. Final boundary: refund SUCCEEDED/routed without lease, later saga DONE/COMPENSATED, later scope still DRAINING, iteration DRAINED, child still CANCELLING with finished_at NULL. No child run.finished and no parent child.finished. Post-CANCEL parent snapshot unchanged. Canonical replay with current authority and the successfully created certificate's original expected revision returns the same complete record/audit, even after further child revision growth. A fresh store/cache can demonstrate reconstruction, not OS restart.

Drain store transactions (`primitives.rs:307,637,1763,2090`) take applicable chain → child Run → child scheduler SHARE/post-wait clock, validate child CAS/exact namespace/owner/path, then own later scope; iteration drain also locks tenant/run usage before scope/iteration/member steps. They refuse in-flight work, atomically persist addressed journal/projections, usage changes and outbox. Worker acknowledgments/completions use actual current-path Run lock, binding and worker lease/fence (`execution.rs:631`, `worker/scoped.rs`, `scoped_reports.rs`). Scope compensation is planned only after drain quiescence (`scoped_sagas.rs:40`); actual worker reports remain necessary. The selected source lock/immutability argument is child serialization plus these exact nonselected addresses, not whole-child immobility.

## Independent oracles and reused evidence

Maintain separate mutable and immutable observations. The accepted `OrdinaryChildSourceSnapshot` includes whole-child Run/journal/steps: those fields must legitimately change here and cannot be equality oracles. Compare its selected-source field and independent exact rows for selected scope, iteration, checkpoint, owner, local steps, selected reserve0, saga absence and pinned definition. Verify all five referenced selected journal facts/keys/details against the complete canonical record, using `.1a`'s `assert_record_matches_actual_source`; preserve their contents after every actual mutation. Exactly one qualified certificate and matching audit persist. Whole-child journal growth, later leases/drain/refund, parent command journal and flags require positive assertions instead of whole-Run equality.

Existing flat later-work and saga proofs cover their own current producers, source fingerprints, stale source reports, planning orders and actual compensation; shared guard/source logic is reused. They do not establish this genuine parent-triggered child cancellation path. This bounded design does not rerun or claim child coverage for flat handoff, expiry, crash, lost-COMMIT or other negative matrices. Missing actual witnesses, unexpected mutations, stale lease or failed compensation are failures to explain, never reasons to weaken the expected oracle or manufacture a refusal. Existing behavior can pass immediately; no fake RED requirement.

## Explicit remaining inventory and ownership

Allocate **pending B1b.2c.1e — detached later execution** in the root ledger when freezing this decision. `.1b` proves DETACH flags only. Minimal later `.1e` contract: fresh ordinary child fixture and genuine certificate; public DETACH parent CANCEL/202 establishes parent settled=true/cancel=false and terminal parent; complete the already leased ordinary later activity through gRPC; assert actual child step/output/revision progress, immutable selected source and original canonical replay under current authority, unchanged post-CANCEL parent state/journal and no child.finished. Stop before whole-child completion; no second flag race matrix. This is an allocated pending verification, not evidence supplied by `.1c` or automatic `.1d` coverage.

Ordered increments, at most five owned repository files each, for the root after independent review:

1. Freeze this `.1c` contract, update master child-source inventory and todo with `.1c`/pending `.1e`; optional active-plan pointer (at most four docs).
2. **Only after `.1b` passes and releases helper ownership:** minimal helper/document parameter and child submodule registration in existing `leaf_certification_child.rs`; new focused `leaf_certification_child_own_cancel.rs`; this contract; one evidence file; todo (five). No production/shared-harness refactor. Use the then-sealed helper signature, not an API guessed from unfinished work.
3. Independent review checks actual stop entry/queue/COMMIT witnesses, selected-source and positive mutation oracles, no terminal overshoot and unchanged activation gates. Root runs the two focused tests in default and `SAGAT_TEST_AS_RUNTIME_ROLE=1` modes on the integrated tree, then the affected focused child suite (.1a/.1b plus .1c) and required repository gates. Record exact names/commands/results/failures separately in evidence/contract/ledger (at most four files). No broad performance run or whole nested-protocol review is required for this increment.

Only `.1c` can close from these tests. `.1b`, `.1d`, newly allocated `.1e` and the master `.1` stay open until their own evidence is accepted. General B1b.2c/B1/B2/C0, mixed legacy writers/retention/backfill, nested runtime/resource acceptance and canonical T014-014/015/016/017/040/042/043/044 remain open. Production source/scoped INSERT grants and callers stay closed; no capacity consumer relaxation is authorized.

## Read provenance

Mandatory AGENTS/constitution/active 014 spec-plan-tasks were read; current deltas and ADR0030 legacy transaction provisions were checked. ADR0031/32 are unchanged from the preceding accepted reads. Source/helper/flat tests were read at the cited functions and relevant excerpts. Hashes identify whole committed files, not exhaustive reads or executed tests. No unfinished `.1b` test draft was inspected. Source snapshot and initial/final repository HEAD were `2d86483f03efa683e13df8d78bb1338f5170602f`; final status was clean when inspected before writing this owned temporary document.

| Read file | SHA-256 at read base |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `0981e53290d2599919b2b0028ceac85a04d1ec0a8e9b94c5f52d98793b808a2b` |
| `specs/014-workflow-primitives/plan.md` | `33580b895a52c01113eacaf9503b95b88d5cb7b0204136fae62206c2c62ecd91` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `docs/adr/ADR-0030-structured-instances-and-continuation-chains.md` | `004bdbfdf8b1616852189cec826a0cfaecd68cd287c404cc210d5979cae6852f` |
| `docs/adr/ADR-0031-declared-operation-result-disclosure.md` | `afd5abf1e0a220e732b6ffece6f5dba84d6fbd41b7ebd11e693ab4d23213c347` |
| `docs/adr/ADR-0032-additive-workflow-ir.md` | `87cbaeae372df88ed0e3aff1ca48baae6b6bbd4e2408d7ddd003ab92c9f8797d` |
| `tasks/project-completion/todo.md` | `79f37a78209b2e16c48b55e5f462d5217e342085d2970d932a79d745fd32839d` |
| `tasks/project-completion/leaf-child-source-preservation.md` | `74c39f356896ec202d36e4442e7150773d551f490beb14f4c25a6870d86fc197` |
| `tasks/project-completion/leaf-child-parent-flags.md` | `ef13a09ce9e6f171e9a1f0e9d3b3ad467e882bcd6df1dad0993393e6332ed7df` |
| `tasks/project-completion/leaf-current-producer-preservation.md` | `716870a727903c24c02f50675129f994193af0d3ab39832851013896f30e122f` |
| `crates/api-http/src/routes/execution.rs` | `fc89bfb3aeb17e1874522d2417074fa6c9d69acd8f264c5e4e0fd43dd35cfc7b` |
| `crates/application/src/runs.rs` | `4841c667904d275ce496fe8e18c488c00121ef56ad85620a2726118f6bc7e0d0` |
| `crates/application/src/worker/scoped.rs` | `5a3fdbaeb1ba59f46d98647988dc2aebd881fc7240cb3b5fe13cfe9cc0595c24` |
| `crates/application/src/worker/scoped_reports.rs` | `cc17ee9d62ded96b855fd6f2c864c4439bcedc59e6aa4b6a9b33abb694778ba3` |
| `crates/domain/src/execution/command.rs` | `55ca47d23f1df9129ccca3d8dc12a7f78ed2746935dabb5e1b161335ed7ba4f7` |
| `crates/domain/src/execution/plan.rs` | `d9c1c67ab83990be5cb89b9e7f1604b29b767a947917a94b5247522403091879` |
| `crates/domain/src/execution/state.rs` | `e802c45c2bdbad9da159c6f60ad28ab6406c3fa04ce17b473ba7cb63f71af0e5` |
| `crates/domain/src/execution/scoped_sagas.rs` | `45cd7bf76463d7b72f318f8a77402e7182cb0a3082af8cccb13a00964fc22e2b` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/maintenance.rs` | `ce11e42dba0f37de24d56631ebce14c61af00f6ed303b8d2689fa5eed0e3f1b5` |
| `crates/infra-postgres/src/primitives.rs` | `0c84e9a5e622e5b70135340de75c69e6e04c3e79ee1caf04cdd704c0eb027b75` |
| `crates/infra-postgres/src/scope_frontier.rs` | `aa15150795d321e6f91fea22fd14a63272ac4bbb28c8abf1b33553d7846d9de2` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `crates/scheduler/src/passes.rs` | `31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |
| `tests/integration/workflow-primitives/leaf_certification_child.rs` | `c797a0e6eb95e1db90ca93bf56137f408990c6bc4f1e07f664e1059ee3d532fd` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` | `679089900793c3e1949c834747aea66900f419f0116a508db61350b6cbb574c7` |
| `tests/integration/workflow-primitives/leaf_certification_later_work.rs` | `e00184bf3cd342965923aeeb9574ab4b69c57e69c4f67d1952ca07991a5bad1c` |
| `tests/integration/workflow-primitives/leaf_certification_saga.rs` | `2ebfd384293d235a1b719f2b6fa11e4640715c02e5b948c30fe6e2cc9c6a933b` |
| `tests/integration/workflow-primitives/leaf_certification_commit_loss.rs` | `ab20f174accfc5d0db192ce64b2bc29b5fc81c572bfbfc2a5119a6155b4a2e8b` |
| `tests/integration/workflow-primitives/bounded_fences.rs` | `750b2de312afc7fbe75d64455a083677cea9dfc6337a37e6e11971eee8f87259` |

## .1c acceptance checkpoint (2026-10-18)

Two real PostgreSQL tests pass in default mode (42.77 s wall / 4.74 s harness) and explicit runtime-role mode (5.30 / 4.66); same two names in both, no aggregate four-test claim. Actual three-PID witnesses establish both original-child writer COMMIT orders. After real public PROPAGATE, cancellation, drain and known-effect refund, selected source/certificate/audit remain unchanged and the complete original record replays cold. The child remains unfinished; End/result/fanout remain separate. Strict Clippy passed 11.69 s, fmt 4.36 s, architecture 0.34 s / 24 crates. Final 1108 source inputs unchanged. See evidence/ordinary-child-own-cancel-20261018 manifest/review/reproduce and retained initial failures. B1b.2c.1c closes only; parent umbrella, .1d/.1d.0 and canonical 014 tasks stay open.
