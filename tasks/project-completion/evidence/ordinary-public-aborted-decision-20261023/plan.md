# Ordinary public scoped refusal: deferred-abort COMMIT reply loss

Status: DESIGN ONLY. No implementation, build, test, SQL execution, container action or acceptance in this design.
Initial read-only baseline: `feature/016-project-completion@9b922d0e9b157b6d9e1032fc94520db5c99b5326`, with the then-uncommitted committed-loss parent/child; retain this design-time provenance.
Parent SHA256 `8fb2cb125db9bda4984a94163ad10ef52891a04ae0e2543f45febf37480e267e`; child SHA256 `31f698c6563ad7f89a72af1ac75842897d39bec00e058c48ac904b4b42e103eb`.
Unchanged proxy SHA256 `98838f211342a30320300786c474e46f0cd87005a9d5674e1787d07a5b662b23`.
P0b and owned fixture extraction are accepted per root. At initial inspection committed-loss C1 gate/extraction and corrected C2 journal-head/original-trace reviews reported None, with runtime still pending.
Root subsequently reports committed counterpart ACCEPTED at `ef12c32c`, trace `997b0335`: default 1/1 (10.03s), runtime 1/1 (7.74s), actual UNAVAILABLE/5000 plus wire Committed/Run-lock invisibility/full fresh API replay; final P0b 1/1 (48.53s), strict Clippy/fmt/architecture pass, 1127 native pins across four pairs and independent owner SQL both DBs valid. This design agent did not execute those gates.
The committed-loss acceptance is the hard implementation dependency, now satisfied by root's accepted checkpoint rather than this design or review. Root will freshly review this aborted plan before code; freeze/recheck the accepted source provenance at that handoff.
Trace: Constitution I/V/VI/VIII; FR-014-005/014/015/017, NFR-014-001/002, SC-014-003/004; partial evidence for T014-017/040 only, no checkbox or canonical closure.

## One bounded unit and ownership

Add one named test: `public_refusal_commit_loss::committed_loss::ordinary_public_scoped_refusal_aborted_commit_reply_loss_applies_after_api_reopen`.
Prefer exactly three changed files, including one evidence note:
1. `tests/integration/workflow-primitives/public_refusal_commit_loss.rs`: minimal fixture resolution parameterization and additive independent snapshot coverage.
2. `tests/integration/workflow-primitives/public_refusal_committed_loss.rs`: retain the committed test; add the aborted test using the same two-gate choreography and narrow outcome-specific assertions.
3. One concise new aborted-loss evidence/contract note; root supplies frozen command/output/inspection artifacts separately.
Keep the existing child-module path/name to avoid registration churn. No production, relay/proxy API, shared support API, schema/grant, dependency, generated-file or new-enum change.
Pass the EXISTING `CommitResolution` into the private fixture/probe/scenario; keep P0b explicitly Committed and unarmed. Do not duplicate setup, PID checks, typed-error checks or recovery teardown into a second behavior fork.
Limit branching to the fixed deferred-trigger outcome, resolution-specific durable oracle and recovery ACK expectations; no callbacks, generic fault framework, new fixture state machine or broad cleanup refactor.
Preserve existing committed and P0b assertions, especially independently frozen committed ACK, actual journal head equality, original Run trace and runtime-role recovery reads.

## Genuine fixture and immutable witnesses

Reuse the accepted public fixture unchanged in behavior: root, unchained 17-item collection, concurrency/quota 1, protocol 1.5 compensable reserve/return body; 16 genuine 250000-byte outputs yield logical/retained 4000000; distinct final 250000-byte output crosses 4194304 while protobuf remains below 262144.
Capture full original protobuf bytes privately, including original key, run/task/step/instance, output, lease token, attempt/generation and traceparent; preserve worker/session capability and every original durable expiry/hash/fence field. No Debug/log dump of capabilities.
Preserve pinned definition, run input digest and payload bytes, source items/checkpoints, OPEN scope, ADMITTED member16/held slot, all earlier steps/results and no original stop, final success or candidate payload.
Use existing complete first API/store/cache/scheduler shutdown, observed Keeper departure, same-owned-DB attachment and supplied max-one `sagat_app` proxy pool. The only new fixture input selects `CommitResolution::Aborted23514` for this case.
Keep 30s statement/idle-transaction bounds, exact database/user/application/backend-PID assertions and original live lease/session checks. No registration, repoll, renewal, expiry rewrite or SQL business-state seeding after setup.

## Exact deferred abort, in this order

