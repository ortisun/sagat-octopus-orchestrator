# Verification — feature 017

This is a chronological record of implementation checkpoints. Statements about pending
work apply to their checkpoint; the final delivery gate section records the closing status.
See [traceability.md](traceability.md) for requirement-to-evidence mapping and explicit limits.

## Inline managed result checkpoint (2026-10-17)

- `cargo test -p sagat-infra-ir --test database_result -p sagat-infra-secrets --test database_credentials`: 2 schema/type tests and 2 credential-codec/scanner tests passed. Encoded-content checks failed before adding decoded Unicode/base64 and duplicate-key scanning.
- `cargo test -p sagat-tests --test integration_database_connections`: 2 combined real PostgreSQL/OpenBao fixtures passed, including missing-pin refusal, provider outage, clean completion, exact receipt replay during outage, old credential detection after rotation, binary encoding, query rejection, write review, cancellation while provider I/O is held and full-storage write review/replay. Rejected credential bytes are absent from persisted payloads; projections match journal replay.
- The initial write assertion incorrectly expected a `NEEDS_REVIEW` step status; the existing contract is step `WAITING` on the operator, run `MANUAL_REVIEW`, journal `step.needs_review`. The corrected assertion verifies those states. The storage test then exposed that generic quota stop settlement cancels an operator wait; the managed-write fallback now records `OUTCOME_UNKNOWN` without that stop request, and the real test passed.
- Final `cargo clippy -p sagat-application -p sagat-infra-ir -p sagat-infra-secrets -p sagat-infra-postgres --lib -- -D warnings`, formatting and architecture (23 crates) passed. The four focused validator/scanner tests passed again. Broader engine/gRPC/security regressions are running at this checkpoint. Independent read-only review found no inline scanner/fencing issue; its initial quota review missed the settlement problem caught by the real test.

This checkpoint does not enable production execution. Blob scanning/commit, authorized result disclosure, isolated runner, SDK/Studio execution and deployment gates remain pending.

Follow-up inline regression: `cargo test -p sagat-tests --test integration_durable_execution --test contract_grpc_durable_execution --test integration_security_multitenancy` passed all 178 durable, 63 gRPC and 95 security tests (336 total, none skipped). The completion methods were then moved into `worker/output.rs` with method bodies unchanged apart from trailing whitespace; `cargo check -p sagat-application` passed.

## Managed blob result checkpoint (2026-10-17)

- RED: the new real gRPC upload of a valid 350 KB result failed at the prior managed-blob gate.
- GREEN: `cargo test -p sagat-tests --test integration_database_connections` passed both combined PostgreSQL/OpenBao fixtures with bounded blob upload/commit, pin-required refusal, split credential rejection before staging, query failure/write review, metadata refusal, full-size bound, provider outage/recovery, receipt replay without provider I/O, foreign-machine receipt refusal, audit rollback and provider-scan timeout before reservation. Fault-injected objects bypassing public upload are still checked at completion; rejected contents never enter payloads.
- `cargo test -p sagat-api-grpc --lib`: 16 passed, including exact collection bound, overflow, corrupt EOF and stalled producer. The timeout assertion initially expected gRPC DEADLINE_EXCEEDED; the existing shared HTTP 408 mapping is UNAVAILABLE, so the assertion was corrected to preserve the protocol.
- Review found and fixed arbitrary media-type metadata, loss of the deadline after collection and terminal blob replay requiring a cleared live session. Replay now checks the immutable managed lease's subject/attempt/generation plus current role and binding. A direct application-call replay test also passes with no provider/codecs configured.
- `cargo clippy -p sagat-application -p sagat-api-grpc -p sagat-infra-postgres --all-targets -- -D warnings` passed after collapsing a nested condition. Existing `contract_grpc_durable_execution` (63) and `integration_security_multitenancy` (95) passed, none skipped. Formatting, diff and architecture (23 crates) checks passed. Independent review closed with no remaining Required/Critical finding after the direct application replay fix.

Managed result disclosure, runner/production wiring and remaining feature tasks are still pending. The test object store is the existing in-memory backend; PostgreSQL and OpenBao are real services. Storage-backend behavior retains the existing blob suite's separate adapter evidence.

## Provider foundation (2026-10-17)

- RED: `cargo test -p sagat-infra-secrets --test openbao --no-default-features` failed because the provider port/adapter did not exist.
- GREEN: `cargo test -p sagat-infra-secrets --locked` passed: 5 existing local-keyring unit tests and 4 new provider tests, including a real OpenBao 2.7.1 container. No tests skipped.
- The real-service test proves binary round trip, immutable CAS refusal, tenant-derived path separation, invalid-token refusal and token-file rotation. Additional boundary tests prove malformed/oversized response handling, credential-free errors, redirect refusal and strict endpoint/mount/deadline configuration.
- `cargo fmt --all` completed.

This evidence does not establish durable provisioning, production TLS/workload policy, migration, catalog, managed executor, API, UI or SDK completion. Those tasks remain open. No product route is enabled by the provider adapter alone.

## Durable lifecycle and resolution (2026-10-17)

- RED: `cargo test -p sagat-tests --test integration_database_connections` failed before `managed_secrets` existed; later focused checks failed before provider-aware worker resolution and explicit migration existed.
- GREEN: `cargo test -p sagat-tests --test integration_database_connections --locked` passed against real PostgreSQL and OpenBao. It proves durable intent with no provider value, reconciliation after a provider receipt, same-key replay, changed-body conflict, cross-tenant refusal, logical rotation, local-write refusal after cutover, metadata-only persistence, retained-byte reservation, leased worker resolution, legacy migration preserving ACLs, and discarding a value when publisher membership is revoked during provider I/O.
- `cargo test -p sagat-tests --test integration_security_multitenancy --locked`: 95 passed, 0 failed/skipped. Covers existing local secret lifecycle, envelope binding/re-sealing, ACLs, lease expiry, output scanning, quotas, schema classification and egress. This run preceded the additional migration-source fence and the final post-provider authorization audit; the feature suite passed after those changes.
- `cargo test -p sagat-infra-secrets --test openbao production_tls --locked`: passed with a real TLS-enabled OpenBao. Proves rejection of an untrusted CA and root token, successful scoped short-lived token use, denial of another namespace, and sealed-provider failure. Fixture setup initially failed because the generated CA filename is `vault-ca.pem`; corrected after inspecting the pinned container's file names.
- `cargo clippy -p sagat-application -p sagat-infra-secrets -p sagat-infra-postgres --lib --locked -- -D warnings`: passed.

Runtime services now support async local/provider resolution. Deployment wiring and management HTTP routes are still pending. Receipt cleanup, production manifests, API/UI/database execution and SDK work remain open; no UI/database product completion is claimed.

## Management API and deployment wiring (2026-10-17)

- `cargo test -p sagat-tests --test integration_database_connections --locked`: passed with managed HTTP create/replay/reconcile, changed-body conflict, mandatory receipt fields, and metadata-only responses against real PostgreSQL/OpenBao.
- `cargo test -p sagat-control-plane -p sagat-execution-engine --test config --locked`: 35 control-plane and 22 engine tests passed, including provider-only secret transport guards.
- `cargo test -p sagat-infra-secrets --test openbao deployment_settings --locked`: passed for complete/partial settings and development-only restrictions.
- `cargo test -p sagat-tests --test contract_platform_foundation --test contract_rest_security_multitenancy --locked`: foundation 30 passed; security initially failed because the new migrate operation lacked a valid request fixture. After adding the fixture, `cargo test -p sagat-tests --test contract_rest_security_multitenancy --locked` passed all 17, including the foreign-tenant battery.
- Studio RED: 2 new assertions failed because the credential form omitted the receipt fields and generated a different reference for retry. GREEN: `npm test -- --run src/features/integrations/IntegrationsPage.test.tsx` passed 12 tests, including stable retry and changed-value/new-request behavior.
- `npm run api` and `npm run typecheck` passed after updating the generated client and React ref typing.
- `cargo clippy -p sagat-api-http -p sagat-control-plane -p sagat-execution-engine -p sagat-infra-secrets --lib --locked -- -D warnings`: passed.

