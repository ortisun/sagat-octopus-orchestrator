# B1b.2c.1d.0Q — collection results, legacy accounting and boundary refusal

Status: **C1 findings amended for C2 review; design selected, implementation and acceptance pending.** This resolves the design choice requested by section 6 of the collection End result decision. It is not acceptance of C1, the correction, child fanout, capacity or a release.

Repository read only: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`. Initial decision baseline was HEAD `6c44a0e49670cf19660792cd1518d241e6c99b3b`; the amendment examined root-owned work through HEAD `dc504a75e7b957e50676111378e3d0c88cf222a7` (initial amendment assignment HEAD980baabb). The authoritative parent is the **C2** `/tmp/sagat-collection-end-result-decision.md`, SHA-256 `3a44b44afb76654e9a7e6fcef9af74e97b0ab595da3cf1137f536c32f86dcc01`, particularly section 6 and its ordinary-reducer End evaluation correction. The corresponding frozen repository contract is `tasks/project-completion/collection-end-result-correction.md`; its observed content SHA at amendment seal was `0003c9c71c57add545b7efc8f9e6b9c500dbbcb8611020fe162f16249fb462b9`. The old parent SHA0e977 is superseded. The original Q decision is preserved byte-exact in `/tmp/sagat-collection-result-quota-decision-c1.md`, SHA-256 `4c4a0ca6de269283a47c4b665b62038a4fd850c8143a55b20598eb397126c9bf`. Section 11 is normative for both C1 amendments. This agent wrote only the authorized temporary decision artifacts and ran no build, test, formatter, migration or agent. Root owns all repository changes and test execution.

## 1. Selected correction and boundary

1. Preserve the stored `run_instance_usage.logical_output_bytes` ledger's current meaning: bytes of actual step-output references. Introduce a **read-only effective total** adding the bytes of verified successful root-collection aggregates whose real owner step still lacks an output. Use the same predicate for expression overlays, preflight and both full/path persistence guards. Never increment the ledger for that historical overlay, backfill an old owner or change a certificate, checkpoint, journal, old End result or closed Run.
2. For a fresh successful finish, retain the aggregate as its owner's actual output and charge that reference exactly once. Advance the successor through the ordinary reducer and evaluate/retain End only if it actually requests `Evaluation::Output`, in the same transaction, as the corrected parent C2 decision specifies. A reducer stop such as INSTANCE_LIMIT must precede any End evaluation. The scope aggregate and owner output are two storage references for **one owner logical output**; equal hashes on different step instances are separate logical outputs. The End output and `runs.result_digest` are similarly one End logical output. Physical tenant content remains content-addressed and charged by the existing payload trigger.
3. Add exactly one internal drain reason, `CollectionDrainReason::RunDataLimit`, for an entered collection that must stop at its own boundary without blaming a member. Its fixed failure code is the existing `RUN_DATA_LIMIT`. It covers a final aggregate that cannot fit, an active legacy Run whose verified effective retained outputs already exceed 4 MiB, and an entered scope winding down an authoritative FAILED/RUN_DATA_LIMIT stop after a successful worker payload was refused (section 11). Keep the existing last-pending-Return `Failed { iteration, code }` behavior whenever it genuinely applies.
4. Keep this reason on the existing drain/saga/finish path. An empty scope has zero members to drain; checkpointed members remain checkpointed. Completed effectful members still undergo the existing scoped compensation plan before finalization. A data limit never converts their successful checkpoints into failed iterations.
5. Introduce no new public operation, workflow revision, runtime shape, caller, flag, grant, profile or capacity mechanism. Deployment is a readers-first, operationally quiesced writer upgrade, described below. The internal new reason maps to the existing public `FAILED` collection-drain hint, so no new AsyncAPI reason enum is needed.

These are **unconditional corrections to current accepted behavior** under G21/D36, FR-014-004/005/014/015/016/017, D314/D315 and ADR-0030/0032. They do not depend on a choice to enable nested execution or capacity groups. The owner's already recorded 256 KiB/4 MiB limits settle the policy. C0, historical backfill, mixed capacity writers, nested finality and production capacity ceilings remain separate undecided work. No new product decision is needed for this correction.

## 2. Why an existing reason cannot represent this failure

| Inspected source | Consequence |
| --- | --- |
| `domain/src/execution/state.rs:2765`, `2021`; `infra-postgres/src/primitives.rs:1856-1916` | `Failed { iteration, code }` requires a real ADMITTED member, with a failed step or an actual pending Return after its entry succeeded. The domain also rejects a checkpointed member as its failure origin. Empty and fully checkpointed scopes cannot use it. |
| `primitives.rs:1848-1855`; `state.rs:2922` | `Cancelled` means the actual enclosing CANCELLED stop, not a failed Run with a data-limit code. |
| `event.rs:424`; `primitives.rs:1791-1804,1833-1844,1917-1949`; migration `20261106000000` | `StorageQuotaExceeded` is pinned to `STORAGE_QUOTA_EXCEEDED`, physical tenant content quota and `RunStorageQuotaExceeded` audit. Reusing it would lie about the failure and effect uncertainty (`event.rs:99-111` treats storage refusal as potentially unknown). |
| `state.rs:2882-2928`; `scoped_sagas.rs:64-87`; `scheduler/src/passes.rs:1249-1392` | An entered collection must finish from its scope lifecycle and completed saga, not by a bare owner `StepFailed` or a manually forced Run status. A general `stop_run(FAILED, RUN_DATA_LIMIT)` alone leaves no truthful collection drain reason when no member failed. |

Changing `Failed` to accept a missing/fictitious iteration would still change durable decoding and weaken a useful invariant. The single explicit reason preserves that invariant and uses the existing lifecycle. No broader “arbitrary boundary failure” enum or expression-failure redesign is selected.

Root's actual reported RED is 0/1 for `collection_end_result::collection_end_retains_owner_aggregate_and_evaluated_run_result`: authoritative aggregate `[{"amount":17}]`, actual owner/End/Run outputs `(None,None,None)`, expected aggregate/scalar17/scalar17. The test source checks a genuine public producer and existing SUCCEEDED journal. I did not rerun it. The quota regressions below are planned, not claimed results.

## 3. Exact read-only overlay and effective bytes

Let `L` be the stored logical-output ledger. Let `M` be the set of verified successful **root** collection owners in this Run whose `steps.output_digest IS NULL`. Let `V = sum(octet_length(aggregate_payload.content))` over **owner identities** in M, not distinct digests. Define `E = L + V`, using checked 64-bit arithmetic. Never clamp E to 4 MiB. Old writers can have committed E above the limit; that is readable historical data, not a migration failure or permission to add output.

A shared crate-private PostgreSQL helper owns the candidate predicate and metadata checks. It is a read/query helper, not a durable table, backfill, new public port or generic DTO layer. Qualify every lookup by tenant, namespace and original Run, then full owner path/node. Require:

- The Run belongs to that namespace and its pinned definition actually has this root ForEach owner. No node-name-only or nested-path inference.
- Scope kind COLLECTION, lifecycle SUCCEEDED, root owner path, matching unique owner identity, all admissions/completions equal item_count, active_count zero, no drain fields, non-null aggregate digest/finish sequence/original finish outbox identity.
- Matching actual owner step is SUCCEEDED, routed, no waiting kind, same full identity/namespace. Missing owner/payload or inconsistent successful metadata is corruption, not absence of an overlay.
- The exact event at `finish_sequence` is the matching root `collection.finished`, successful with the same aggregate digest. Its actual instance id/path/node match the pinned owner. The event at `finish_sequence + 1` is matching owner `step.routed` to its pinned successor. Use bounded projected detail fields, as the existing source-certifier does (`leaf_certification.rs:552-650`); do not hydrate the whole journal or invoke/create a certificate. The outbox row itself need not remain after delivery; the retained original identity is sufficient.
- If the owner has an output digest, it must equal the scope aggregate digest. Use that real step output and add zero overlay bytes. A contradictory present digest is corruption, never overwritten or counted twice.

Byte preflight fetches scalar lengths and bounded metadata, not every aggregate body. FKs already bind `(tenant_id,result_digest)` to payloads; full content hydration additionally verifies SHA-256 against the retained digest and valid JSON. The byte total is the size of actual stored content even before that hash check. An absent/corrupt payload must not be filtered out by an inner join and silently reduce E; detect it explicitly. The helper never re-evaluates historical member expressions.

When E is at most 4 MiB, `PgRunTx::data()` adds each missing owner aggregate under its exact `ExecutionInstanceId`, without writing storage. The resulting complete `RunData.outputs` sum equals E before new outputs are staged. End sees old `steps.lines.output` and the just-staged current owner aggregate. It must distinguish JSON `null` from an absent value.

**Boundedness when E is already too large:** compute E before hydrating the legacy aggregate overlay. Add `RunTx::logical_output_bytes()` with the same documented effective meaning as the existing PathExecutionTx method; PostgreSQL overrides it with the scalar helper. A compatibility default for test/in-memory adapters may sum their complete RunData. This is a cohesive budget read, not another execution protocol. The full scheduler must inspect collection lifecycle/E before calling `data()`: terminal scopes need no data; a draining scope progresses metadata/sagas without a global overlay; an unentered over-budget owner uses the existing pre-entry limit stop; an entered OPEN over-budget scope uses the new `RunDataLimit` drain. Do not make a 2 GiB legacy overlay just to discover a 4 MiB ceiling.

An unexpected request for full RunData while E exceeds the bound returns the existing typed `StoreError::QuotaExceeded { what: "output retained by one run" }`, matching current application quota wording; it does not classify the valid historical Run as corrupt. Scheduling/refusal paths must consume the scalar first so this is not their normal retry loop. Existing metadata inspection, journal replay/verify_run, result disclosure and exact local scoped contexts continue without this full hydration. A closed old Run still exposes its recorded `runs.result_digest` only: never infer its result from the overlay.

Exact-path worker contexts remain local; **do not inject root legacy aggregates into scoped expression/worker context**. `PathExecutionTx::logical_output_bytes()` includes E for the whole Run while its RunData remains path-local. `ScopeFrontierStore::retained_output_bytes()` must also return E. Existing collection/loop shape restrictions stay unchanged.

## 4. All write guards use the same arithmetic

Sources: migration `20261204000000_logical_output_byte_ledger.up.sql` seeds L from step references only. `execution.rs:3044-3107` charges full writes; `execution/path.rs:639-695` charges exact-path writes and currently reads L alone at lines 6-15. `scope_frontier.rs:506-525` currently sums actual step outputs. `application/worker/output.rs:279,476`, `runs.rs:1522`, `chain_progress.rs:886`, scheduler `passes.rs:349,2000,2518,2781` and ordinary End evaluation `application/execution.rs:1544` contain affected preflights.

- A new physical step-output reference of size d increments **L** by d. Its transactional guard is `L + V + delta <= 4194304`, where delta is the sum of newly introduced step-output references, counted once per exact instance. Equal hashes do not reduce delta. Existing output/replayed references have delta zero.
- Both full and exact-path store guards use the helper under their existing own-Run lock; the runtime role receives no new grant. Application preflights use the same effective total. Successful scoped report retention refusal uses section 11's truthful success-plus-stop treatment; do not preserve its current lossy `fail_path_instance(RUN_DATA_LIMIT)` behavior. Replace full-writer calls that hydrate RunData only to count it with the scalar method. End's shared helper may use the complete bounded overlaid/staged RunData sum; it must include staged aggregate bytes once.
- Zero-delta metadata, cancellation, drain, saga and replay operations remain writable even when historical E exceeds 4 MiB. Compensation output is already discarded (`worker/output.rs:258-275`) and must remain uncharged. Do not block accepted effect resolution by a generic `E > limit` assertion on every transaction.
- In the fresh collection-finish transaction, calculate/charge new owner and End outputs while its scope is still OPEN and its owner output absent. Persist their step output projections before marking that scope SUCCEEDED. Then it can never enter M as a provisional historical aggregate. Everything still commits atomically. Do not subtract the candidate size from V by digest or leave a transient “newly succeeded but missing output” projection for a later quota query. Existing own-Run serialization permits this ordering.
- A fresh successful aggregate must satisfy both 256 KiB and E + aggregate_size <= 4 MiB **before** any aggregate payload INSERT. Stage it for End evaluation. If aggregate fits but End does not, collection succeeds and End fails through the normal reducer. If aggregate does not fit, collection itself enters the new drain and no aggregate/owner/End/Run result is persisted.
- Physical tenant content failure remains the existing payload-trigger failure/whole-transaction rollback/current-revision storage-quota fallback. Never map the new logical guard to that path. Runtime source CAS mismatch discards all calculations and retries from a fresh snapshot.

This correction accounts for the missing owner outputs and their later consumers. It does not redefine snapshots, loop carries, receipts, blob reservation or capacity accounting. Preserve their existing independent quota checks; do not claim this is a new proof of every ADR-0030 capacity clause.

## 5. Durable reason, transient evidence and lifecycle

### Exact vocabulary

- Domain enum: `CollectionDrainReason::RunDataLimit` (no member, arbitrary code or string payload).
- Journal event remains `collection.drain_started`, with exact new detail `{"reason":"run_data_limit"}`. Decoder derives `FailureCode::run_data_limit()`. No payload digest/body is retained for this refusal.
- Projection: `drain_kind='RUN_DATA_LIMIT'`, `drain_iteration=NULL`, `drain_failure_code='RUN_DATA_LIMIT'`, positive recorded sequence and original outbox UUID. Lifecycle is DRAINING then FAILED, with `result_digest=NULL`.
- Start: ordinary `run.stop_requested {reason:"FAILED",code:"RUN_DATA_LIMIT"}`, then collection drain event, applicable real cancellation requests, same atomic audit/outbox/revision/telemetry head. Existing prior cancellation/timeout/failure wins; a stale attempt must not replace it.
- Finish: existing `CollectionOutcome::Failed(FailureCode::run_data_limit())`, owner FAILED with that code and no output. Existing scope saga owns compensation, irreversible/unknown/manual-review behavior and final Run status (FAILED, COMPENSATED, PARTIAL, etc. as already determined). No fabricated iteration-drained event for checkpointed/empty members.

### Revalidation without SQL expression evaluation

The scheduler is the trusted bounded evaluator; SQL must not evaluate expressions. Extend the **existing** CollectionDrain command with optional private typed evidence, required exactly for the new reason:

`CollectionDataLimitEvidence::ExistingOutputs`, `CollectionDataLimitEvidence::StoppedForRunDataLimit`, or `CollectionDataLimitEvidence::Aggregate { byte_len: u64, source_digests: Vec<(u32, Digest)> }`. The additional StoppedForRunDataLimit case is specified in section 11; it carries no arbitrary caller code or payload.

This has independent semantics: it distinguishes an already-over-budget historical Run from refusal of a particular final aggregate. Construct Aggregate evidence from the actually serialized candidate and existing ordered checkpoint digest bindings; do not accept a caller-entered length through any public API. The candidate bytes are not persisted or logged. Normal successful CollectionFinish already binds these source digests; reuse that verification logic, not an evaluator inside the adapter. Keep the evidence constructor bounded and reject duplicate/unsorted source indices. The ordinary `CollectionDrain::new` remains for existing reasons; a new checked constructor supplies this evidence and the required engine audit.

Under existing chain-before-own-Run entry, scheduler fence and exact revision:

- StoppedForRunDataLimit is valid only for an OPEN scope under the exact authoritative stored stop `(FAILED, RUN_DATA_LIMIT)`, with native Run revision/fence validation; it never creates or replaces a stop and does not require E itself to exceed the ceiling. It covers E <= ceiling when a real successful report's E + new output exceeded it.
- ExistingOutputs is valid only when freshly recomputed E > 4194304 and the scope is OPEN. This permits stopping an old active scope before more work; actual admitted members drain normally.
- Aggregate requires the actual terminal-ready OPEN scope: all members admitted/checkpointed, no active/held slot, exact ordered stored source digests and current pinned owner. Recompute E; require `byte_len > 262144 OR E + byte_len > 4194304`. Empty scope additionally requires byte_len=2 and an empty source list, exactly JSON `[]`. Nonempty byte_len is the trusted evaluator's typed answer, protected by immutable definition/source digests and Run CAS, exactly as other evaluated commands are; this is not a claim that SQL independently re-executes the expression.
- Wrong audit, reason/evidence mismatch, a candidate that fits, a missing/stale source binding or different Run revision refuses without writes. Never use an internal store error as a successful workflow failure.

The scheduler retains the existing last-Return preflight while a genuine pending member exists. The new path handles empty/final-checkpoint states and already-over-budget legacy execution. Metadata-only replay compares original scope reason and outbox identity before fresh budget/source checks, following current drain replay behavior; it never re-evaluates or recharges. Replay need not retain candidate byte_len: the original committed drain identity/outcome is authority, not an instruction to retry the candidate. A different reason/key is a mismatch.

### Readers to change before production emission

- Domain `event.rs`: enum and event cause (`failure`, `RUN_DATA_LIMIT`); `state.rs`: restored lifecycle/expected owner code, apply and finish match; `scoped_sagas.rs`: cause is RUN_DATA_LIMIT. Keep `Failed`'s member validation intact. `path_state.rs` already carries the generic reason; verify scalar restoration/replay without adding a new parallel lifecycle.
- PostgreSQL `event_codec.rs` encode/decode plus negative fixtures; `execution.rs` full collection restoration; `execution/path_read.rs` local restoration; `primitives.rs` replay, start/finish drain mapping and same-transaction audit/hint. Exact iteration drain retains its current rule that only `drain_kind='FAILED'` can label a real failure entry (`primitives.rs:2221`); RUN_DATA_LIMIT never impersonates that case.
- Scheduler consumes the restored reason through the existing drain and scoped saga flow; the later producer change selects it only with the evidence above.
- Journal inspection, runtime graph and telemetry consume the updated event codec/domain cause. Unknown-event telemetry fallback is not a compatible execution reader and cannot substitute for the reader upgrade.
- Leaf certification remains success-only (`leaf_certification.rs:321`); new failed/draining scopes cannot be certified. Earlier successful certified owners are only read by the overlay and remain byte-identical.

## 6. Additive schema and exact rollout/rollback

Allocate a new migration timestamp only after root checks parallel branches. Do not edit applied migrations `20261103000000`, `20261106000000` or `20261204000000`. The new up migration replaces only `iteration_scopes_drain_reason` to add the closed RUN_DATA_LIMIT branch above. Make the complete predicate total (`... IS TRUE`, with explicit non-null fields in each required branch) so SQL UNKNOWN cannot admit a null discriminator/code. Preserve every valid old OPEN/SUCCEEDED/FAILED/STORAGE_QUOTA/CANCELLED form. A pre-existing contradictory row fails migration visibly; no automatic repair. New reason has no new columns, FKs, index, grant, profile or capability row. Existing same-tenant scope/run/payload FKs, owner unique identity, drain/finish outbox uniqueness, terminal-result checks and immutable source guards stay intact.

Update the storage-classification evidence if it names the enum explicitly; its protected-system category and bound do not change. Audit action is stored as current bounded system text, not a new user-defined value.

**Reader bridge A:** assemble a build that can decode, restore, inspect, compensate and finish the new reason, contains the effective-accounting/overlay corrections, and has no scheduler call site that emits it yet. The staged source commits below are not separately deployable until this whole bridge passes. A merely tolerant telemetry reader is insufficient.

**Producer B:** contains the reviewed collection result correction and selection of the new boundary reason. No caller flag enables it: the exact existing root collection capability and retained facts select behavior. A and B keep workflow IR v1 and worker minor 1.5. There is no meaningful per-definition capability bit distinguishing these bug fixes, and an old binary would not honor a newly invented bit.

Deployment gate: use a maintenance window to stop execution writers and new starts/publications while rolling A, then B. Verify all old control-plane/worker-report/scheduler/maintenance readers and writers have stopped and their DB sessions/transactions have drained; inventory the actual process/artifact hashes. Apply additive schema, install compatible readers, then install **all** corrected execution writers before resuming. Read services may operate on A; no mixed old finalizer is permitted to complete a collection during cutover. This operational gate uses existing process/start controls and introduces no production runtime flag or grant. The stopped interval is explicit; no rolling zero-downtime guarantee is claimed.

Rollback of B retains compatible readers and schema. A may inspect retained new reasons; it is not an acceptable replacement active finalizer because it still omits End results and lacks the new refusal producer. Keep affected execution writers stopped until a compatible corrected writer is restored; do not continue old buggy production merely because decoding works. Disabling new publication/start alone does not protect active Runs. Do not downgrade to a pre-A reader while new reason events are retained, even after every affected Run is terminal.

The down migration locks `runs` ACCESS EXCLUSIVE first, then `iteration_scopes` and `run_events` ACCESS EXCLUSIVE, in one migration transaction, following the existing own-Run-first downgrade pattern (`20261211000000_leaf_success_certification.down.sql`). Under these locks refuse if **any** scope has `drain_kind='RUN_DATA_LIMIT'` or any retained `collection.drain_started` event has reason `run_data_limit`. If empty, restore the exact prior constraint. Do not remove source rows/events to make down succeed. Schema down eligibility is not binary rollback authorization: bug-correction/accounting compatibility and operational writer inventory remain separate gates. D298 retains history, so new durable events ordinarily make old-reader rollback unavailable indefinitely.

Real compatibility tests must exercise A/current reads and guarded down/up on actual produced records, not replace an old enum arm with a permissive stub. This bounded result upgrade does not accept arbitrary old/mixed capacity writers, C0 backfill or retention redesign.

## 7. Wire, audit and observations

- Public failure remains `RUN_DATA_LIMIT`, with current typed step failure and Run stop conventions. No OpenAPI Error.code, worker protocol, endpoint or SDK builder change. Existing optional result/JSON-null behavior and current classified Result disclosure remain as in the parent decision.
- Existing AsyncAPI `CollectionDrainStarted.reason` accepts FAILED, STORAGE_QUOTA, CANCELLED (`events.asyncapi.yaml:602-620`). Map new internal RUN_DATA_LIMIT to **FAILED**. Keep SQL/journal discriminator separate from hint reason; `primitives.rs:2070` currently reuses `kind`, so change that mapping explicitly. Hint contains identifiers/trace only, no iteration, candidate size/digest or payload. CollectionFinished remains FAILED. Add mapping assertions to the existing hint-contract test and document the internal/public mapping; do not enlarge the public enum.
- Add the truthful engine action `AuditAction::RunDataLimitExceeded` -> `run.data_limit_exceeded`, `ResourceType::Run`, exact Run id, tenant/current engine actor and original trace, fixed reason `collection aggregate data limit exceeded` (or fixed `retained run outputs exceed data limit` for ExistingOutputs). It does not collapse repeats; the successful drain identity guarantees one audit. Require it like the existing physical-storage drain requires its own audit. Same transaction, no new resource type or free-text payload. Replay emits no second audit. Do not write a RunFinished audit at drain start.
- The domain event cause supplies closed RUN_DATA_LIMIT. Existing journal-derived collection Draining/Failed and scope compensation observations remain truthful; existing `WorkflowPrimitiveRejectionCode::from_reason("RUN_DATA_LIMIT")` maps to the contracted bounded rejection series. Increment that counter only after acknowledged first drain commit, not on replay, rejected CAS, rollback or lost acknowledgement. Durable journal remains authority. No IDs in metric labels, no output/expressions/candidate data in logs or spans.
- Mandatory audit/outbox/telemetry_pending head commit with the transition; injected audit/outbox failure rolls back the whole refusal. A subsequent genuine retry is acceptance after the injected failure, not an unexplained flaky-test retry.

## 8. Planned genuine RED and acceptance tests

All names below are proposed; root records exact commands and observed outcomes. Preserve the current scalar17 RED. Fixtures use the public draft/publish/start and real worker completion paths, repository-owned disposable PostgreSQL, exact barriers and the actual runtime role. No SQL edits to results, statuses, ledger totals, source facts or certificates.

1. **`empty_collection_aggregate_obeys_remaining_run_bytes` (new RED):** ordinary legal prefix activities produce sixteen serialized outputs of exactly 262144 bytes, then a zero-item collection. All real reports fit L=4194304. Old code succeeds retaining `[]`; expected collection/Run RUN_DATA_LIMIT because two additional bytes do not fit. Assert no member was ever admitted, no fake iteration event, no aggregate/owner/End result, one typed drain/audit/outbox, bounded failure completion and verify_run. A nearby control with two bytes of room succeeds with `[]` and charges exactly two bytes; no seeded ledger.
2. **`legacy_collection_aggregate_is_read_without_repair`:** freeze the actual pre-correction writer from HEAD6c44a0e4, including executable/source hashes. It publicly creates two collections, finishes the first and stops with later real work outstanding. Restart with current readers/writers against the same fixture DB. End reads old first aggregate plus new later output and retains the real scalar result; old owner remains NULL. Compare exact old scope/items/iterations/checkpoints/owner/events/certificate before/after. Re-read/replay closed old no-result Run separately: no inferred End or Run result.
3. **`legacy_outputs_count_in_full_and_exact_path_budgets`:** actual old producer makes enough closed aggregates that L fits but E changes the next real output/aggregate decision. Compare scalar full/path/frontier totals, bounded full overlay sum, real ledger L, physical content charge and E. Exercise equal-digest distinct owners and output-present owner exclusion. A real later successful scoped worker output that fits L but not E is retained as genuine execution success plus an authoritative RUN_DATA_LIMIT stop, with no stored payload/reference (section 11); the effect must remain eligible for compensation. No root overlays leak into its path-local context. Current source/certificate stays unchanged.
4. **`checkpointed_collection_data_limit_survives_restart`:** an old writer genuinely commits all checkpoints, having checked only its old incomplete retained sum; stop before its finalizer with a fixture-only held barrier/controlled scheduler tick. Current finalizer sees verified old overlays plus this aggregate exceed 4 MiB. All member rows stay CHECKPOINTED, owner drains with the new no-iteration reason, no result retained, cold full/path replay and verify_run agree. This is a true producer upgrade; never erase a current owner output to manufacture legacy state.
5. **`already_over_budget_legacy_collection_still_drains`:** old writer genuinely creates E >4 MiB while L stays legal and leaves an entered scope. Current scalar preflight returns the full E without clamping or hydrating every old aggregate, starts ExistingOutputs refusal, and metadata/saga progress continues. A companion effectful final-checkpoint case runs actual compensation/reconciliation obligations and proves the owner cannot finish while any obligation is open; final status follows existing compensated/partial/manual-review rules, no fabricated FAILED terminal state.
6. **Codec/domain/schema refusals:** first capture raw journal decode RED for `run_data_limit` on current code. Prove empty/all-checkpointed restoration, exact code match, no fictional member, old reason fixtures unchanged, malformed new projection fields/NULL discriminator/foreign identity rejected. New vocabulary tests may be added after its compiled declaration; do not mislabel an unresolved-name compiler error as behavioral RED. Validate new domain behavior against the original raw-decode and public quota counterexamples.
7. **Atomicity/replay:** use held own-Run/commit/audit barriers and a separate observer; before commit no refusal state/audit/hint/ledger change is visible. Inject actual deferred commit or audit/outbox rejection, observe unchanged state, then one valid retry. Replay original matching start/drain-finish and normal success commands under current fence, assert zero extra byte charge/audit/hint/event; wrong evidence/revision/source/fitting size refuses. Race cancellation with refusal in both held-lock orders: first committed stop wins.
8. **Upgrade/downgrade and privacy:** new produced drain is readable in A full/path/codec/graph/projector; old readers are explicitly unsupported. Locked empty down/up works; retained scope **or journal** blocks down; a writer waiting behind the downgrade lock cannot sneak a new reason through. Runtime-role access and protected storage classification pass without grants. Hint validates as existing FAILED. Value canary absent from audit/span/log/hint; new actual Run result obeys current outputSchema classification.

For the frozen old writer, use an isolated temporary source copy/executable and the fixture's disposable DB, like existing actual-binary compatibility tests (`tests/integration/payload-classification/compat.rs`). A test-only phase driver may stop after real scheduler ticks, retain DB ownership while the current phase runs, or hold a real finish transaction at a barrier and terminate it. No production source modification, hand-started unrelated service or persisted source seeding is needed. Build at most two heavy artifacts at once with private target directories and disk check; clean only task-owned temporary artifacts.

Focused planned commands include `cargo test -p sagat-tests --test integration_workflow_primitives collection_end_result --locked -- --nocapture`, corresponding new quota/legacy selections in default and `SAGAT_TEST_AS_RUNTIME_ROLE=1`, domain collection/recovery/saga tests, event-codec tests, existing `byte_ceilings`, hint contract, current run-result/classification and source-certification regressions. Exact old/current binary compatibility runner is a separate explicit gate, not counted as a normal reconstruction test. Root reruns merged-tree gates itself. No claim of tests passed appears in this decision.

## 9. At-most-five-file increments

Each is a review boundary; complete bridge A and producer B are deployment units. Test/evidence commits may remain intentionally RED. Do not make an incomplete intermediate reader build a release artifact. Required compiler exhaustiveness can be met in the five-file vocabulary slice by explicitly refusing the new native writer until its evidence/schema support lands; label that temporary internal refusal, never a pretend implementation.

| Unit | Exact bounded ownership (maximum five files) | Gate |
| --- | --- | --- |
| Q0 current counterexamples | new integration quota/legacy module, integration main registration, existing event_codec inline tests, one contract decision, one evidence manifest | Empty/public and raw-codec real RED; existing scalar RED preserved. Detailed raw evidence may be its own artifact-only increment. |
| Q1 domain/codec vocabulary | domain execution/event.rs, state.rs, scoped_sagas.rs; PG event_codec.rs, primitives.rs | Exhaustive mapping compiles, new native producer still explicitly unavailable; old cases unchanged. Domain test files land in a separate <=5 test increment. |
| Q2 readers | PG execution.rs, execution/path_read.rs, primitives.rs; domain collection restore test, integration compatibility module | Complete new reason restoration/finish; no scheduler producer. |
| Q2b scheduler reader lifecycle | scheduler passes.rs, one cold drain regression module, one evidence note | Move terminal/draining lifecycle handling ahead of full data hydration. A can resume an already committed new drain at E above the ceiling without emitting a new reason; scalar port dependency is Q6. |
| Q3 typed audit/evidence | domain audit.rs, domain/tests/audit.rs; application primitives.rs; one application evidence-constructor test; feature contracts README | Closed audit/evidence semantics and rejection tests. |
| Q4 schema | new migration up, new migration down, existing/new schema test module, storage-classification.json if needed, one evidence note | Old forms preserved, total new check, locked guarded down/up, runtime role. No applied migration modified. |
| Q5 effective helper/full read | new PG collection-output helper, PG lib.rs module declaration, PG execution.rs, one integration overlay test module, evidence | Exact scalar/overlay/corruption rules and immutability. |
| Q6 scalar consumers | application execution.rs (RunTx method), worker/output.rs, runs.rs, chain_progress.rs, one focused application test | Current typed preflight outcomes; no full hydration merely to count. Existing mocks can use the truthful default. |
| Q7 path/frontier guards | PG execution/path.rs, scope_frontier.rs, existing byte_ceilings tests, legacy integration module, evidence | Full/path/frontier arithmetic identical; actual worker refuses, metadata/compensation still progresses. |
| Q8 native boundary writer | PG primitives.rs, application primitives.rs if constructor wiring needed, quota integration module, fault module/registration as room permits, evidence | Evidence/CAS/fence/audit/source checks, actual refusal and replay. Scheduler still emits no new reason. |
| Q9 producer/result correction | scheduler passes.rs, PG primitives.rs, PG execution.rs, result regression module, evidence | Depends on parent's shared End preparation increment; no sixth hidden file. Effective preflight before hydration, aggregate/End transaction, new typed drain selection. |
| Q10 actual old/current process gate | bounded runner script, integration legacy compatibility module, registration, compatibility/runbook doc, manifest | Genuine producer upgrade; active-old reads, immutable source, safe downgrade refusal. Separate runtime-role play. |
| Q10b finite rejection vocabulary | telemetry/src/metrics.rs, telemetry/tests/primitive_vocabulary.rs, application/tests/primitive_telemetry.rs, feature contracts README, evidence | Add RUN_DATA_LIMIT to the explicit finite metric label set; prove from_reason and privacy. No public error enum change. |
| Q11 wire/observations/acceptance | hint-contract test, primitive telemetry test, feature contracts README, validation-report, task ledger | Existing public FAILED mapping, truthful audit/counters and actual evidence; no canonical broader task closure. Additional source/classification test files get their own <=5-file increment. |

Q5/Q6 may be ordered as an application-port declaration followed by the PostgreSQL override; the default must preserve compilation without claiming the PostgreSQL acceptance gate passed. A prepared helper is not production activation. Never land Q9 before Q2–Q8 and the parent's core preparation are reviewed together. The existing five-file parent core estimate is superseded only by these explicitly separated quota prerequisites, not by one oversized patch.

## 10. Read provenance and seal

Initial read: repository AGENTS, constitution, full active 014 spec/plan/tasks, parent decision (initial hash verified; current C2 hash verified for this amendment), owner G21, D314/D315, ADR0030/0032 and the sources cited above. `.specify/feature.json` is absent in this checkout; the requested active feature014 and parent task identify the scope. Checked worktree/014 branch inventory. No nested AGENTS found in targeted crates/tests/docs/specs/tasks inventory. The initial read observed root-owned `main.rs`, untracked `collection_end_result.rs`, and its evidence directory. Root continued working during this amendment; that initial status is historical provenance, not a claim about the current tree. No repository file was written by this agent.

Selected production source SHA-256 at the **initial** read (historical provenance, not current amendment hashes):

| Source | SHA-256 |
| --- | --- |
| application/execution.rs | c2efa47b53de70747cdd9aa0dceb57d5eb5334c6b76d8c81c8dff85637fb79c2 |
| application/primitives.rs | bd58299fa2104543cb4d84d529e1d105c2ee92254fe76ca444319d3b91a617ea |
| application/worker/output.rs | 17164bad40b57b1f640d46e4949e099cc0bd55e64fad7f232b68b76c759bb74c |
| infra-postgres/execution.rs | fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f |
| infra-postgres/execution/path.rs | 012044db467dc12d378889c889bcc197dbe56c204f790016325057573d2716d7 |
| infra-postgres/execution/path_read.rs | aab1a881a9943cfaef8e70efdc7330d682feb116096d83b6b12e80b7a2dca7a7 |
| infra-postgres/primitives.rs | 0c84e9a5e622e5b70135340de75c69e6e04c3e79ee1caf04cdd704c0eb027b75 |
| infra-postgres/event_codec.rs | cf627e09262eedb4d7811b0ba7d11931ecb2b47f50a432385bd83fbec512279c |
| infra-postgres/scope_frontier.rs | aa15150795d321e6f91fea22fd14a63272ac4bbb28c8abf1b33553d7846d9de2 |
| domain/execution/event.rs | e740724607e7c5ad1a9dfe89c3fc1387d3ec00144453fff98f3fa75a7a8e64a6 |
| domain/execution/state.rs | e802c45c2bdbad9da159c6f60ad28ab6406c3fa04ce17b473ba7cb63f71af0e5 |
| domain/execution/scoped_sagas.rs | 45cd7bf76463d7b72f318f8a77402e7182cb0a3082af8cccb13a00964fc22e2b |
| domain/audit.rs | c8cab1ac89edc03466422468c7b80463d7c974cd68ddac50b560543f486593e7 |
| scheduler/passes.rs | 31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d |
| migration 20261204000000 up | 2928476cc78e7c415f773b333cdf637ec08fdb4d06c2453bc2349f240e11f8b4 |
| migration 20261106000000 up | 3ca61a50f3e01b56fff3864532f351b6b12a2e671ee5bda12563a700d0a941b7 |

This is a concrete bounded correction design. The choice is no longer an indefinite quota inventory. Implementation, actual binary upgrade evidence, RED/GREEN acceptance and root's merged-tree verification remain required before C1/.1d.0 can close.


## 11. C1 amendment — known successful effects and the production frontier

This section resolves both actionable C1 findings. The preserved C1 file records the reviewed original; this amended file is the C2 review input. No fourth general protocol cycle or wider capacity decision is implied. All new behavior below still needs actual RED/GREEN evidence.

### 11.1 Source-backed failure and selected treatment

`application/worker/scoped_reports.rs:246-260` currently discards an oversized completion output and calls `fail_path_instance(RUN_DATA_LIMIT)`. `domain/execution/command.rs:1012` preserves a cancelled attempt's effect uncertainty only for ambiguous/storage-quota codes; `domain/execution/scoped_sagas.rs:149-164` selects SUCCEEDED or failed/cancelled steps with `leaves_outcome_unknown()`. RUN_DATA_LIMIT is deliberately absent from that predicate (`event.rs:99-111`). Consequently the current report-before-drain path loses an actual produced effect, and drain-before-report can erase it behind cancellation. Changing the scalar to E exposes this existing defect to genuine old in-flight reports after upgrade.

**Decision:** preserve the successful execution fact with existing durable success events, while stopping forward progress for the truthful RUN_DATA_LIMIT cause. Do not convert a known successful forward report to failure, fabricate a payload or classify every RUN_DATA_LIMIT as unknown. A successful completion and a retained output are separate facts in the current event/storage design: `StepCompleted` carries attempt/generation, while `RunTx::persist_addressed` receives output references separately. The zero-payload success is only admitted by the narrow refusal path below, alongside the authoritative stop. It is not a new generally available output-free completion mode.

Use the existing full RunTx for this exceptional metadata transaction. It can atomically commit a root RunStopRequested persist followed by a separate scoped StepCompleted persist in the **same transaction**. Its scoped addressed writer rejects mixed root events (`infra-postgres/execution.rs:4541-4625`), and the bounded PathExecutionTx intentionally cannot carry root events (`infra-postgres/execution/path.rs:319-378`). Do not submit a mixed root/scoped batch. Do not widen that bounded writer or introduce a second projection protocol. `Engine::run_addressed` only hydrates/stages data when given outputs; with `outputs=[]` its command and persistence path do not call `data()` (`application/execution.rs:1453-1480`). The full transaction restores metadata proportional to retained instances once on refusal; it must not hydrate the potentially over-budget legacy output overlay. This exception does not claim constant-cost metadata restoration or change normal successful path reporting.

Use a small domain preparation command for this exact refusal. It checks the currently leased successful report kind and complete address/fence, and applies **only** root `RunStopRequested { reason: Failed, code: Some(RUN_DATA_LIMIT) }` if no stop already exists; it returns root Progress with no evaluation or settlement. Persist that root-only phase through existing `RunTx::persist` before applying the next phase. Then call the existing genuine `complete_instance` (or Applied reconciliation command) and persist its scoped-only AddressedProgress with zero outputs through existing `RunTx::persist_addressed`. State version must match each individual persist; do not pre-apply both phases then persist only the first batch. Both persists, audit and ACK stay inside one existing full RunTx, with no intermediate commit. The existing Engine root/addressed wrappers can execute these two commands sequentially, provided the preparation command returns no Evaluation. **Do not call `stop_run`/ordinary root settlement first:** current `plan::drain` rejects an OPEN failed collection as UnsupportedStructuredNode (`plan.rs:329-340`). Persisting only the existing root stop event, then applying/persisting the existing scoped success and scoped settlement within the same transaction is the selected composition; the collection scheduler owns the subsequent collection drain. `RunState::apply` accepts that stop without evaluating output (`state.rs:4045-4054`), and `plan_addressed` fences declared forward routing when the stop exists (`plan.rs:95-132`). Assert no unexpected Evaluation or forward StepRouted/StepReleased is produced. Do not add an End evaluator to the refusal path.

For a successfully parsed reconciliation `Applied` answer whose substitute forward output cannot fit, preserve **both** the actual reconciliation success/verdict and the resulting forward success using the existing `complete_reconciliation_instance(..., Applied, ...)` events after the same stop. Do not discard Applied into a failed reconciliation or a made-up unknown verdict. NotApplied/Unknown/invalid-answer handling is unchanged; compensation completions already retain no output and remain accepted. The new domain command may have a closed internal successful-report kind (ordinary completion / Applied reconciliation) so it cannot be invoked with a failure report or arbitrary verdict.

The public StepAck truthfully reports the resulting successful step and stopping Run, normally `step_status=SUCCEEDED`, `run_status=CANCELLING`; existing prior review/compensation status rules still determine the actual Run status. No report failure payload or new public enum is introduced. The retained Run stop is RUN_DATA_LIMIT when this operation owns the first stop. This intentionally corrects the old lossy StepAck/failure outcome for this narrow case.

### 11.2 Exact report transaction and races

1. Existing report authentication, current worker binding/namespace access, immutable report fingerprint, bounded payload validation and secret scan run before selecting this path. A secret-tainted or failed report must not enter it. Neither an output blob descriptor nor its reservation is committed on refusal. Do not read blob bodies or grant any new secret/blob access.
2. The locked path transaction reads E and detects `checked(E + additional) > 4194304`. Retain only the already-scanned request in transient memory. Drop/roll back that path transaction before reopening the existing full transaction, respecting chain-before-own-Run ordering. Do not hold both locks/transactions, and do not pass stale path state into the full writer.
3. Under the fresh full transaction, recheck tenant, namespace, full instance identity, pinned activity, current principal/binding/session authorization, current durable receipt using the **original** idempotency request fingerprint, and current attempt/generation/token fence. Replay an existing matching ACK before a new mutation; reject wrong principal/foreign namespace/fingerprint/stale lease without a refusal event. Revalidate cancellation using the current completion policy: a still-valid successful completion may be accepted after cancel_requested. Blob-report expiry checks remain at least as strict as their existing path checks. Recheck that the exact admitted scope/instance is still live; never reopen a terminal scope or counterfeit an iteration.
4. Recompute E and the exact already-scanned output/substitute length under this own-Run lock. If the refusal predicate no longer holds, abandon this exceptional transaction and retry the ordinary checked report path; do not fabricate a failure from a stale budget read. No new report operation/receipt key is created.
5. In one transaction, persist the first RUN_DATA_LIMIT stop if absent as a root-only phase, then persist the genuine success/verdict events as a scoped-only phase, with zero output references, zero blob commits, existing reported-lease evidence, typed audit, original ACK receipt and transactional journal/projection updates. Each nonempty persist advances `telemetry_pending` in that transaction; the final committed head equals the final journal sequence. The refusal audit/report provenance belongs to the scoped phase even when a prior stop makes the root phase unnecessary. Preserve any existing stop **byte-for-byte** and skip the empty root persist. Neither phase commits separately; there is no ordinary root settlement or expression hydration. This report transaction releases no task, starts no collection drain and creates no outgoing-message obligation; its expected new `event_outbox` row count is zero. The committed stop, success metadata and collection state are the durable scheduling condition. Every applicable outgoing obligation remains atomic with its creating transition. The later actual drain writes `collection.drain.started` in the drain transaction. Do not fabricate a READY-task hint or announce a drain during report refusal.
6. Use the already selected `RunDataLimitExceeded` typed audit action for this refused report as well: engine actor, ResourceType::Run, exact Run id/trace and fixed reason `successful scoped output refused by run data limit`. The event actor/reported-lease record retains the reporting worker provenance. One audit accompanies the report-refusal identity; a later collection drain has its own separately described audit/identity. Do not assert that these two different transitions produce only one total Run audit. Report replay adds neither. Existing bounded output-quota telemetry can report the committed report refusal; collection primitive rejection counters still describe actual drain commits only. No output bytes, candidate hashes, variable expressions or user strings enter the audit reason or labels.
7. After commit, ordinary `Scheduler::tick` discovers the CANCELLING OPEN collection through `collection_candidates` under its owned-shard window, then applies the reviewed Q8/Q9 decision and native transaction using `StoppedForRunDataLimit`. This requires no report notification, broker delivery or surviving report-process memory. Existing cancellation/different-failure stops retain their supported drain policy; an existing DRAINING collection resumes its recorded obligations. A cold restart after report commit and before drain must recover from durable rows. The actual later drain retains its own atomic journal, audit, telemetry and outbox requirements.

**Effect/liveness result:** report-before-drain leaves SUCCEEDED plus an authoritative stop before any successor can be released; drain-before-report leaves the existing draining scope/stop and records the valid in-flight success, rather than replacing it with CANCELLED/failure. The live member cannot be drained while its worker is still active; after the success report it can settle/drain and the existing saga planner sees the SUCCEEDED activity. Successful known effects do not require a new operator decision to be recognized, and cannot be blindly rerun. Compensations with sufficient declared inputs execute normally. If a declared compensation requires the refused output, do not synthesize it: existing task-input/failure/manual-review policy must retain that open obligation and prevent a false compensated/final outcome. Tests must demonstrate both a real successful compensation and this honest missing-input limitation. Existing audited operator intervention remains available only through its existing policy; this design does not introduce a new review-first policy or grant.

For the output-dependent compensation test, the requirement belongs to the declared activity's
actual worker implementation. The engine's ordinary task context exposes only retained outputs;
it does not infer arbitrary worker input requirements from an activity name. Exercise the same
strict worker handler against actual retained/refused lease input, then report its genuine typed
application failure through the existing FailStep/manual-review policy. A fixture-defined error
code is not a new built-in platform diagnosis or IR dependency declaration. An authorized failed
resolution must finish COMPENSATION_FAILED honestly; it must not imply a successful release,
external provider undo or old-binary acceptance. The bounded decision is recorded with the test
evidence; no production input contract changes.

`StoppedForRunDataLimit` is transient internal evidence, not durable vocabulary. Native start requires OPEN scope and exact stored `(stop_reason='FAILED', stop_code='RUN_DATA_LIMIT')`, native revision, scheduler fence, owner/namespace and audit. It does not accept CANCELLED, timeout, another failure code, a stopped Run with no matching code, or E<=limit with no stop. It never creates a stop itself. ExistingOutputs still freshly proves E>limit, and Aggregate still proves the actual final candidate/source bindings. No source row, checkpoint origin, aggregate length or member failure is invented to justify a refused report.

### 11.3 Frontier guard on the actual production dispatcher

The production `advance_collection_in_view` first calls `collection_frontier` and returns its answer before full fallback (`scheduler/passes.rs:1609-1629`). The current frontier directly handles nonfinal Return checkpoints, admissions, and Waiting. A limit check only in `advance_collection_from_run_in_view` is therefore insufficient.

Extend the existing frontier snapshot with scalar `CollectionFrontier.effective_output_bytes: u64` and include the stored `ScopeRun.stop_code: Option<FailureCode>` alongside its current stop_reason. PostgreSQL computes E with the same metadata/length helper in the **same repeatable-read snapshot** as Run revision, scope counters and active members (`scope_frontier.rs:247-374`). Do not issue an unrelated unlocked count and associate it with a different frontier revision. Keep frontier RunData local; adding this scalar does not hydrate successful historical aggregates or change context visibility. Test/memory frontier producers must supply their actual effective total, never a silent default zero.

In the normal frontier dispatcher, order the decisions as follows:

1. Terminal scope: existing AlreadyFinished, no repair. DRAINING: resume the existing reason/obligations without a new quota refusal or full-data hydration.
2. OPEN with an existing durable stop: respect its exact reason/code first. CANCELLED uses existing cancellation. FAILED/RUN_DATA_LIMIT starts the new drain with StoppedForRunDataLimit, even when E<=limit. Other existing stops follow their existing supported drain/fallback; they cannot be replaced by a new data-limit stop or allow forward admission/checkpoint. Do not manufacture support for an unrelated currently unsupported structured timeout.
3. OPEN with no stop and an actual active member failure: retain the existing truthful member failure path and native validation.
4. OPEN with no earlier stop/failure and E>4194304: select ExistingOutputs **before** inspecting/evaluating a pending nonfinal Return, admitting another member, or returning ordinary Waiting. This guard applies even with no pending Return/admission slot.
5. Only after these checks may the existing nonfinal checkpoint, admission, terminal-ready full fallback, or Waiting logic run. The full fallback repeats the same scalar/lifecycle rules for unentered/terminal-ready paths; it is not the sole enforcement point.

The native mutation retains chain-before-Run lock, exact scheduler lease and Run revision CAS. start_collection_drain recomputes the relevant predicate under lock, including the matching prior stop for StoppedForRunDataLimit. A stale frontier loses CAS; it must not admit/checkpoint using its previous E or replace a prior stop. Native admission/checkpoint retain their existing same-revision checks, so a new worker report committed after frontier read invalidates the decision. No counter may be updated or payload inserted before those checks. Returning Waiting has no durable mutation and no admission authority; the next decision takes a fresh snapshot.

Collection discovery already includes OPEN CANCELLING scopes (`maintenance.rs:204-218`), so the successful-report fallback requires no MANUAL_REVIEW discovery expansion. Regression tests must call the public normal `advance_collection` entrypoint and observe that frontier-capable cases take the guard. Testing only the full fallback is not acceptance.

### 11.4 Required additional genuine RED and acceptance cases

The following are requirements, not claimed test results. Use the genuine old binary producer from section 8; no SQL seeding/deletion of outputs, ledger, statuses or scope source. Hold real transaction/barrier order rather than sleeps. Every test replays/verifies the cold journal and reports actual default/runtime-role commands.

- **Old in-flight success / report before drain:** old producer publicly leaves a genuine compensable member leased while closed legacy aggregates make E>4MiB. After upgrade submit its actual valid known-success report first. Before the normal scheduler call, assert stop RUN_DATA_LIMIT, true SUCCEEDED (not FAILED/CANCELLED), no output/payload/blob reference or logical charge, no successor release, one receipt/reported-lease/audit. Normal advance_collection starts the matching new drain; actual compensation runs and finalization waits for it. Cold restart immediately after report receipt is a required subcase.
- **Old in-flight success / drain before report:** same genuine producer state; normal advance_collection first starts ExistingOutputs drain and cancels the live attempt. Submit the still-valid success under that cancellation fence, then continue normal scheduling. The worker remains SUCCEEDED and compensation-eligible; prior stop and drain identity are unchanged. A separate cancellation-first run keeps CANCELLED precedence while preserving the success/effect. The preserved prior stop skips the root persist rather than inventing a second root event. Duplicate report and lost-ACK replay produce the original ACK and no second effect event/audit.
- **Under-limit retained total, refused increment:** old producer leaves E<=4MiB but E+the new real completion/substitute output >4MiB (while L+output would fit). Both report-before-drain and an already authoritative cancellation/drain order must preserve known effects. For the new first RUN_DATA_LIMIT stop, normal advance_collection uses StoppedForRunDataLimit; ExistingOutputs is explicitly rejected because E itself fits. This proves no operator-wait dead end and no forward work between ACK and drain.
- **Applied reconciliation:** real old in-flight reconciliation produces actual Applied plus substitute output refused by E; current journal records the real verdict, successful reconciliation and successful forward effect with no output. The actual forward compensation is selected exactly once; duplicate/restart and drain-first ordering retain it. NotApplied/Unknown and malformed-answer controls retain their existing meanings.
- **Frontier bypass REDs:** genuine E>4MiB active legacy states with (a) free admission capacity, (b) a nonfinal pending Return, and (c) only live work that would otherwise return Waiting. Invoke normal advance_collection, expect ExistingOutputs DrainStarted before admission/checkpoint/wait. Assert no next_admission increment, new checkpoint/result, successor, or payload. Include E==4MiB control, existing cancellation/different failure precedence, and a held-lock report/frontier race that forces native CAS rejection then a fresh correct decision.
- **Bounded refusal recovery:** a test adapter/trace assertion fails if this exceptional report or its drain calls full `RunTx::data()` when E>limit. Exact local context may still be read. Check the same underlying PostgreSQL state through full/path replay without overlay hydration. Report audit, receipt and telemetry faults, including an injected error after root-only persist and before scoped persist, roll back the whole report transaction; a valid retry commits both. Separately, failure of the actual drain outbox insertion rolls back that drain while retaining the previously committed report and stop. Revoked binding, wrong namespace, changed fingerprint, stale fence and terminal scope refuse without mutation.
- **Output-dependent compensation:** a real compensation whose task input uses the absent refused output cannot silently produce a default, be skipped, or permit final COMPENSATED. Observe its existing typed failure/manual-review obligation and exercise the existing authorized audited resolution as applicable. This is a known limitation of refusing payload retention, not permission to omit the successful effect.

### 11.5 Bounded implementation additions and rollout

Add these explicit <=5-file review units to section 9; root may split further but must not hide a sixth file. Production activation remains last.

| Unit | At most five files | Required gate |
| --- | --- | --- |
| Q7a known-success command | domain execution/command.rs; domain execution/mod.rs only if export needed; two domain report/drain tests; evidence | Root stop preparation and genuine ordinary/Applied phase composition, prior-stop preservation, zero forward settlement, exact fence; original failure semantics unchanged. |
| Q7b report refusal application | application/worker/scoped_reports.rs; new private worker refusal helper module; worker.rs module wiring; one integration report test module; evidence | Current authority/receipt/fence revalidation, fresh scalar check, root-only then zero-output scoped-only persists in one transaction and correct ACK; no full data hydration. Helpers reuse existing RunTx, no new public port/grant. |
| Q7c frontier snapshot | application/scope_frontier.rs; PG scope_frontier.rs; up to two existing frontier fixture/test files; evidence | Effective bytes and stop code from one snapshot; no fabricated defaults; stale-version behavior. Remaining fixture adaptations get their own <=5-file increment. |
| Q8a stopped-limit evidence | application/primitives.rs; PG primitives.rs; one constructor/native evidence test; one integration replay test; evidence | Matching durable stop only, old-stop priority, zero new stop on ExistingStop, native CAS/fence/audit; no extra schema vocabulary. |
| Q9a production frontier | scheduler/passes.rs; one normal-entrypoint frontier regression module; registration if needed; evidence | All three real frontier bypass cases refuse before forward admission/checkpoint/wait; stale CAS and prior-stop controls. |
| Q10c old in-flight compatibility | old/current phase runner; compatibility integration module; registration if needed; evidence manifest; runbook | Actual producer and both report/drain orders for E>limit and E+delta>limit, Applied reconciliation, actual effect compensation and missing-output limitation; default/runtime role. |

The two-phase, single-transaction known-success writer and normal frontier consumer must both be reviewed and present before producer B resumes. Reader bridge A needs no new journal variants for these successful report events beyond the already selected RunDataLimit drain reason, but an old active writer/finalizer is still unsupported. A rollback must retain both corrected refusal and drain behavior or stop affected execution writers; reader tolerance alone is insufficient. No new capability bit, feature flag, public caller, grant or profile is selected.

### Amendment provenance

Verified parent C2 SHA3a44 and preserved original Q SHA4c4a before editing. Actual relevant source hashes read at amendment seal:

| Source | SHA-256 |
| --- | --- |
| application/worker/scoped_reports.rs | cc17ee9d62ded96b855fd6f2c864c4439bcedc59e6aa4b6a9b33abb694778ba3 |
| application/worker.rs | 01d78ebd4a21953bc5827ddf03450bff188efbfbe47bb840ea74fbc536046100 |
| application/execution.rs | 70c79b172b57867cf61484785df2d25e7801fa9a3710c557b71e6b2415a2d9e7 |
| domain/execution/command.rs | 55ca47d23f1df9129ccca3d8dc12a7f78ed2746935dabb5e1b161335ed7ba4f7 |
| domain/execution/plan.rs | d9c1c67ab83990be5cb89b9e7f1604b29b767a947917a94b5247522403091879 |
| domain/execution/state.rs | d813db22574e5657763ab1a9b97476d3aabe97b0f15b5d0896c7ae42bee4a133 |
| domain/execution/scoped_sagas.rs | 38090402ef266554e8f87a455ca6e6f3c3ab106ab588ad85df54fab5d3b00452 |
| application/scope_frontier.rs | 3c72e53c5d2267f7e3b0d9f9078c899d070edc0f83f0a98e4d37dc655c9a2f2e |
| infra-postgres/scope_frontier.rs | aa15150795d321e6f91fea22fd14a63272ac4bbb28c8abf1b33553d7846d9de2 |
| scheduler/passes.rs | 31fb13e7ed8be8aaab7586cfe2c6d885252dd213cc9dd988ad0338155750717d |

No design alternative remains selected-but-unresolved for the two C1 findings. The real producer tests, exact event composition and merged-tree verification remain acceptance gates; this seal is a design artifact, not proof that they pass.

## Root bounded design checkpoint

Original Q C1 (4c4a...) found two actionable defects: known effect loss on output refusal and bypass through the production frontier. Both were amended in section11. Fresh bounded C2 on2983c9b5... returned None. The transient fadad proposal was superseded before review after checking the mixed-event writer restriction. Only the final separate root/scoped persists within the same transaction are selected. No implementation, new production reason, actual old-binary upgrade, RED/GREEN quota recovery or release acceptance follows from this design checkpoint.

## Effective output scalar port declaration — 2026-10-18

RunTx now declares logical_output_bytes before staging, with checked complete-RunData compatibility default. Fresh one-file C1 None; all application targets compile under strict Clippy19.63 s and fmt passes,1109 inputs unchanged. No test execution or effective PostgreSQL behavior is claimed for this declaration. The mandatory PG override/overlay and consumer activation remain Q5/Q6b; no caller changed. Evidence: evidence/effective-output-scalar-port-20261018/manifest.json.

## Report discovery wording correction — 2026-10-18

The bounded [discovery amendment](../../tasks/project-completion/scoped-output-refusal-discovery.md) corrects unconditional report-outbox wording under Constitution I/VII and ADR-0002/0017. Q7b uses existing durable collection discovery and adds no report-specific message. No actual outgoing obligation loses atomicity. Activation still requires Q8/Q9 and cold-discovery evidence. The original2983 design is retained; this reviewed correction supersedes only its report-outbox requirement. Fresh bounded C1 on amendmentbd5ff35f52ef12bc8d0cd406d7ac81e97d2007409aef9e317a023f93ae295395 found no required issue. No runtime acceptance follows.
## Coupled implementation and dormant-worker discovery — 2026-10-18

The PostgreSQL/scheduler increment `109908be` and successful-report increment `3710a5b7`
have [combined focused evidence](evidence/collection-finalization-integration-20261018/reproduce.md).
This includes actual successful forward/Applied reports, durable stopped-run discovery and
17 real compensations in each case, without retaining refused output or creating report outbox rows.
Existing result, byte, drain, compensation and native source/scoped Finish regressions pass.

The separately reviewed Q9a candidate hint now has
[genuine old/current acceptance](evidence/collection-discovery-integration-20261018/reproduce.md):
both current pool modes discover E>L while full-concurrency work is still RUNNING, start the
truthful ExistingOutputs drain, respect its real lease and complete normally without rewriting
the closed old source. The hint precedes existing shard-local/global queue limits and uses
metadata/lengths only; the shared snapshot verifier/native guarded command retain authority.
No total database-work bound is inferred from the bounded result window.

Q7c/Q8/Q9/Q10 and the master Q remain open for their unexecuted controls and acceptance cases.
In particular, section11.4's old compensable in-flight reports, both orders, prior-stop/current
secret races, cold report recovery, unknown COMMIT, output-dependent compensation and rollout
are not replaced by this waiting-worker discovery case. Canonical task checkboxes stay unchanged.


## Current worker missing-output acceptance — 2026-10-18

The section 11.2 worker-defined missing-output limitation now has actual current-version
[evidence](evidence/refused-output-worker-20261018/README.md): all 16 retained compensation
inputs pass the same strict handler; refused input causes genuine typed FailStep, preserved
WAITING/OPERATOR and MANUAL_REVIEW, then authorized audited failed resolution produces
COMPENSATION_FAILED without claiming release. Final-source focused test passes in both
invocations under actual sagat_app; strict integration Clippy/fmt/architecture pass.
This does not substitute for old in-flight compatibility or external provider evidence.
The earlier checkpoint's pending statement is superseded only for this current worker case;
Q10c/master Q and canonical 014 checkboxes remain open.


## Current fresh-full binding authority checkpoint — 2026-10-18

Section 11.2 item 3 now has focused current-version [binding revocation evidence](evidence/refusal-fresh-binding-20261018/README.md)
at the actual exceptional full restoration barrier, after the checked path ends. The public
report returns NOT_BOUND with no successful-refusal or execution effect; the existing required
policy-denial audit remains exact and trace-bound. Native unbind's physical binding-byte refund
and audit are checked independently. Final focused test passes in both invocations, alongside
strict integration Clippy/fmt/architecture. This refines test wording, not the transaction/API
contract: no-refusal-event never prohibited the platform's mandatory security denial audit.
Other current-authority/secret/fence controls and master Q/Q10c remain open.


### Immutable-old ordinary success before current drain — 2026-10-18

The real frozen 6c44a0e4 producer/current consumer coordinator now passes all four phases
(owner and actual sagat_app). An original still-leased compensable member reports success
with its original authority/request before any current scheduler tick, at legacy E=4,200,020.
Current success is retained without output, exact receipt/replay is stable, durable discovery
starts the genuine drain and real compensation reaches COMPENSATED with matching target/key.
Closed old source witnesses and fresh native journal reconstruction pass. Final strict
Clippy/fmt/architecture and 1124 matching input hashes; two failed fixture expectations are
retained separately. Evidence: `tasks/project-completion/evidence/legacy-inflight-ordinary-v1-20261018/README.md`.
This closes only ordinary report-before-drain at E>limit. Drain-first, Applied, the other byte
condition, cold post-ACK process/unknown-COMMIT, external effects and the full Q10c/canonical
014 release gates remain open. No production nesting activation is inferred.


### Fresh full exact OpenBao version recheck — 2026-10-18

Actual OpenBao 2.7.1 public secret rotation now passes while the genuine seventeenth
completion is held at observed full checkpoint restoration after its checked path/provider scan.
Exact immutable v1 rows, one native v2 operation/version, two operation-specific audits and
physical retained-byte charge are verified. Releasing the gate returns UNAVAILABLE without
execution/refusal effects for the stale scanned vector. Same original key/lease retry accepts
SUCCEEDED/RUN_DATA_LIMIT without output/new report outbox; cold journal reconstruction and
stable replay pass. Both secret values remain absent from captured logs. Final focused 1/1
default/runtime, both actual sagat_app; strict Clippy/fmt/architecture and1125 identical inputs.
Evidence: `tasks/project-completion/evidence/refusal-secret-versions-20261018/README.md`.
Two fixture-oracle failures are retained (shared key across instances and SQL alias/column
collision). This is not a literal pre-begin fence/terminal race or an owner/runtime matrix.
Other authority, old-order/Applied, cold-process/uncertain-COMMIT, nesting and release gates
remain open; canonical014 checkboxes are unchanged.


### Immutable-old ordinary drain-before-report — 2026-10-18

Primary D1 passes all four real old/current phases in both API pool modes,149.81s.
A live original member retains its slot/reservations through actual normal drain and an extra
waiting tick; the original success stays SUCCEEDED, without output/new report outbox or journal
payload reference. Exact first stop/drain and receipt replay remain stable. Actual original-target
compensation must finish before COMPENSATED. Default report-before-drain regression also passes
all four phases147.54s. Strict Clippy/fmt/architecture and1125 identical source inputs.
Evidence: `tasks/project-completion/evidence/legacy-inflight-drain-first-20261018/README.md`.
Only ordinary E>limit ordering is accepted; cancellation-first, other byte condition, Applied,
cold-process/uncertain-COMMIT and full Q10c/canonical014 release gates remain open.


### Current scanned original-lease expiry — 2026-10-18

Actual public binding narrowing preserves the report activity but natively closes its incompatible
session and expires the original lease while full checkpoint restoration remains observed blocked.
After release, the scanned inline report returns Aborted/STALE_ATTEMPT with raw post-mutation
execution and complete tenant authority unchanged. Exact quota/audit/expiry-only deltas, original
natural-expiry control, checkpoint/catalog preservation, no refusal/receipt/outbox/head effect,
cold journal reconstruction and log redaction pass. Final focused1/1 default/runtime, both actual
sagat_app; strict Clippy/fmt/architecture and1125 identical source inputs.
Evidence: `tasks/project-completion/evidence/refusal-current-lease-20261018/README.md`.
One fixture registration-order failure is preserved. This proves current lease expiry after path
drop inside full restore, not a literal pre-begin/generation/terminal race or full Q7b closure.


### Immutable-old public cancellation before success — 2026-10-18

D2 passes four real old/current phases in198.77s, actual postgres/postgres and
sagat_app/sagat_app pools. Public CANCEL owns the exact first CANCELLED/null stop and operation
receipt before normal cancelled drain. Original live lease/reservations survive waiting; original
success records SUCCEEDED without output/report outbox and remains genuinely compensation-eligible.
Exact original report receipt/ACK and operation receipt replay preserve their separate semantics.
Only one truthful successful-output-refusal audit is allowed; all other/NULL data-limit reasons
are excluded after waiting/success/replays/finality. Real original-target/key compensation must
complete before final CANCELLED, with DONE/COMPENSATED saga and unchanged first stop/drain.
Shared default and drain-first regressions each pass four phases (173.73s/154.46s).
Final strict Clippy33.54s, fmt7.03s, architecture0.97s; each execution pins1125 unchanged inputs.
Source/evidence:5ad34370, `tasks/project-completion/evidence/legacy-cancellation-first-20261018/README.md`.
This supersedes the earlier cancellation-first pending status only. Ordinary E>limit is accepted
for these three orderings; under-limit increment, Applied, OS-process/uncertain-COMMIT recovery,
output-dependent old compensation, full Q10c and all eight canonical014 tasks remain open.


### Supplied-pool public bootstrap prerequisite — 2026-10-18

Source/evidence c7f62b4a adds only a test-support supplied PgPool constructor and genuine public
Worker reopener. Original capability/current JWT survives exact owned-DB reconstruction; one
sagat_app backend is observed at the exact public receipt gate. Actual250000-byte persisted output,
session credential hash, independently canonical receipt digest and full replay identity pass.
Final process-guarded api_reopen:: default2/2 (3.04s) and runtime2/2 (2.66s), both API pools sagat_app;
strict Clippy18.07s/fmt4.91s/architecture1.02s and1125 unchanged inputs. Owner inspection confirms
all four terminal fixtures and removed gates. The90s inner timeout is cooperative; required final
GNU timeout supervises the whole process at180s/TERM+5s/KILL, covering synchronous lifecycle.
Evidence: `tasks/project-completion/evidence/public-pool-bootstrap-20261018/README.md`.
This accepts P0 bootstrap only. Supplied-pool scoped rollback/full reopen and actual lost-COMMIT
recovery remain separate; no Q7b/canonical task or production activation closes.


### Immutable-old fitting total / refused increment — 2026-10-18

8b8046e5 accepts only ordinary report-first E<=limit<E+candidate over genuine frozen6c44a0e4
history: L2000000,V2000020,E4000020, actual250000-byte candidate makes4250020>4194304.
Full production overlay fits without backfill; normal pre-report Waiting/candidate exclusion and
valid native ExistingOutputs refusal preserve complete execution/telemetry/saga footprint.
Original successful report stays SUCCEEDED/nooutput with exact canonical receipt/replay/zerooutbox;
fresh owned scheduler starts StoppedForRunDataLimit with its own truthful audit/outbox. Real original
target/key compensation and terminal original-ACK replay pass with retained sources/bytes unchanged.
Newcase four phases156.91s; original default/drain/cancel regressions four phases each132.53/135.47/
145.53s. All16separate phases pass with actual postgres/postgres or sagat_app/sagat_app pools,
1125identicalinputs across all before/after pairs and8created DB/private-receipt cleanups verified.
Strict Clippy42.33s/fmt5.74s/architecture0.76s/shellsyntax pass. Evidence:
`tasks/project-completion/evidence/legacy-underlimit-increment-20261018/README.md`.
Only this report-first byte condition closes locally. Under-limit prior cancellation/drain, Applied,
post-ACK OS/uncertainCOMMIT, frontier/CAS/rollout/general nesting and full Q10c/canonical014 stay open.


### Public scoped refusal after complete API reopen — 2026-10-23

Source/evidence98a1247a accepts P0b only. Real public17-member setup retains4M of sixteen outputs;
the original seventeenth250000-byte command is submitted after exact owned-DB/capability reconstruction.
An actual receipt advisory barrier ties its write to the supplied max1 sagat_app backend. First
FAILED/RUN_DATA_LIMIT plus SUCCEEDED/nooutput, original canonical receipt/ACK and byte-identical replay
preserve complete session/expiry/authority, tenant iteration ledger and retained source/payload rows.
Default1/1 (47.69s including compilation), runtime1/1 (6.63s), api_reopen2/2 (4.58s), ordinary-refusal1/1
(9.59s); no ignored tests. Root SQL independently verifies both positive databases. Final Clippy17.99s,
fmt4.89s/architecture0.39s pass;1126native hashes match all before/after pairs. The first actual0/1
fixture failure is retained: shard release expires rows instead of deleting owner history; the corrected
oracle requires0liveleases/0members and empty Placement. No production state was patched.
The2026-10-23 executions use only repository-declared Compose PostgreSQL on exclusive loopback65118
and a fresh private target outside all checkouts. Evidence/reproduction:
`tasks/project-completion/evidence/public-scoped-reopen-baseline-20261018/README.md`.
Local .5b closes only the unarmed reopener prerequisite. Actual committed/aborted COMMIT reply loss,
post-ACK process recovery, Q7b.5/master quota correction and all canonical task gates remain open.
