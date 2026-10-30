# capabilities

## Overview

Attempt-scoped gRPC blob transfers and verified asynchronous readers. [blobs.rs:BlobReader]

## Interface

- Internal Capabilities::write_blob/read_blob implement blob RPCs; public BlobReader implements Stream and exposes is_verified. [blobs.rs:write_blob] [blobs.rs:read_blob] [blobs.rs:BlobReader] [blobs.rs:is_verified]

## Invariants

- Blob operations require negotiated minor >=3 and active attempt authorization. Uploads enforce header/chunk sequencing and matching size/digest; reads verify headers, 256 KiB chunk bounds, total length and digest before verified EOF. Unsent lease tokens are zeroized on drop. [blobs.rs:write_blob] [blobs.rs:UploadStream] [blobs.rs:poll_next] [blobs.rs:drop]

## Side Effects

- Performs gRPC WriteBlob/ReadBlob and consumes or yields asynchronous byte streams. [blobs.rs:write_blob] [blobs.rs:read_blob]

## Verification

Run from the repository root:

```sh
cd sdks/rust && cargo test --locked
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
