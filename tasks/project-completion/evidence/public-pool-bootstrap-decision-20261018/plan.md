# Current public scoped refusal: actual COMMIT response loss

Read-only design, 2026-10-18. Inspected repository: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`, HEAD `d30f5a74320bdb3f1c250277ac9e2698585007bd`. No repository edit, test, build, formatter, container operation, SQL mutation, commit, external write or subagent was performed. The only written artifact is this file. Root's unrelated in-progress legacy cancellation work is outside this design.

## Decision and scope

The existing PostgreSQL wire relay is sufficient for the actual fault. Reuse it unchanged. A separately reviewed test-only API bootstrap seam is a prerequisite because the current public API harness always constructs its own eight-connection direct pool. Do not add a production failure hook or call the private refusal method instead of public Worker `CompleteStep`.

The bounded test unit covers current-produced, unchained depth-one root `forEach` refusal, with 16 actual retained 250,000-byte member outputs and a genuine seventeenth candidate. It covers ordinary success and a genuine `Applied` reconciliation separately. Each has two server resolutions: the full report transaction commits, or it aborts inside its deferred COMMIT trigger. The reporting API observes the same lost PostgreSQL reply in both cases. A new API/store/cache later resends the exact original public command and distinguishes receipt replay from first application by `already_applied`.

The first unit makes no old-producer, OS-process-death, PostgreSQL-failover, secret-provider, blob, prior-stop, external exactly-once, nested execution or full protocol claim. Those are independent evidence units. Reconstructing services in one Rust test process must be called service/store/cache reconstruction, never process restart.

Refs: Constitution I, III, V, VI, VII, VIII; FR-014-005/014/015/017; NFR-014-001/002; SC-014-003; the still-open canonical T014-017/040. Normative local boundary: `tasks/project-completion/collection-result-quota-correction.md` sections 11.2 and 11.4, as corrected by `scoped-output-refusal-discovery.md`. No canonical checkbox closes from this bounded unit.

## Inspected source evidence

| Source | Consequence |
| --- | --- |
| `crates/api-grpc/src/service.rs:876`, `:969`, `:1225`, `:1249` | The public operation authenticates, parses the complete attempt/instance and key, validates inline bytes, invokes ordinary `workers.complete`, then translates `StepAck`. Nonempty JSON bytes reach the application unchanged. |
| `crates/application/src/worker/output.rs:125` | Completion constructs the original report idempotency from the attempt and exact output bytes before selecting the scoped path. |
| `crates/application/src/worker/scoped_reports.rs:223`, `:254`, `:272`, `:360` | Checked path replay/lease checks precede report application. A secret scan can commit an earlier path transaction before provider I/O and reopen another. The oversize candidate leaves the path and enters the fresh full refusal transaction. Arming on “the next COMMIT after request start” is not a safe general selector. |
| `crates/application/src/worker/scoped_output_refusal.rs:24`, `:39`, `:49`, `:62`, `:90`, `:129`, `:139`, `:152`, `:166` | Fresh full own-run entry, current authority/receipt/fence, fresh scalar, root preparation, scoped completion, reported lease/audit/original receipt, and exactly one final `tx.commit`. Both persists share this final transaction. |
| `crates/infra-postgres/src/execution.rs:4937`, `:4972`, `:4995` | Scoped persist writes telemetry, step/run projection and audit before `remember` writes `command_dedupe`. Receipt insertion is a late, observable point in the exact final refusal transaction. |
| `crates/infra-postgres/src/execution.rs:4425`, `:4452`; `identity.rs:723`, `:748`, `:804` | Full-run commit calls SQLx commit and ordinary `store_error`. IO loss maps to `StoreError::Unavailable`. It has no special public `CommitOutcomeUnknown` result. |
| `crates/application/src/error.rs:220`; `crates/api-http/src/error.rs:329`; `crates/api-grpc/src/status.rs:15` | Expected IO-loss response is tonic `Unavailable`, machine metadata `UNAVAILABLE`, safe unavailable message and ordinary retry guidance (currently `retry-after-ms=5000`). This is a retryable transport/storage outcome, not proof of rollback. |
| `crates/application/src/worker.rs:2093`, `:2427`, `:2464`; `worker/scoped.rs:11`, `:153`; `infra-postgres/src/definitions.rs:776` | Replay is tenant + exact lease-qualified scoped identity + original key + original request hash. Current activity binding and worker membership precede receipt access. A matching receipt wins before a new mutation. |
| `tests/integration/workflow-primitives/capacity_finish_proxy.rs:45`, `:108`, `:125`, `:139`, `:246` | Existing one-shot relay uses one authenticated `sagat_app` connection, forwards real simple-query `COMMIT`, closes only the client reply direction, retains upstream, and observes actual server resolution. |
| Same proxy `:27`, `:259`, `:301` | Boundaries: TCP protocol 3, TLS disabled, pinned SQLx Q/COMMIT, one connection, max frame 1 MiB, at most eight withheld post-COMMIT frames; no TLS/COPY/pipelining/cancel support. It accepts `CommandComplete(COMMIT)` or exact synthetic 23514 constraint marker, then `ReadyForQuery(I)`. |
| `capacity_finish_commit_loss.rs:35`, `:50`, `:94`; `leaf_certification_commit_loss.rs:12`, `:96` | Existing deferred constraint/advisory gates establish actual unresolved COMMIT and cold direct recovery. These native port tests are reusable patterns, not existing proof for the public refusal. |
| `tests/support/api.rs:300`, `:307`, `:312`, `:329`, `:509` | `start_on_db` can reopen a caller-owned already-migrated DB. It still creates a new direct pool, then shares it across identity/REST/gRPC/run stores. There is no supplied-pool seam today. |
| `tests/support/mod.rs:214`; `tests/integration/workflow-primitives/api_reopen.rs:159` | `attach_existing_db` reopens only the harness-owned exact DB without migration or reprovisioning; current API recreation and original public worker authority have a local precedent. Keep the original test server alive with `hold_server`. |
| `byte_ceilings.rs:110`, `:133`, `:368`, `:593`, `:783`, `:1687`, `:1900`, `:2010` | Current real 17-member fixture, complete snapshot and successful-refusal oracles, ordinary/Applied public reports, between-persist and late-receipt rollback patterns. Existing replay tests observe normal replies; they are not actual COMMIT acknowledgement-loss proof. |
| `domain/src/execution/command.rs:600`, `:647`, `:1236`, `:1289` | First stop is `run.stop_requested(FAILED, RUN_DATA_LIMIT)`. Ordinary report records exact `step.completed`. Applied records lookup `step.completed` and forward `reconciliation.concluded(Applied, Succeeded)`; the latter changes the forward step to SUCCEEDED. Do not invent a second forward `step.completed` event as an oracle. |
| `scoped-output-refusal-discovery.md:23` | The report creates zero outgoing-message obligations and zero new `event_outbox` rows. Durable stopped/open scope discovery drives the later drain; the drain owns `collection.drain.started`. |

The complete root AGENTS/constitution and 014 spec/plan/tasks were read. Existing branch/worktree inventory was checked first. The report-path files above were read at the stated HEAD; unrelated root edits were only noticed in status, not used as design evidence.

## Prerequisite P0: test-only API pool bootstrap

Add a narrow constructor to `tests/support/api.rs`, for example `start_on_db_with_pool(db, options, pool)`. It must use the caller's exact PgPool for the same identity, execution, REST and gRPC stores that normal startup uses. Existing `start`, `start_with` and `start_on_db` continue to construct their normal pool. Share the existing construction body; do not create a second partial Worker service or fake authentication stack. This is test support only.

The caller owns proxy lifecycle. The supplied pool must be checked by the focused test for `current_database`, `current_user`, `pg_backend_pid`, one maximum connection and the relay application name. Configure bounded statement/idle timeouts explicitly on that one proxied session before its use; constructor bypass must not silently pretend `Options.statement_timeout` was applied. Keep production pool defaults unchanged. Do not log any connection URL or authentication bytes.

P0 bounded ownership, at most three logical files:

1. `tests/support/api.rs`: only constructor/pool plumbing and existing-constructor delegation.
2. `tests/integration/workflow-primitives/api_reopen.rs`: prove a supplied runtime pool is the actual public Worker backing pool, against the exact already-owned DB, with original current authority and no reprovisioning. A benign public command through an unarmed existing relay can validate the one-connection/250 KiB path if kept in this bounded test.
3. One P0 evidence/contract note, if a separate note is required.

The prerequisite gate must run before attributing failures to refusal recovery. A fresh single-connection API must serve authentication and CompleteStep without a second hidden connection, deadlock or protocol frame failure. `TestApi.api_pool` equality checks alone do not prove gRPC was wired to that pool; include a public command plus database backend observation. No new production pool option, grant, migration, hook or dependency is necessary.

## Exact public fixture and retained private command

Use current API publication/start, a genuine scheduler and worker protocol 1.5, with the same restricted definition as `successful_refusal_collection`. Set comfortable finite lease/session TTLs (e.g. 300 s) in test startup before registration, never by changing a live row. This is fixture configuration, not an expiry bypass. Use a `Platform` fixture with an explicit `Keeper`/`Placement` so clean scheduler departure and later fresh ownership can be exercised; do not use unfenced `Platform::tick` to claim the owned-shard discovery gate.

Create 17 members at concurrency one. Complete/checkpoint 0..15 through public gRPC and normal scheduler ticks. Use 250,000 bytes per retained output. Prefer a different harmless 250,000-byte final candidate so absence of its digest is a meaningful payload oracle; repeated equal retained hashes still represent 16 separately charged logical references. Assert from actual rows: L=E=4,000,000, 16 genuine checkpoints/retained outputs, collection OPEN, member 16 admitted and really leased, no existing stop, no parent/chain, and 4,000,000+250,000>4,194,304.

For Applied, call public FailStep with genuine `OUTCOME_UNKNOWN`, let normal scheduling release `reserve~r`, and poll that lookup. Send the actual Applied wrapper with the 250,000-byte substitute, verifying the whole wrapper remains below the 256 KiB report limit. Snapshot this real pre-report state: forward WAITING on reconciliation; lookup leased. Do not SQL-set it.

Retain in one private in-memory fixture value the complete `CompleteStepRequest`: task/run/step IDs, complete instance path/definition node, attempt, generation, original lease token, stable report idempotency key, exact `output_json` bytes, blob/digest fields and traceparent. Retain tenant/namespace/subject/version, worker session identity and the original lease separately for checking. Do not reconstruct the request from a digest, create a new key, register a replacement worker, repoll for a replacement lease, alter generation/expiry, change subject, or substitute fresh output serialization on retry. Keep raw tokens, session material and payload bytes out of logs and evidence. No disk command checkpoint is needed in this same-process unit.

Before discarding the setup API, call the real setup Keeper's `leave`, verify durable departure, and capture/close its pool after dropping API/worker/scheduler/cache references. Preserve only the owned TestDb identity and private original command, and keep the disposable server alive. This clean shutdown is not an OS death simulation. Reopen the same DB for the faulting API through the injected relay pool; no migration, provisioning, registration, grant or binding write is allowed in this reopen. New fixture JWT signing keys may issue a token for the same durable subject; current database authentication must still run.

If existing helpers cannot be reused without exposing private Flow internals or exceeding the file boundary, duplicate the small pure workflow/output builders in the dedicated test module and compose the public support APIs. Do not refactor the broad Flow harness merely to reduce a few setup lines.

## Two-stage gate selecting the exact report COMMIT

Use test-owned DDL in the disposable DB, installed via the owner inspection pool. It never writes execution history, receipts, stop/status or ledger rows. Both trigger predicates are bound to the exact tenant, original report scope and original key; include original request hash and expected `SUCCEEDED|CANCELLING|...` result validation. Scope/fingerprint derivation is a test oracle from the public original command and the documented current encoding, not a replacement receipt producer.

The report scope is lease-qualified: root Run + scoped path segments + reported definition node + first eight token-hash bytes, as `worker.rs:2427` encodes. The request hash is SHA-256 of the length-prefixed `complete`, big-endian attempt, big-endian generation and exact nonempty output JSON bytes. For Applied this includes the complete Applied wrapper, not just the substitute. Every actual receipt query includes tenant + scope + key. Never expose raw lease/session tokens through trigger text or logs.

Gate A is an ordinary BEFORE INSERT trigger on `command_dedupe` for that exact identity. It obtains advisory lock A then returns NEW, with no forced error. A test owner transaction holds advisory lock A before the report. Gate B is a separate AFTER INSERT CONSTRAINT TRIGGER, DEFERRABLE INITIALLY DEFERRED, for the same exact receipt. At COMMIT it obtains advisory lock B; its selected action is either `RETURN NEW` or the existing relay's exact `ERRCODE=23514`, constraint `capacity_finish_proxy_forced_abort`, with a fixed harmless test-only refusal message. A different owner transaction holds B before the report.

Sequence:

1. Start the fault API with the existing relay unarmed. Capture its actual backend PID from the one proxy pool before issuing the report. Use direct owner observers for everything thereafter; querying the proxy pool while the report owns its only connection would deadlock.
2. Invoke public `WorkerServiceClient.complete_step` once, with the complete retained original command and current authentication. No scheduler tick, heartbeat, registration, health poll or other request runs on the fault API during selection.
3. Observe that exact PID in `pg_stat_activity`: correct database, `sagat_app`, relay application name, active lock wait, `INSERT INTO command_dedupe`, blocked by Gate A's PID. Recheck the blocker while recording a pre-commit snapshot. This proves this is the real final refusal receipt insertion, after root and scoped persists, not an initial path/provider/worker-registration transaction.
4. Only now call `proxy.arm()`, then release Gate A. The only remaining COMMIT on this isolated connection is the target report COMMIT. No query-text counting, sleeps, first-COMMIT guess or proxy frame-body matching is needed.
5. Await `proxy.wait_sent()`. Independently observe the SAME backend PID with `query='COMMIT'`, active Lock wait and Gate B as its blocker. A Q/COMMIT packet without this server-side witness is insufficient.
6. While B remains held, the observer must see the entire pre-report execution snapshot and no original-key receipt. `SELECT ... FROM runs ... FOR UPDATE NOWAIT` on the exact own Run must fail 55P03, establishing that the unresolved report still holds its business lock. Do not claim the scheduler SHARE guard from unrelated native Finish tests: this worker report is authorized by its worker fence, not a scheduler mutation fence.
7. Call `proxy.lose_reply()` while the observed actual COMMIT is still blocked. Await the public gRPC failure. Expect tonic Unavailable / machine UNAVAILABLE, safe unavailable text, no StepCommandResponse. Reobserve the same PID at COMMIT behind B and the unchanged visible snapshot AFTER the caller has failed. This is the unknown-outcome boundary: absent visible receipt cannot justify a second effect or establish rollback.
8. Release only B. Await `proxy.resolution()` and require the configured actual `CommandComplete(COMMIT)` or labelled ErrorResponse(23514), followed by ReadyForQuery idle. Both replies were withheld from SQLx; both original public calls must have the same unavailable outcome. Close/join the proxy after resolution, remove the two exact test triggers/functions, and close/drop the fault API/pool/cache.

All awaits and pg_stat_activity observation loops have finite deadlines. Gate release is driven by observed state, not a sleep. Server statement timeout must exceed the short observation windows but remain finite. If exact PID, receipt identity, dispatched COMMIT, real deferred wait or exact resolution cannot be observed, fail the fixture; do not reclassify a timeout/earlier transaction error as successful injection. The existing relay hardcodes the abort marker; reuse it exactly instead of broadening its accepted error classifier. The relay remains a one-shot test tool.

## Durable-state and fresh recovery oracles

Read the full affected state through a separate transaction/single statement as the existing refusal snapshot does: Run, full steps including lease/reported-lease fields, collection/admission/checkpoint/source rows, whole event rows/payload references, run ledger, telemetry head, tenant payload/reference accounting, full tenant receipt/audit/outbox rows, blobs/reservations, scope sagas/entries. Include immutable pinned version and authority witnesses; do not print private values on assertion failure.

After original server resolution and before any recovery write:

| Original outcome | Required durable state |
| --- | --- |
| Committed | One root first stop FAILED/RUN_DATA_LIMIT; one exact successful report's effects; original full receipt; matching reported token/attempt/generation; exactly one truthful fixed refusal audit with the Run trace; final run_version = last event seq = telemetry head. Collection remains OPEN/CANCELLING, no Return/End release, no new scope drain/saga. No candidate output/blob reference, no logical charge or candidate payload, and ZERO new report outbox rows. Applied has lookup SUCCEEDED plus forward SUCCEEDED and the real Applied verdict; no fabricated forward failure/completion event. |
| Aborted23514 | Full affected execution/receipt/audit/head/payload/quota/outbox state is byte-for-byte the pre-report state. Original lease and actual Applied waiting state remain; no root stop, successful report event, reported lease, receipt or audit survives. Real TTL passage is observed as time, never normalized by rewriting stored expiry. |

For the committed case, retain the entire actual receipt `(tenant_id, scope, idempotency_key, request_hash, resource_id, created_at)` and exact newly created audit/event identifiers, not only counts or an ACK prefix. Validate request_hash against the complete original command. Its stored `resource_id` supplies `SUCCEEDED|CANCELLING|originalVersion`, and the response command ID must be `originalRunId:originalVersion`. Created-at and original audit/event rows must remain identical on all replay attempts. For abort, assert the exact full key has no row before recovery.

Create a new direct runtime API/store/DefinitionCache on the same owned DB. Start a new public worker client using `connect`, not `worker`/`worker_at` (those helpers provision authority). Send the exact retained original command under current authentication. Do not call RegisterWorker or PollTasks before this recovery report. Verify the tenant, namespace, membership, bindings and original session's stored identity are unchanged; authentication lookup counters should show the fresh API resolved identity.

Expected first recovery response:

- Original committed: `already_applied=true`, exact stored command ID, step SUCCEEDED, run CANCELLING. The complete committed snapshot and receipt are unchanged, including audit/event/outbox/head identities.
- Original aborted: `already_applied=false`, step SUCCEEDED, run CANCELLING. The original key/lease first applies one truthful refusal. Its new actual full receipt becomes the recovery reference; nothing is copied or fabricated from the discarded transaction.

Then repeat the same public command once in both cases: `already_applied=true`, identical ACK fields, exact whole receipt and committed snapshot unchanged. Check the current exact lease before the aborted retry and configure sufficient TTL initially; a slow test that loses its required premise is a failure to investigate, not permission to mutate it. No extra caller report-result cache can satisfy these checks because all fault API/store/cache instances have been dropped.

Perform fresh native full journal verification and fresh exact-path load with a new direct PgRunStore/DefinitionCache. Compare the original admitted identity, stopped Run, successful metadata and Applied verdict. These are read/rebuild operations; they do not invent missing history or invoke full RunData to reconstruct refused payload bytes. Ordinary fitting-budget paths are not changed.

Finally construct a fresh Keeper/Placement/store/cache, acquire real scheduler ownership after the earlier clean departure, and call normal `Scheduler::tick` without a report hint, broker delivery or direct drain call. The first actual drain must be RUN_DATA_LIMIT, one `collection.drain.started` outbox row, and a distinct truthful drain audit; it may not alter the retained report receipt. Reuse the original worker session transport identity for public polling and complete the 17 real compensations. Assert original forward targets/idempotency keys, each exactly once, and final COMPENSATED with no admitted/live obligation. This demonstrates actual current durable discovery/compensation, not external provider rollback. If this final composition would exceed the bounded file/review scope, keep it as a separately declared follow-on test; do not label report-only recovery as cold scheduler discovery acceptance.

Do not add metrics assertions that treat the original hidden commit as an acknowledged success: `worker_scoped_output_refused_total` increments only after acknowledged `tx.commit`. A committed-but-lost original can have zero live counter increment and still have one durable refusal. Durable receipt/journal/audit are authority. If telemetry is inspected, use the persisted final head, not a process metric as a commit oracle.

## Bounded implementation and verification units

P0 above is independently reviewed and gated first. Main unit P1 owns at most five logical files:

1. `tests/integration/workflow-primitives/public_refusal_commit_loss.rs` (new): current public fixture, private retained request, exact two-stage receipt probes, snapshots, two named tests and all four outcome cases. If helper size needs another file, it consumes a slot here.
2. `tests/integration/workflow-primitives/main.rs`: register the module. Import the existing `crate::capacity_finish::commit_proxy` unchanged.
3. One bounded contract/evidence note under `tasks/project-completion/`; do not hide test helpers in an evidence directory.
4–5. Reserved only for a clearly identified helper or necessary evidence/task linkage. No production implementation belongs to this test-only unit. Extensive raw command outputs can be sealed in a separate artifact-only increment.

Suggested test names:

- `ordinary_public_refusal_recovers_actual_committed_and_aborted_commit_reply_loss`
- `applied_public_refusal_recovers_actual_committed_and_aborted_commit_reply_loss`

Each name executes two independent disposable-database cases (committed / deferred abort). Report two tests and four cases, not four Rust test names. The tested API and proxy are explicitly `sagat_app` in both suite invocations; default plus `SAGAT_TEST_AS_RUNTIME_ROLE=1` is not an owner/runtime role matrix for the faulted API.

Root's intended gates, not commands executed here:

1. Before builds, `df -h`; set a private CARGO_TARGET_DIR outside the repository; coordinate at most two heavy builds. No cargo clean/prune.
2. P0 focused constructor/public-operation test and existing `api_reopen::current_api_reopens_owned_database_and_resumes_original_public_worker` in normal and explicit runtime invocations. A passing bootstrap does not establish refusal recovery.
3. `RUST_TEST_THREADS=2 cargo test -p sagat-tests --test integration_workflow_primitives --locked public_refusal_commit_loss:: -- --nocapture`.
4. Same selection under `SAGAT_TEST_AS_RUNTIME_ROLE=1`, with complete logs and case-resolution/PID witnesses. Redact private command/connection data.
5. Relevant existing `byte_ceilings` ordinary and Applied refusal, between-persist, late-receipt and drain-outbox cases; do not claim a combined full suite when only named selections ran. Existing uncertain-COMMIT relay tests are regressions if its shared code changes; the selected design leaves it unchanged.
6. Strict integration-target Clippy, fmt check, architecture and agent-config checks according to root's merged-tree gate. Broader required gates remain root's responsibility. Preserve any failure with its source/command; do not retry unexplained flakes into a pass.

Tests should be introduced before any behavior correction. If the new coverage passes current production source, report characterization/coverage evidence, not a reproduced production bug or invented RED/GREEN. If it reveals a failure, preserve genuine RED, isolate the cause, and review a separate <=5-file production correction before changing runtime semantics. Root reruns the final merged-tree selection and records exact executable inputs, test names/cases, commands, results, gates and limitations.

## Explicit dependency gaps and stop conditions

- **Real present prerequisite:** the API harness cannot currently use `CommitAckLossProxy.pool`; P0 is necessary. Connecting a different store directly is not public Worker coverage.
- **One-connection viability:** prove public authentication plus path rollback/full reopen is compatible with the relay's single backend. A hidden extra pool/connection, unsupported protocol frame or fixture timeout blocks the injection design until bounded test infrastructure is corrected.
- **Target fidelity:** both A's exact receipt/PID witness and B's same-PID actual COMMIT witness are mandatory. A receipt trigger that only matches a generic SUCCEEDED prefix or a relay armed before bootstrap/provider commits is insufficient.
- **No public unknown enum:** current code yields Unavailable for lost IO. Do not require private native Finish error variants or add a new public contract just for the test. If observed public classification differs, report the actual driver/error path and resolve it before claiming the intended fault.
- **No fallback imitation:** if the native final COMMIT cannot be selected/replied to safely, deliver an explicit test prerequisite gap. Do not ignore a normal ACK, cancel a future before dispatch, terminate an unrelated backend, create receipt/history rows, change live lease/generation or convert an arbitrary 23514 into proof of unknown commit.
- **Current authority boundary:** recovery reauthenticates and uses existing role/binding/lease/receipt rules. The initial positive current-authority proof does not replace separate revoked/wrong-namespace/stale-fence/secret-race matrices; do not broaden this test to all of them.
- **Deferred abort is real but synthetic:** it proves the actual PostgreSQL transaction aborts at COMMIT while its response is lost. It is not a machine crash, provider outage or PostgreSQL failover.
- **In-memory original command:** deliberately sufficient for service reconstruction only. Durable private command checkpointing plus OS death requires a later separately reviewed unit. Immutable-old producer recovery likewise stays separate.
- **Discovery composition:** no report outbox is expected. A later real drain outbox and current scheduler ownership are separate obligations. Removing or pretending to deliver hints is unnecessary; simply never run an outbox consumer during this fixture.

This plan establishes source-grounded feasibility and bounded prerequisites only. It contains no implementation, execution, acceptance or release result.
