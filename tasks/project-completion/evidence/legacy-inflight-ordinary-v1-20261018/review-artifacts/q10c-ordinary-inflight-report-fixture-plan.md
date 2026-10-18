# Q10c first bounded fixture: ordinary success reported before drain

Repository inspected: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
Read-only inspection at HEAD `21b2b79391a2e5ca0fdfe91e5fc1a42f96a371dc`; no repository edits, builds, formatting, tests or commits were run by this design agent. Root is the sole executor. Constitution, feature 014 spec/plan/tasks, and collection-result-quota-correction.md §§11.2/11.4/11.5 govern the proposal.

## Exact first claim

One actual immutable-old-binary producer leaves a genuine ordinary compensable forward task leased after ten closed legacy collection aggregates make E > 4 MiB. That process exits. A separately built current test process migrates and reopens the same database, authenticates through current HTTP/gRPC machinery as the same persisted worker principal, and submits the original valid lease and retained successful completion request before any scheduler advancement. Current code must preserve SUCCEEDED plus RUN_DATA_LIMIT without retaining the refused output, then select and complete the actual engine compensation task through the worker protocol.

This first unit proves public worker/report and engine compensation semantics, to the same business-effect boundary as `byte_ceilings::a_known_successful_seventeenth_member_is_preserved_without_its_refused_output`. It does **not** prove a durable external business effect, external compensation, or exactly-once behavior. No external business record survives this process boundary unless a separate genuine durable provider is introduced and tested. Do not add a provider service for this unit or label successful synthetic activity results as provider evidence.

E + delta with E <= limit, drain-before-report, independent cancellation-first, Applied reconciliation, post-report OS-process death/restart, output-dependent compensation, full authority/race/fault matrices, and external-provider behavior are separate future units. This one passing case must not close the whole Q10c row or canonical 014 release tasks.

## Actual reusable seams and gaps

| Existing location | Useful behavior / restriction |
|---|---|
| `tests/support/api.rs:300`, `start_with(Options)` | Complete real REST/gRPC wiring, fresh JWKS-backed current JWT authentication, `PgRunStore::with_scoped_worker_protocol`, owner/runtime pool selection. It unconditionally calls `migrated_db()`: there is no retained-DB constructor. |
| `tests/support/mod.rs:167`, `TestDb` | Contains owner pool, URL/admin URL/name and private `_server: Arc<Server>`. `fresh_db` creates a new DB; `migrated_db` creates then migrates. A consumer cannot construct TestDb from its sibling module today. |
| `tests/support/worker.rs:37`, `Platform` | Public `api`, `tenant`, `other_tenant` fields permit reconstruction from persisted original IDs, without provisioning. Frozen commit has the same seam. |
| `tests/support/worker.rs:279/291`, `Platform::connect/connect_at` | Connects transport only. No membership, binding, or registration mutation. Correct seam for the current phase. |
| `tests/support/worker.rs:309`, `Worker` | `worker_id` and `session` are public, so restore the original real registration result after `connect`. `register_version(..., "1.5")` exists on old and current fixtures. |
| `tests/support/worker.rs:319`, `Worker::req` | Signs a fresh current JWT for the original subject and tenant through the restarted TestIdp; normal auth resolves persisted tenant/membership. Never reuse an expired old JWT. |
| `tests/support/worker.rs:551`, `Worker::complete_message` | Copies the real task ID/run/step/attempt/generation/token/instance into the ordinary wire request. Persist that exact request with its chosen idempotency key. |
| `tests/support/scope_flow.rs`, `Flow` | Its `worker`, `worker_id`, `session_token` are private; `Flow::start` always forces `runtime_role: true`. Do not change it to expose credentials or use it to claim two old-producer access modes. Reuse public document helpers and scheduler patterns only. |
| `tests/integration/workflow-primitives/legacy_collection_output_view.rs:205`, `phase` | Genuine database identity check, pre/post migration witness, owner/runtime store reopen, exact legacy L/V assertions. This is a store-only consumer, not an API restart. Keep its code/evidence intact. |
| Same file, `legacy_scheduler_case` | Observes old scheduler owner and uses `Placement::new(old_owner)` plus `Keeper::round` to renew normal durable authority after the old process is reaped. No SQL lease expiry or owner replacement needed. |
| `tests/integration/workflow-primitives/byte_ceilings.rs`, `assert_successful_refusal`, `refusal_identity`, `drain_and_compensate_every_reservation` | Source for exact report, telemetry, audit, receipt, no-outbox and compensation assertions. Private functions take Flow; copy only narrow assertions into the new fixture rather than make Flow or this large module a new generic framework. |
| `crates/scheduler/src/passes.rs:647`, `Scheduler::advance_collection`; `Scheduler::tick` | Real normal entrypoints. Do not call `advance_collection_from_run` or native drain directly as acceptance substitutes. Prefer first `tick` to prove durable discovery and then inspect the matching drain. |
| `crates/infra-postgres/src/execution.rs:365`, `PgRunStore::verify_run` | Reconstructs metadata from the journal and compares the projection through a fresh full transaction. Call using a fresh store/cache/pool after report and after compensation; this is cold journal replay, not an OS restart. |
| `tests/integration/workflow-primitives/scoped_leaf_finish_process.rs:162`, `launch` | Future OS-reexec recipe uses `current_exe`, exact ignored test, child-only env credentials, null stdio, kill-on-drop. It only opens a store; it supplies no retained-DB API bootstrap. Do not add this third phase yet. |

