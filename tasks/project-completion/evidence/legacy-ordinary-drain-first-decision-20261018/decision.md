# Q10c ordinary legacy success after drain starts — bounded implementation plan

Status: source-backed design; no execution evidence from this agent. Root is the sole repository writer/executor. Read baseline: `feature/016-project-completion`, HEAD `3a9f8c4fb4ccb13f8c87c40683764373a757e654`, in `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`. The five-file ordinary report-before-drain fixture is root-owned work in progress. This document does not review that fixture or replace its retained review/execution evidence.

## 1. Decision and exact scope

Reuse the existing immutable-old producer and add one independently named current consumer for **ordinary, inline, no-declared-secret success after the current normal scheduler has committed ExistingOutputs drain**. The original worker still holds the exact live `c10/0/reserve` lease. The success request is the unchanged request retained by the old producer. A cancellation request is not a cancellation acknowledgement, lease revocation, failed attempt, or reconciliation verdict.

The supported result is:

1. Current normal `Scheduler::tick` discovers genuinely old `E > 4 MiB` state while the scope is OPEN and its sole member is still LEASED. It commits FAILED/RUN_DATA_LIMIT stop, RUN_DATA_LIMIT scope drain, and a cancellation request for that attempt.
2. The attempt stays LEASED with the original token, worker/session, attempt and generation. Its admitted iteration keeps its slot. A further ordinary tick waits; it cannot drain that member or plan compensation while the attempt remains live.
3. The original worker submits the genuine success. The current refusal path preserves the existing stop and drain, records SUCCEEDED with no retained output, writes the original report receipt and reported-lease provenance, and releases no forward work. The ACK is `SUCCEEDED/CANCELLING`, with its actual committed report version. `cancel_requested` clears on the successful step, as the existing reducer specifies.
4. Normal scheduling then drains the now-settled iteration and plans exactly its real `line.release` compensation. Once the saga is active, Run and saga are COMPENSATING. Only an actual successful compensation report and normal finalization permit Run COMPENSATED; the owner/scope remain FAILED/RUN_DATA_LIMIT and the forward step remains SUCCEEDED.

No production change is indicated by this source read. First execute the new regression against the current implementation. If it fails, retain the failure and localize that precise prerequisite; do not preemptively add a new command, grant, hook, status, reconciliation mode or retry policy.

This closes only the ordinary E>limit drain-first ordering if its gates pass. E<=limit<E+candidate, Applied reconciliation, unknown COMMIT, post-ACK process death, output-dependent compensation, secret races, general nested execution, rollout and release gates remain separately scoped.

## 2. Source-backed behavior and boundaries

| Contract / implementation | Consequence for this case |
| --- | --- |
| Constitution I/III/IV/VIII; 014 FR-014-005/014/015/016; master quota decision section 11.2/11.4 | Preserve current fencing, immutable historical source, known success and truthful compensation. Old-produced runtime state is required. |
| `collection-result-quota-correction.md:203-240`, `scoped-output-refusal-discovery.md:23-44` | A prior stop wins; root persist is skipped; report writes no outbox obligation. Already DRAINING scope resumes its recorded obligations. |
| `scheduler/src/passes.rs:604-635,1876-1984`; `infra-postgres/src/maintenance.rs:176-225` and following nomination query | Normal tick runs collection discovery first. ExistingOutputs is checked before forward progress when there is no stop; a stopped/DRAINING scope has priority over a new quota refusal. The bounded candidate query is a hint, not authority. |
| `infra-postgres/src/primitives.rs:823-905,2406-2515,2633-2776` | Locked native writer revalidates E, exact scope, revision and scheduler fence. It appends stop/drain/cancel events and sets only `cancel_requested` on the leased member; it does not clear its lease or replace its attempt. Audit reason is the current enum string `existing_outputs`. |
| `scheduler/src/passes.rs:1699-1809,1951-1984,5292-5320`; `infra-postgres/src/primitives.rs:2800-2910` | Both planning and native iteration drain reject still-LEASED/RUNNING work. An admitted slot is not released merely because cancellation was requested. |
| `application/src/worker/scoped_reports.rs:219-280,350-405`; `worker/scoped.rs:12-84` | Current worker binding and membership precede receipt access. This no-secret inline path uses existing LeaseMatch policy. The fixture must independently prove its lease remains unexpired, since LeaseMatch itself does not sample expiry. |
| `application/src/worker/scoped_output_refusal.rs:29-174` | Fresh full transaction repeats current authority/receipt/fence/budget checks. A prior stop validates through a clone and requires empty root progress, then commits only the genuine addressed completion with report provenance, typed audit and the original ACK receipt. |
| `domain/src/execution/command.rs:555-695`; `plan.rs:95-132`; `state.rs:1363-1373` | DRAINING ancestry is still live while the iteration is ADMITTED. Preparation checks exact identity/fence; success is legal with `cancel_requested`; no forward routing occurs under a stop; success clears the cancellation flag. |
| `domain/src/execution/scoped_sagas.rs:67-97,125-183` | Scope compensation waits for active_count=0, selects successful forward steps, and carries RUN_DATA_LIMIT as the saga cause. A known SUCCEEDED report needs no manufactured unknown/Applied verdict. |
| `domain/src/execution/state.rs:428-441,1538-1558,2379-2399`; `infra-postgres/src/primitives.rs:3060-3198` | Status follows real lifecycle: CANCELLING before saga, COMPENSATING during saga, then COMPENSATED for this failed scope after successful undo. Successful undo of cancellation instead yields Run CANCELLED. |
| `collection-end-result-correction.md:83-105`; master quota decision section 3/4 | Old successful owners and missing historical End/Run results are never repaired. Effective output bytes can remain above the ceiling while zero-output drain/compensation progresses. |