Control plane and engine now accept explicit OpenBao settings; provider-only deployments need no legacy keyring. Existing integration credential creation sends the recoverable write contract. The connection catalog, database execution, standalone secret UI, deployment profiles/cleanup and SDK support remain incomplete. No browser verification has been performed for this slice yet.

## Connection domain and catalog checkpoint (2026-10-17)

- RED: the domain test could not import the missing database module. GREEN: `cargo test -p sagat-domain --test database_connections --test identity --locked`: 3 new boundary/limit/redaction tests and 27 identity/role tests passed.
- `cargo test -p sagat-tests --test integration_database_connections --locked`: real PostgreSQL/OpenBao catalog scenarios passed: receipt replay, changed-content refusal, foreign tenant refusal, competing revisions with one winner, immutable prior settings with current paused status, immutable-row trigger, byte accounting and tombstone/no resurrection. Managed HTTP create/replay responses validate against canonical OpenAPI; embedded password fields are rejected.
- `cargo test -p sagat-tests --test integration_security_multitenancy --test integration_database_connections --test contract_rest_security_multitenancy --locked`: 95 security, 1 combined feature scenario and 17 REST security tests passed. This regression initially could not compile an existing local-key assertion after keyId became optional; the assertion now explicitly requires the local key. The exhaustive storage-classification/ALWAYS-trigger check passed with the new tables.
- `cargo check -p sagat-api-http -p sagat-control-plane --locked`, generated API client and Studio typecheck passed. Initial compile checks caught a missing tracing attributes feature and SQLx 0.9's prohibition of dynamic SQL strings; queries now use literal concatenation and bound parameters.

This checkpoint provides a metadata catalog. It does not execute database steps, populate run bindings, test connections, disclose results or add the connection UI. Those tasks remain open. Configurable catalog ceilings/retention and additional audit-failure/authorization races still need coverage before release.

The follow-up `requiredProvider: openbao` guard passed with 2 feature tests: an instance with only the local keyring returns 503 and creates no secret. Catalog/API/application/PostgreSQL clippy passed with `-D warnings`. The first follow-up stale-principal test incorrectly expected metadata reads to fail after demotion to viewer; the approved role matrix permits those reads. The corrected test expects metadata access and rejects catalog writes using the stale administrator principal.

Final checkpoint rerun: both feature tests passed, including current-role metadata/write separation. Generated Studio client/typecheck, architecture (22 crates), formatting and diff checks passed.

## Studio connection and secret management (2026-10-17)

- RED: the connection journey could not import the missing ConnectionsPage. GREEN: focused Studio tests passed 27 tests across connections, secrets, integration credentials and navigation. Cover inline OpenBao creation, stable uncertain-write receipts, saved-secret reuse after connection failure, selection/migration gating, rotation's expected version, metadata-only query caches, namespace switching, viewer controls and explicit revision-fenced deletion.
- `npm run typecheck`, `npm run lint` and production Studio build passed. Type checks caught defaulted OpenAPI limits being nonoptional in the generated client; forms now submit finite defaults and validate the edited values. Lint caught unsafe test matcher/JSON typing, which was corrected.
- `cargo test -p sagat-tests --test e2e_database_connections --locked -- --ignored --nocapture`: passed against real PostgreSQL, OpenBao 2.7.1 and isolated Playwright Chromium. The test really executed (1 passed, 0 ignored). It creates credentials and a connection, loses the response AFTER the API commits, retries the same request, checks one saved secret, reloads and verifies metadata-only editing. No SQL execution is claimed.
- Axe checks at widths 320, 768, 1024 and 1440 reported no selected WCAG A/AA violations. The page did not overflow horizontally. Keyboard traversal reached Save; Escape restored focus to the secret action. No page errors or unexpected console warnings/errors; the one aborted resource is intentional fault injection. This is automated accessibility coverage, not a full manual audit.
- Screenshots were generated under `frontend/e2e/test-results/database-connections-*/` and visually inspected at 320 and 1440. The first view showed an unnecessarily tall form; the final layout groups target and credential fields and collapses optional CA settings. The browser test passed again after that change.

The separate browser harness sets `STUDIO_DATABASE_CONNECTIONS=1`; the existing workflow-only harness does not discover this journey because it does not start OpenBao. CI wiring of the complete feature remains T017-025. Probe, database workflow steps, ad hoc execution, result view and five-SDK support remain pending.
# Managed database IR checkpoint — 2026-10-17

- `cargo test -p sagat-infra-ir -p sagat-domain --tests`: 383 passed, 0 failed, 0 ignored across 20 targets; then `cargo test -p sagat-domain --test database_connections`: 4 passed after adding cache-weight/policy-mutation coverage (one additional test).
- `cargo clippy -p sagat-domain -p sagat-application -p sagat-infra-ir --lib -- -D warnings`: passed.
- `python3 scripts/check-architecture.py`, `cargo fmt --all`, `git diff --check`: passed.
- Initial red tests rejected the unsupported database node and exposed ordinary reserved-activity forgery. Both pass with the typed node metadata and domain guard. Six IR tests cover lossless values/shared fixture, write safety overrides, reserved names, canary redaction, expression syntax, UTF-8 byte ceilings and invalid tagged values.
- Domain cache weight includes SQL/parameter bytes. Writes carry existing high-risk/irreversible semantics and one attempt. No SQL or network operation executes in the codec/domain. Ordinary leasing explicitly excludes managed nodes; dedicated admission/executor capability work remains T017-012..017, and five-SDK consumption remains T017-018..019.

# Connection admission checkpoint — 2026-10-17

- `cargo test -p sagat-tests --test integration_database_connections --test integration_durable_execution -- --nocapture`: 2 feature tests and 178 durable-engine regression tests passed, 0 failed/ignored. The feature fixture uses real OpenBao and PostgreSQL and now checks publication authority, immutable run pins, historical revision selection, current pause, current author/actor ACL, demotion with a stale principal, ordinary-worker exclusion, concurrent same-key starts, rerun/child/event admission and denials. Dropping an open admission transaction leaves no run or pin; ACL revocation waits for that transaction's share lock and subsequent starts are denied.
- After adding target-namespace promotion and AlreadyActive revalidation, `cargo test -p sagat-tests --test integration_database_connections --test integration_gitops_versioning -- --nocapture`: 2 + 35 passed, 0 failed/ignored. Promotion fails until the target has its own OpenBao secret/connection; replay fails after the target is paused.
- `cargo test -p sagat-application --test definition_service`: 39 passed. `cargo check --workspace --all-targets`: passed after updating three older ServeConfig fixtures for the optional OpenBao field and two storage-test publication calls for the typed definition argument.
- Safe internal `DatabaseUnavailable`/`DatabaseLimit` failures distinguish a denied managed child from a vanished parent reservation; the former durably fails the parent step with `DATABASE_UNAVAILABLE`. API misses remain opaque. Publication/start/promotion/trigger/child denials use bounded `database.denied` audits. Admission emits a skip-all trace and closed-label counter/duration; it performs no external/provider I/O.
- The shared creation transaction covers current callers; final ad hoc/probe evidence awaits T017-016. Managed leasing/capabilities and actual SQL execution remain pending and are not claimed by these tests.


# Managed capability checkpoint — 2026-10-17