Install existing exact-tenant/scope/key Gate A BEFORE INSERT on command_dedupe. Install Gate B as an AFTER INSERT CONSTRAINT TRIGGER, DEFERRABLE INITIALLY DEFERRED, with the same exact filter and a distinct advisory transaction lock.
For Aborted23514 only, INSIDE the matching branch, after acquiring B, execute fixed DDL: `RAISE EXCEPTION USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort',MESSAGE='test-only forced public refusal commit abort';`. Unmatched rows return NEW; Committed retains its current return behavior.
The constraint literal is required by the unchanged relay. Generated identities keep existing safe-DDL checks; never interpolate arbitrary exception clauses or log SQL frame bodies.
Hold A/B in separate owner transactions; save distinct keys and blocker PIDs. Own the public report JoinHandle with existing abort-on-drop cancellation.
1. Send exactly the saved CompleteStep with current JWT for the persisted subject and original session/worker. No concurrent API work uses the one connection.
2. Observe the captured writer PID blocked at the exact receipt INSERT by A, with database/user/application checks; only then call `proxy.arm()`.
3. Release A, require `proxy.wait_sent()`, and observe the SAME PID active at query=`COMMIT`, blocked by B's exact PID. COMMIT dispatch or another backend alone is insufficient.
4. Independently read full prestate through the owner connection and require canonical byte equality. Exact own-Run `FOR UPDATE NOWAIT` must fail specifically with 55P03, proving the unresolved transaction still holds its guard.
5. Call `lose_reply()`, then await the actual public error before releasing B: tonic Unavailable, message `service temporarily unavailable`, `x-sagat-error=UNAVAILABLE`, `retry-after-ms=5000`; never an ACK or arbitrary error.
6. After observing that error, re-observe same PID still unresolved at COMMIT/B, same full prestate and same own-Run 55P03. Current authentication lookups must have increased.
7. Only now release B. Require `proxy.resolution()==Aborted23514`: actual backend ErrorResponse with code 23514 and the fixed constraint, followed by ReadyForQuery idle. Client EOF, ROLLBACK, timeout, cancelled query or dropped backend is not abort evidence.

## Rollback oracle BEFORE any retry

From independent owner reads, compare deterministic serialized full snapshots byte-for-byte against the original `before`; no ignored timestamps, sanitized fields or counter-only substitute. The command bytes and exact lease witness must also match.
Snapshot coverage: full run/version/input reference; source items, scope, members, checkpoints, all steps/retained outputs; every journal row and referenced payload bytes; telemetry_pending; run_instance_usage; tenant_iteration_usage; tenant_retained_usage; every tenant payload/blob, receipt, audit and outbox row; chains, scope_sagas/scope_saga_entries; memberships, bindings/pools, drains and complete original worker_sessions.
Add tenant-scoped deterministic arrays for capacity_groups (group_id), capacity_group_members (group_id/member_id) and capacity_group_finish_receipts (group_id/operation_key), proving all remain absent in this ordinary fixture and unchanged at every later checkpoint.
Explicit payload-ledger coverage is `payloads` with content/digest plus tenant_retained_usage and run_instance_usage; add owner-only quota_transaction_deltas ordered by transaction_id/tenant_id, requiring no residue. Do not grant runtime access to this private table.
Require exactly ZERO receipt for original tenant/canonical scope/key, ZERO new RUN_DATA_LIMIT stop, ZERO member16 step.completed, ZERO refusal audit, output/blob/event-payload references or extra task/Return/End/saga, and ZERO outbox delta. Earlier genuine successes/audits/receipts remain byte-identical.
Run stays at its prestate status/version/head/trace; member16 stays ADMITTED and its reserve step LEASED with original token/fence/expiry; logical bytes remain 4000000, iteration capacity remains one held slot and source/input/payload ledgers stay exact. Verify journal max(seq), Run version and telemetry head remain mutually equal to their original values.
Require current original lease/session deadlines still live without modifying them. A post-abort own-Run NOWAIT read lock can now succeed and must be released immediately without writes; retain actual wire abort as the authoritative outcome witness.

## Full fresh recovery and two exact retries

After the rollback oracle, remove both test triggers/functions through the owner connection; assert their absence. Drop reporting/worker/faulted API and its stores/cache, close the resolved proxy and bounded-close its runtime and owner pools; keep only held server/database identity and immutable witnesses.
Attach the SAME owned DB without migrations/provisioning; create a fresh direct runtime pool and API with fresh stores/caches. Assert actual current_database and current_user=sagat_app. Full owner snapshot after reconstruction must still equal `before`.
Connect using a current JWT for the same subject and original worker/session; verify complete original session/lease row and live expiry, plus byte-identical protobuf. No register/repoll/expiry rewrite/new idempotency key.
Send original CompleteStep once. Its actual FIRST SUCCESSFUL ACK must have `already_applied=false`, step SUCCEEDED and run CANCELLING; do not synthesize an ACK for the aborted attempt.
Immediately inspect persistence afresh through the independent owner pool using the existing committed-refusal inspector. Require exactly one root FAILED/RUN_DATA_LIMIT stop preceding exactly one genuine member16 success, original reported token/attempt/generation, output NULL, OPEN scope/ADMITTED held member and no forward work/drain.
Require fresh journal max(seq)=run_version=telemetry head, unchanged original Run trace, one fixed engine refusal audit with that original trace, exactly one canonical original receipt and zero new outbox. Reuse all retained-source/authority/payload/quota/saga/capacity preservation assertions; logical bytes stay 4000000.
Independently compute scope and length-prefixed complete/attempt-u32/generation-u64/output SHA256 from the original command. Receipt tenant/scope/key/hash must match and resource must be `SUCCEEDED|CANCELLING|<fresh verified revision>`; retain its exact created_at.
Compare the retained actual ACK to `command_id=<original run>:<fresh independently verified revision>` and those fixed statuses. Never derive expected revision/status/resource from the ACK under test or merely compare two ACKs.
Only after that independent validation freeze expected ACK fields, complete receipt and post-success snapshot. Send exact original command again: `already_applied=true`, all other ACK fields equal the independently frozen values, complete receipt including timestamp identical, full snapshot bytes identical and zero extra effects.
Assert fresh current-auth lookups for each recovery command. With a NEW DefinitionCache/PgRunStore on the recovery API's actual runtime pool, run verify_run and load_path for lines[16]/reserve; owner snapshot after those reads stays byte-identical. Bounded-close all owned recovery pools.

