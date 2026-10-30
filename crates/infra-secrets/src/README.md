# crates/infra-secrets/src

## Overview

AES-256-GCM secret sealing, strict database-credential decoding/scanning and an OpenBao KV-v2 provider adapter.

## Interface

- `KeyError`, re-exported `CipherError`, `KeyRing` parse/introspection/seal/open, `SecretsConfig` and `generate_key` expose key rotation and authenticated sealing. [lib.rs:KeyError] [lib.rs:CipherError] [lib.rs:KeyRing] [lib.rs:SecretsConfig] [lib.rs:generate_key]
- `JsonDatabaseCredentials` implements decode/content scanning. `OpenBaoConfig`/`OpenBao` implement provider create/read/delete; `OpenBaoSettings` reads deployment flags and loads provider/intent-key configuration. [database.rs:JsonDatabaseCredentials] [openbao.rs:OpenBaoConfig] [openbao.rs:OpenBao] [openbao.rs:OpenBaoSettings]

## Invariants

- Keys are 32 decoded bytes with unique safe 1–32-character IDs; the first is active. Sealing uses a fresh 12-byte nonce and caller AAD. Credential JSON denies unknown fields and bounds usernames/passwords; scanning recognizes raw/JSON/base64 forms but cannot prevent arbitrary transformations by authorized SQL. [lib.rs:KeyRing::parse] [lib.rs:KeyRing::seal] [database.rs:CredentialJson] [database.rs:contains_credential]
- OpenBao requires HTTPS in production, restricts development access to loopback, rejects URL userinfo/query/fragment/non-root paths, requires safe mounts/absolute token paths and bounds timeout to 10 seconds. Tokens are capped at 8192 bytes; production requires non-root policy and TTL 1–86400. [openbao.rs:OpenBao::new] [openbao.rs:OpenBao::token] [openbao.rs:MAX_TOKEN]
- Bodies are capped at 64 KiB. Creates use CAS 0/version 1; reads require intact version 1 and retirement establishes version-1 metadata before destruction. [openbao.rs:MAX_RESPONSE] [openbao.rs:OpenBao::create_inner] [openbao.rs:OpenBao::read_inner] [openbao.rs:OpenBao::retire_inner]

## Side Effects

- Sealing/key generation use OS randomness and zeroizing key/secret storage. Provider setup reads token/CA/intent-key files; requests use bounded HTTP with redirects/proxies disabled and emit metrics/spans. [lib.rs:KeyRing::seal] [lib.rs:generate_key] [openbao.rs:OpenBaoSettings::provider] [openbao.rs:OpenBaoSettings::intent_key] [openbao.rs:OpenBao::request]
- Create/delete mutate OpenBao KV state; conflict confirmation reads existing content and retirement may create a tombstone before destroying version 1. [openbao.rs:OpenBao::create_inner] [openbao.rs:OpenBao::retire_inner]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-secrets --locked
```

These commands were not run in this documentation-only pass.