- `cargo test -p sagat-tests --test integration_database_connections`: 2 passed (multiple real-service scenarios inside the fixtures), 0 failed/ignored. Latest run includes real gRPC `ResolveDatabase`, all eleven shared typed parameters, cross-tenant and ordinary-worker denial, dedicated/current role and session ownership, protocol refusal, connection concurrency across sessions, immutable credential pins surviving six rotations, current publisher ACL revocation, connection pause, role demotion, lease expiry, cancellation, and same-name secret deletion/recreation. Barrier tests require concurrent mutations to commit within two seconds while OpenBao I/O is held; no run/catalog/membership lock crosses that I/O.
- Audit injection proves a failed resolution audit rolls back its credential pin and returns no value. Pin UPDATE and deletion of a referenced provider operation are refused. Repeated identical resolution shares the existing bounded per-attempt/generation/version audit entry.
- `cargo test -p sagat-tests --test integration_security_multitenancy`: 95 passed, including exhaustive storage classification and ALWAYS-trigger checks. `contract_grpc_security_multitenancy`: 10 passed. `contract_grpc_durable_execution`: 62 passed initially; its one failure was a stale expected negotiated minor (4 instead of 5), corrected and rerun individually: 1 passed, 62 filtered. No claim that the full 63-test target was rerun after that expectation change.
- `cargo test -p sagat-infra-ir --test database -p sagat-api-grpc --test compatibility`: 7 + 2 passed. The compatibility test passed against the previous baseline before the additive baseline was regenerated; regeneration ran its 2 tests successfully. Domain identity tests: 28 passed (the new role test failed before the role existed).
- `cargo check --workspace --all-targets`, Studio generated-client refresh/typecheck, architecture check (22 crates), formatting/diff checks and focused library clippy passed during the checkpoint. The final feature delivery still needs the full SDK/browser/runtime gates.
- Independent read-only review via the `code-review-and-quality` skill found one required issue: managed results were reachable before credential-field scanning. It is fixed with explicit checkpoint guards: managed completion and blob writes are refused; failures accept only fixed `WORKER_ERROR`/`OUTCOME_UNKNOWN`, and the transport discards safe messages. Regression tests exercise attempted credential disclosure through completion, failure code and blob admission. This is a temporary closed gate, not a finished database result path. T017-017 must replace it with tested pin-based field scanning before activating the SQL runner.
- The protocol/storage capability is implemented. Customer SQL, target network/TLS enforcement, physical executor isolation, configurable policy ceilings, provider cleanup and complete results remain pending T017-014..025.

# PostgreSQL target adapter checkpoint — 2026-10-17

- `cargo test -p sagat-infra-database`: 8 passed (3 unit, 3 policy, 1 combined PostgreSQL scenario, 1 TLS fixture), 0 failed/ignored. The later hostile-server target adds one passing integration test; it sends only a 2 GiB frame header after authentication and verifies bounded refusal/socket closure without sending a body.
- Latest focused command `cargo test -p sagat-infra-database -p sagat-infra-secrets --test database_credentials --test postgres --test tls --test policy --test hostile_server`: 7 passed across those targets, 0 failed/ignored. PostgreSQL is real version 16 (`16-alpine`, consistent with the repository fixtures). Scenarios include all supported value families, duplicate columns, arbitrary precision, injection-shaped parameter text, READ ONLY rejection of a SECURITY DEFINER write, privileged-role/wrong-password denial, row/byte limits, cancellation/deadlines and unknown writes. Saturating two live sessions rejects a third; cancellation releases both and a new query succeeds immediately.
- The custom-CA PostgreSQL TLS fixture passes for the original hostname and rejects a mismatched IP name and untrusted CA. It initially exposed a teardown race: the parent returned before the aborted pump released capacity. The permit now belongs to the pump and normal execution awaits its termination/socket destruction before returning.
- Actual driver execution with TRACE enabled passes SQL-literal, parameter, password and server-NOTICE canaries. Source inspection found that both tokio-postgres and sqlparser emit SQL diagnostics. A newly added literal canary failed through sqlparser; the hard log target suppression now covers both crates, including normalized bridged `log` records. Fixture provisioning logs are cleared before capturing managed execution because Docker setup logs contain test fixture environment values and is not part of the runner binary.
- `cargo test -p sagat-telemetry --test logs`: all 11 passed after the final parser filter, including JSON and text-format suppression. `cargo clippy -p sagat-infra-database -p sagat-infra-secrets -p sagat-telemetry -p sagat-application --all-targets -- -D warnings`: passed. The expanded all-target gate caught an older OpenBao fake-server read-count warning; that test now verifies nonempty request input. Architecture check passes with 23 crates; formatting and diff checks pass.
- Independent read-only review found missing destination-port enforcement. Fixed with bounded deployment/namespace destination scopes (CIDRs and ports); disallowed ports fail before DNS. Tests cover both namespace and operator restrictions. The review found no other high-confidence required driver issue; root review additionally caught the SQL-parser logging issue above.
- Fresh sessions close after each attempt; there is no idle session/credential cache. This implements the finite connection/idle bounds without promising warm-pool reuse. The adapter is not wired into control-plane/scheduler processes. The isolated runner, retained fixed probe, result validation/scanning/disclosure, SDK/UI execution and deployment evidence remain pending. Managed result storage gates are still closed. The new credential codec is a shared strict decoder with initial raw/JSON-escaped detection; encoded/binary scanning is not claimed complete yet.

### T017-017 result disclosure — verified checkpoint

Added the dedicated namespace/run/node endpoint, current-role recheck after bounded
object I/O, committed provenance, EOF/digest/type validation and synchronous audit
before returning bytes. Production control-plane and test composition roots provide
the result reader; no target SQL driver enters the control plane. OpenAPI and the
Studio generated client include the route and typed result. This is not completion
of SDK/UI execution or the managed runner.

Initial endpoint test failed with 404 as expected, then passed with a 350KB committed
blob while OpenBao was deliberately unavailable. Extended tests passed after correcting fixture mistakes (journal column/event name,
Vec access and role-update helper), without changing production semantics. Successful read audits
are per disclosure; denials persist synchronously and deduplicate.

The independent review identified an unbounded response-delivery lifetime despite a
bounded read. The fix adds a native connection abort handle and retains a deadline
guard with the Bytes owner and transfer permit. Real TCP test, 4MiB response, 1KiB
receive buffer, periodic progress, 500ms absolute budget versus 10s stall timeout:
`cargo test -p sagat-api-http --lib absolute_deadline --locked` — 1 passed. Permit
recovered before 900ms while the client still held the socket. Reviewer found no
remaining Required/Critical issues on the focused second pass; reviewer ran no tests.

Final evidence for this checkpoint:

- `cargo test -p sagat-tests --test integration_database_connections --locked`: 3 passed (two combined real-service fixtures plus the OpenAPI/retained-schema parity test). Subscenarios include inline/blob success, unavailable OpenBao, four denied roles, foreign tenant and second namespace, missing/ordinary node, unchanged structural inspection, synchronous audit failure, current-role revocation while object I/O is blocked, corruption and read timeout.
- `cargo clippy -p sagat-application -p sagat-api-http -p sagat-infra-postgres -p sagat-control-plane --all-targets --locked -- -D warnings`: passed.
- REST contract regression: 9 passed; platform fault regression: 38 passed; security/multitenancy regression: 95 passed.
- `cargo test -p sagat-api-http --lib --locked`: 7 passed, including native TLS/keep-alive and absolute delivery deadline.
- Feature integration target clippy (`-p sagat-tests --test integration_database_connections -- -D warnings`): passed.
- Studio `check:api` and `typecheck`: passed. Architecture check: 23 crates passed. Formatting and diff whitespace: passed.

The full feature is still incomplete. T017-015/016 runner/ad hoc/probe, all-five SDK execution, Studio execution/results, deployment, cleanup/restore and final fault/performance gates remain pending.

### T017-015 isolated runner — 2026-10-17

Added the separate `database-runner` process and operator policy file, authenticated
protocol-1.5 sessions, scoped capability decoding, per-attempt policy reload, typed
PostgreSQL dispatch, heartbeat cancellation and idempotent result/failure reporting.
The production execution engine now supplies the previously tested credential and
result validators. Managed polls carry no unrelated run context; managed generic
blob reads fail closed. Namespace/connection/tenant capability fields are additive.

