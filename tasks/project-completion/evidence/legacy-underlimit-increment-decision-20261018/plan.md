# Q10c first under-limit retained-total compatibility unit

Status: bounded design, source inspection only. No implementation, test pass, acceptance, release, or task closure is claimed.

Baseline: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`, commit `d30f5a74320bdb3f1c250277ac9e2698585007bd`. All line references below refer to this commit unless explicitly marked immutable-old. Existing ordinary report-before-drain and drain-before-report cases are the baseline. The separate cancellation-first D2 work is not an input or dependency of this decision. When shared working-tree consumer changes became visible during inspection, further source references were pinned to the baseline; no D2 conclusions are included.

Read: repository AGENTS, complete constitution, feature014 spec/plan/tasks, the relevant collection-result-quota-correction contract (especially §§11.2/11.4), and scoped-output-refusal-discovery. `.specify/feature.json` is absent here; the assigned feature014 files and explicit Q10c scope govern this bounded work. Branch/worktree inventory was inspected. Root remains sole repository writer and executor. This agent ran read-only discovery/source commands, no builds, tests, formatter, container, database operation, commit, or subagent; its only write is this temporary plan.

## Decision and exact arithmetic

Add one closed test-only producer variant and one named current consumer for an ordinary inline successful report, submitted before any drain, over genuine immutable-old collection aggregates. Reuse the established two-process runner and original capability handoff. No production change is justified by inspection.

Keep the published graph unchanged in shape: c0…c9 each contain one `line.work` activity and a Return of `steps.work.output`; c10 contains one `line.reserve` with `compensateWith: line.release`, followed by Return of literal `1`; all eleven root collections contain the single input item, concurrency1, then root End. The compensator can execute with original task context and target identity; it does not require the refused output.

The new closed preset uses the following exact serialized UTF-8 byte sizes:

| Quantity | Construction | Bytes |
| --- | --- | ---: |
| Each old ordinary output | `Value::String("x".repeat(199_998))`, including JSON quotes | 200,000 |
| Each successful old scope aggregate | one-element JSON array containing that string | 200,002 |
| Stored ledger L | ten distinct scoped output references, even though contents share a digest | 2,000,000 |
| Verified virtual owner outputs V | ten distinct successful root owner identities, each missing its physical output | 2,000,020 |
| Effective retained outputs E=L+V | prior to the new report and after refusal | 4,000,020 |
| Actual retained completion candidate d | `Value::String("y".repeat(249_998))`, serialized once into the original CompleteStep request | 250,000 |
| Existing limit | 4×1024×1024 | 4,194,304 |
| Headroom | limit−E | 194,284 |
| E+d | exceeds limit by 55,716 | 4,250,020 |
| L+d | fits if virtual aggregates are wrongly omitted | 2,250,000 |

Every old output and array and the new candidate is smaller than the independent 262,144-byte value limit. Thus this is incremental run-output refusal, not per-value rejection or already-over-limit recovery. The candidate is a real wire completion value, not a caller-entered budget length. All bytes are asserted from actual serialization and independently measured stored payload lengths; arithmetic constants alone do not prove fixture provenance.

## Source evidence and implications

| Source | Consequence for this unit |
| --- | --- |
| Existing producer157–189,365–465 | Public published root collection chain, ordinary compensable final body, public binding/quota/start/register/poll/complete, normal Keeper/tick. Ten completed source members already produce the required historical shape. Only closed fixture payload values and namespace labels need a variant. |
| Immutable-old `6c44a0e49670cf19660792cd1518d241e6c99b3b`, `infra-ir/src/convert.rs:740–792` | The already-supported referenced activity-chain bodies remain legal. There is no new graph, capability, worker minor, effect policy, or collection count. |
| Immutable-old `infra-postgres/src/primitives.rs:2664–2669,2742–2757,2840–2874` | Old finish retains the aggregate on the successful scope, updates owner status/routing without output_digest, and appends collection.finished plus matching step.routed. These are genuine virtual owner outputs after upgrade. |
| Immutable-old `infra-postgres/src/execution/path.rs:6–15,635–685` | Old path ledger reads/charges actual step references only. Ten 200,000-byte successful reports fit; scope aggregate storage is not an owner-output ledger charge. No SQL ledger manipulation is needed. |
| Current `infra-postgres/src/collection_outputs.rs:27–89,145–245`; execution.rs:3283–3297,3486–3491 | Effective totals verify pinned root owner/scope/journal metadata. E=4,000,020 permits the bounded full overlay read, while the scalar consistently returns L+V. SQL witness totals are supplemental; actual production readers must agree. |
| Current `application/src/worker/scoped_reports.rs:350–399`; scoped_output_refusal.rs:24–174 | Ordinary candidate bytes enter checked E+d preflight, then fresh full authority/receipt/fence/budget validation. Refusal records genuine success without output and owns the first RUN_DATA_LIMIT stop. Existing ordinary no-secret LeaseMatch policy is preserved. |
| Current `infra-postgres/src/maintenance.rs:218–273` | Before report, a healthy OPEN full-concurrency scope containing only the original live lease and E<=limit is intentionally excluded from collection_candidates. After report, CANCELLING itself nominates the OPEN scope even though E still fits. Expecting pre-report discovery would be incorrect. |
| Current `scheduler/src/passes.rs:1888–1948,1986–2065` | With no stop and E fitting, normal explicit advance_collection returns Waiting for this live-only scope. After refusal, the durable FAILED/RUN_DATA_LIMIT stop selects StoppedForRunDataLimit before any output-limit/frontier work. |
| Current `infra-postgres/src/primitives.rs:875–900` | StoppedForRunDataLimit requires authoritative CANCELLING/FAILED/RUN_DATA_LIMIT. ExistingOutputs requires live RUNNING/WAITING, no stop, and freshly verified E>limit. Test its rejection before the report so a prior stop cannot mask the byte predicate. |
| Existing consumer1309–1655; helpers1176–1304 | Original identity handoff, real current gRPC report, stable receipt, cold verify, normal tick, actual compensation poll/report, and final COMPENSATED checks are established patterns. Existing hardcoded totals/value assertions must stay exact for the original cases. |
| Master§11.2/11.4 plus discovery amendment | Zero report outbox rows is required. The later real drain creates its own outbox/audit. No forward work may occur between ACK and drain. This bounded unit does not close the full Q10c family. |

## Closed selector and preservation of existing v1 cases

Proposed exact new names:

- CLI: `--case under-limit-increment-before-drain`.
- Old test: `legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_under_limit_increment_producer`.
- Current test: `legacy_collection_inflight_report_v1::old_under_limit_increment_before_drain_preserves_the_report_and_compensation`.
- Private receipt format: `q10c-ordinary-under-limit-increment-v1`.
- Artifact namespace: `sagat-q10c-ordinary-under-limit-increment-v1`.
- Log label: `ordinary-under-limit-increment-before-drain`.

Use a closed two-variant test-only enum or equally explicit named preset in the producer: original 210,000-byte sources plus candidate `1`; new 200,000-byte sources plus 250,000-byte candidate. No arbitrary numeric environment variables, public API parameter, runtime flag, production profile, or generic fixture language. Parameterize only the fixed source/candidate construction, expected source totals, and unique fixture labels. Give the new preset its own workflow name, worker subject, scheduler owner, and report key prefix. Leave the no-argument selector and both original mappings, old format, graph, 210,000-byte sources, candidate `1`, test names, labels, and assertions unchanged in meaning.

The current consumer must select its expected preset from its named test, not trust byte counts from a receipt. Keep the existing 19-field private receipt shape; its format discriminates presets. Existing `Receipt::parse` can remain the original wrapper around a narrowly parameterized parser. The new test accepts only its exact new format. Reject mismatches/unknown fields. Helpers that currently hardcode original totals/output must retain original wrappers or accept a closed preset with explicit old values; do not weaken them to relative inequalities. Shared fixture boilerplate may be extracted locally only where it preserves the old assertions; no production abstractions are warranted.

Build the unchanged immutable-old production commit via the existing archive plus test-only module overlay/main registration. Record/compare the complete source manifest and old migrations before/after removing the overlay. Freeze both test binaries before either producer creates a lease. The old source pin, Cargo.lock, toolchain, complete overlay/consumer/coordinator source hashes, binary hashes and exact selected test names remain mandatory evidence. Restore the original old test main after compilation. Never edit archived engine source or old migrations to make the smaller values work.

## Producer and original capability handoff

1. Use only the frozen old public paths for tenant/member setup, binding, iteration quota, workflow publication/start, scoped protocol 1.5 registration, ten real completions and normal scheduler closure. The new output sizes do not change publication semantics.
2. Leave the actual final c10/0/reserve lease unreported. Assert ten successful old scopes/owners, missing owner outputs, real finish/routing events, L/V/E exact values, live original worker/session/attempt/generation/token, scope OPEN, member ADMITTED with slot held, no stop/drain/report receipt/compensation/selected checkpoint. The original source witness remains an exact record of c0…c9 and its journal prefix.
3. Construct the real 250,000-byte CompleteStepRequest from that exact lease and retain it unchanged. The current phase may not replace its output, idempotency key, task/step identity, attempt, generation, lease token, worker ID, or session token. Serialize the canonical protobufs into the receipt exactly as today.
4. Preserve 0700 receipt directories and create-new, fsynced, atomically renamed 0600 receipts; remove credentials and owned database after every phase, including failure/signal handling. Retain the coordinator's ownership checks before cleanup. Original lease/session values and full protobufs must never appear in logs, assertion diagnostics, manifests, review files, or committed evidence. Use boolean comparisons with fixed messages for sensitive structures, as the current fixture does; safe byte lengths and hashes of private receipt/source artifacts suffice for manifest provenance.
5. Current startup attaches to the exact owned database, verifies whole projection and source witnesses before/after current migrations, and verifies original durable memberships/bindings/session identity. Connect with a fresh JWT from the current test identity provider and assign the persisted original worker/session to the transport; do not register, grant, bind, rewrite deadlines, or manufacture authority. Both old and current API roles must match the selected owner/runtime pass.

## Exact current sequence and mandatory oracles

### A. Positive control while E fits, before any new report

Use the same genuine old run rather than create another producer case. This is a positive control for readable under-limit historical state and non-stopping scheduler behavior; it is not a claim that a second smaller completion was submitted.

- Assert L=2,000,000,V=2,000,020,E=4,000,020 from stored source references; verify fresh production scalar equals E. A read-only full RunTx::data overlay may assert ten root aggregates of 200,002 bytes, ten real member outputs of 200,000, and summed outputs=E, followed by dropping the transaction. Assert source/projection witness unchanged: hydration must not backfill owner outputs or ledger.
- Renew the genuine original scheduler owner with normal Keeper and construct a normal current scheduler under its real placement. Assert c10 absent from collection_candidates (owned window or existing Window::everything diagnostic), then explicit public `advance_collection` returns Waiting. A normal tick should advance zero collections and leave target stop/drain/journal/outputs/reservations unchanged. Compare execution footprints, not scheduler-shard lease timestamps renewed by Keeper. This prevents a false positive that drains any legacy history regardless of E.
- Build one exact native ExistingOutputs command from the actual current namespace/run/scope/revision/owner and retained current scheduler lease, with the required engine audit, fresh outbox identity and no invented source fields. Call the existing PgCollectionStore on the selected API role pool; require `PrimitiveStoreError::InvalidTransition` and complete unchanged execution/audit/outbox/receipt/output footprint. Because no prior stop exists and the other authority/identity preconditions are proven, this specifically exercises E<=limit. Do not seed state, accept another error as success, or directly start the successful drain.
- Assert the original lease/session remain live immediately before the report. There are no sleeps and no deliberate lease expiry in this unit. If setup unexpectedly exhausts TTL or the graph does not produce the required shape, retain the failure and fix fixture timing/assumptions through a separate reviewed correction; do not patch durable rows.

### B. Submit the original successful candidate before drain

- Send the exact original request over current gRPC with current authentication. Require first ACK already_applied=false, step_status=SUCCEEDED, run_status=CANCELLING and command_id equal the actual committed run/journal head. Demonstrate current authentication lookup as the existing consumer does.
- Require exactly one first root stop `(FAILED,RUN_DATA_LIMIT)` and one c10/0/reserve successful completion, with original reported token evidence, attempt/generation and worker event actor. The stop precedes success; no prior stop existed. There is no reserve failure/cancel/retry/reconciliation verdict.
- Scope remains OPEN, selected member ADMITTED and slot held. No selected Return, root End, forward successor release, checkpoint, aggregate, or saga exists at this boundary. This includes no forward work in the interval after ACK and before drain.
- Retain no candidate payload, step output, blob commit/reference or logical byte charge. Compare output-reference fingerprint, payload count and exact L/V/E against before; candidate uses a distinct `y` string, so its payload cannot be hidden by equality with a source. c0…c9 source witness remains identical. No report outbox is added.
- Require exactly one successful-output-refusal Run audit with fixed reason, engine actor and trace. Final telemetry_pending head equals run_version and max journal seq. Audit/receipt/report provenance are checked independently rather than conflated into a total Run-audit count.
- Verify the complete original command receipt tenant/scope/key/request hash/resource/created_at against the exact canonical request. Repeat the identical request before drain: already_applied=true, same command_id and original SUCCEEDED/CANCELLING ACK; no new event/audit/outbox/payload/head/receipt identity change. Never substitute a second request key.
- Re-query normal collection_candidates: c10 must now be included solely through its committed CANCELLING OPEN state although E is unchanged and still fits. No broker delivery or invented READY hint is needed.

### C. Fresh store/cache discovery and actual compensation

- Reconstruct a fresh PgRunStore with fresh pool/DefinitionCache, fresh Placement and Keeper ownership after report commit, then a new Scheduler over it. Drop the pre-report scheduler before this reconstruction. This demonstrates cold store/cache/placement recovery using durable rows. The current worker transport may continue using the persisted capability. This is not an OS-process-death or third-process acceptance claim.
- Invoke ordinary Scheduler::tick, not a direct native successful-drain call or the full-run-only fallback. Require failed=0 and exactly one collection advance starting the c10 drain. Assert scope DRAINING with RUN_DATA_LIMIT, no iteration blamed, no aggregate, one unchanged original stop, exactly one drain event/original drain identity, one real collection.drain.started outbox with existing public FAILED reason, and one separately identified drain audit with reason `stopped_for_run_data_limit`. Require zero `existing_outputs`/aggregate drain audits. E remains4,000,020; thus an ExistingOutputs implementation cannot legitimately explain this accepted drain.
- Record the original stop and drain identities; later ticks, report replay and compensation must not replace them. Never assume a total audit count of1 across report plus drain.
- Poll through the original worker using bounded loops and normal ticks until the actual one engine-created COMPENSATION task is offered. Require line.release, reserve~c at c10/0, original target_instance and target_idempotency_key. No reconciliation task or duplicate compensation entry is offered. Inspect actual lease input for the original task context and absent refused forward output; do not synthesize that output.
- Hold the compensation lease without reporting, take one additional normal tick, and require COMPENSATING, unfinished run, DRAINING scope, exactly one open saga compensation obligation and no collection/run terminal event. Existing assert_pending_compensation provides the baseline pattern. Finalization must wait for this real report.
- Complete the actual compensation through public CompleteStep with a simple ordinary release answer. Require successful ACK; normal ticks then reach run COMPENSATED, selected scope FAILED/RUN_DATA_LIMIT, saga DONE/COMPENSATED, original forward SUCCEEDED without output, one successful reserve~c and one compensation entry. Its output remains discarded/uncharged under the existing compensation contract. This is engine compensation evidence, not an external provider undo.
- Replay the original forward completion after drain/finalization as well: original stored ACK remains SUCCEEDED/CANCELLING with already_applied=true and original command_id, even though the current run is terminal. Require receipt identity and execution footprint unchanged. Compare the captured receipt with `stored_report_receipt` at this stage; the existing `verified_report_receipt` helper derives the current head and therefore is only appropriate at the original report boundary. The baseline policy already replays receipts ahead of new mutation checks.
- Run fresh pool/cache verify_run after report and terminal compensation, and assert immutable source/projection pieces, original capability identity, L/V/E, stop/drain identity, no owner backfill, and no refused output across all phases. Do not describe metadata replay as literal process restart.

## At most five logical files per implementation/review unit

First implementation unit (four existing files; optional fifth is one concise local evidence/reproduction record):

1. `tests/fixtures/legacy-collection-inflight-report-v1/producer.rs`: closed second producer preset and named wrapper; keep original preset exact.
2. `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs`: closed receipt/preset validation, bounded helper adaptation and one ignored named consumer. Registration already exists; no main.rs edit is needed.
3. `scripts/test-legacy-collection-inflight-report-v1.sh`: one additional closed selector mapping old/current exact tests, format-associated namespace/log labels; all isolation/build/cleanup logic preserved.
4. `tests/fixtures/legacy-collection-inflight-report-v1/README.md`: exact new command/arithmetic and narrow claim limits; original invocations stay documented.
5. If needed, one evidence/reproduce document containing final source pins, actual commands/output summaries, failure locations and links to private-safe retained artifacts. Do not hide a sixth source/evidence file in this count.

Canonical spec/plan/tasks/validation and master-ledger updates belong to a separately reviewed documentation increment of at most five files after actual results; they are same-delivery evidence updates, not completion of canonical014 tasks. Large raw output collections must have their own explicit bounded evidence increment/manifest accounting. Root should rebase this plan onto its eventual frozen tree while preserving any independently accepted additions; this plan neither replaces nor reviews D2.

## Root execution gates

These are proposed commands/checks, not results from this agent.

1. Review diff and source pin before execution. Prove the two existing selector mappings/producer values/assertions remain semantically unchanged, no production/migration/grant/lockfile/golden/flag changes, no SQL state seeding, and no credentials in diagnostics. Validate shell syntax and git diff whitespace. Use private absolute CARGO_TARGET_DIR outside repository, check df -h, and coordinate at most two heavy builds. Use only root-owned repository-declared disposable PostgreSQL infrastructure.
2. Runner listing must find each selected old/current test exactly once. New case runs one old producer process then one exact ignored current consumer per fresh database, first owner then runtime. Execute `scripts/test-legacy-collection-inflight-report-v1.sh --case under-limit-increment-before-drain` with the established owned-PostgreSQL/private-target environment. Retain all producer/consumer exits, actual1-passed summaries, role assertions, final source/diff/binary/overlay/migration manifest hashes, and cleanup outcome. Any skipped/no-test/timeout/fixture-shape error fails this gate.
3. Execute the original default case and `--case drain-before-report` independently against fresh databases in both API modes through the same coordinator. Preserve their original210,000/1 arithmetic and old format. Do not reuse a previously consumed receipt/database or combine historical baseline counts with new source results.
4. Run formatting check and strict integration-target Clippy (`cargo clippy -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings`), architecture/agent-config checks as applicable, and the focused native ExistingOutputs/StoppedForRunDataLimit regression if new helper integration reveals a concrete risk. Root executes final merged-tree gates itself; draft reports are claims until rerun. Broader gates remain the existing release scope.
5. Existing production already implements the selected contract, so a new test passing first is compatibility evidence, not a fabricated historical RED. If a new assertion fails, preserve the exact failure; investigate the source assumption first and require genuine RED plus a separate <=5-file reviewed correction before any engine change. Do not weaken assertions, reclassify an unexplained failure, retry it away, or silently widen this unit.

No production prerequisite was found missing by read-only inspection. The fixture/producer arithmetic, pre-report candidate exclusion/Waiting, native rejection, post-report discovery, and compensation are nevertheless unexecuted hypotheses until the gates pass. A literal fresh OS process immediately after ACK remains an explicit later subcase.

## Deferred variants and bounded claims

Separate later units: under-limit cancellation/already-authoritative drain order; Applied reconciliation/substitute output and NotApplied/Unknown controls; uncertain COMMIT; pre-begin races; post-ACK process death; actual smaller fitting completion acceptance (the present positive control is before this report); boundary E==limit; output-dependent old-worker compensation; secret/blob policies and wider authority/fault/race matrices; general nesting, rollout and production ceilings. None is implemented or accepted here. The first report-before-drain unit cannot close Q10c or T014-014/015/016/017/040/042/043/044.

## Baseline source seal

SHA-256 of git objects at d30f5a74320bdb3f1c250277ac9e2698585007bd, computed read-only:

| File | SHA-256 |
| --- | --- |
| producer.rs | `c12a5986c7561fb27c711c505c1fad1da17d2e751fefeb2301d2231cbca616e4` |
| phased coordinator | `e72e92b8821b0b9a851192b1f47468988d70dbf0d407a092b94c654263d7843c` |
| current consumer | `6de755b15e5ddee31081b36fb7ba345190f58b18bb5ab99b4b9f67bee184e509` |
| fixture README | `ae3e7e303ccbcadb0dce67983df1878ed6cf51990b5b95cc7c2bf8bc837673c9` |
| master collection-result-quota-correction.md | `814cd71d4e5277aaa9c4f6b57ad959a333535a9549c755fcf3893e6c54d31578` |
| scoped-output-refusal-discovery.md | `bacbbf6371b136a1a7e6509b0153a3b38353b3be8ef5a06323483afc11cef9a0` |
