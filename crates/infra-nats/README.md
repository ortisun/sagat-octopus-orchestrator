# crates/infra-nats

## Overview

sagat-infra-nats publishes outbox task hints and run events through JetStream.

## Interface

- NatsConfig, NatsPublisher, stream provisioning and EventPublisher publication expose the adapter. [src/lib.rs:NatsConfig] [src/lib.rs:NatsPublisher] [src/lib.rs:publish]
- [src](./src/README.md) — JetStream EventPublisher adapter for durable outbox hints/events; acknowledgements and broker deduplication do not make external delivery exactly-once. [src/lib.rs:SUBJECT_FILTER]

## Invariants

- Subjects must have the sagat.v1. prefix; broker acknowledgement/deduplication do not make delivery exactly once. [src/lib.rs:SUBJECT_FILTER] [src/lib.rs:publish]

## Side Effects

- Connection may read credentials and open sockets; first use may create/update a stream and publish waits for PubAck. [src/lib.rs:NatsPublisher::connect] [src/lib.rs:NatsPublisher::ensure_stream] [src/lib.rs:publish]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-nats --locked
```

These commands were not run in this documentation-only pass.
