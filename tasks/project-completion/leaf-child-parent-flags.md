# B1b.2c.1b — Incoming parent flags preserve the ordinary child source

Status: bounded decision accepted after fresh Astra/xhigh C1 and C2 review. C1 required alignment with the real LEASED unfinished fixture; C2 found no remaining substantive issue. Root checked the public command transaction and exact legacy child UPDATEs. Baseline .1a passed actual default/runtime tests and was committed at8df0ae90; .1b execution now passes the focused default/runtime child suite (checkpoint below). The original design task executed no tests/builds/formatters, agents or repository mutations. Reviewed sealed draft:6c7b12d6d7d099ac7102623b5d5422909abd48c0640fb5108de06cc1c53baf41. Optional external review was declined by the user.

## Decision

Prove only the existing legacy incoming-parent flag family: PROPAGATE, DETACH and REQUEST_CANCEL against **first** native child-source certification. Six policy/order scenarios plus one real certifier-COMMIT guard scenario. Reuse the .1a genuine ordinary unchained-child fixture under its accepted characterization contract; baseline execution acceptance is recorded at8df0ae90. Reuse the existing OrdinaryLeafV1 command, identity, store and scheduler-authority types. No new profile, DTO, mapper, production behavior or public interface.

Use a root parent whose only active instance is its waiting Child node (`kid` here), followed by an unactivated End. The actual child has selected root path `lines/0` genuinely closed and `later` open with its forward activity LEASED and unfinished. Parent and child have no chain membership; no incoming child-chain link or outgoing child inside the child definition. Before each race require the actual parent/child association, declared policy, unfinished child, flags `(false,false)`, zero selected certification/audit and the .1a closure invariants. After the real later-activity lease is granted, capture its exact step/lease snapshot and one command with the resulting **child** revision and own child scheduler authority. Use that captured post-lease command unchanged before and throughout both competitors; do not use a pre-lease revision.

## Correct public entry and transaction

Correction to the generic warning in the accepted baseline document: for this fixture, **public parent CANCEL writes the flags in the same command transaction, before HTTP 202 returns**. There is no intervening scheduler tick. An awaited CANCEL before starting the certifier is already a completed parent-first case, not a race. `.1a` performs no CANCEL, so its baseline contract is unaffected.

Source route: `api-http/src/routes/execution.rs:935–980` admits the parent operation and calls `Runs::operate`; `application/src/runs.rs:1180–1260` takes the parent command transaction, invokes `stop_run`, commits and only then returns the parent record. `domain/src/execution/command.rs:96,129,1861` applies RunStopRequested and settles planner emissions immediately. `domain/src/execution/plan.rs:325–421` emits the policy-specific child event. `infra-postgres/src/execution.rs:4028–4047` appends that parent event and calls `release_child` before committing parent journal, projections, audit, idempotency and outbox.

The authenticated command uses the existing parent Run lock, not invented parent scheduler authority. `execution.rs:1130,169` performs applicable chain lookup, parent `RUN_FOR_UPDATE`, server time and parent restoration in that transaction; the root-parent chain latch is a no-op. The parent entry query begins `SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id,` and ends `FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE`. Its step restoration starts `SELECT node, instance_path_nodes, instance_path_indices, status, attempt, generation, cancel_requested, routed, condition, switch_case_id,`. Neither is the child writer. A path entry beginning `SELECT run_version FROM runs` is also not this producer's child UPDATE.

`execution.rs:5060` obtains the real reserved child via `SELECT child_run_id FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3`. It first tries `UPDATE child_chain_links ... WHERE tenant_id = $1 AND parent_run_id = $2 AND child_chain_id = $3`; fixture assertions establish the legacy fallback. Actual competing statements are:

```sql
-- PROPAGATE and DETACH, settle_child:5028
UPDATE runs SET parent_settled = true,
  parent_cancel = (parent_cancel OR ($4 AND finished_at IS NULL))
WHERE tenant_id = $1 AND id = $2 AND parent_run_id = $3
-- $1=tenant, $2=child, $3=parent; $4=true for PROPAGATE, false for DETACH.

-- REQUEST_CANCEL, release_child:5090
UPDATE runs SET parent_cancel = true
WHERE tenant_id = $1 AND id = $2 AND parent_run_id = $3

-- Native source certifier, leaf_certification.rs:61
SELECT workflow_version_id,run_version,trace_id FROM runs
WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE
-- $3 is the original child, never the parent.
```

These flag UPDATEs append no child event and change neither child run_version nor updated_at. The parent journal/revision legitimately advances. No child CAS conflict follows from these flags: **all six first certifications must succeed with the same captured expected child revision**, `replayed=false`. Do not refresh the revision after CANCEL to hide a wrong expectation or fabricate SourceNotClosed/StaleRunRevision.