## Root verification, evidence and stop conditions

Implementation sequence: prerequisite acceptance -> minimal shared helper/fixture change plus committed/P0b regressions -> new test and fixed deferred abort -> fresh read/retry oracles -> final-source root gates. Tests precede any production correction; production correction is outside this unit.
Root alone executes and records real commands/exits/counts, source/proxy/binary hashes, original failures, UTC/durations and independent read-only owner inspection. No acceptance inferred from this plan, subagent review or a zero-test filter.
Use only repository Compose PostgreSQL under a private task-owned project, pinned image, loopback port/healthcheck and owned DB; no manually started server or other project's service. Preserve run/DB identities and non-sensitive oracle evidence for independent inspection, including pre-retry rollback evidence that later successful retry would overwrite.
Before compile, root runs df -h and uses a private CARGO_TARGET_DIR outside the repo with at most two heavy builds. Compile separately, freeze executable inputs and select the exact native integration binary; do not spend runtime containment on compilation.
New test keeps the cooperative 90s scenario deadline; all report/relay/join/pool-close waits remain bounded. Gate-held reads must fit the existing 30s SQL bounds; locks, not sleeps, establish ordering.
Each exact native test process uses `gtimeout --signal=TERM --kill-after=5s 180s "$TEST_BINARY" --exact "$TEST_NAME" --nocapture --test-threads=1`, without --foreground/--preserve-status; default uses `env -u SAGAT_TEST_AS_RUNTIME_ROLE`, runtime-labelled uses `env SAGAT_TEST_AS_RUNTIME_ROLE=1`. Both assert actual sagat_app pools.
Root runs new aborted test, existing committed-loss exact test and P0b exact test in both profiles on final source. Require exactly one passed/zero ignored each; record exit 124/137, SQL timeout and nonzero as failures, never rerun without diagnosis.
Required strict local gates: `cargo fmt --check`; `cargo clippy -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings`; `python3 scripts/check-architecture.py`; applicable available agent-config check. Missing tooling is reported, not called passed; these focused gates do not imply full workspace/SDK/release acceptance.
Root independently verifies rollback evidence, canonical final receipt/head/trace, single stop/success/audit, source/capacity/payload ledger invariants, removed DDL and fresh-runtime reads. Capture byte digests/counts without tokens, output content, credentials or SQL frames.
STOP for changed writer PID/next-COMMIT selection, missing barrier, early B release, unexpected public error/wire resolution, non-identical rollback, expired original authority, fresh-retry mismatch or any residual effect. Preserve the genuine RED source/log and diagnose; do not relax assertions, rewrite state, invent an unknown-outcome enum or patch production within this test-only unit.
If new test passes current production, report added coverage with no fabricated production RED. If failure proves a production defect or requires a proxy/API change, return that bounded finding for a separate test-first correction before proceeding.
Fresh review must specifically check refactor equivalence, both fixed trigger branches, no after-EOF/early-release inference, snapshot completeness, ACK provenance and teardown/drop ordering. Re-run affected gates after any source correction; do not treat review None as a runtime result.

## Pending risks and explicit exclusions

Committed-loss acceptance arrived during design at ef12c32c/trace997b0335; the inspected parent/child/proxy SHA256 values above stayed unchanged at the final read. Rebase/freeze the implementation handoff against that accepted checkpoint; aborted-case runtime remains entirely pending.
Snapshot additions can enlarge gate-held work; keep tenant-only deterministic queries bounded and preserve all existing P0b/committed assertions. Private quota delta reads stay owner-only; fresh runtime verification must use sagat_app.
The narrow existing-enum parameterization must not turn committed ACK replay into aborted newly-applied behavior, nor make P0b call proxy.close while unarmed. Wire constraint text must stay exact despite its historical capacity name.
This proves one current ordinary inline public aborted-COMMIT retry. No OS death, post-ACK scheduler/drain/provider undo, Applied reconciliation, old producers, prior-stop matrix, nesting, production ceiling, rollout or canonical feature closure is claimed.
