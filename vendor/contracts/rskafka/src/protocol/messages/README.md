# messages contract

## Overview

Internal Kafka request/response wire models and version-aware codecs, exposed externally only by unstable-fuzzing.

Source directory: [`vendor/rskafka/src/protocol/messages`](../../../../../rskafka/src/protocol/messages). This contract is outside the verified upstream snapshot.

## Interface

- `ReadVersionedType`, `WriteVersionedType`, and `RequestBody` plus their typed errors define the codec boundary. [vendor/rskafka/src/protocol/messages/mod.rs:ReadVersionedType] [vendor/rskafka/src/protocol/messages/mod.rs:WriteVersionedType] [vendor/rskafka/src/protocol/messages/mod.rs:RequestBody]
- Wire models cover API-version discovery, topic creation/deletion, record deletion, fetch, offset lookup, metadata, produce, and SASL handshake/authentication. [vendor/rskafka/src/protocol/messages/api_versions.rs:ApiVersionsRequest] [vendor/rskafka/src/protocol/messages/create_topics.rs:CreateTopicsRequest] [vendor/rskafka/src/protocol/messages/delete_records.rs:DeleteRecordsRequest] [vendor/rskafka/src/protocol/messages/delete_topics.rs:DeleteTopicsRequest] [vendor/rskafka/src/protocol/messages/fetch.rs:FetchRequest] [vendor/rskafka/src/protocol/messages/list_offsets.rs:ListOffsetsRequest] [vendor/rskafka/src/protocol/messages/metadata.rs:MetadataRequest] [vendor/rskafka/src/protocol/messages/produce.rs:ProduceRequest] [vendor/rskafka/src/protocol/messages/sasl_msg.rs:SaslAuthenticateRequest]
- RequestHeader/ResponseHeader encode correlation/version/client/tag metadata; IsolationLevel and NORMAL_CONSUMER supply shared protocol values. [vendor/rskafka/src/protocol/messages/header.rs:RequestHeader] [vendor/rskafka/src/protocol/messages/header.rs:ResponseHeader] [vendor/rskafka/src/protocol/messages/constants.rs:IsolationLevel] [vendor/rskafka/src/protocol/messages/constants.rs:NORMAL_CONSUMER]

## Invariants

- The protocol module is private in normal builds and exposed only under the explicitly unstable fuzzing feature. [vendor/rskafka/src/lib.rs:unstable-fuzzing]
- RequestBody binds each request to an API key, response type, version interval, and tagged-field transition. Supported ranges include Fetch 4, Metadata 0–4, Produce 3–7, SaslHandshake 1, and SaslAuthenticate 0–2. [vendor/rskafka/src/protocol/messages/mod.rs:RequestBody] [vendor/rskafka/src/protocol/messages/fetch.rs:FetchRequest] [vendor/rskafka/src/protocol/messages/metadata.rs:MetadataRequest] [vendor/rskafka/src/protocol/messages/produce.rs:ProduceRequest] [vendor/rskafka/src/protocol/messages/sasl_msg.rs:SaslHandshakeRequest] [vendor/rskafka/src/protocol/messages/sasl_msg.rs:SaslAuthenticateRequest]
- Version-conditioned fields use protocol defaults or reject unsupported values with FieldNotAvailable. Flexible versions use compact encodings and empty tagged fields when absent. Array readers reject lengths below -1 and use incremental allocation. [vendor/rskafka/src/protocol/messages/create_topics.rs:CreateTopicsRequest] [vendor/rskafka/src/protocol/messages/fetch.rs:FetchRequest] [vendor/rskafka/src/protocol/messages/list_offsets.rs:ListOffsetsRequestPartition] [vendor/rskafka/src/protocol/messages/metadata.rs:MetadataRequest] [vendor/rskafka/src/protocol/messages/mod.rs:write_compact_versioned_array] [vendor/rskafka/src/protocol/messages/mod.rs:read_versioned_array]
- The SASL response WriteVersionedType implementations currently return success without writing bytes; their writer path is not a completed response serializer. [vendor/rskafka/src/protocol/messages/sasl_msg.rs:SaslHandshakeResponse] [vendor/rskafka/src/protocol/messages/sasl_msg.rs:SaslAuthenticateResponse]

## Side Effects

- Codecs consume/append bytes through caller-provided Read/Write values; these wire models do not open brokers or execute the named operations. [vendor/rskafka/src/protocol/messages/mod.rs:ReadVersionedType] [vendor/rskafka/src/protocol/messages/mod.rs:WriteVersionedType]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