`CompleteStepRequest` has no worker ID/session token fields (canonical `worker.proto:143` vicinity). It carries current bearer auth and the actual lease fence. The retained session token is still necessary to resume authenticated polling and prove real compensation leasing. Never invent a report-session field or new authorization API. Production `scoped_reports`, `scoped_output_refusal`, `open_worker_path`, and current membership/binding checks remain untouched.

## Minimum test harness dependency (proposed, not existing functions)

1. Add `tests/support/mod.rs::attach_existing_db(admin_url: &str, database_name: &str) -> TestDb` (name may follow local style). It accepts only an explicitly owned existing `t_[0-9a-f]{32}` database, constructs its URL from the known owned server/admin URL, connects an owner inspection pool, and asserts `current_database()` equals the requested name. It performs no CREATE DATABASE, migration, role creation, grants, provisioning, status/output/ledger mutation, or fallback server startup. It holds an external `Server { _container: None, ... }`; caller owns server lifetime. In the current-current smoke test a `hold_server()` guard spans both API lifetimes. In the old/current runner the already owned external PostgreSQL lifetime spans both processes. Do not expose any of this outside `tests/support`.
2. Factor `tests/support/api.rs::start_with` to allocate/migrate as today, then delegate to proposed `start_on_db(db: TestDb, options: Options) -> TestApi`. The latter reuses the exact existing API wiring and starts a fresh current TestIdp. Consumer explicitly performs current `migrate` between old witness checks before passing the handle. A low-level test helper must not hide migrations or recreate membership.
3. Test the dependency with a **current-current** restart: original test API provisions the normal identities and bindings and leases a public task; retain original IDs/session/request, stop the old API, attach the same DB, start new API, reconnect with `Platform::connect`, restore worker ID/session and report using current JWT. Assert same durable database and identities, one existing session (no new registration), current auth lookups, and successful original-lease completion. A second genuine task/poll can demonstrate session reuse if needed. No old-version compatibility claim belongs to this bootstrap smoke.
4. Behavioral RED should demonstrate failure to use retained state with the old always-new-DB construction, then GREEN with the injected DB constructor. An unresolved-symbol/compile failure is not the requested behavioral RED. Root can choose the smallest isolated smoke that observes the missing retained principal/run through the real API.

The fixture has no declared secrets, managed databases, signing artifacts, or blobs. Therefore a new TestIdp key and a new unused test vault key are enough; no old OIDC private key or vault material needs serialization. Keep issuer/audience/tenant/subject the same, issue a fresh current token, and rely on existing persisted authorization. Future secret-bearing fixtures would need explicit key continuity and a separate test.

## Distinct immutable-old producer v1

New file: `tests/fixtures/legacy-collection-inflight-report-v1/producer.rs`.
Proposed exact test: `legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_success_before_drain_producer`.
Frozen production base remains `6c44a0e49670cf19660792cd1518d241e6c99b3b`. Overlay only this new test module plus one registration in the archived integration main. No old production/support code changes, no migration changes. Do not modify existing `tests/fixtures/legacy-collection-output/producer.rs`, its runner, or its previous evidence.

Use frozen `worker::platform_with(Options { worker: WorkerSettings { lease_ttl: 3600 s, session_ttl: 3600 s, ..default }, runtime_role: requested mode, ..default })`; this already exists at the immutable commit. Both binaries are built **before** producer execution, so compilation never consumes the lease between old and current phases. Check real deadlines before reporting and fail clearly if inadequate; never extend them with SQL or add sleeps to establish ordering.

Set up the original worker once in the old fixture using the existing normal test identity bootstrap, an administrator's HTTP namespace binding to the explicit activity names, and real gRPC RegisterWorker 1.5. `Platform::grant` is an existing old fixture provisioning helper; grant no new runtime/database privilege. Do not use `Platform::worker()`/`worker_at()` in the current phase: those insert membership and wildcard binding. Persist the original tenant IDs (including Platform's real other_tenant if reconstructing that wrapper), namespace ID/slug, worker subject, tenant alias, worker ID and session token.