## Required positive oracles

For the minimal single-active-child parent, assert this exact delta after both operations commit:

| Policy | Parent Run | Parent `kid` status / waiting_kind / cancel_requested / routed | Child parent_cancel / parent_settled | Parent journal suffix |
| --- | --- | --- | --- | --- |
| PROPAGATE | CANCELLED, finished_at set | CANCELLED / NULL / false / true | true / true | run.stop_requested, step.cancelled(kid), run.finished(CANCELLED) |
| DETACH | CANCELLED, finished_at set | CANCELLED / NULL / false / true | false / true | same suffix |
| REQUEST_CANCEL | CANCELLING, finished_at NULL | WAITING / CHILD / true / false | true / false | run.stop_requested, step.cancel_requested(kid) |

Parent `kid.child_run_id` still equals the actual child (the step update preserves it with COALESCE: `execution.rs:5886`); the successor End was never activated and stays absent. Require stop_reason CANCELLED, one corresponding command audit/idempotency record, HTTP 202 with the matching committed status/revision and no child.finished event. Parent revision increases by the exact suffix length. RunFinished/outbox is present only in the first two cases; do not demand all parent rows, tenant usage or outbox remain unchanged. The exact state follows `state.rs:3770–3840,4386`, `RunWorkSummary::status:426` and `end_status:1517`, not a guessed HTTP status.

Keep three independent snapshots:

1. **Selected immutable source:** exact scope/iteration/checkpoint/owner/local steps, all five referenced journal facts, selected reserve0 and saga absence, pinned definition/source identity. Use the accepted selected-source oracle; neither flag policy changes it.
2. **Child execution:** full own journal and revision unchanged, no own stop request/drain, unchanged status/finished_at/updated_at and later LEASED, unfinished activity. Preserve the exact captured later step/lease snapshot, including its attempt, generation, token hash, owner and lease/deadline metadata; original ancestry/policy unchanged. Compare the Run excluding only the two flags, then assert their exact positive values separately. No whole-Run equality assertion that hides legitimate flags.
3. **Certification:** exactly one qualified canonical record and its matching certification audit. Correct child identity/profile, original source references and captured revision; same identity cold-store replay returns the same complete record/audit, with no second metadata write. Parent journals/audits are separately qualified and may grow.

## Six deterministic queue scenarios

Use a new disposable fixture for each policy and each order. Test helpers accept explicit parent/child Run ids. Preserve the test-only source INSERT grant; assert the calls themselves use `sagat_app`. Stop manual scheduler driving once the source is prepared: no tick/drive, worker poll/complete, child cancellation or fanout during these cases.

1. In an owner observation transaction, capture backend PID **H**, then lock the exact qualified original child `FOR UPDATE` (the child-specific equivalent of existing RunHold). Capture baseline snapshots, including the exact later step/lease snapshot, and the post-lease command beforehand; retain this hold.
2. Construct but do not poll the second operation. Locally pin the real HTTP parent-CANCEL future and native certifier future. `Flow` is not Send; do not spawn a task capturing it. Poll only the desired leader until its exact competing SQL is blocked by H.
3. Poll both futures until the actual queue direction is visible. Record distinct H/leader/follower PIDs, database, role, query, wait_event_type and blocking PIDs. Require both competitors to be `sagat_app`, waiting on Lock; H must block the leader, and leader must appear in `pg_blocking_pids(follower)`. The leader/follower SQL must match the policy UPDATE (including exact tenant/id/parent predicate) and certifier SELECT in the requested order. A count of two blocked sessions, the parent's initial full restore query, or its zero-row link attempt is insufficient.
4. While blocked, independent reads still show original flags and no visible new selected certificate/audit. A separate parent `FOR KEY SHARE NOWAIT` probe must fail with 55P03, proving the producer owns its parent Run while queued at the child. Every failed NOWAIT probe uses a fresh transaction/autocommit connection.
5. Roll back H; locally join both futures under a bounded deadline and require both successes. Assert all positive oracles above, including unchanged child revision and original-command first certification. Reconstruct a fresh store/cache for canonical replay.

Observation loops use bounded timeout plus yield_now, never sleeps or elapsed-delay inference. Fail if a competitor completes before the required witness or if the witness is absent. Match full stable statement fragments rather than a generic `UPDATE runs`; record PIDs/queries as evidence without payloads. SQL text does not expose bind values: exact held child identity, source/relationship preconditions, observed dependency on H and positive target-row changes jointly establish the target witness.

