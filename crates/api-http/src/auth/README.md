# crates/api-http/src/auth

## Overview

Public OIDC/JWT authentication boundary: configuration, bounded JWKS retrieval/cache, token verification and deployment-controlled webhook identity profiles.

## Interface

- `AuthConfig` configures fixed issuer/audience/JWKS URI, tenant claim, lifetime/leeway/cache settings and optional required token type; implementation submodules are private with selected types re-exported. [config.rs:AuthConfig] [mod.rs:AuthConfig]
- `JwksFetcher`, `HttpJwksFetcher`, `JwksCache`, key/error types and crypto-provider setup implement bounded key retrieval/cache lookup. [jwks.rs:JwksFetcher] [jwks.rs:HttpJwksFetcher] [jwks.rs:JwksCache] [jwks.rs:key_for]
- `TokenVerifier::verify` returns platform subject/tenant/expiry claims; `verify_identity` returns only external identity subject. `AuthError` separates rejected tokens from retryable key unavailability. [verifier.rs:TokenVerifier] [verifier.rs:verify] [verifier.rs:verify_identity] [verifier.rs:AuthError]
- `WebhookOidcVerifiers` builds up to 32 deployment-controlled issuer/audience profiles and implements profile support/subject verification. [webhooks.rs:WebhookOidcVerifiers] [webhooks.rs:supports] [webhooks.rs:verify]

## Invariants

- JWKS is HTTPS-only unless development HTTP is enabled; redirects are disabled, connect/total timeouts are 2/5 seconds, documents are capped at 256 KiB and transport errors omit URLs. [jwks.rs:HttpJwksFetcher]
- Only keyed RSA/RS256 and P-256/ES256 signature-verification keys pass filtering. Cache hits avoid waiting on provider I/O; allowed stale keys trigger background single-flight refresh, while unknown/expired keys share rate-limited refresh and fail closed when unavailable. [jwks.rs:usable_key] [jwks.rs:key_for] [jwks.rs:refresh_and_wait]
- Tokens are at most 8 KiB with a nonempty kid up to 128 bytes. Only RS256/ES256 are accepted, critical JOSE extensions are rejected, and selected key/configuration control algorithm and optional token type. [verifier.rs:MAX_TOKEN_BYTES] [verifier.rs:verify_claims]
- Issuer/audience are fixed; required claims/lifetime checks constrain subject to 1–255 bytes, tenant to a nonempty string up to 63 bytes, future iat to leeway and exp-iat to maximum lifetime. [verifier.rs:check_lifetime] [verifier.rs:tenant_of]
- Webhook profile URLs require a host, no userinfo/fragment and HTTPS outside development; endpoint authors cannot select arbitrary key URLs. Token-type enforcement defaults off and there is no token revocation-list lookup. [webhooks.rs:new] [config.rs:required_token_type] [config.rs:max_token_lifetime]

## Side Effects

- `HttpJwksFetcher::fetch` performs bounded HTTP(S) GET to configured JWKS endpoints; cache refresh updates in-memory state, emits metrics/safe logs and may spawn Tokio refresh tasks. [jwks.rs:fetch] [jwks.rs:refresh] [jwks.rs:refresh_in_background]
- Token verification performs local JOSE signature/claim work and may trigger cache-driven JWKS retrieval. Each webhook profile owns its verifier/cache; successful identity verification does not create a database session. [verifier.rs:verify] [webhooks.rs:new]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-http --locked
```

These commands were not run in this documentation-only pass.
