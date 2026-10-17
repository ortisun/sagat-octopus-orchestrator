# Verification — feature 017

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