Independent read-only review found first-namespace starvation and ineffective local
pool narrowing across per-task adapters. A persistent polling cursor and shared
namespace/connection session counter fix them. Real process regressions exercise
both. Second review found no remaining Required/Critical findings; reviewer ran no
tests. It explicitly retained active-write process interruption and shutdown/restart
during reporting as later fault-test gaps, not proven guarantees.

Actual evidence:

- `cargo test -p sagat-database-runner --lib --locked`: 4 passed. Configuration,
  capability scope/types/deadlines and shared/reloaded capacity bounds.
- `cargo build -p sagat-database-runner --bin database-runner --locked`: passed.
- `cargo test -p sagat-tests --test integration_database_runner --test integration_database_connections --locked`: 1 process fixture + 3 feature tests passed. The process fixture starts real PostgreSQL/OpenBao and a separate binary with its environment cleared; checks exact integer/decimal/text values, 350KB blob output, one target write despite a lost committed completion acknowledgment, durable receipt replay, live heartbeats, cancellation after heartbeat loss with socket closure, mismatched namespace refusal before write, narrower shared pool limit, policy reload, fair progress of a second namespace while the first has eight queued runs, SIGTERM drain, fixed-label metrics, exported tenant/namespace/run/parent-correlated OTLP spans and credential/SQL/parameter/result TRACE canaries.
- Initial fixture compile errors (UUID serialization) and journal-table lookup were
  corrected; subsequent complete commands above passed. No production guarantee is
  inferred from those failed initial fixture runs.
- `cargo test -p sagat-api-grpc --test compatibility --locked`: 2 passed against
  the prior additive baseline before regeneration.
- `cargo clippy -p sagat-database-runner -p sagat-application -p sagat-execution-engine --all-targets -- -D warnings`: passed.
- `cargo clippy -p sagat-tests --test integration_database_runner --test integration_database_connections --locked -- -D warnings`: passed.
- Architecture check: 24 crates passed; formatting and whitespace checks passed.

See `docs/runbooks/database-runner.md` for exact configuration and reproduction.
The process enforces local destination/resource policies; actual container/network
isolation is still a T017-023 deployment gate. Ad hoc/probe admission, five SDKs,
Studio execution/results, cleanup/restore and final gates remain incomplete.


### T017-018/019 five SDK builders and real execution — 2026-10-17

Added public managed database builders in TypeScript, Go, Python, C# and Java;
explicit literal/expression parameters, scoped connection references, bounded
optional limits and snapshots of caller values. No SDK receives credentials or
runs SQL. Synchronized canonical worker protocol copies/generated stubs while
retaining ordinary SDK protocol 1.4. Added a five-language guide, SDK specification
and CI prerequisite builds of the isolated executor.

The shared `database-connections` scenario compares exact canonical documents and
14 invalid authoring vectors. Each SDK then uses a separate real platform, OpenBao
2.7.1, target PostgreSQL 16 and a separate database-runner process. It starts typed
queries, binds a later database parameter to an earlier result, consumes those
inline results in an ordinary SDK handler, writes one exact decimal row and rejects
DDL with no retry. Independent hard assertions verify success/failure, exact values,
target effects and that no managed task/credential reaches an ordinary SDK worker,
even during golden regeneration. Blob expansion is not claimed by this scenario;
large results retain descriptors and require explicit verified reading by ordinary
handlers, as the guide now explains.

Initial focused tests failed for missing public builders as intended. Initial Go
and Python full regressions detected stale protocol copies; regenerating with the
repository scripts resolved them. Initial clippy found two earlier fixture unwraps;
these now use descriptive expects. Independent review caught Java fixture numeric
truncation and TypeScript final-newline acceptance; exact conversions/full matches
and shared invalid vectors fix both. Second pass reported no Required/Critical code
findings and requested the inline/blob documentation qualification, now addressed.
The reviewer ran no tests.

Actual evidence:

- TypeScript `npm test`: 113 passed; focused database tests: 2 passed after the
  final full-match correction. Canonical scenario includes the new newline cases.
- Go `go test ./...`: passed; `go vet ./...` and gofmt checks passed.
- Python pytest: 244 passed; strict mypy: 38 source files passed; Ruff passed.
- C# `dotnet test`: 421 passed on net10.0; conformance builds for net10.0 and net8.0
  passed; `dotnet format --verify-no-changes --no-restore` passed.
- Java `./mvnw -q -B verify`: passed; SDK unit reports: 217 tests, zero failed,
  errored or skipped.
- `SAGAT_SDK_SCENARIO=database-connections SAGAT_SDK_REUSE=1 cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`: passed all five languages in 26.51s against reviewed goldens. TypeScript-only initial real execution also passed before the five-language comparison.
- `cargo clippy -p sagat-tests --test sdk_conformance --locked -- -D warnings`:
  passed; Rust formatting and diff whitespace checks passed.

This closes the SDK slice, not the feature. Retained ad hoc/probe admission,
Studio execution/results, deployment isolation, cleanup/restore and final feature
regression/fault gates remain pending.


## 2026-10-17 — retained ad hoc executions and fixed probes (T017-012/014/016)

- Added private immutable definitions indexed by tenant/namespace/actor/canonical digest. Internal
  workflows remain inactive and absent from normal definition/version/publication APIs. Existing
  workflow and content quotas apply; identical definitions reuse their version at the count ceiling.
- The private definition/version, run, connection pins, initial journal/tasks/outbox, audit and
  receipt commit atomically. A deferred precommit fault proved every affected table count unchanged,
  then the same key succeeded after removing the injected fault. Concurrent submissions returned
  one run. Changed input/actor/origin conflicted. Ad hoc submission of the exact fixed probe IR reused
  its definition without changing probe provenance.
- New closed HTTP routes admit ad hoc step/workflow and fixed connection probes; normal runs omit
  origin. Probe status, tested revision and safe failure are read from one SQL snapshot. Paused
  connections refuse admission. Connection edits retain evidence for the tested older revision.
- Current membership/ACL checks cover replay and later capabilities. Review identified missing
  current RunOperate on private reruns and missing early permission-denial auditing; both were
  corrected. The cached Admin→Developer test permits an ordinary ad hoc query but rejects new and
  replayed reruns. HTTP and direct-use-case permission denials emit namespace-scoped, content-free
  audits through the existing bounded asynchronous writer; tests await its documented completion.
- Real OpenBao/PostgreSQL/separate runner executed the fixed probe successfully, returned typed
  integer 1, and produced DATABASE_AUTHENTICATION_FAILED for wrong credentials. A retained ad hoc
  workflow bound run input into one query and the first inline result into a second query (value 42).
- `integration_database_connections`: **3 passed** (12.76 s in the final expanded regression before
  the runtime-role variant); `integration_database_runner`: **1 passed** (19.41 s).
- `integration_durable_execution`: **178 passed** (50.90 s); `integration_gitops_versioning`:
  **35 passed** (9.60 s). REST durable-execution **2 passed**, REST GitOps-versioning **1 passed**.
- Clippy with `-D warnings` passed for application, HTTP, PostgreSQL and scheduler all targets,
  and for both feature integration targets. Formatting and diff checks passed. Architecture check
  passed for **24 crates**, with **8 checker tests**. Generated Studio OpenAPI parity and TypeScript
  typecheck passed.
- Initial red tests failed because the new service/types were absent. During implementation,
  compile checks caught method visibility, UUID serialization and dynamically built SQL in a test;
  these were corrected. Tests caught the exact no-store header expectation and asynchronous denial
  audit timing; the tests now match existing behavior, without strengthening the audit guarantee.
- Independent read-only review found no remaining Required/Critical findings. The reviewer did not
  run tests. Studio controls, policy ceilings, provider cleanup, deployment and final delivery gates
  remain unchecked tasks; this checkpoint does not claim the complete feature is shipped.

### Runtime application-role verification

