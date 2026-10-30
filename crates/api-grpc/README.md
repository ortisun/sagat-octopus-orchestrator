# crates/api-grpc

## Overview

sagat-api-grpc adapts the shared worker protocol through Tonic and compiles its Protobuf contract at build time.

## Interface

- The generated pb server/client family and WorkerGrpc expose worker RPC handling through application services, with optional blob transport; TlsConfig, GrpcConfig, AdmissionProvider, worker_service and serve compose the server. [src/lib.rs:pb] [src/service.rs:WorkerGrpc] [src/service.rs:with_blobs] [src/server.rs:TlsConfig] [src/server.rs:GrpcConfig] [src/server.rs:AdmissionProvider] [src/server.rs:worker_service] [src/server.rs:serve]
- [src](./src/README.md) — Tonic worker-protocol adapter generated from the shared worker.proto contract, with authentication, request/status mapping, process/connection/stream admission, optional blob transport and application WorkerService delegation. [src/lib.rs:pb]

## Invariants

- The build suppresses generated Debug for resolved secrets/databases/values and uses byte buffers for blob chunks. Generated RPC availability does not establish runtime capability support, which remains application/admission-controlled. [build.rs:main] [src/server.rs:AdmissionProvider] [src/service.rs:WorkerGrpc]

## Side Effects

- The build script compiles the shared worker contract and emits Cargo rebuild directives. At runtime serve opens the gRPC boundary and BlobTransfer connects configured blob operations. [build.rs:CONTRACTS] [build.rs:main] [src/server.rs:serve] [src/blobs.rs:BlobTransfer]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-grpc --locked
```

These commands were not run in this documentation-only pass.
