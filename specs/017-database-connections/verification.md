# Verification — feature 017

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

