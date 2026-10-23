# Ordinary public scoped refusal: committed COMMIT reply loss

Status: design only; P0b actual acceptance is a hard prerequisite. Root remains sole source writer and executor.
Read-only baseline: `15c27e05d38c6c9a232c9f1d2340890333cb690d`; pending C2 P0b source is prospectively reusable, not accepted. Root later reports default/runtime P0b 1/1 each plus independent owner SQL and reopen/refusal regressions passing; final-source Clippy/freeze/acceptance remain pending, and this agent did not rerun those gates.
P0b source SHA256 at initial inspection, before root corrected Keeper departure to count active rows: `4acade938d9532fda4a104b5e1694e30f7905fe5944f1410347d428c39805c86`.
Existing proxy SHA256: `98838f211342a30320300786c474e46f0cd87005a9d5674e1787d07a5b662b23`.
No source edit, build, test, SQL execution, container operation or acceptance occurred during this design.

## Contract and bounded unit

Trace: Constitution I/V/VI/VIII; FR-014-005/014/015/017; NFR-014-001/002; SC-014-003/004; partial evidence for T014-017/040, never task closure.
Use the accepted exceptional full own-Run transaction: root-only first RUN_DATA_LIMIT stop, then genuine scoped success with no retained candidate output, both persists plus audit/receipt/telemetry in one COMMIT.
The amended report contract requires zero new outbox rows: this report releases no task and begins no drain. Durable collection discovery and actual later drain remain separate acceptance.
Exactly one new test covers **Committed** resolution. Deferred abort, Applied reconciliation, prior stops, secret/blob races, old producers, process death, post-ACK scheduler recovery and provider undo are separate units.

Prefer three implementation/contract files (maximum five):
1. `tests/integration/workflow-primitives/public_refusal_commit_loss.rs`: narrowly extract its existing setup and read-only post-state assertions into private reusable helpers; retain the existing P0b test and assertions.
2. New `tests/integration/workflow-primitives/public_refusal_committed_loss.rs`, included as a child module `committed_loss` of the first file, so parent-private helpers are accessible without widening shared support.
3. One concise evidence/contract note, with separate bounded evidence-artifact and canonical-linkage increments if needed; no hidden sixth implementation file.
`main.rs` needs no new registration; `capacity_finish_proxy.rs`, production code, support API, schema/grants, dependencies and goldens remain unchanged.
Use an owned test fixture containing Platform, proxy, immutable CompleteStepRequest, original session/worker identity, exact lease witness, baseline snapshot and held-server guard. Do not store a borrowing Worker inside it or derive Debug on capability/request holders.
Extract only existing setup/inspection blocks, not a generic fault framework. Rerun P0b after extraction; any defect requiring production changes needs a separately bounded genuine-RED correction.

## Genuine premise and first full reopen

Reuse P0b's public publication/start, explicit iteration quota 1, protocol 1.5 registration, compensable reserve/return body, 17 real members and controlled normal scheduler ticks.
Sixteen genuine 250000-byte JSON outputs produce L=E=4000000, V=0, sixteen successes/checkpoints; the seventeenth original live reserve lease reports distinct 250000-byte JSON, yielding 4250000>4194304 while the complete protobuf stays below 262144.
Assert root/no parent/no chain, OPEN scope, no stop, member16 ADMITTED with held slot, exact source rows, pinned version, retained bytes and no candidate digest. No business-state/expiry/fence/output/receipt SQL seeding.
Preserve the full original protobuf privately, including original key, task/run/step/instance, output, lease token, attempt/generation and traceparent; preserve original durable worker/session hashes and authority.
Setup Keeper leaves its shards/membership with an observed witness; drop setup clients/API/stores/cache/scheduler and bounded-close both pools. Keep only the fixture-owned server/database identity and immutable command/capability witnesses.
Attach that exact owned database without migrations/provisioning. Start the current API on the existing **unarmed**, max-one-connection proxy pool; explicit 30s statement and idle-transaction bounds; assert database, current_user=sagat_app, application name and backend PID.
Connect the same subject with a current JWT; restore original worker/session values without registering, repolling, renewing, rebinding or inventing a key. Assert full snapshot/lease equality and current unmodified lease/session expiry before sending.

## Select the exact full refusal COMMIT: two independent gates