## One actual COMMIT-lifetime case

Choose PROPAGATE. Reuse the existing *test-only* deferred trigger design in `leaf_certification_commit_loss.rs:11`: AFTER INSERT on the certification table, DEFERRABLE INITIALLY DEFERRED, calling pg_advisory_xact_lock on a generated key and returning NEW. An owner transaction holds that key (PID G). No network loss proxy, forced abort or production hook is needed.

Poll the first certifier on an isolated one-connection runtime pool with a unique application_name. Wait for its captured PID C to have `query='COMMIT'`, wait_event_type Lock and G among its blockers. Require certificate/audit externally invisible. Only then poll the real parent HTTP CANCEL until PID P is blocked on the exact legacy child UPDATE with **C** among its blockers; C must still be executing COMMIT behind G. Require G/C/P distinct and keep polling both local futures while observing.

While this chain is held, probe the exact child Run `FOR KEY SHARE NOWAIT` →55P03; its scheduler shard `FOR NO KEY UPDATE NOWAIT` →55P03; same shard `FOR SHARE NOWAIT` →success. Probe the parent Run as above. This proves original child FOR UPDATE and own scheduler SHARE survive through actual COMMIT while the real incoming writer waits. It is not a retained-quota wait or a promise about capacity G. Independent parent/child reads still show pre-CANCEL state; no new parent command journal/audit or source metadata is visible.

Release only the advisory gate, join both successes, assert positive PROPAGATE oracles and original-record replay. Remove the test-only trigger/function and close the dedicated pool. Do not terminate the backend or release a substitute RunHold to claim this proof. No sleeps; use the same PID-based bounded observation loop, not a helper that silently sleeps.

## Ownership, verification and boundary

Order after fresh review:

1. Freeze this family in the existing contract and retain the already allocated unchecked `.1b` ledger item; correct its earlier CANCEL/tick warning. At most three documentation files if an active-plan pointer needs updating.
2. After .1a helper ownership is sealed, add one focused parent-flags test submodule and register it from the existing child module. Reuse its genuine creation/closure and source readers, making only necessary test-local explicit-id/policy parameters. Own at most: child module, new submodule, contract, one evidence file, todo (five). No shared-harness refactor or production edit.
3. Independent review checks witnesses, policy/source oracles, guard lifetime and unchanged production gates. Root executes default and least-privilege focused tests on the integrated tree and records exact names, commands, output and failures. Use the existing integration_workflow_primitives target with the registered child-parent-flags module filter; do not assume unfinished helper names. Recheck the accepted .1a genuine-certificate consumer-refusal control with the focused child suite. No executed result is supplied here. Existing behavior may pass unchanged; do not invent RED through a false refusal.

This may close `.1b` only. Own-child cancellation/drain and later compensation (.1c), terminal fanout (.1d), detached later execution, general B1b.2c, B1/B2, C0/mixed writers/retention/backfill, nested progress/resource acceptance and canonical 014 tasks remain open. ADR0030's legacy parent→child-row protocol is unchanged; ADR0031/0032 boundaries remain. Native source acceptance already exists; this adds evidence in one parent context. Scoped Finish still rejects every parent/chain context before source/receipt, including replay (`scoped_leaf_finish.rs:357`). Source/scoped INSERT release grants, production callers and capacity transition authority remain closed.

## Read provenance

Mandatory AGENTS/constitution, 014 spec/plan/tasks and ADR0030/31/32 were read in the preceding accepted design; current 9b61b3e4 spec/plan/todo deltas were read here. Production entry, reducers, writers, source/consumer guards and existing barrier tests were read at the cited functions/excerpts. Hashes identify whole files, not exhaustive reads or test evidence. No unfinished child helper was inspected.

