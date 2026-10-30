# crates/infra-nats/src

## Overview

JetStream EventPublisher adapter for durable outbox hints/events; acknowledgements and broker deduplication do not make external delivery exactly-once.

## Interface

- `SUBJECT_FILTER`, `NatsConfig`, `NatsError` and `NatsPublisher` expose configuration, retrying connection, stream provisioning and EventPublisher publication. [lib.rs:SUBJECT_FILTER] [lib.rs:NatsConfig] [lib.rs:NatsError] [lib.rs:NatsPublisher] [lib.rs:NatsPublisher::connect] [lib.rs:NatsPublisher::stream] [lib.rs:publish]

## Invariants

- Configuration validates nonempty URL/safe stream names. Defaults are one-hour retention, 120-second duplicate window, one replica and five-second acknowledgement timeout. Streams use file storage/limits retention and sagat.v1.> subjects with single-flight first use. [lib.rs:NatsPublisher::connect] [lib.rs:NatsConfig::new] [lib.rs:NatsPublisher::ensure_stream]
- Publish rejects subjects outside sagat.v1., carries the stable ID as Nats-Msg-Id and bounds provisioning/publication/PubAck by ack_timeout. [lib.rs:publish]

## Side Effects

- Connect can read a credentials file and open/retry NATS sockets. First use can create/update the stream; publish sends bytes and waits for PubAck. Failures/timeouts map to retryable unavailability with credential-free tracing. [lib.rs:NatsPublisher::connect] [lib.rs:NatsPublisher::ensure_stream] [lib.rs:publish]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-nats --locked
```

These commands were not run in this documentation-only pass.
