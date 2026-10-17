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