Before dispatch, install two test-only triggers on command_dedupe, both filtered by the exact original tenant, canonical receipt scope and idempotency key. Generated identities must pass the existing safe-DDL checks; never log tokens, request bodies or SQL frame bodies.
Gate A: existing BEFORE INSERT trigger takes advisory transaction lock A. Owner connection A holds A in its own transaction.
Gate B: new AFTER INSERT CONSTRAINT TRIGGER, DEFERRABLE INITIALLY DEFERRED, takes distinct advisory transaction lock B and returns NEW. Separate owner connection B holds B in its own transaction. No forced exception in this unit.
Capture both owner PIDs; require different A/B keys and connections. Independent observers use the owner pool, never the one-connection runtime pool.

Ordered protocol:
1. Send the exact saved public CompleteStep once. Own the spawned report task with abort-on-drop cleanup; do not use a detached task on timeout.
2. While the proxy is still FORWARDING, observe its captured exact PID as active sagat_app in the correct database/application, executing INSERT INTO command_dedupe and blocked by A's PID. This identifies the actual original receipt writer after authentication and checked-path rollback/full-Run reentry.
3. Only after that witness call proxy.arm(); then release A. B stays held. Max-one runtime connection plus no competing API work excludes an authentication/read COMMIT taking the armed slot.
4. Await proxy.wait_sent(), then observe **the same captured writer PID** active at query='COMMIT', wait_event_type='Lock', blocked by B's exact PID. Recheck database/user/application and gate identity. A ROLLBACK or another transaction does not satisfy this witness.
5. With B held, take an independent full durable snapshot: it must equal the pre-report snapshot, with no visible receipt, stop, success, audit or telemetry advance. An owner SELECT of the exact Run FOR UPDATE NOWAIT must fail with SQLSTATE 55P03.
6. Call proxy.lose_reply() while B remains held; await the original gRPC result within five seconds. Require an error, not an ACK, with tonic Code::Unavailable, x-sagat-error=UNAVAILABLE, safe message `service temporarily unavailable`, and current retry-after-ms=5000.
7. Repeat the same-PID COMMIT/B-blocker observation, Run NOWAIT 55P03 and full pre-state equality after the client error. The public failure does not establish rollback; the actual server transaction is still unresolved and locked.
8. Release B. Require proxy.resolution()==Committed, which validates server CommandComplete COMMIT and ReadyForQuery idle after the lost client reply. No ambiguous timeout or client result substitutes for this witness.
Do not release B early to obtain a convenient error. Missing gates, changed PID, unexpected status, timeouts or aborted resolution fail the case and remain recorded.

## Independent committed state, before any retry

Inspect through the independent owner pool after actual Committed resolution, before opening recovery or issuing another command.
Require exactly one FAILED/RUN_DATA_LIMIT root stop preceding exactly one genuine scoped reserve success; run CANCELLING, exact step SUCCEEDED, output_digest NULL, original reported token hash/attempt/generation, and final run_version=telemetry head=journal head.
Scope stays OPEN/member16 ADMITTED/slot held; sixteen checkpoints and all earlier retained outputs/source metadata remain exact. No Return, End, forward release, scope saga or extra task; no candidate payload/blob/event payload/output reference or logical byte charge. L remains 4000000.
Require one engine `run.data_limit_exceeded` audit, fixed successful-output-refusal reason and existing Run trace. Compare prior payloads, retained ledger, iteration ledger/member reservations, pinned version, authority/binding pools/drains/sessions, source items/checkpoints, chain and saga-entry snapshots unchanged; outbox delta is exactly zero.
Derive the original lease-qualified scoped receipt identity and length-prefixed complete/attempt-u32/generation-u64/output SHA256 independently, as P0b does. Require one exact tenant/scope/key/hash/resource/created_at receipt with resource `SUCCEEDED|CANCELLING|<committed revision>`.
Freeze that complete receipt and the full canonical snapshot **now**. Derive expected ACK command_id `<run>:<committed revision>` and frozen statuses from these independently verified rows. No first successful ACK was delivered; do not claim one was observed.
Remove both test triggers/functions only after resolution; join/close the armed proxy through its existing close() path and bounded-close all faulted API/owner pool references after dropping its clients/API/cache.
Keep the held-server guard throughout. Do not query the broken one-shot pool or reuse the faulted API for replay.

## Second full API reopen and exact original public replay

