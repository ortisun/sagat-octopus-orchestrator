# crates/infra-blobstore/src

## Overview

BlobBackend implementations for memory, local filesystem, S3-compatible storage and Cloudflare R2, with environment-derived settings and bounded multipart streaming.

## Interface

- `ObjectStoreBackend`/`build_backend` implement provider/put/get/delete. Configuration exports redacted `Secret`, `Credentials`, `S3Config`, `R2Config`/`Jurisdiction`, `BlobStoreConfig`, checked `PartSize`, `BlobSettings::from_lookup` and typed `ConfigError`. [backend.rs:ObjectStoreBackend] [backend.rs:build_backend] [config.rs:Secret] [config.rs:Credentials] [config.rs:S3Config] [config.rs:R2Config] [config.rs:Jurisdiction] [config.rs:BlobStoreConfig] [config.rs:PartSize] [config.rs:BlobSettings] [config.rs:ConfigError]

## Invariants

- Parts are 5–512 MiB (8 MiB default), four in flight; connect/request timeouts are 10/120 seconds with bounded retries. Blob size is at most 5 GiB, pending count 1–100000 and TTLs one second–365 days. [config.rs:PartSize::MIN] [config.rs:PartSize::MAX] [backend.rs:PARTS_IN_FLIGHT] [backend.rs:CONNECT_TIMEOUT] [backend.rs:REQUEST_TIMEOUT] [backend.rs:retries] [config.rs:MAX_BLOB_BYTES_CEILING] [config.rs:BlobSettings::from_lookup]
- Endpoints/prefixes reject unsafe forms; plaintext requires explicit development allow_http. R2 constrains account/jurisdiction. Streaming coalesces fixed parts, bounds retained memory and attempts multipart abort after failure. Memory is process-private/volatile; filesystem persistence requires an appropriate volume. [config.rs:check_endpoint] [config.rs:clean_prefix] [config.rs:R2Config::endpoint] [backend.rs:next_part] [backend.rs:ObjectStoreBackend::put_stream] [config.rs:BlobStoreConfig]

## Side Effects

- Memory changes local bytes; filesystem setup/operations use disk; S3/R2 use network and S3 may use ambient AWS credentials. Put creates/replaces/multipart-uploads objects, get streams bytes and delete tolerates missing objects. [backend.rs:build_backend] [backend.rs:s3_store] [backend.rs:r2_store] [backend.rs:ObjectStoreBackend::put_stream] [backend.rs:get] [backend.rs:delete]
- Setup may install Rustls ring; logs contain provider/category failures without credentials or object contents. [backend.rs:build_backend] [backend.rs:ObjectStoreBackend::failure]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-blobstore --locked
```

These commands were not run in this documentation-only pass.
