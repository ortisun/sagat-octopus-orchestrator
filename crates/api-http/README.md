# crates/api-http

## Overview

sagat-api-http provides Axum HTTP, authentication, error envelopes, native serving and optional Studio hosting.

## Interface

- AuthConfig, JwksCache and TokenVerifier expose authentication; AppState, ExecState, GovState and router composition mount configured identity/execution/governance capabilities. [src/auth/config.rs:AuthConfig] [src/auth/jwks.rs:JwksCache] [src/auth/verifier.rs:TokenVerifier] [src/lib.rs:AppState] [src/exec.rs:ExecState] [src/governance.rs:GovState] [src/lib.rs:router_with_governance_and_approvals]
- [src](./src/README.md) — Axum HTTP adapter exposing authentication, state/configuration, router composition, typed errors, server limits, SSE/blob/Studio support and deployment-gated control-plane capabilities. Business operations delegate to application services. [src/lib.rs:AppState]

## Invariants

- Authenticated handlers derive tenant/authority from Caller; webhook and health boundaries are separate. Tokens/JWKS and request families have explicit size, algorithm, timeout and lifetime limits. Optional route mounting follows configured services. [src/routes/connectors.rs:admitted_connector] [src/routes/events.rs:receive_webhook] [src/routes/health.rs:health] [src/auth/verifier.rs:MAX_TOKEN_BYTES] [src/lib.rs:governance_routes]

## Side Effects

- Handlers invoke application/storage ports, emit HTTP responses/traces/metrics and may spawn bounded SSE polling; JWKS verification may perform bounded HTTP retrieval and native servers accept sockets. [src/routes/execution.rs:start_run] [src/routes/execution.rs:run_stream] [src/auth/jwks.rs:HttpJwksFetcher::fetch] [src/server.rs:serve]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-http --locked
```

These commands were not run in this documentation-only pass.