Workflow is the original accepted root collection-chain shape with **two bodies**:
- c0..c9: ordinary `work -> returned`, return `steps.work.output`, one item/concurrency 1, outputs exactly 210,000 JSON bytes as the established Q5 producer does.
- c10: `reserve (line.reserve, compensateWith=line.release) -> returned`, return a constant `1`, one item/concurrency 1, then end. c10 is the only compensable body. Its compensation relies on the protocol's target identity/idempotency key or unchanged input, never the refused output.

Drive through public scheduler and gRPC exactly as Q5 until ten prior owners are SUCCEEDED and c10/0/reserve is actually LEASED. Read-only assertions establish L=2,100,000, V=2,100,020, E=4,200,020; ten successful old scopes have absent owner output refs and real finish/route journal rows. E is above 4,194,304 while L plus the small pending report would fit. Save the genuine final TaskLease and exact small successful CompleteStepRequest. Do **not** report this last task in the old process; do not manufacture a SQL success, cancellation, receipt, worker session, or scope member.

The original Q5 `finish_then_hold` discards the selected lease and receipt omits all worker credentials. It cannot be used unchanged as the new producer. The new producer can reuse the shape arithmetic and public operation pattern without changing the old artifact.

## Private phase receipt and provenance

Use a versioned private receipt (format `q10c-ordinary-before-drain-v1`) distinct from Q5's. It carries source commit, actual database name, original tenant/namespace/worker identities and session, full original TaskLease, exact original CompleteStepRequest, selected owner, scheduler owner and read-only source witness. `prost::Message::encode_to_vec` plus base64 is already available in tests/Cargo.toml for faithful protobuf serialization; it avoids hand-reconstructing the secret token or instance identity. Decode with the current compatible generated protobuf types. The report key/output bytes are fixed at producer time; do not regenerate the key on current replay.

The receipt is a credential capability, not a public evidence artifact: root directory 0700/umask 077, atomic create-new temporary file with 0600, sync/rename, no Debug dump or contents in logs, argv or checked-in artifacts. The coordinator passes only its path. Prefer receipt database name plus existing child environment admin URL; derive runtime URL using `TestDb::url_for`. Keep the original lease/session private. Publish hashes and nonsecret observations only; remove private handoff artifacts after phase teardown, including failure cleanup. Do not print command arguments containing secrets.

Preserve source/archive/migration/binary hashes, exact registered test counts and the old-process successful exit/reap witness using the already reviewed coordinator mechanics. Use new artifact namespace `sagat-q10c-ordinary-before-drain-v1` and new receipt environment name; do not overwrite Q5 evidence. This is reuse of the accepted two-phase sequencing, not a fourth general coordinator review or protocol redesign.

## Current first acceptance sequence

New current file: `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs`.
Proposed exact ignored test: `legacy_collection_inflight_report_v1::old_ordinary_success_before_drain_preserves_the_report_and_compensation`.

1. Coordinator confirms the old producer test passed once and its process exited/reaped, then starts current binary on the same owned DB. No scheduler/server process from old remains.
2. Read/validate private receipt and source commit; connect existing DB, assert actual database identity and read-only old witness. Run current migrations as owner, compare the same immutable projection witness. Start current TestApi on this already migrated DB with requested owner/runtime mode. Inspect `current_user` of api_pool to record the actual mode.
3. Reconstruct Platform from original persisted IDs; `connect(original_subject, original_tenant)`; restore original `worker_id`/`session`. Fresh JWT goes through current JWKS/auth lookup. Do not call tenant(), provision(), grant(), bind_everything(), worker(), register(), or worker_at(). No membership/binding recreation on the current side.
4. Verify real target still belongs to original namespace/instance/lease/session and deadline is live. No heartbeat/scheduler call is required before the first report. Check E/L/V through read-only metadata or existing scalar interfaces without full `RunTx::data()` hydration.
5. Submit decoded original CompleteStepRequest through current WorkerServiceClient::complete_step and Worker::req. Expect `already_applied=false`, `step_status=SUCCEEDED`, `run_status=CANCELLING`. This is the only first-order acceptance case.
6. **Before any scheduler call:** assert exact root stop FAILED/RUN_DATA_LIMIT, c10 OPEN, actual forward step SUCCEEDED, same attempt/generation and `reported_lease_token_hash=hash(original lease token)`, absent output digest/blob commit, unchanged ledger and relevant payload references/counts, no returned/end successor release, no report-created outbox row, one original dedupe receipt, one report-refusal typed audit with existing fixed reason and worker event provenance, telemetry head equals final run version/journal head. Snapshot count/hash comparisons must not rely on SQL writes.
7. Fresh store/cache/pool `verify_run` proves cold journal/projection replay. Replay the **exact original request** under fresh current authentication and assert original command_id/ACK plus `already_applied=true`, unchanged report event/audit/receipt/output/outbox counts. This is exact receipt replay; it is not a lost-response or OS-crash test.
8. Reuse actual old scheduler owner through new Placement/Keeper round (normal durable renewal). Invoke normal `Scheduler::tick` first, or normal `advance_collection` if root explicitly isolates that entrypoint; never the full-fallback-only/native-drain shortcut. Expect the matching RUN_DATA_LIMIT drain, no new stop, no forward work, and one actual drain outbox/audit distinct from the report audit. Tick provides the stronger no-hint durable-discovery proof.
9. Continue bounded real scheduler passes and restored-session gRPC polls. Observe exactly the real c10/0/reserve~c COMPENSATION task for line.release. Assert target_instance matches c10/0/reserve and target_idempotency_key equals the original forward task's effect key. Assert finalization has not occurred before compensation completion. Submit ordinary public successful compensation report, then drain scheduler work to the expected COMPENSATED outcome. Do not call helper completions as external provider proof.
10. Verify exact compensation event/plan count, retained original forward success, unchanged previous legacy source/aggregate witness, truthful final scope/run/saga outcome, no forbidden output retention, and cold journal consistency. If actual code cannot reach this outcome, preserve the failure as a real gap; do not change source history, counters or status to make it pass.

