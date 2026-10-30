# crates/infra-egress

## Overview

sagat-infra-egress provides policy-enforced outbound HTTP with guarded DNS and redirects.

## Interface

- Guard, NameResolver, EgressLimits, request/response types and EgressClient form the outbound boundary. [src/lib.rs:Guard] [src/lib.rs:NameResolver] [src/lib.rs:EgressLimits] [src/lib.rs:EgressClient]
- [src](./src/README.md) — Policy-enforced outbound HTTP with guarded DNS, controlled redirects and bounded responses. [src/lib.rs:Guard]

## Invariants

- Policy precedes DNS, every answer is checked, proxies/automatic redirects are disabled and each permitted manual redirect is revalidated. [src/lib.rs:EgressClient::check] [src/lib.rs:EgressClient::send_with]

## Side Effects

- The adapter performs DNS and HTTP/TLS I/O and may transmit a request over permitted redirects. [src/lib.rs:SystemResolver::lookup] [src/lib.rs:EgressClient::send_with]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-egress --locked
```

These commands were not run in this documentation-only pass.
