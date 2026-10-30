# producer contract

## Overview

Public aggregation contracts plus internal batch flushing and one-shot broadcast coordination.

Source directory: [`vendor/rskafka/src/client/producer`](../../../../../rskafka/src/client/producer). This contract is outside the verified upstream snapshot.

## Interface

- The public aggregator module exposes Aggregator, StatusDeaggregator, AggregatorStatus, TryPush, boxed Error, RecordAggregator, and RecordAggregatorStatusDeaggregator. [vendor/rskafka/src/client/producer/aggregator.rs:Aggregator] [vendor/rskafka/src/client/producer/aggregator.rs:StatusDeaggregator] [vendor/rskafka/src/client/producer/aggregator.rs:AggregatorStatus] [vendor/rskafka/src/client/producer/aggregator.rs:TryPush] [vendor/rskafka/src/client/producer/aggregator.rs:Error] [vendor/rskafka/src/client/producer/aggregator.rs:RecordAggregator] [vendor/rskafka/src/client/producer/aggregator.rs:RecordAggregatorStatusDeaggregator]
- BatchBuilder/ResultHandle/FlushResult are internal batching machinery; BroadcastOnce/BroadcastOnceReceiver are crate-internal through their module visibility. [vendor/rskafka/src/client/producer/batch.rs:BatchBuilder] [vendor/rskafka/src/client/producer/batch.rs:ResultHandle] [vendor/rskafka/src/client/producer/batch.rs:FlushResult] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnce] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnceReceiver] [vendor/rskafka/src/client/producer.rs:broadcast]

## Invariants

- RecordAggregator returns an input unchanged if its approximate size would exceed the batch limit. Accepted tags are record indexes, and flush resets the accumulated state. [vendor/rskafka/src/client/producer/aggregator.rs:RecordAggregator::try_push] [vendor/rskafka/src/client/producer/aggregator.rs:RecordAggregator::flush]
- Each flush returns a fresh BatchBuilder; empty aggregates broadcast an empty status immediately, while nonempty batches produce asynchronously. [vendor/rskafka/src/client/producer/batch.rs:BatchBuilder::background_flush]
- BroadcastOnce asserts a single publication. Dropping an unpublished sender installs Dropped and wakes waiters; receivers register notification before checking/rechecking shared state. [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnce::broadcast] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnce] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnceReceiver::receive]

## Side Effects

- May spawn a Tokio task, call ProducerClient::produce, broadcast status, and emit tracing events. Broadcast/drop mutates shared state and wakes tasks. [vendor/rskafka/src/client/producer/batch.rs:BatchBuilder::background_flush] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnce::broadcast] [vendor/rskafka/src/client/producer/broadcast.rs:BroadcastOnce]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
