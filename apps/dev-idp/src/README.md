# dev-idp source

## Overview

Development-only ES256 identity issuer with discovery/JWKS/token routes and optional persistent signing keys.

## Interface

- `Config`, `ConfigError`, `Lookup` and `MAX_TTL_SECS` expose environment parsing and allowed Host authorities; `DevKey` / `KeyError` expose key generation/loading, JWK and signing. [config.rs:Config] [config.rs:ConfigError] [config.rs:Lookup] [config.rs:MAX_TTL_SECS] [key.rs:DevKey] [key.rs:KeyError]
- `router` defines `/livez`, `/.well-known/jwks.json`, `/.well-known/openid-configuration` and `/token`; binary commands provide serve/healthcheck/help/version and signal handling. [server.rs:router] [main.rs:main]

## Invariants

- Startup requires `SAGAT_ENV=dev`; non-loopback binding needs its explicit exposure flag. Issuer URLs reject credentials and whitespace. [config.rs:Config]
- Identity routes require one known Host authority; token input is bounded JSON with no unknown fields, domain-validated subject/tenant and TTL of 1–3600 seconds. [server.rs:host_known] [server.rs:TokenRequest] [server.rs:MAX_BODY] [config.rs:MAX_TTL_SECS]
- Persistent keys reject symlinks/insecure permissions and unsafe parents; creation uses a private temporary file, sync and hard-link installation with concurrent-winner handling. [key.rs:DevKey::load_or_create] [key.rs:DevKey::create] [key.rs:directory_is_private]

## Side Effects

- Binds HTTP, issues ES256 tokens, serves public metadata and performs local health probes. Logs subject/tenant/TTL and listener/key metadata without logging issued tokens. [server.rs:token] [server.rs:router] [main.rs:run] [main.rs:healthcheck]
- Generates randomness and optionally reads/creates/syncs/links/removes signing-key files. [key.rs:DevKey::generate] [key.rs:DevKey::load_or_create] [key.rs:DevKey::create]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-dev-idp --locked
```

This is the manifest-derived package check; it was not run during this documentation pass.
