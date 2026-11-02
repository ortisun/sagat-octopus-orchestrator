# crates/api-grpc/src

## Overview

Tonic worker-protocol adapter generated from the shared worker.proto contract, with authentication, request/status mapping, process/connection/stream admission, optional blob transport and application WorkerService delegation.

## Interface

- The facade exports generated pb client/server/messages, WorkerGrpc, BlobTransfer/BLOB_CHUNK_BYTES, AdmissionProvider, GrpcConfig/TlsConfig, serve/worker_service, to_status and status metadata names. Credential-bearing generated Debug implementations are locally redacted. [lib.rs:pb] [lib.rs:WorkerGrpc] [lib.rs:ResolvedSecret] [lib.rs:ResolvedDatabase] [server.rs:AdmissionProvider] [server.rs:GrpcConfig] [server.rs:TlsConfig] [status.rs:to_status]
- The build script compiles shared worker.proto with protox/tonic, generates client and server, uses bytes for blob chunks and suppresses generated credential Debug. Protobuf service/messages are imported generated contracts. [../build.rs:CONTRACTS] [../build.rs:build_server] [../build.rs:build_client] [../build.rs:skip_debug]
- WorkerService RPCs are RegisterWorker, PollTasks, StreamTasks, Heartbeat, CompleteStep, FailStep, ResolveSecrets, ResolveDatabase, WriteBlob and ReadBlob; StreamTasks/ReadBlob are server streams and WriteBlob is a client stream. WorkerGrpc implements each handler through the application worker service, with optional with_blobs transport. [../../../specs/001-platform-foundation/contracts/worker.proto:WorkerService] [service.rs:WorkerGrpc] [service.rs:with_blobs]
- server.rs exposes worker_service for generated-server wrapping/message limits and serve for startup. BlobTransfer/BLOB_CHUNK_BYTES are public; BlobAccess, upload parsing/body, Download and error translation are adapter-private. [server.rs:worker_service] [server.rs:serve] [blobs.rs:BlobTransfer] [blobs.rs:BLOB_CHUNK_BYTES] [blobs.rs:BlobAccess] [blobs.rs:Download]
- Local Attempt/idempotency_key/worker_id validate requests; database response/value map typed application/domain values into protobuf. to_status, ERROR_METADATA and RETRY_AFTER_METADATA expose stable error mapping; invalid constructs INVALID_ARGUMENT metadata. [request.rs:Attempt] [request.rs:idempotency_key] [request.rs:worker_id] [database.rs:response] [database.rs:value] [status.rs:to_status] [status.rs:ERROR_METADATA] [status.rs:RETRY_AFTER_METADATA] [status.rs:invalid]

## Invariants

- Every RPC authenticates one Bearer authorization value through shared HTTP Core and holds the tenant permit in Authenticated; TLS/mTLS does not replace bearer identity. [service.rs:authenticate] [service.rs:bearer] [server.rs:TlsConfig]
- Generated methods do not imply enabled capabilities: application registration negotiates protocol, scoped identities check scoped support, and blob RPCs/output_blob_ref require with_blobs. [service.rs:register_worker] [service.rs:attach_instance] [service.rs:with_blobs] [service.rs:blob_access]
- Attempt parsing requires task-id run/node/generation agreement and positive fences except legacy heartbeat attempt omission; optional step IDs are UUIDs, lease tokens at most 128 bytes and idempotency keys 8–160 printable ASCII. [request.rs:Attempt::parse_with] [request.rs:MAX_TOKEN] [request.rs:idempotency_key]
- GrpcConfig defaults include 30-second unary/600-second transfer timeouts, 64 calls/128 HTTP2 streams per connection, 2,048 connections/256 per peer, 10-second first-byte/handshake, 300-second idle, 1,800-second age, 1,024 process calls and 30/10-second keepalive. [server.rs:GrpcConfig::default]
- worker_service caps decoded messages at MAX_PAYLOAD_BYTES plus 64 KiB. Transport uses the longer deadline while Admission enforces shorter unary timeouts; discovery sheds above three-quarters process capacity, lease/transfer calls at full capacity. [server.rs:MAX_MESSAGE_BYTES] [server.rs:worker_service] [server.rs:serve] [limits.rs:Admission::new] [limits.rs:Class::of]
- Incoming stops at connection capacity, refuses excess peer shares, groups non-mapped IPv6 by /64 and closes silent/idle sockets. Finite task streams can arm a watchdog for stalled HTTP2 body polling. [limits.rs:Incoming] [limits.rs:peer_key] [limits.rs:Capped] [limits.rs:CappedConnectInfo::arm]
- StreamTasks accepts 1–64 tasks within a five-second receive window and byte budget, reauthenticates before each claim and admits 32 process/8 tenant/2 subject streams. Credit/byte/window/token limits end streams; stalled delivery has a connection deadline. [service.rs:STREAM_WINDOW] [service.rs:stream_tasks] [limits.rs:TaskStreams::admit] [service.rs:TaskBody]
- Blob chunks are at most 256 KiB; default transfer capacity is 32 for 600 seconds. Upload requires header first then chunks; download sends header then verified chunks, returns DATA_LOSS for corruption and retains transfer/auth guards through stream lifetime. [blobs.rs:BLOB_CHUNK_BYTES] [blobs.rs:BlobTransfer::default] [blobs.rs:upload_body] [blobs.rs:Download::new] [blobs.rs:data_loss]
- Managed database result uploads are fully collected under the domain maximum before scanning/storage, use application/json and zeroize temporary buffers. Resolved database Debug is redacted. [blobs.rs:collect_database_result] [service.rs:write_blob] [lib.rs:ResolvedDatabase]
- HTTP status mapping is 401 UNAUTHENTICATED, 403 PERMISSION_DENIED, 404 NOT_FOUND, 409 ABORTED, malformed/validation statuses INVALID_ARGUMENT, 408/502–504 UNAVAILABLE and 429 RESOURCE_EXHAUSTED; metadata carries machine code/retry. Task-window expiry may return DEADLINE_EXCEEDED and corruption DATA_LOSS. [status.rs:to_status] [service.rs:TaskBody] [blobs.rs:data_loss]
- The imported protocol is at least once: lost responses retry the same idempotency key and replay can return already_applied. This adapter does not promise exactly-once external execution. [../../../specs/001-platform-foundation/contracts/worker.proto:already_applied] [service.rs:step_response]

## Side Effects

- RPC handlers register workers, claim tasks, heartbeat, complete/fail attempts and resolve secret/database capabilities through application WorkerService. [service.rs:register_worker] [service.rs:poll_tasks] [service.rs:heartbeat] [service.rs:complete_step] [service.rs:fail_step] [service.rs:resolve_secrets] [service.rs:resolve_database]
- Blob RPCs perform bounded object-storage I/O through BlobService; reads and capability resolutions invoke audits, and downloads spawn a producer task. [service.rs:write_blob] [service.rs:read_blob] [service.rs:resolve_secrets] [blobs.rs:Download::new]
- RPC/admission/stream/connection paths emit traces, closed-label metrics and bounded denial audits. serve accepts HTTP2 sockets, optionally handshakes TLS/mTLS and drains under configured deadlines. [service.rs:observe] [limits.rs:Admission] [limits.rs:TaskStreamTicket] [server.rs:serve] [limits.rs:Incoming]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-api-grpc --locked
```

These commands were not run in this documentation-only pass.