Read/hash HEAD: `9b61b3e40a6e5bcb8bac93a3543abf9771f6cc44`. Repository: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`. Concurrent baseline changes appeared during the final hash snapshot: native-module registration modified; child helper and its baseline evidence directory untracked. They are another agent's work. The native harness hash below is the committed version read before that registration edit; the unfinished child helper was not read. All other hashes identify the current files at this snapshot.

| Read file | SHA-256 |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `ebbe7ab5e391ed6a35d7e630ac13ecbe569bf4a8dd95cec07a9bea5c4f973d37` |
| `specs/014-workflow-primitives/plan.md` | `4dc93225071eb799cf1de2bf800e2e064ad8d4899ad9a1ef81ded850a9d7cb74` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `docs/adr/ADR-0030-structured-instances-and-continuation-chains.md` | `004bdbfdf8b1616852189cec826a0cfaecd68cd287c404cc210d5979cae6852f` |
| `docs/adr/ADR-0031-declared-operation-result-disclosure.md` | `afd5abf1e0a220e732b6ffece6f5dba84d6fbd41b7ebd11e693ab4d23213c347` |
| `docs/adr/ADR-0032-additive-workflow-ir.md` | `87cbaeae372df88ed0e3aff1ca48baae6b6bbd4e2408d7ddd003ab92c9f8797d` |
| `tasks/project-completion/todo.md` | `869b2508f2a33e56cfacb53f9d6013d9d65e0ac95853de49c690f034d311089b` |
| `tasks/project-completion/leaf-child-source-preservation.md` | `0c91e479bb3386672e2d58a8315d3c49a1d6ef4fdce0cbbf2522becb053b1144` |
| `crates/api-http/src/routes/execution.rs` | `fc89bfb3aeb17e1874522d2417074fa6c9d69acd8f264c5e4e0fd43dd35cfc7b` |
| `crates/application/src/runs.rs` | `4841c667904d275ce496fe8e18c488c00121ef56ad85620a2726118f6bc7e0d0` |
| `crates/domain/src/execution/command.rs` | `55ca47d23f1df9129ccca3d8dc12a7f78ed2746935dabb5e1b161335ed7ba4f7` |
| `crates/domain/src/execution/plan.rs` | `d9c1c67ab83990be5cb89b9e7f1604b29b767a947917a94b5247522403091879` |
| `crates/domain/src/execution/state.rs` | `e802c45c2bdbad9da159c6f60ad28ab6406c3fa04ce17b473ba7cb63f71af0e5` |
| `crates/domain/src/execution/event.rs` | `e740724607e7c5ad1a9dfe89c3fc1387d3ec00144453fff98f3fa75a7a8e64a6` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `crates/scheduler/src/passes.rs` | `31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |
| `tests/integration/durable-execution/children.rs` | `9a8e80d16128d9bb8c3af2ee2214f8572e5d65d82efb19770e7904f9bdb0dcb7` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` (committed version read before concurrent registration edit) | `468e0505b935e3696e5ee0aa28d32f51d338e23eda5241b0d5bb7361b454f14d` |
| `tests/integration/workflow-primitives/leaf_certification_late_reports.rs` | `4d4fba7f2e0e8647b220678890eba55c74095b704136cd71fc6a8d2d81f75e7e` |
| `tests/integration/workflow-primitives/leaf_certification_commit_loss.rs` | `ab20f174accfc5d0db192ce64b2bc29b5fc81c572bfbfc2a5119a6155b4a2e8b` |


## Actual incoming-parent flag preservation — 2026-10-18

Real public parent CANCEL competes with first native certification of a genuine ordinary unchained child under PROPAGATE, DETACH and REQUEST_CANCEL in both observed child-row queue directions. Exact three-PID role/database/query/wait/blocker witnesses, parent Run conflict probes, original post-lease child CAS, policy-specific parent journal/status/step/flags, atomic audit/dedupe/outbox and immutable selected source/full child core/journal/steps/exact later lease are asserted. All six first certifications succeed without changing the captured revision. A seventh fresh fixture pauses the actual certifier COMMIT behind a deferred advisory gate; the real parent UPDATE waits behind that COMMIT while child Run, parent Run and child scheduler SHARE guards remain held. Releasing only the gate commits both. After revoking the disposable source INSERT grant, a cold runtime store/cache returns the exact original complete record/audit and preserves the full snapshot.

Actual focused child suite:4/4 default in53.65s wall/16.71s harness and4/4 runtime-role in17.43s wall/16.78s harness. These are the same four named tests in separate invocations:two existing baseline controls and two new tests executing six policy/order fixtures plus one actual COMMIT fixture. Seven asserted three-PID observations are retained per mode. Strict integration Clippy11.27s, fmt4.33s and architecture0.36s/24 crates passed. All1105 frozen executable/contract inputs match.

Local B1b.2c.1b closes only this incoming legacy flag writer family. No production code, grant, caller, profile, schema or capability changed. Later remains LEASED and unfinished; no own-child cancellation/drain, detached later progress or terminal fanout is claimed. The parent flag writers do not advance child revision/journal/updated_at. B1b.2c.1c/.1d/.1e and the master .1 remain open, as do broader B1/B2/C0, production activation, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044. Initial default invocation passed the two baseline tests and failed both new tests at the shared uppercase dedupe expectation; the six-cell loop then stopped in its first cell. Initial Clippy failed two test-only lints. All original records/source pins are retained separately, and no partial run is combined with the final suite.
Evidence: tasks/project-completion/evidence/ordinary-child-parent-flags-20261018/manifest.json.
