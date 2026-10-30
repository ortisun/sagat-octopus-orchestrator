# src contract

## Overview

Crate root exposing the client and record/topic types, with private retry, connection, validation, throttling, and messenger machinery.

Source directory: [`vendor/rskafka/src`](../../../rskafka/src). This contract is outside the verified upstream snapshot.

## Interface

- Stable root exports include BackoffConfig, BackoffError, ConnectionError, SaslError, chrono, and public build_info/client/record/topic modules. Messenger/protocol become public only under unstable-fuzzing. [vendor/rskafka/src/lib.rs:BackoffConfig] [vendor/rskafka/src/lib.rs:ConnectionError] [vendor/rskafka/src/lib.rs:SaslError] [vendor/rskafka/src/lib.rs:chrono] [vendor/rskafka/src/lib.rs:unstable-fuzzing]
- BackoffConfig configures initial/max delay, base, and optional deadline; BackoffError::DeadlineExceded retains the deadline/source error. DEFAULT_CLIENT_ID uses the Cargo package name. [vendor/rskafka/src/backoff.rs:BackoffConfig] [vendor/rskafka/src/backoff.rs:BackoffError] [vendor/rskafka/src/build_info.rs:DEFAULT_CLIENT_ID]
- Record carries optional key/value bytes, ordered headers, and UTC timestamp; approximate_size totals their payload bytes. RecordAndOffset adds the Kafka offset, and Topic carries its name and ordered partition IDs. [vendor/rskafka/src/record.rs:Record] [vendor/rskafka/src/record.rs:Record::approximate_size] [vendor/rskafka/src/record.rs:RecordAndOffset] [vendor/rskafka/src/topic.rs:Topic]
- BrokerConnector/BrokerCache/MetadataLookupMode, Backoff, maybe_throttle, ExactlyOne, and Messenger implement private connection/retry/codec coordination. override_version_ranges is fuzz-feature-only. [vendor/rskafka/src/connection.rs:BrokerConnector] [vendor/rskafka/src/connection.rs:BrokerCache] [vendor/rskafka/src/connection.rs:MetadataLookupMode] [vendor/rskafka/src/backoff.rs:Backoff] [vendor/rskafka/src/throttle.rs:maybe_throttle] [vendor/rskafka/src/validation.rs:ExactlyOne] [vendor/rskafka/src/messenger.rs:Messenger] [vendor/rskafka/src/messenger.rs:Messenger::override_version_ranges]
- [client](client/README.md) — Public Kafka client API for metadata, topic administration, partition operations, stream consumption, and batch production.
- [connection](connection/README.md) — Private broker topology and socket transport implementation supporting the public client.
- [protocol](protocol/README.md) — Internal Kafka protocol keys, versions, errors, frames, primitive encodings, records, and codec traits.

## Invariants

- Backoff defaults are 100 ms initial, 500 s maximum, base 3, and no deadline. Jittered intervals are capped and optional deadline checks use accumulated delay. [vendor/rskafka/src/backoff.rs:BackoffConfig] [vendor/rskafka/src/backoff.rs:Backoff::next]
- Only unfiltered all-topic metadata is cached, while every successful response updates topology; arbitrary-broker invalidation is generation-fenced. [vendor/rskafka/src/connection.rs:BrokerConnector::request_metadata] [vendor/rskafka/src/connection.rs:BrokerCache]
- Messenger keys active requests by correlation ID, selects the highest overlapping client/broker API version, rejects trailing response bytes, and poisons/drains active requests after frame-read/send failure. [vendor/rskafka/src/messenger.rs:Messenger::request_with_version_ranges] [vendor/rskafka/src/messenger.rs:match_versions] [vendor/rskafka/src/messenger.rs:MessengerState::poison] [vendor/rskafka/src/messenger.rs:Messenger::send_message]
- Cancellation removes unsent request state; an in-progress frame write is rescued in a spawned task so cancellation does not leave a partial frame. ExactlyOne accepts precisely one vector element and otherwise returns the observed count. [vendor/rskafka/src/messenger.rs:CleanupRequestStateOnCancel] [vendor/rskafka/src/messenger.rs:CancellationSafeFuture] [vendor/rskafka/src/validation.rs:ExactlyOne]

## Side Effects

- Retry handling emits traces and sleeps for backoff/throttling. Broker connection opens transport, negotiates API versions, and optionally authenticates SASL; metadata work updates caches/topology. [vendor/rskafka/src/backoff.rs:Backoff::retry_with_backoff] [vendor/rskafka/src/connection.rs:BrokerRepresentation] [vendor/rskafka/src/connection.rs:BrokerConnector::request_metadata]
- Messenger spawns a response-reader task, serializes/writes/flushes requests, awaits correlated responses, synchronizes supported versions, and exchanges SASL authentication messages. Dropping it aborts the reader. [vendor/rskafka/src/messenger.rs:Messenger::new] [vendor/rskafka/src/messenger.rs:Messenger::send_message_inner] [vendor/rskafka/src/messenger.rs:Messenger::sync_versions] [vendor/rskafka/src/messenger.rs:Messenger::do_sasl] [vendor/rskafka/src/messenger.rs:Messenger]
- Negative throttle values emit a warning; the helper returns positive throttle duration without sleeping itself. [vendor/rskafka/src/throttle.rs:maybe_throttle]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
