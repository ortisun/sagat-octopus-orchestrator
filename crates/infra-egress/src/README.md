# crates/infra-egress/src

## Overview

Policy-enforced outbound HTTP with guarded DNS, controlled redirects and bounded responses.

## Interface

- `Guard`, `NameResolver`/`SystemResolver`, `EgressLimits`, `EgressError`, request/response/body types and `EgressClient` new/with/send/send_with expose policy, resolution and transport boundaries. [lib.rs:Guard] [lib.rs:NameResolver] [lib.rs:SystemResolver] [lib.rs:EgressLimits] [lib.rs:EgressError] [lib.rs:EgressRequest] [lib.rs:ResponseBody] [lib.rs:EgressResponse] [lib.rs:EgressClient]

## Invariants

- Destination policy precedes DNS; every resolved address must pass and the accepted addresses are returned to the client. Userinfo/fragments are rejected. Automatic redirects/proxies are disabled; only GET/HEAD may manually redirect and each hop is rechecked. [lib.rs:EgressClient::check] [lib.rs:GuardedResolver::resolve] [lib.rs:EgressClient::with] [lib.rs:EgressClient::send_with]
- Connect/total timeouts and incremental response-size limits apply; Skip avoids reading the body. Resolver denials remain typed Denied errors. [lib.rs:EgressClient::send_with] [lib.rs:Refused] [lib.rs:classify]

## Side Effects

- OS DNS and HTTP/TLS connections transmit caller headers/body, potentially repeated over permitted redirects; headers and optional bounded bodies are retained. The adapter can install Rustls ring and logs transport classes. [lib.rs:SystemResolver::lookup] [lib.rs:EgressClient::send_with] [lib.rs:install_crypto_provider] [lib.rs:classify]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-egress --locked
```

These commands were not run in this documentation-only pass.