The real HTTP and separate-runner fixtures now connect as `sagat_app`. This exposed
a PostgreSQL privilege bug: `SELECT ... FOR SHARE` on membership/namespace/secret ACL
rows needs update authority, which this role intentionally lacks. Migration 031 adds
three narrowly scoped SQL SECURITY DEFINER read-lock functions with fixed search
paths, qualified relations, tenant filters, and EXECUTE revoked from PUBLIC. Callers
retain the same transaction and lock order. No UPDATE privileges were added.

- Real feature regression: **3 passed** (17.97 s), including assertions that UPDATE
  of all three authority tables still fails with 42501 and PUBLIC has no EXECUTE
  permission on the functions.
- Separate-runner regression with the runtime role: **1 passed** (17.67 s); preceding
  feature run with the same role: **3 passed** (12.45 s).
- Clippy for both integration targets passed after the privilege assertions.
- Independent review of the functions and all call sites found no Required/Critical
  findings; the reviewer did not run tests.

## 2026-10-17 — Studio managed authoring, execution and results (T017-021/022)

- Registered the canonical database node in the existing visual editor, with a namespace catalog
  picker, revision, query/write choice, SQL, typed bindings and limits. Nested expression bindings
  follow node renames and participate in dangling-reference advice. Invalid/inexact JSON cannot
  silently submit an older or rounded value. Selected draft/version API responses also reject
  unsafe numbers before the editor can autosave them.
- Added a temporary ad hoc list/inspector editor (supported kinds, links, input, deadline,
  undo/redo) and connection probe/status dialog. Origins label/filter private runs without trying
  to read their hidden normal definitions. Runtime graph exposes authored type `database`, with
  no statement, values or connection credential reference; real HTTP assertions cover this.
- The initial independent review found a wrong timeout field, unsafe numeric conversion,
  uncertainty lost after later refusals/navigation, and missing success-receipt validation.
  These are fixed. Session-scoped immutable intents retain body/key across navigation, preserve
  ambiguity after 403/404/429, cap retained requests, clear on session end, and require a run
  receipt before accepting success. Tests include empty 201 bodies and remount recovery.
- Result reads require explicit permission/action; owned component state is cleared on close,
  identity/namespace/run changes. Complete responses are capped at 4 MiB/30 seconds; error bodies
  are capped at 64 KiB and 401 clears the session without reading a body. Review caught and fixed
  the initial unbounded error-body path. A chunked never-ending 502 test proves bounded cancellation.
  Results preserve exact numeric tokens, duplicate column names and escaped text, paginate rows/
  columns and bound long-cell previews. No result data enters React Query or browser storage.
