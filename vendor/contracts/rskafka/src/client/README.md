# client contract

## Overview

Public Kafka client API for metadata, topic administration, partition operations, stream consumption, and batch production.

Source directory: [`vendor/rskafka/src/client`](../../../../rskafka/src/client). This contract is outside the verified upstream snapshot.

## Interface

- ClientBuilder configures bootstrap brokers, client ID, frame size, backoff, SASL, and optional TLS/SOCKS5; Client creates controller/partition clients and lists non-internal topics. [vendor/rskafka/src/client/mod.rs:ClientBuilder] [vendor/rskafka/src/client/mod.rs:ClientBuilder::build] [vendor/rskafka/src/client/mod.rs:Client::controller_client] [vendor/rskafka/src/client/mod.rs:Client::partition_client] [vendor/rskafka/src/client/mod.rs:Client::list_topics]
- ControllerClient creates/deletes topics. PartitionClient exposes topic/partition identity, produce/fetch/get_offset/delete_records, with UnknownTopicHandling, Compression, and OffsetAt options. [vendor/rskafka/src/client/controller.rs:ControllerClient] [vendor/rskafka/src/client/partition.rs:PartitionClient] [vendor/rskafka/src/client/partition.rs:UnknownTopicHandling] [vendor/rskafka/src/client/partition.rs:Compression] [vendor/rskafka/src/client/partition.rs:OffsetAt]
- Error/RequestContext/ServerErrorResponse expose typed failures and partial broker response context. StreamConsumerBuilder/StartOffset configure a StreamConsumer yielding RecordAndOffset with the high watermark. [vendor/rskafka/src/client/error.rs:Error] [vendor/rskafka/src/client/error.rs:RequestContext] [vendor/rskafka/src/client/error.rs:ServerErrorResponse] [vendor/rskafka/src/client/consumer.rs:StreamConsumerBuilder] [vendor/rskafka/src/client/consumer.rs:StartOffset] [vendor/rskafka/src/client/consumer.rs:StreamConsumer]
- ProducerClient, BatchProducerBuilder, and BatchProducer expose generic aggregation, linger/compression, produce, and flush. MetadataCache and its generation type remain crate-internal through module visibility. [vendor/rskafka/src/client/producer.rs:ProducerClient] [vendor/rskafka/src/client/producer.rs:BatchProducerBuilder] [vendor/rskafka/src/client/producer.rs:BatchProducer] [vendor/rskafka/src/client/metadata_cache.rs:MetadataCache] [vendor/rskafka/src/client/mod.rs:metadata_cache]
- [producer](producer/README.md) — Public aggregation contracts plus internal batch flushing and one-shot broadcast coordination.

## Invariants

- Building a client refreshes metadata. list_topics requests uncached all-topic metadata and filters broker-marked internal topics; topic create/delete requires exactly one result and refreshes metadata after success. [vendor/rskafka/src/client/mod.rs:ClientBuilder::build] [vendor/rskafka/src/client/mod.rs:Client::list_topics] [vendor/rskafka/src/client/controller.rs:ControllerClient::create_topic] [vendor/rskafka/src/client/controller.rs:ControllerClient::delete_topic]
- Partition responses must contain exactly the requested topic/partition or become InvalidResponse. Empty production sends no request; nonempty production asks for all acknowledgements with a 30-second timeout and derives consecutive offsets from the broker base. [vendor/rskafka/src/client/partition.rs:process_produce_response] [vendor/rskafka/src/client/partition.rs:process_fetch_response] [vendor/rskafka/src/client/partition.rs:process_list_offsets_response] [vendor/rskafka/src/client/partition.rs:process_delete_records_response] [vendor/rskafka/src/client/partition.rs:PartitionClient::produce] [vendor/rskafka/src/client/partition.rs:build_produce_request]
- Fetch/list-offset requests use read-committed isolation; fetched control records and records before the requested offset are filtered. Broken connections and leader errors retry with invalidation; unknown topics retry only under UnknownTopicHandling::Retry. [vendor/rskafka/src/client/partition.rs:build_fetch_request] [vendor/rskafka/src/client/partition.rs:build_list_offsets_request] [vendor/rskafka/src/client/partition.rs:extract_records] [vendor/rskafka/src/client/partition.rs:maybe_retry]
- Cache invalidation checks generations, making stale invalidation a no-op. Requested-topic lookup returns only requested topics and invalidates if any is missing. [vendor/rskafka/src/client/metadata_cache.rs:MetadataCache::invalidate] [vendor/rskafka/src/client/metadata_cache.rs:MetadataCache::get] [vendor/rskafka/src/client/partition.rs:PartitionClient] [vendor/rskafka/src/client/controller.rs:ControllerClient]
- The stream consumer sorts by offset, advances past the last returned record, retries earliest/latest races after one second, and otherwise emits a fetch error once then terminates. [vendor/rskafka/src/client/consumer.rs:StreamConsumer]
- Batch production flushes on capacity/linger; a flush token prevents an old timer from flushing a newer batch. Cancellation preserves producer state but does not establish whether submitted data was produced; an errored flush may be incomplete. [vendor/rskafka/src/client/producer.rs:ProducerInner::try_push] [vendor/rskafka/src/client/producer.rs:ProducerInner::flush] [vendor/rskafka/src/client/producer.rs:BatchProducer::produce] [vendor/rskafka/src/client/producer.rs:BatchProducer::flush]

## Side Effects

- Requests broker metadata, creates/deletes topics, publishes/fetches/deletes records, and queries offsets; retry paths may sleep for backoff/throttle and emit traces. [vendor/rskafka/src/client/mod.rs:ClientBuilder::build] [vendor/rskafka/src/client/mod.rs:Client::list_topics] [vendor/rskafka/src/client/controller.rs:ControllerClient] [vendor/rskafka/src/client/partition.rs:PartitionClient] [vendor/rskafka/src/client/partition.rs:maybe_retry]
- Streaming consumption buffers fetched records and repeats requests. Batch production spawns linger/flush tasks, writes through ProducerClient, and manual flush waits for task completion; dropping inner state aborts outstanding tasks. [vendor/rskafka/src/client/consumer.rs:StreamConsumer] [vendor/rskafka/src/client/producer.rs:BatchProducer::produce] [vendor/rskafka/src/client/producer.rs:BatchProducer::flush] [vendor/rskafka/src/client/producer.rs:ProducerInner]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
