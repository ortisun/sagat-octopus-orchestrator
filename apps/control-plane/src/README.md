# control-plane source

## Overview

Control-plane composition, environment configuration, operator commands, PostgreSQL migrations and HTTP/Studio serving.

## Interface

- `cli::{Command, parse, USAGE}` defines serve, migrate, seed-dev, tenant provisioning, secret resealing/cleanup, quota/weight changes, key generation and healthcheck commands. [cli.rs:Command] [cli.rs:parse]
- `config` exposes environment/typed errors, database/shared/serve configuration, auth/secrets/OpenBao/scripts readers, seed/provision/ceiling/weight inputs, telemetry and safe URL/TLS helpers. [config.rs:Environment] [config.rs:ConfigError] [config.rs:DatabaseConfig] [config.rs:ServeConfig] [config.rs:read_auth] [config.rs:read_secrets] [config.rs:read_scripts]
- `RunError`, `SeedOutcome`, pool/migration, seed/provisioning, quota/weight and resealing functions form the operator library API. [lib.rs:RunError] [lib.rs:SeedOutcome] [lib.rs:open_pool] [lib.rs:run_migrations] [lib.rs:seed_dev] [lib.rs:seed_dev_namespace] [lib.rs:provision_tenant] [lib.rs:grant_quota_ceiling] [lib.rs:set_scheduling_weight] [lib.rs:reseal_secrets]
- `serve` / `serve_with_admin`, admin routers, metrics upkeep, shutdown helpers, signing-key generation and plain/TLS probes compose the process. Cleanup exposes `CleanupConfig` and `run`. [lib.rs:serve_with_admin] [lib.rs:admin_router_with] [lib.rs:spawn_metrics_upkeep] [lib.rs:shutdown_signal] [lib.rs:new_script_key] [lib.rs:healthcheck_tls] [cleanup.rs:CleanupConfig] [cleanup.rs:run]

## Invariants

- Environment defaults to production; non-development database URLs require TLS settings. Startup refuses development migration/JWKS allowances outside development and verifies the runtime role cannot rewrite the audit trail. [config.rs:Environment] [config.rs:requires_tls] [lib.rs:serve_with_admin]
- Secret-bearing plaintext API deployment outside development requires its explicit termination allowance. Script/secret/connector capabilities are installed only when their required backends/keys are configured; blob storage rejects the in-memory backend for separate API/engine processes. [config.rs:ServeConfig] [config.rs:read_blobs] [lib.rs:serve_with_admin]
- Development seeding checks development mode before opening storage. Cleanup bounds its batch, requires OpenBao, uses a command deadline and reports deferred work as failure. [lib.rs:seed_dev] [lib.rs:seed_dev_namespace] [cleanup.rs:run]
- Native TLS healthcheck bounds CA input, validates server name, pins the local socket and preserves hostname/certificate verification with proxies and redirects disabled. [lib.rs:HealthcheckTlsConfig] [lib.rs:healthcheck_tls]

## Side Effects

- Opens PostgreSQL pools and applies migrations, provisioning, quota/weight and secret-management operations; serving assembles the sagat-api-http router and optional Studio files. [lib.rs:run_migrations] [lib.rs:provision_tenant] [lib.rs:serve_with_admin] [cleanup.rs:run]
- Binds public HTTP/TLS and optional admin listeners; may fetch JWKS, use OpenBao/blob storage, read TLS/config files and generate signing keys. Admin exposure is separately checked and warned on non-loopback binding. [lib.rs:serve_with_admin] [lib.rs:new_script_key] [config.rs:admin_addr_is_exposed]
- Initializes and flushes telemetry, starts pool/metrics upkeep and handles process shutdown signals. [main.rs:run] [config.rs:telemetry] [config.rs:TELEMETRY_FLUSH] [lib.rs:shutdown_signal]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-control-plane --locked
```

This is the manifest-derived package check; it was not run during this documentation pass.