The source citations identify inspected code, not executed guarantees. Parent subsequently reported that report-first commit `f026e928` passed all four actual old/current phases (`postgres/postgres`, `sagat_app/sagat_app`), with the COMPENSATING expectation corrected, frozen 1124 inputs and strict gates. I did not rerun it. This plan follows that domain behavior and does not infer that drain-first has run.

### Reconciliation boundary

The selected frozen definition has `compensateWith: line.release` and **no** `reconcileWith`, highRisk or irreversible setting. Assert no reconciliation task/event/verdict appears for `c10/0/reserve` in this case. Ordinary success is already known and should directly remain compensation-eligible. Do not add reconciliation merely to make drain-first finish.

If the original attempt actually expires, receives an acknowledged cancellation/failure, is retried, or loses its fence before success, this is no longer the planned case. The scheduler's existing lost-attempt/unknown-effect policy owns that path; stale original success must not be used to overwrite the resulting state. A new fixture for that behavior requires its own bounded scope. No Applied claim follows here.

### Current secret and blob policy is not widened

Declared-secret completion takes the existing live fence with cancellation disallowed during scanning. Blob completion has its separate live/cancellation-tolerant policy. The selected old producer has neither. Do not generalize the ordinary no-secret success result to secret scans, blobs or expired leases, and do not change those policies to obtain this proof.

## 3. Minimal fixture reuse

Keep these root-owned files unchanged for the first increment:

- `tests/fixtures/legacy-collection-inflight-report-v1/producer.rs` — same old commit, same public workflow/start/completions, same genuine leased final activity, same private 19-field handoff, same 3600-second lease/session settings.
- `tests/integration/workflow-primitives/main.rs` — existing module registration is sufficient.

The producer's historical name/format `q10c-ordinary-before-drain-v1` describes an already reviewed handoff and need not be renamed. It contains the required **pre-current-action** state for either ordering. Document that the consumer case selects the subsequent order. The new evidence namespace and test name must explicitly identify drain-first; never silently relabel prior report-first artifacts.

Within the existing consumer module, share only unchanged attach/migrate/source-witness/auth/receipt helpers. Add an exact named ignored test:

`legacy_collection_inflight_report_v1::old_ordinary_success_after_drain_preserves_the_report_and_compensation`

Add a closed **fixture runner** case selector, e.g. `--case drain-before-report`, with default preserving the current report-before-drain run. It selects the exact current test name and fresh artifact namespace; it changes no production runtime option. Each selected case still runs a fresh old producer + current consumer separately for owner and runtime roles. Never run two destructive consumers against one receipt/database or reuse a database from the report-first gate. Build both binaries before producing any retained lease, retaining the current coordinator's owned database/credential cleanup and manifest behavior.

Required public authority is unchanged: fresh JWT from current test IdP for the original subject/tenant; persisted original namespace Worker membership, exact activity binding, worker UUID, session token, TaskLease and CompleteStepRequest. Reconnect without grant/bind/register calls or session/lease SQL updates. TaskLease.tenant_alias remains intentionally empty: tenant authority comes from authentication. A fresh connection is not worker re-registration. Renew scheduler ownership using the already persisted scheduler identity through the real Keeper; it is independent of worker authority.

