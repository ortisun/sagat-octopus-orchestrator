# crates/infra-connectors/src

## Overview

Inbound NATS/Kafka/SQS consumers, guarded NATS/Kafka/SQS/Redis publishers, webhook sending, credentials and consumer supervision.

## Interface

- Credential parsing/opening exposes `NotAnObject`, redacted `Credentials`, `CredentialsError`, `open` and `expects`; broker policy exposes `ResolveFailure`, `BrokerGuard`, `PolicyGuard` and test-feature-only `Unguarded`. [credentials.rs:NotAnObject] [credentials.rs:Credentials] [credentials.rs:CredentialsError] [credentials.rs:open] [credentials.rs:expects] [guard.rs:ResolveFailure] [guard.rs:BrokerGuard] [guard.rs:PolicyGuard] [guard.rs:Unguarded]
- `Outcome`, `Ingest` and `Pacer` handle admission/progress/backpressure. Kafka/NATS/SQS expose run/timing/stop families; SQS also exposes `Received`, `Unreadable` and `parse_receive`. [ingest.rs:Outcome] [ingest.rs:Ingest] [ingest.rs:Pacer] [kafka.rs:Timing] [kafka.rs:run] [kafka.rs:run_with] [nats.rs:Stopped] [nats.rs:run] [sqs.rs:Timing] [sqs.rs:Received] [sqs.rs:Unreadable] [sqs.rs:parse_receive] [sqs.rs:run_with]
- `Publisher` implements BrokerPublisher. Redis exposes config/errors/bounds/valid_stream/append and `RedisPublisher`; `Supervisor` manages consumers and provider wiring. [publish.rs:Publisher] [redis.rs:MAX_PAYLOAD] [redis.rs:MAX_STREAM_LEN] [redis.rs:RedisError] [redis.rs:RedisConfig] [redis.rs:valid_stream] [redis.rs:append] [redis.rs:RedisPublisher] [supervisor.rs:stopped] [supervisor.rs:Supervisor]
- Webhook APIs expose `ClientFactory`, `Guarded`, `Clients` policy/client caching and `GuardedSender` implementing WebhookSender. [webhook.rs:ClientFactory] [webhook.rs:Guarded] [webhook.rs:Clients] [webhook.rs:GuardedSender]

## Invariants

- Only Admitted/Duplicate/SetAside outcomes acknowledge broker work; admission/dead-letter/checkpoint persistence precedes ack/delete/advance. Backlog/supersession/store failures leave redelivery possible. [ingest.rs:Ingest::judge] [ingest.rs:Ingest::set_aside] [ingest.rs:Outcome::done]
- Publisher bodies/headers are bounded at 256 KiB, 32 headers and 16 KiB combined header bytes. Credentials are validated at use, and NATS secrets/Kafka SASL require TLS. Configured endpoints are guarded, but Kafka-advertised listener addresses retain a documented client-library guard gap. [publish.rs:MAX_PAYLOAD_BYTES] [publish.rs:MAX_HEADERS] [publish.rs:MAX_HEADER_BYTES] [nats.rs:sends_a_secret] [kafka.rs:sasl] [guard.rs:BrokerGuard]
- Kafka caps partitions/fetches and uses database checkpoints/leases; NATS uses explicit acknowledgements and bounded in-flight work; SQS bounds batches/poll/visibility/XML. Redis validates DB 0–15, stream/fields/bytes/timeout, pins a checked address and requires a canonical nonzero XADD ID. [kafka.rs:MAX_PARTITIONS] [kafka.rs:FETCH_BYTES] [nats.rs:ACK_WAIT] [sqs.rs:BATCH] [sqs.rs:MAX_RESPONSE_BYTES] [redis.rs:RedisConfig::new] [redis.rs:append]
- Webhook policy refreshes per attempt, cache size is 256 namespace/policy pairs, POST redirects are not followed and response bodies are skipped. Provider dedupe metadata does not make publication atomic with platform state. [webhook.rs:MAX_CLIENTS] [webhook.rs:Clients::client] [webhook.rs:send] [publish.rs:publish]

## Side Effects

- Credential opening reads/decrypts secrets; supervisor starts/aborts consumer tasks and stores progress/errors/metrics. Consumers open broker/HTTP connections, maintain Kafka leases, admit inbox/dead letters and subsequently acknowledge/delete. [credentials.rs:open] [supervisor.rs:Supervisor::run] [ingest.rs:Ingest::flush] [nats.rs:run] [kafka.rs:read_partition] [sqs.rs:run_with]
- Publishers open per-attempt connections and create external messages. Redis XADD can have reached the server before timeout, so retries remain at-least-once. Policy/webhook operations perform DNS, policy reads, denial audits and guarded HTTP. [publish.rs:Publisher::nats] [publish.rs:Publisher::kafka] [publish.rs:Publisher::sqs] [publish.rs:Publisher::redis] [redis.rs:append] [guard.rs:PolicyGuard::resolve] [webhook.rs:send]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-connectors --locked
```

These commands were not run in this documentation-only pass.
