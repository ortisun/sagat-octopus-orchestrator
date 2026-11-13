# crates/infra-connectors

## Overview

sagat-infra-connectors adapts inbound/outbound brokers, Redis and guarded webhooks.

## Interface

- Supervisor, broker run loops, Publisher, RedisPublisher and GuardedSender expose consumer/publication boundaries. [src/supervisor.rs:Supervisor] [src/publish.rs:Publisher] [src/redis.rs:RedisPublisher] [src/webhook.rs:GuardedSender]
- [src](./src/README.md) — Inbound NATS/Kafka/SQS consumers, guarded NATS/Kafka/SQS/Redis publishers, webhook sending, credentials and consumer supervision. [src/credentials.rs:NotAnObject]

## Invariants

- Broker acknowledgement follows durable admission/checkpoint decisions and publication remains at least once. Kafka advertised listeners retain the documented guard gap. [src/ingest.rs:Ingest::judge] [src/ingest.rs:Outcome::done] [src/publish.rs:publish] [src/guard.rs:BrokerGuard]

## Side Effects

- Consumers/publishers open broker/HTTP connections, update checkpoints/dead letters through ports and create external messages. [src/supervisor.rs:Supervisor::run] [src/publish.rs:Publisher::kafka] [src/webhook.rs:send]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-connectors --locked
```

These commands were not run in this documentation-only pass.