## 4. Required real oracle, in order

### A. Genuine handoff and current read

Reuse the producer and migration/read-only witnesses. Confirm the original ten owners `c0..c9` remain successful with their old missing owner outputs. Verify `L=2,100,000`, `V=2,100,020`, `E=4,200,020 > 4,194,304`; the final inline output is exactly `1`. All bytes came from old public execution. The selected c10 scope is OPEN with one ADMITTED, slot-held member, zero checkpoints and a genuine original LEASED reserve. Assert current database time is before both actual lease and session expiry before any current scheduler action.

Capture the existing identity/source witnesses and current c10/run/usage/journal/report-receipt/saga state. Initial report receipt, selected completion, cancellation request and scope saga counts are zero. The source witness remains exact throughout; missing old certificates remain missing rather than being fabricated for this test.

### B. Current normal discovery and first drain commit

Reconstruct current Scheduler/Keeper over the same migrated database and call **normal `Scheduler::tick`**, with no direct native drain command, target seeding or broker delivery. Assert `failed=0`, a selected collection transition occurred, and the persisted result—not merely the Report count—has:

- Run CANCELLING, stop `(FAILED,RUN_DATA_LIMIT)`; c10 DRAINING, drain_kind RUN_DATA_LIMIT, drain_iteration NULL, failure code RUN_DATA_LIMIT, no aggregate/result.
- Exactly one new root stop, one `collection.drain_started` with `reason=run_data_limit`, one exact scoped `step.cancel_requested` for the original reserve. No StepCancelled, failure, retry, success, return/checkpoint, reconciliation, scope saga, compensation or End/Run terminal event.
- The original step is still LEASED, `cancel_requested=true`, with the same step/task identity, attempt/generation, lease-token hash, leased_by and live expiry. c10/0 remains ADMITTED with its original slot and reservations; no new admission.
- One `run.data_limit_exceeded` audit with `existing_outputs`, engine actor and original trace. One new `collection.drain.started` outbox row with public `reason=FAILED`; no extra report outbox/receipt.
- Journal head = Run version = telemetry head. L/V/E and payload references are unchanged; original source witness is exact.

Freeze the complete **stop/drain identity** after this commit: stop event row, reason/code, scope drain_kind/iteration/code/sequence/outbox identity and original drain event/outbox/audit rows. Compare those exact values after later success, replay and compensation. Do not compare a whole mutable scope row after finalization.

Sequence nuance: `start_collection_drain` stores the **final transaction sequence** in `scope.drain_sequence`, after any cancellation-request events. With one leased member, the drain event precedes that final cancellation event. Do not assert `scope.drain_sequence == collection.drain_started.seq`. Assert actual event order `stop < drain < cancel_requested < later success`, with the post-drain head equal to the stored drain_sequence.

### C. Prove the live-member wait

Call one further normal tick before submitting success. This is a deliberate liveness control, not a timing race. Confirm the lease is still live on database time, no report occurred, no failure/expiry transition occurred, and exact c10 member/stop/drain/saga state is unchanged. Collection advancement is Waiting/skipped; no slot release or compensation task exists. Assert no `collection.iteration_drained`, no second drain, no forward release. A bounded worker poll may confirm no available task but is unnecessary if it would add session renewal noise; persisted absence is sufficient with the native tick witness.

If this control instead ends the member or creates compensation, retain that genuine failure as a prerequisite defect. Do not submit a success under a replaced lease and call it the requested case.

### D. Public original success and exact replay

Submit the **exact retained CompleteStepRequest** via current gRPC using original worker authority. Assert first ACK `already_applied=false`, `step_status=SUCCEEDED`, `run_status=CANCELLING`, `command_id=<original Run>:<report committed head>`.

Inspect durable state before another scheduler tick:

- The original step is SUCCEEDED, no output_digest, correct reported_lease_token_hash and original attempt/generation. `cancel_requested=false` follows actual success. Exactly one original-worker StepCompleted exists with no payload_digest.
- Run stop and complete frozen drain identity from B are byte-for-byte unchanged. No second root stop or drain event/audit/outbox; scope stays DRAINING, member stays ADMITTED until the scheduler drains it. The report has only the current `successful scoped output refused by run data limit` typed engine audit plus its worker event/receipt provenance.
- No Return/End/successor release, aggregate, output reference, blob commit or additional logical byte charge. Preserve unchanged L/V/E and relevant payload/reference snapshots. A raw payload-count check alone cannot prove absence of retention because scalar `1` may already be content-addressed in old inputs.
- The report's new outbox count is **zero relative to the post-drain baseline**. The drain's already committed outbox remains. Final telemetry, Run and journal heads agree.