- Added pinned MIT `lossless-json@4.3.2` for pre-conversion authoring validation and lossless result
  display, following its [primary API documentation](https://github.com/josdejong/lossless-json).
- Studio regression before the final selected-response/error-body additions: **1,131 passed**
  across **82 files** (24.14 s). Focused authoring/command/result/API tests after safe draft parsing:
  **24 passed** (1.47 s). Final result/error-body focused suite: **5 passed** (0.69 s).
- Real HTTP feature regression after the runtime node-kind change: **3 passed** (12.69 s).
- Real Chromium harness with PostgreSQL 16, OpenBao 2.7.1, runtime-role API and separate runner:
  **4 journeys passed** (13.9 s; Rust harness 18.23 s). Covered catalog partial-save recovery,
  active probe success, paused probe refusal, write committed then empty acknowledgement, SPA
  navigation/retry returning the same run (target independently has exactly one row), reload/run
  recovery, a two-query workflow with input/output expressions and exact integer/decimal/JSON
  results, a published UI-authored node, viewer/developer permissions, and refused DDL with safe
  durable failure. Zero page errors in monitored journeys; Axe checks at 320/768/1440 px passed
  with no page overflow. Result-table and mobile editor screenshots were visually inspected.
- Generated OpenAPI/Studio parity passed. Production build passed. Initial regression failures
  were three expected navigation/palette-count changes; assertions were updated. Initial lint
  caught unsafe object stringification and redundant test casts; both were corrected.
- Independent read-only review reported no remaining Required/Critical findings after the bounded
  error-body fix; reviewer ran no builds/tests. Deployment, cleanup, remaining policy ceilings and
  final whole-delivery gates are still tracked separately below.

Final checks for this Studio checkpoint: **1,133 tests passed** across **82 files** (22.75 s),
TypeScript typecheck and ESLint passed, and Clippy with `-D warnings` passed for the HTTP crate and
both modified Rust integration/browser targets. The final bounded error path has its focused unit
coverage above; full deployment/SDK/crash gates remain T017-023/024/025.

## Current database policies and driver discovery — 2026-10-17

- Added 13 resources to existing quota governance, with SQL range/scope constraints and the
  existing operator grant/admin narrowing/audit paths. The nine positive per-operation ceilings
  default to hard maxima; smaller operation/connection defaults remain unchanged. Count defaults
  remain 100 connections, 16 namespace/64 tenant leases and 32 pending provider operations.
- Shared tenant/namespace policy fences cover one-snapshot reads inside publication, admission,
  new claims, both capability phases and output validation. Tests prove operator narrowing of an
  existing override, all nine delivered bounds, zero concurrency, same-lease resolution at cap=1,
  provider-policy change during external I/O and a policy writer blocked by an admission fence.
- Catalog capacity includes paused entries. Tests prove new creation refusal, replay/edit/delete
  at zero capacity, pending intent settlement after lowering its count to zero, and one final
  tombstone at revision 1,000. Retained revisions and historical result bounds are preserved.
- Added authenticated namespace driver discovery, advertising PostgreSQL and explicit defaults/
  maxima/type families. Real runtime-role HTTP checks validate the canonical response schema and
  foreign/missing namespace refusal. Generated Studio client and API parity/typecheck pass.
- `cargo test -p sagat-tests --test integration_database_connections --locked`: 3 combined real
  PostgreSQL/OpenBao targets passed in 26.30 seconds after the final revision-cap assertion.
  Includes a dispatched write whose result is refused after lowering the current result bound:
  no output retained, expired lease settled to MANUAL_REVIEW, step.needs_review journal, attempt=1,
  no new task and journal/projection consistency. A previously retained authorized result remains
  readable. This tests server settlement; the separate process-crash drill remains T017-024.
- `cargo test -p sagat-domain --locked`: 312 tests passed across 15 targets, including positive
  policy bounds, zero-count semantics, scopes/units and compatibility defaults.
- `cargo test -p sagat-tests --test integration_security_multitenancy quota --locked`: 28 passed,
  67 filtered out, 7.98 seconds. Initial run exposed the expected-list expansion and missing storage
  classification for private-definition actor/digest and run origin from T017-016. Added concrete
  protected bounds/owner evidence; the exhaustive catalog/trigger check now passes.
- OpenAPI lint passes with the existing seven tag-description warnings. Architecture passes for
  24 crates. Independent read-only review found no Required/Critical policy/discovery issue and
  requested the result-tightening/expiry evidence above, which was subsequently added and passed.
- Focused `cargo clippy -p sagat-api-http -p sagat-infra-postgres -p sagat-tests --test
  integration_database_connections --locked -- -D warnings` passes in 10.93 seconds. Its first
  run caught identical validation branches; combined the range predicate and reran successfully.
  Formatting and diff whitespace checks pass.

## Recoverable provider retirement checkpoint

- Real OpenBao retirement suite: 6 passed (1.46 s), including a separate scoped token that cannot read credentials or delete metadata. Existing and absent paths remain fenced against delayed CAS=0 writes after destruction.
- `cargo test -p sagat-tests --test integration_database_connections --locked`: 3 passed (14.49 s). New scenarios run cleanup/managed-intent writes through `sagat_app`, prove lost destruction acknowledgement, expired/superseded claims, no early quota refund, provider-outage deferral, 24-hour terminal retention, preserved live/manual-review pins and the current plus four prior versions. Fixture time is advanced only through the isolated owner connection.
- `cargo test -p sagat-control-plane --test cli --locked`: 19 passed (8.94 s), including missing provider and invalid batch without configuration/value leakage.
- Focused application/PostgreSQL/secrets/control-plane `clippy --all-targets --locked -- -D warnings`, formatting, diff whitespace and architecture (24 crates): passed.
- Independent read-only review: no Required/Critical findings.
- Test development caught an off-by-one rotation fixture and cancellation still awaiting lease expiry; both fixtures were corrected to exercise the actual lifecycle. No failing test is claimed as evidence.
- Production scheduling, coordinated backup/restore and final regression remain separate open tasks. OpenBao metadata tombstones accumulate permanently; only value bytes and unreferenced PostgreSQL receipts are reclaimed.

## Existing consumer I/O checkpoint

- Regression tests first reproduced both defects: a blocked provider read blocked heartbeat (1-second timeout), and migrated HMAC verification accepted an old local key while OpenBao was unavailable.
- Ordinary resolution and completion/failure-code scanning now commit their snapshot transaction before provider I/O, then reacquire and recheck current lease, generation, cancellation, binding and relevant secret metadata. Receipt replay precedes provider access and is repeated after I/O.
- Real-service feature suite passes with blocked-read heartbeat/cancellation for resolve/complete/fail, rotation-during-scan refusal followed by a fresh successful scan, report receipt replay with the provider unavailable, healthy HMAC migration grace, no event admission or outbound send during outage, and durable outbound retry after recovery.
- Independent review caught the initial shared fence rejecting derived saga recovery during stopping. Corrected to match domain semantics; a cancelled saga's secret-bearing compensation and reconciliation both resolve/report and finish consistently. No Required/Critical findings remain after review.
- `cargo test -p sagat-tests --test integration_security_multitenancy secrets --locked`: 31 passed, 64 filtered (20.01 s).
- `cargo test -p sagat-tests --test integration_events_webhooks_messaging --locked`: 111 passed (18.64 s).
- Final combined database-connections and sagas-compensations integration run: all passed (3 feature tests and 74 saga tests; saga suite 9.63 s). Application/scheduler clippy with warnings denied, formatting and diff whitespace passed.
- Existing ordinary raw/JSON-escaped UTF-8 scanning scope is unchanged; managed result scanning/pins remain separate. Deployment and remaining recovery/final gates are still open.


## Deployment and workload identities — T017-023

- Built actual control-plane, execution-engine, development IdP and database-runner Docker
  targets. The runner image contains its executable, CA roots and non-root account; deployment
  mounts only its OIDC identity, engine trust and bounded configuration.
- The development overlay runs persistent OpenBao 2.7.1 with verified TLS, declarative audit,
  one-hour scoped application tokens and a separate restricted TLS PostgreSQL target. The
  preparation directory is private and excluded from source/build contexts. Sandbox runner
  transport also switches to verified engine TLS, preserving the combined profile.
- `python3 tests/e2e/database-connections/compose.py`: passed with these built images,
  runtime-role API, real secret provisioning, probe, typed retained result, idempotent admission
  and controlled provider outage. The fixture tears down only its own Compose project.
- Official OpenBao chart 0.30.0 and server/agent 2.7.1 are pinned by verified archive/image
  digests. Production values use three TLS Raft voters, retained data/audit volumes, finite
  resources and declarative audit. The bootstrap sets exact SA/namespace/audience roles,
  30-minute tokens with a one-hour maximum, and separate management/runtime/cleanup policies.
- Real Kubernetes proof passed on kind 0.33.0 / Kubernetes 1.35.8, three nodes and Calico
  3.31.4: all three Raft members healthy; actual projected JWT login; short-lived token
  renewal/re-authentication; wrong audience/SA denied; scoped credential/create/destroy ACLs;
  target PostgreSQL and engine sockets allowed; platform PostgreSQL, OpenBao, Kubernetes API
  and metadata sockets refused from the runner. TokenReview still works after default-deny.
  Auth/network probes reuse rendered application templates with test containers. This is
  separate from the built-image SQL execution proof above. The temporary cluster was deleted.
- Initial live deployment attempts exposed fixture CA key-usage, Raft unseal startup timing,
  named-volume ownership and Docker internal-network port-publication issues. These were
  corrected and the complete proofs rerun. No failed attempt is represented as a pass.
- Helm supplies distinct backend SAs and memory token sinks, projected JWTs mounted only by
  agents, checksummed agent configuration, bounded cleanup CronJob, stable runner StatefulSet
  identities, explicit egress, TLS and resource validation. Optional OTLP requires HTTPS and
  explicit collector egress. No provider/platform credential is mounted in the runner.
- Chart contract suite: **23 passed**. Runner config unit tests: **4 passed**. Runner clippy
  with all targets and warnings denied passed; Python compilation, shell syntax and formatting
  passed. Independent review found the sandbox TLS compatibility issue above; it is fixed,
  with no remaining Required/Critical findings in this deployment slice.
- Reproduction and operator responsibilities: `docs/runbooks/openbao.md`,
  `tests/e2e/database-connections/kubernetes.sh`. Production image publication, real issuer,
  cluster certificates, storage class and private CIDRs remain deployment-specific inputs.


## Recovery, key replacement and process crash — T017-024

- `integration_database_runner`: the real process was SIGKILLed after PostgreSQL confirmed
  its insert and before the proxy forwarded CompleteStep. After the actual eight-second
  lease expired, the scheduler retained MANUAL_REVIEW. A restarted runner executed a fresh
  read while the original attempt stayed at one and the target effect count stayed at one.
  Combined runner/fault suite passed in **46.24 s** before the later timing instrumentation.
- `integration_database_connections`: **3 passed in 19.76 s**, including new runtime-role
  intent-key replacement coverage: wrong-key pending activation refused, old-key settlement,
  active reconciliation under the new key, unchanged credential read, old PUT replay conflict,
  and a new rotation without duplicate original operations. Existing migration, authority,
  ACL, cleanup and credential-rotation tests run in the same target.
- Built-image Compose recovery passed: stop/drain the writers, take PostgreSQL custom dump,
  Raft snapshot and matching intent key; delete actual provider metadata and recreate the
  platform database; restore both and restart/unseal OpenBao. The retained typed result and
  original admission receipt survive, a fresh query resolves the restored credential, and a
  destroyed KV version still rejects delayed CAS=0 creation. Provider outage then fails closed.
- This proves a quiesced, same-cluster recovery point. It does not establish arbitrary mixed
  PITR, fresh-cluster force restore, missing-unseal-key recovery or reversal of customer effects.
  `docs/runbooks/database-recovery.md` states the isolation, credential/key rotation, migration,
  finite-token, blob-backup and external-effect reconciliation responsibilities.
- Independent read-only review found no Required/Critical issue. Its operational-order comment
  was applied: settle pending intents while reconciliation/cleanup remain available, then stop
  every writer and suspend cleanup before taking the recovery point.

## Final delivery gates — T017-025

Closed locally on 2026-10-17. All required assertions have passing evidence. Results below
distinguish the complete first pass from focused reruns; no failed command is described as green.

### Rust workspace and failed-target reruns

`cargo test --workspace --locked --no-fail-fast` completed all **181 test/doc-test targets**:
**2,298 passed, 6 failed, 50 ignored** on its first pass. All five affected targets then passed
in full, totaling **67 passing tests** in the reruns:

| Rerun | Passed | Initial failure and correction |
|---|---:|---|
| `cargo test -p sagat-cli --test diagnostics --locked` | 7 | Added the missing `INVALID_DATABASE_NODE` help entry and linked user documentation. |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` | 8 | Regenerated API and workflow-schema references; ordinary unblessed comparison passes. |
| `cargo test -p sagat-tests --test contract_rest_security_multitenancy --locked` | 17 | Supplied valid JSON for probe/ad hoc requests and a concrete node path. Foreign/missing 404, response equality and no-mutation assertions remain unchanged. |
| `cargo test -p sagat-tests --test e2e_developer_experience --locked` | 32 | Doctor now expects server maximum protocol 1.5. Four environment-dependent examples remain ignored in this target. |
| `cargo test -p sagat-tests --test integration_database_connections --locked` | 3 | First pass stopped during fixture startup with Docker `PortNotExposed(8200)`, before any provider assertion. Unchanged full target passed on rerun in 32.74 s. The startup failure was not reproduced or attributed to a product defect. |

Thus all **2,304 nonignored tests** from the workspace inventory have a passing result after
reruns. The whole workspace command was not repeated after these focused corrections.
The final workspace includes real provider/target TLS, managed results, tenant/ACL/current-role
fences, secret canaries, admission/outbox atomicity, quota/storage limits, event redelivery,
timeout/signal races, failed compensation and restart suites. In particular,
`integration_database_runner` passed in **37.80 s**, including SIGKILL after the target commit
and a restarted runner with no repeat of that effect.

### Five SDKs

- Unit/tooling gates passed: TypeScript **113**, Python **244**, C# **421**, Java **217**;
  Go `go test -race ./...`, vet and formatting passed. Python mypy (38 source files)/ruff,
  C# format verification and Java verify passed.
- Initial TS heartbeat and C# stream-stop tests depended on fixed sleeps/immediate asynchronous
  observations. They now await the actual events with bounded existing helpers. Deadline
  extension and the C# stop-under-two-seconds assertion are preserved; full language suites
  passed on rerun.
- `SAGAT_SDK_REUSE=1 cargo test -p sagat-tests --test sdk_conformance
  --test fault_sdk_multilanguage --locked --no-fail-fast -- --ignored` ran all **20 shared
  scenarios in all five languages**, plus **9 fault tests**. All fault tests passed. Nineteen
  shared scenarios conformed; the managed scenario differed only because its four graph nodes
  still said `activity` in the golden file after Studio began exposing `database`.
- Corrected only those four golden fields and added an independent graph-type assertion.
  `SAGAT_SDK_REUSE=1 SAGAT_SDK_SCENARIO=database-connections cargo test -p sagat-tests
  --test sdk_conformance --locked -- --ignored` then passed all five languages in **25.27 s**.
  Typed values, downstream consumption, one target write, denied DDL and no ordinary-worker
  SQL dispatch remain independently asserted. All 100 scenario/language combinations now
  have passing evidence across the full run and this focused rerun.
- The six SDK end-to-end stories have passing evidence: five passed on the initial full target;
  protocol negotiation initially expected 1.4 for offers above the server maximum. Corrected to
  1.5, preserving `1.4 → 1.4`, and reran `e2e_sdk_multilanguage us04:: -- --ignored`: **1 passed
  in 37.21 s** across all languages. No golden blessing or protocol fallback masked the mismatch.
- `SAGAT_SDK_REUSE=1` reuses already built toolchains; it does not skip the selected scenarios.

### Studio, containers and static gates

- Frontend **1,133 tests in 82 files passed**; lint, application/E2E type checks, production
  build and generated OpenAPI-client parity passed. npm audit reported zero vulnerabilities.
- Real Chromium feature journeys: **4 passed** (catalog/secret partial save, probes/ad hoc and
  navigation recovery, published visual nodes/current result permissions, safe failure).
  Existing Studio journeys: **33 passed, 2 skipped** by their suite configuration.
- Rebuilt the application/runner images after updating Rustls and reran the baseline Compose
  smoke and managed Compose proof. Both passed. Managed proof includes verified engine/target
  TLS, actual OpenBao credentials, typed retained results, admission replay, coordinated
  PostgreSQL/Raft restore and fail-closed provider outage. The baseline smoke's optional
  first-day CLI/browser path was not selected in this final invocation; browser evidence is
  the native real-service Playwright suites above.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, formatting and diff
  whitespace checks passed. After the last shared-scenario assertion, focused conformance
  clippy also passed. Architecture checker: **24 crates**, no forbidden edges/cycles; its
  eight unit tests passed.
- Canonical OpenAPI lint passed with its existing seven tag-description warnings; Protobuf
  compilation, AsyncAPI validation (one existing informational note) and IR example validation
  passed. Canonical generated reference tests passed after regeneration.
- `cargo audit --deny warnings` passed under the repository advisory policy. Rustls was updated
  to **0.23.46**, addressing RUSTSEC-2026-0346 through 0349; verified TLS container proofs were
  rerun with the updated dependency. See the [upstream release](https://github.com/rustls/rustls/releases/tag/v%2F0.23.46).
- Independent read-only reviews covered provider boundaries, execution/retention, SDK/UI,
  policies, deployment, recovery, measurements and final gate corrections. No Required/Critical
  findings remain. CI now runs the managed browser suite and built-container restore proof.

### Measurements and remaining operational limits

- Final `integration_database_runner measure_managed_database_phase_latency -- --ignored`
  passed in **63.57 s** after **10 warmups and 200 successful measured reads** with the patched
  dependencies. [Phase report](../../tests/performance/reports/database-connections.md) records
  environment, shared-host load, p50/p95/p99, boundaries and exclusions. No throughput or
  production TLS/HA SLO is inferred.
- [Native response resource probe](../../tests/performance/reports/database-result-memory.md)
  records the 4 MiB slow-reader test, absolute delivery deadline, recovered transfer permit and
  measured test-process RSS. This is not a production capacity estimate.
- The default workspace's 50 ignored tests are explicitly environment-dependent suites or
  benchmarks. Required SDK/browser/feature measurement gates were invoked separately above;
  unrelated benchmark and example targets were not all executed in this delivery gate.
- Production deployment still needs operator-supplied identities, image digests, certificates,
  storage/routes and protected backups. The real Kubernetes auth/network proof and same-cluster
  restore scope are described above. No production rollout, new-cluster force-restore proof,
  dynamic database accounts, Kafka target or exactly-once external effect is claimed.
- All tasks T017-001..025 are complete. [Traceability](traceability.md), the
  [UI guide](../../docs/guides/managed-database-executions.md),
  [SDK guide](../../docs/guides/managed-database-sdk.md) and
  [recovery runbook](../../docs/runbooks/database-recovery.md) describe the delivered behavior.

## Integration into develop — 2026-10-24

Feature 017 was built on `6ede1c78` and merged into the feature-014 base of `develop`
(`9a57861b`) on `feature/017-database-connections-integration`; the 30 original commits are
kept in history. Numbers D600+/G600+/O600+ were taken after scanning every local and remote
branch (highest in use: D527, G526, O501), so they cannot clash. Nothing of feature 016 payload
classification, of the 014 follow-up runtime work or of `tasks/project-completion` was taken;
`feature/016-project-completion` was only read, to see how its author resolved the same conflicts
(its migration numbers were reused so the open branches agree, see D602).

### Decisions

- **D600 — worker protocol.** `1.5` stays the scoped execution identity of feature 014
  (ADR-0030, ADR-0032). Managed database capabilities are `1.6`, a superset of `1.5`. The ceiling a
  session may negotiate is `protocol_ceiling(role, scoped_store, database_store)`: a store that
  proves only the database path leaves ordinary workers at `1.4`; scoped without database gives
  `1.5`; both give `1.6`; the `database-runner` machine role, which handles root work only,
  reaches `1.6` on the database proof alone. `WorkerStore::supports_database_worker_protocol`
  is the new port, true for PostgreSQL. `/version` and the CLI doctor report `1.6` as the
  maximum. Workers that offer `1.0`-`1.5` are served at the version they offer; the SDK
  default stays `1.5`; a managed lease still needs the runner role, a current binding and `1.6`.
  The runner registers and expects `1.6`. Conformance: `protocol-negotiation` (offer `1.9`
  is served at `1.6`) and `database-connections` (ordinary SDK workers pinned at `1.4`).
- **D601 — a managed database node is root-only.** `step_database_credentials` keeps a
  `instance_path_nodes/indices` pair that a CHECK forces empty, the executor ownership and
  credential pin are addressed by root node, and the runner refuses a lease whose instance path
  is not empty. The domain therefore rejects (`INVALID_DATABASE_NODE`) a database node in a loop or
  collection body (the schema does not even know the node type in a body), a database node in a
  definition that also has a `forEach`, `while` or `repeatUntil` node, and one with a continuation
  policy. Help text and `docs/quickstarts/errors.md` say so. See G600.
- **D602 — migrations.** The seven 017 migrations sort after develop's latest
  (`20261122000000_steps_open_index`): `20261122000100_openbao_secrets` and
  `20261123000000_database_connections` ... `20261128000000_secret_cleanup`, up/down pairs
  kept. The names and the six later versions equal those the other open branches already
  carry; only `openbao_secrets` differs (`...000100` instead of `20261122000000`, which now
  belongs to `steps_open_index`). Content changes: the provider intent has a `pools` column;
  the step credential pin is keyed by the root instance path; `database_policies` keeps
  `iteration_concurrency` in the quota ceiling CHECK that `iteration_capacity_quota` had added
  (it dropped it, so the iteration ceiling could not be stored), up and down.
- **D603 — ADR number.** 017's ADR-0033 collided with develop's ADR-0033 (payload classification).
  It is `ADR-0035-database-connections-and-managed-execution`, like on the other open branches;
  every link and code comment was updated.
- **D604 — secret pools with OpenBao.** The worker pools of a secret (feature 009 G48, merged
  after 017 was written) are part of the durable provider intent, bound by its HMAC, applied
  in the activation transaction and audited; `put_with_pools` exists on the managed service. The
  HTTP `pools` field works for both authorities.
- **D605 — report fence.** A failure that acknowledges a cancellation (`CANCELLED`, not
  retryable) of a step with declared secrets is judged on lease and attempt when its code is
  scanned; every other failure and every completion keeps the full fence (a cancellation during
  provider I/O still discards the report).
- **D606 — run creation order.** A retained ad hoc definition is created before the declared
  state of the version is read back (014 verifies it), in the same transaction.

### Gaps and owner question

- **G600** — managed database nodes cannot yet run inside loop or collection bodies, or in a
  definition with a continuation policy (D601). Needs scoped credential pins and executor
  ownership.
- **G601** — the five SDK builders do not pre-check D601; the server refuses it at publication
  with `INVALID_DATABASE_NODE`.
- **G602** — `approval-chain` (014 scenario) failed once in the first full `sdk_conformance` run with
  TypeScript at `runVersion` 4 against 3 in the other four languages (a `queryChain` read right after
  the head change) while the machine load average was above 20; it passed on rerun. The race
  belongs to the scenario, not to 017, and is not explained here.
- **G603** — the `database_policies` fix changes the content of a migration other open branches
  carry unchanged; they must take the same two lines (up and down).
- **O600** — does the owner want managed database operations inside loop and collection bodies
  (G600)? It changes the executor ownership model and needs a plan of its own.

### What was run on the merged tree

Machine: shared, load average 18-30 during most runs; Docker is the `sagat-remaining` Colima VM;
`RUST_TEST_THREADS=2`, private target directory, `CARGO_PROFILE_DEV_DEBUG=0`.

| Command | Result |
|---|---|
| `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `python3 scripts/check-architecture.py`; `python3 scripts/check-agent-config.py`; `python3 -m unittest discover -s scripts/tests` | pass (24 crates, no cycle); ok; 24 tests |
| `cargo test --workspace --locked --no-fail-fast` (the CI command, final tree, runner binary built first) | **3132 passed, 1 failed, 57 ignored**. The failure is `cron::a_schedule_missed_while_nobody_was_looking_fires_once_and_not_for_every_time_it_missed`: the assertion allows a wait of `0.0..=60.5` s for an every-minute schedule and saw `-0.0345`, i.e. the minute boundary passed between the dispatch and the read under load. The events cron code is not touched by 017; `integration_events_webhooks_messaging` alone then passed 111/111 |
| `integration_database_connections` (3) and `integration_database_runner` (1, the isolated process; the measurement test stays `--ignored`) | pass, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1` |
| runtime-role sweep: `integration_database_connections`, `integration_database_runner`, `integration_platform_foundation`, `integration_workflow_primitives`, `integration_durable_execution`, `integration_security_multitenancy` | all pass except two `integration_workflow_primitives` tests that hit `No space left on device` while migrating (the Colima VM, shared with other sessions); both pass alone as the runtime role (2/2) |
| `sdk_conformance`, all 54 scenarios x 5 languages | first run: `database-connections` (the five workers reported 1.5; scenario now pinned to 1.4 like the other root-work scenarios) and `approval-chain` (TypeScript only, G602) failed; both pass on rerun; `protocol-negotiation` was blessed after reading the diff (offer 1.9 -> 1.6, and the merge had put the 017 accepted-version change on the wrong worker: `start-current` is 1.4) and passes unblessed |
| `e2e_sdk_multilanguage` / `fault_sdk_multilanguage` (`--ignored`) | 6 / 9 passed (us04 expects 1.9 -> 1.6) |
| `e2e_developer_experience -- --ignored us03:: order_processing:: saga:: scripts::` | 4 passed |
| `e2e_workflow_studio -- --ignored journeys`; `e2e_database_connections -- --ignored` (real Chromium, OpenBao, PostgreSQL, the runner) | 1 passed; 1 passed |
| Studio: `npm ci`, `lint`, `typecheck` (studio and e2e), `npm test`, `build`, `check:api`; `node --test e2e/compose-security.test.mjs` | pass: 83 files / 1141 tests; 3/3. `npm audit --audit-level=moderate` reports 7 findings (vitest, dompurify, undici advisories published after the lock file); the identical report comes from develop's lock file, which 017 changes only by adding `lossless-json` |
| SDKs: Python `pytest` (260), `mypy`, `ruff check`; TypeScript `npm test` (130); Go `test -race`, `vet`, `gofmt -l`; .NET `dotnet test` (449), `dotnet format --verify-no-changes`; Java `./mvnw -q verify` | all pass (ruff needed the `__all__` order fixed after the merge) |
| Contracts: `contract_developer_experience` (docs/reference blessed after reading the diff, rerun unblessed), redocly lint of `openapi.yaml`, `protoc` of `worker.proto`, AsyncAPI validate, ajv `--strict=false` of the payment example and of `tests/contract/database-connections/typed-parameters.workflow.json` | pass (redocly: 7 warnings that exist on develop) |
| `helm lint infra/helm -f ha.yaml` and `-f ha.yaml -f database.yaml`; `contract_scalability_ha` (renders the chart) | pass |
| `scripts/smoke-compose.sh` with `SAGAT_BIN` set (no `SAGAT_SMOKE_BROWSER`) | passed after one environmental workaround: the Colima VM has no host mounts (`mounts: null`), so the bind-mounted `postgres-init` directory was empty and the runtime role was never created; the script was copied into the VM path once |
| `cargo audit --deny warnings` | 5 advisories published after the lock file (crypto-bigint, ctutils, rustls-webpki, webpki-roots, tokio); develop's lock file reports those plus the four rustls ones that 017's `fix(deps)` already clears |

Not run: `tests/e2e/database-connections/compose.py`, `kubernetes.sh`, `recovery.py` and
`e2e_developer_experience onboarding_browser::` (Compose with host bind mounts and a kind
cluster; the Colima VM has no mounts), `perf_*` and `measure_managed_database_phase_latency`
(`--ignored` measurements), the Compose overlay `compose.database.yaml` (needs the
development directory those scripts prepare). The evidence of T017-023 and T017-024 for those
paths is the 2026-10-17 record above; nothing in this integration changes them except the
protocol minor, the migration numbers and D601.
