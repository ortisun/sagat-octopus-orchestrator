# protocol contract

## Overview

Internal Kafka protocol keys, versions, errors, frames, primitive encodings, records, and codec traits.

Source directory: [`vendor/rskafka/src/protocol`](../../../../rskafka/src/protocol). This contract is outside the verified upstream snapshot.

## Interface

- ApiKey retains known/unknown numeric keys; ApiVersion/ApiVersionRange model supported versions, and protocol Error preserves known/unknown Kafka error codes. [vendor/rskafka/src/protocol/api_key.rs:ApiKey] [vendor/rskafka/src/protocol/api_version.rs:ApiVersion] [vendor/rskafka/src/protocol/api_version.rs:ApiVersionRange] [vendor/rskafka/src/protocol/error.rs:Error]
- AsyncMessageRead/AsyncMessageWrite provide typed asynchronous frame I/O. Primitive wrappers cover fixed/variable integers, strings, bytes, tags, arrays, and record batches. [vendor/rskafka/src/protocol/frame.rs:AsyncMessageRead] [vendor/rskafka/src/protocol/frame.rs:AsyncMessageWrite] [vendor/rskafka/src/protocol/primitives.rs:Boolean] [vendor/rskafka/src/protocol/primitives.rs:UnsignedVarint] [vendor/rskafka/src/protocol/primitives.rs:TaggedFields] [vendor/rskafka/src/protocol/primitives.rs:CompactArray] [vendor/rskafka/src/protocol/primitives.rs:Records]
- RecordHeader/Record/ControlBatchRecord/RecordBatch/RecordBatchBody model record wire formats. ReadType/WriteType and their error types define codecs; VecBuilder supplies bounded-block collection assembly. These are internal or fuzz-feature APIs. [vendor/rskafka/src/protocol/record.rs:RecordHeader] [vendor/rskafka/src/protocol/record.rs:Record] [vendor/rskafka/src/protocol/record.rs:ControlBatchRecord] [vendor/rskafka/src/protocol/record.rs:RecordBatch] [vendor/rskafka/src/protocol/record.rs:RecordBatchBody] [vendor/rskafka/src/protocol/traits.rs:ReadError] [vendor/rskafka/src/protocol/traits.rs:WriteError] [vendor/rskafka/src/protocol/traits.rs:ReadType] [vendor/rskafka/src/protocol/traits.rs:WriteType] [vendor/rskafka/src/protocol/vec_builder.rs:VecBuilder]
- [messages](messages/README.md) — Internal Kafka request/response wire models and version-aware codecs, exposed externally only by unstable-fuzzing.

## Invariants

- These nested public declarations are crate-private in default builds and externally visible only under unstable-fuzzing. [vendor/rskafka/src/lib.rs:unstable-fuzzing]
- ApiVersionRange requires min <= max. Kafka error zero maps to None, while unknown key/error codes survive conversion. [vendor/rskafka/src/protocol/api_version.rs:ApiVersionRange::new] [vendor/rskafka/src/protocol/error.rs:Error::new] [vendor/rskafka/src/protocol/api_key.rs:ApiKey]
- Frames reject negative lengths, drain oversized frames before MessageTooLarge to preserve alignment, and refuse writes beyond Int32 length. [vendor/rskafka/src/protocol/frame.rs:AsyncMessageRead::read_message] [vendor/rskafka/src/protocol/frame.rs:AsyncMessageWrite::write_message]
- Fixed integers use network byte order, zero Boolean is false and nonzero true, and signed varints use zig-zag. Nullable encodings reserve -1, compact nullable encodings reserve zero, and non-null compact lengths add one. [vendor/rskafka/src/protocol/primitives.rs:Int16] [vendor/rskafka/src/protocol/primitives.rs:Boolean] [vendor/rskafka/src/protocol/primitives.rs:Varint] [vendor/rskafka/src/protocol/primitives.rs:NullableString] [vendor/rskafka/src/protocol/primitives.rs:CompactNullableString] [vendor/rskafka/src/protocol/primitives.rs:CompactArray]
- Records accepts complete batches and stops without failure when the final batch ends with unexpected EOF. [vendor/rskafka/src/protocol/primitives.rs:Records::read]
- Record decoding consumes exactly its declared length; absent keys/values use -1. Batches require magic 2, matching CRC-32C, and no trailing body bytes. [vendor/rskafka/src/protocol/record.rs:Record] [vendor/rskafka/src/protocol/record.rs:RecordBatch]
- Control batches contain one record; control version is zero and type is abort(0) or commit(1). Compression identifiers are none/gzip/snappy/lz4/zstd; disabled codecs return an unimplemented-compression error. [vendor/rskafka/src/protocol/record.rs:RecordBatchBody::read_records] [vendor/rskafka/src/protocol/record.rs:ControlBatchRecord] [vendor/rskafka/src/protocol/record.rs:RecordBatchCompression]
- VecBuilder panics when its block cannot hold a nonzero-sized element or pushes exceed the declared count, and otherwise grows in bounded blocks. [vendor/rskafka/src/protocol/vec_builder.rs:VecBuilder::new_with_block_size] [vendor/rskafka/src/protocol/vec_builder.rs:VecBuilder::push]

## Side Effects

- Frame traits read/write supplied Tokio streams and consume rejected oversized payloads. Primitive codecs consume/append bytes through supplied Read/Write objects and allocate decoded values. [vendor/rskafka/src/protocol/frame.rs:AsyncMessageRead::read_message] [vendor/rskafka/src/protocol/frame.rs:AsyncMessageWrite::write_message] [vendor/rskafka/src/protocol/primitives.rs:Records] [vendor/rskafka/src/protocol/primitives.rs:TaggedFields]
- Record codecs read/write supplied byte streams, allocate buffers, and decompress enabled formats. Snappy output bounds grow incrementally instead of immediately trusting advertised expansion length. [vendor/rskafka/src/protocol/record.rs:RecordBatchBody] [vendor/rskafka/src/protocol/record.rs:RecordBatchBodyRef] [vendor/rskafka/src/protocol/record.rs:carefully_decompress_snappy]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