Use the existing exact receipt oracle: `(tenant_id,scope,idempotency_key,request_hash,resource_id,created_at)`, canonical scoped address plus lease hash-prefix scope, length-prefixed `complete`/attempt/generation/exact output fingerprint, stored resource `SUCCEEDED|CANCELLING|<original report version>`. Retain the complete receipt, not only a count or key.

Replay the exact request immediately under fresh current JWT with the same worker subject and original authority. ACK preserves command_id and statuses with only `already_applied=true`; exact receipt row and report state/audit/outbox/ledger/head snapshots stay equal. Perform cold full `verify_run` after the drain wait and after report/replay using a fresh pool. This proves reconstruction, not process death or an unknown COMMIT.

Do not require the replay command_id to equal the **latest** Run head after subsequent scheduling. An optional later replay must compare to the saved original ACK/receipt; its stored run_status stays CANCELLING even if the Run is now COMPENSATING/COMPENSATED.

### E. Real compensation and terminality

Drive normal ticks until current public polling yields exactly the real COMPENSATION lease `reserve~c`, activity `line.release`, target_instance equal to the original reserve instance, target_node reserve, target_idempotency_key equal to the original forward effect key. No reconciliation/extra forward task may be offered. Assert one saga entry for that original success, and that the iteration was drained exactly once before compensation planning.

While the compensation lease remains unfinished, run another bounded normal tick and assert no terminal Run/collection finish. Run and saga are COMPENSATING, saga outcome is NULL, and the compensation remains the real leased obligation. Submit its genuine public success and drive normal completion. Assert Run COMPENSATED, scope/owner FAILED with RUN_DATA_LIMIT, saga DONE/COMPENSATED, original forward SUCCEEDED/no output, exactly one compensation completion, one collection finish and one Run finish with their actual outbox obligations; no End result. Verify journal and immutable source/stop/drain/receipt identities and unchanged effective bytes.

This proves public engine compensation semantics using the fixture worker. It does not prove a provider effect, provider undo or exactly-once external behavior. The missing-output limitation is already a separate current-version test and is not repeated or claimed as old-binary acceptance here.

## 5. Cancellation precedence: distinct narrow companion

Use a **fresh invocation of the same old producer** for a later companion test:

`legacy_collection_inflight_report_v1::old_ordinary_success_after_cancellation_drain_preserves_the_original_stop`

Before any current scheduler tick, perform the authorized current HTTP `POST /runs/{id}/operations` action CANCEL through the existing original admin/operator context, namespace header and an ordinary dedupe key. Assert its accepted durable stop is `(CANCELLED,NULL)`. Then normal tick must start CANCELLED drain despite E>limit and request cancellation of the original lease. Repeat the same live-wait, original success, receipt replay and real compensation oracle.

| First committed stop | Drain / report behavior | Final result after successful compensation |
| --- | --- | --- |
| Scheduler ExistingOutputs failure | Keep `(FAILED,RUN_DATA_LIMIT)` and original RUN_DATA_LIMIT drain; ordinary report adds no stop. Any later CANCEL is AlreadyStopping/state conflict, not a replacement. | scope/owner FAILED, saga COMPENSATED, Run COMPENSATED |
| Public CANCEL before scheduler drain | Keep `(CANCELLED,NULL)` and CANCELLED drain; no ExistingOutputs/StoppedForRunDataLimit drain audit. The report may still write its distinct output-refusal audit while preserving success. | scope/owner CANCELLED, saga COMPENSATED, Run CANCELLED |

Capture and compare the first stop's exact event row and cancellation operation audit/receipt. A report with no output retention does not retroactively change cancellation to RUN_DATA_LIMIT. The cancellation-first ACK before saga planning remains SUCCEEDED/CANCELLING; final cancellation is intentionally different from the primary case. No failed-worker acknowledgement is used as a stand-in for an original successful effect.

These ordered commits prove precedence, not simultaneous race scheduling. A full lock-order/cancel-versus-report matrix belongs to the separate remaining race gate. If the public cancellation path fails against genuine E>limit, retain that as its own prerequisite defect rather than directly writing a stop row or weakening the test.

## 6. Ownership and exact verification scopes

Every increment has at most five logical files; root may split further. No change to production, old producer, migration, SDK, public contract, grants or module registration is presently required.