Run independent owner and explicit SAGAT_TEST_AS_RUNTIME_ROLE=1 passes on fresh old-produced DBs; record real role of both producer API and consumer API. Exactly one producer test and one consumer test per pass, not a fabricated multi-case aggregate. Any first regression must retain its actual output. Existing Q5 regression is separately rerun by root at the final merged source, not relabeled as this new report fixture.

## Exact review increments (every path is a file, not a directory counted as one)

A — retained database test harness, **4 source files**:
1. `tests/support/mod.rs` (attach existing owned database; no schema/identity writes).
2. `tests/support/api.rs` (start_on_db extraction, original start_with preserved).
3. `tests/integration/workflow-primitives/api_reopen.rs` (current-current real public bootstrap test).
4. `tests/integration/workflow-primitives/main.rs` (register api_reopen).
Optionally a fifth file is its evidence manifest, e.g. `tasks/project-completion/evidence/api-retained-db-bootstrap-20261018/manifest.json`; do not silently count a second log or notes file as the same file. Root runs behavioral RED/GREEN and appropriate affected tests before B.

B — first genuine immutable-old/current fixture, **5 files**:
1. `tests/fixtures/legacy-collection-inflight-report-v1/producer.rs`.
2. `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs`.
3. `tests/integration/workflow-primitives/main.rs` (register current consumer).
4. `scripts/test-legacy-collection-inflight-report-v1.sh` (distinct, retain reviewed coordinator mechanics).
5. `tests/fixtures/legacy-collection-inflight-report-v1/README.md` (exact one-case scope, private receipt and reproducible command, explicit gaps).
Archive main registration diff and runtime MANIFEST are generated artifacts of the runner, not extra modified repository source files. Any committed artifact files beyond this set require a separate <=5-file evidence unit.

C — final bounded evidence/traceability update, **5 files**:
1. `tasks/project-completion/evidence/legacy-inflight-report-v1-20261018/manifest.json`.
2. `specs/014-workflow-primitives/spec.md` (precise checkpoint, no whole-Q10c closure).
3. `specs/014-workflow-primitives/plan.md` (same limited checkpoint).
4. `specs/014-workflow-primitives/tasks.md` (evidence only; retain canonical open tasks).
5. `tests/fixtures/legacy-collection-inflight-report-v1/README.md` (actual command/results/hashes).
If root's traceability convention also requires validation-report.md/todo.md or checked-in raw logs, make a separate documentation/evidence-only increment listing those exact files, at most five. Do not hide a sixth file in C.

No production API, feature flag, profile, schema, runtime grant, membership metadata or coordinator protocol change belongs in these units.

## Honest remaining questions to settle by execution, not inferred success

- New old workflow with two bodies/only last body compensable is supported by inspected old public helper/shape seams, but this exact old producer has not been compiled or executed here.
- The API constructor gap is real; no zero-edit existing TestApi restart seam was found. All proposed helper names above are additions, not claims of current availability.
- Choosing 3600-second configured leases/session TTLs preserves real clock-based checks; constructor tests/phase timing must prove validity rather than patch durable expiry. Both builds precede the old phase.
- Cold verify_run is metadata replay. It is not the §11.4 required process death immediately after report ACK. That remains a separate third-process unit.
- Public compensation-task completion does not establish durable provider-side effect/undo. This explicit boundary is intentional for the first increment, per root's latest scope.
- E <= limit / E + delta > limit, both drain/report orders, Applied, cancellation precedence, missing-output limitation and authority/fault controls remain open. No broad protocol review or production activation is proposed.