Attach the same database again; create a fresh **direct** runtime pool and new current API/stores/auth caches. Verify actual sagat_app/database/pool identity and the frozen full snapshot before replay.
Connect the same subject using current authentication and the untouched saved worker/session capability. Do not register, poll, renew, regrant or alter any command field. Assert the in-memory protobuf encoding still equals the saved original encoding without printing it.
Retry the exact original CompleteStepRequest once. Require already_applied=true and command_id/step_status/run_status equal the independently frozen receipt-derived ACK; current authentication lookup count must advance.
Require the complete receipt including timestamp unchanged and canonical serialized full snapshot bytes identical before/after replay, including execution/journal/payload/quota/telemetry/audit/outbox/source/authority. Do not ignore timestamps or omit payload bytes.
A fresh direct PgRunStore/DefinitionCache verifies the journal and loads the exact seventeenth path without mutation; use the runtime pool for this verifier. Compare the snapshot again if verification can add a durable read-side effect.
No scheduler tick, broker consumption or drain after the report; stop at the replay proof and bounded cleanup. This proves API reconstruction in one process, not OS/process restart.

## Finite root-only verification protocol

Dependencies: actual P0b accepted by root first; then reviewed shared-fixture extraction, preserved P0b pass, then this committed-loss case. Existing C1/C2 history is retained; do not reopen the declined external-review request.
Suggested exact new test: `public_refusal_commit_loss::committed_loss::ordinary_public_scoped_refusal_commit_reply_loss_replays_after_api_reopen`.
Root compiles separately with a private CARGO_TARGET_DIR outside the repository after df -h, respecting the two-heavy-build limit. Freeze executable inputs and select the resulting exact native integration binary.
Each complete test process, including synchronous fixture/container startup, uses GNU timeout TERM at 180s and KILL after 5s, without --foreground/--preserve-status. A cooperative inner deadline alone is insufficient.
Default: `env -u SAGAT_TEST_AS_RUNTIME_ROLE gtimeout --signal=TERM --kill-after=5s 180s "$TEST_BINARY" --exact "$TEST_NAME" --nocapture --test-threads=1`.
Runtime-labelled: `env SAGAT_TEST_AS_RUNTIME_ROLE=1 gtimeout --signal=TERM --kill-after=5s 180s "$TEST_BINARY" --exact "$TEST_NAME" --nocapture --test-threads=1`.
Each invocation must report exactly one named test, one pass, zero failures/ignored, exit 0, and actual sagat_app execution. They are two labelled invocations of the same runtime-role test, **not** an owner-versus-runtime privilege matrix.
Run required strict integration Clippy/fmt/architecture and the extracted P0b regression; broaden only for a concrete shared-code risk. Root independently executes gates on the final merged tree and records commands/output/source hashes, initial failures and timeouts.
All waits/joins/pool closes are bounded; advisory locks establish ordering, observer polling only observes it. After any failure preserve evidence and investigate before correction; no blind retry, relaxed oracle or state repair.
Do not tick T014-017/040 or final release tasks. Cleanup only root-owned targets/temp artifacts, never shared caches or unrelated containers.

## Open contract risks to retain

- **P0b final acceptance is pending:** root-reported successful executions do not waive final-source gates/freeze/acceptance; this design cannot bootstrap that dependency. Preserve the original Keeper departure failure and its active-row correction as root reports them.
- **Error mapping is inferred from source, not executed:** SQLx I/O -> StoreError::Unavailable -> AppError::Unavailable -> gRPC UNAVAILABLE/retry-after. If actual EOF maps otherwise, preserve the failure; do not invent CommitOutcomeUnknown, add an enum, or broaden this test to accept arbitrary failure.
- **Timing and target selection:** the unchanged proxy intercepts the next Q/COMMIT. Gates A/B, one connection, exact PID continuity and no concurrent API work are mandatory evidence, not optional diagnostics. Keep gate-held work comfortably inside the existing 30s statement timeout.
- **Frozen ACK provenance:** there is no delivered first ACK to compare. The expected original acknowledgement comes from independently validated committed receipt/head before replay; deriving it from the replay would be circular.
- **Observability:** the refusal counter increments only after acknowledged tx.commit; lost reply may leave it unincremented despite durable commit, and replay must not invent a first-commit metric. This unit asserts durable audit/journal/telemetry and existing safe gRPC error, not a new counter/trace-metadata guarantee.
- **Scope:** current ordinary inline, no declared secrets, depth1 unchained genuine public source only. No abort, revocation matrix, durable process death, effect undo, post-ACK drain, quota ceiling benchmark or final acceptance is established.

Read references: constitution; feature014 spec/plan/tasks; collection-result-quota-correction section11 and scoped-output-refusal-discovery amendment; pending P0b source/decision/C2 review; existing proxy and leaf/scoped COMMIT fixtures; worker scoped_reports/scoped_output_refusal; PG execution commit/store_error; gRPC status mapping and current API/worker/DB support.