| Increment | Files (maximum five) | Gate |
| --- | --- | --- |
| D1 ordinary drain-first fixture | existing current consumer; existing coordinator script; existing fixture README; master quota contract checkpoint or bounded plan; one evidence manifest | New exact current test plus old producer in each independent owner/runtime invocation; live cancellation fence, exact original receipt/replay and actual compensation. Preserve report-first case unchanged. |
| D2 cancellation-first companion | same consumer; coordinator selector; README; contract checkpoint; separate evidence manifest | Fresh old production per role, public cancel before drain, original success and Run CANCELLED only after compensation. Separate result from D1. |
| D3 evidence/ledger if needed | retained raw logs/manifests/notes and task ledger, no more than five logical files per artifact increment | Actual commands/results/source pins only; no full Q10c or canonical 014 closure. |

Planned coordinator commands after implementing the selector (not executed here):

```sh
CARGO_TARGET_DIR=/absolute/private/root-owned-target \
SAGAT_TEST_DATABASE_URL='<owned fixture PostgreSQL admin URL>' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case drain-before-report
```

The coordinator performs one exact old producer and one exact current consumer in owner mode, then fresh old producer/current consumer in runtime mode. State the four process invocations and the two role-specific compatibility cases accurately; do not call them a large protocol matrix. For D2 select `--case cancellation-before-report` in a separate invocation/evidence directory.

Exact current binary selector for D1, owned by the coordinator:

```text
--ignored --exact legacy_collection_inflight_report_v1::old_ordinary_success_after_drain_preserves_the_report_and_compensation --nocapture
```

Exact old producer selector stays:

```text
--exact legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_success_before_drain_producer --nocapture
```

Do not report a standalone current `cargo test` as old/current compatibility: it requires the privately produced receipt/database. Root reruns the original report-first selector because D1 changes its shared coordinator/module; it is one direct regression scope, not a whole feature run. Run relevant integration-target strict Clippy, formatting, architecture/agent-config gates required by the repository. No additional all-five or full workspace/fanout/performance run is requested by this bounded test-only design.

Retain immutable old archive/source/migration, overlay, current source/diff and binary hashes, exact test names/role/count/durations/exit statuses, original failures, and the nonsecret execution witnesses. Do not retain the credential-bearing receipt in public evidence. Keep the existing private 0700 directory/0600 receipt, original worker secret handling and cleanup. Check disk first, private target outside repo, at most two heavy builds; only root-created temporary resources may be removed.

## 7. Prerequisites, limitations and read provenance

No prerequisite production defect was found in the inspected ordinary still-live path. This is an inference from source, not a passing test. The decisive points to stop and diagnose are: old/current attach or authority mismatch; discovery missing the live full-concurrency scope; drain replacing the lease; an extra tick releasing the live slot; stale report refusal despite the unchanged live fence; false CANCELLED/FAILED instead of successful report; a replaced prior stop/drain; duplicate receipt effects; or finalization before real compensation. Preserve such failure without fixture SQL repairs or blind reruns.

Read repository AGENTS, constitution, complete active 014 spec/plan/tasks, the three requested correction/discovery contracts and targeted current source above; checked local branches/worktrees and found no nested AGENTS under the targeted crates/tests/scripts/specs/tasks inventory. No builds, tests, formatters, container/DB mutation, commits, repository edits, messages outside this agent team or subagents were performed. Only this authorized temporary document was written.

Selected inspected SHA-256 pins (current fixture may continue changing under root ownership):

| File | SHA-256 |
| --- | --- |
| application/worker/scoped_output_refusal.rs | `5c9d80bf363360d76c6764fee582f6a6a1045c947ce0b6cc68b973e71413affb` |
| application/worker/scoped_reports.rs | `a6b2eba23746c4a1c67fa51bb8597280935357209084dcc53e1f567e4748120f` |
| domain/execution/command.rs | `8b0ca691f687b2a6c2264011bc2e353b9d7f9d611e1a9203a3ce5bcc0701a70c` |
| domain/execution/scoped_sagas.rs | `38090402ef266554e8f87a455ca6e6f3c3ab106ab588ad85df54fab5d3b00452` |
| infra-postgres/primitives.rs | `14532a4fb34ae5a7cecd5c8ce8cef80b6f2cbfa8e2a824349b50bb2999921418` |
| scheduler/passes.rs | `7798c15380bdd10d43072316467cb990326090519fa1750c89da190ccb5d7d40` |
| existing old producer overlay | `c12a5986c7561fb27c711c505c1fad1da17d2e751fefeb2301d2231cbca616e4` |
