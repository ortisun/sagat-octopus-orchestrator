# crates/infra-blobstore

## Overview

sagat-infra-blobstore implements memory, filesystem, S3-compatible and R2 blob storage.

## Interface

- ObjectStoreBackend, BlobStoreConfig, BlobSettings and build_backend expose bounded put/get/delete configuration and behavior. [src/backend.rs:ObjectStoreBackend] [src/config.rs:BlobStoreConfig] [src/config.rs:BlobSettings] [src/backend.rs:build_backend]
- [src](./src/README.md) — BlobBackend implementations for memory, local filesystem, S3-compatible storage and Cloudflare R2, with environment-derived settings and bounded multipart streaming. [src/backend.rs:ObjectStoreBackend]

## Invariants

- Size, part, concurrency, retry and timeout bounds are explicit; plaintext endpoints need development opt-in. Memory storage is volatile and filesystem durability requires persistent deployment storage. [src/config.rs:PartSize::MIN] [src/config.rs:MAX_BLOB_BYTES_CEILING] [src/config.rs:check_endpoint] [src/backend.rs:build_backend]

## Side Effects

- Backends mutate memory/filesystem or perform S3/R2 network operations; multipart failures attempt abort. [src/backend.rs:ObjectStoreBackend::put_stream] [src/backend.rs:delete]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-blobstore --locked
```

These commands were not run in this documentation-only pass.
