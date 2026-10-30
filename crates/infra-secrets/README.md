# crates/infra-secrets

## Overview

sagat-infra-secrets implements AES-256-GCM sealing, database-credential scanning and an OpenBao KV-v2 adapter.

## Interface

- KeyRing, SecretsConfig, JsonDatabaseCredentials, OpenBaoConfig, OpenBao and OpenBaoSettings expose crypto/provider boundaries. [src/lib.rs:KeyRing] [src/lib.rs:SecretsConfig] [src/database.rs:JsonDatabaseCredentials] [src/openbao.rs:OpenBaoConfig] [src/openbao.rs:OpenBao] [src/openbao.rs:OpenBaoSettings]
- [src](./src/README.md) — AES-256-GCM secret sealing, strict database-credential decoding/scanning and an OpenBao KV-v2 provider adapter. [src/lib.rs:KeyError]

## Invariants

- Keys/nonces, credential JSON, provider URLs/tokens/bodies and OpenBao version/CAS behavior are bounded and validated. [src/lib.rs:KeyRing::parse] [src/lib.rs:KeyRing::seal] [src/openbao.rs:OpenBao::new] [src/openbao.rs:MAX_RESPONSE]

## Side Effects

- Sealing/key generation uses OS randomness; provider setup reads files and OpenBao performs bounded HTTP and KV mutations. [src/lib.rs:generate_key] [src/openbao.rs:OpenBaoSettings::provider] [src/openbao.rs:OpenBao::create_inner] [src/openbao.rs:OpenBao::retire_inner]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-secrets --locked
```

These commands were not run in this documentation-only pass.
