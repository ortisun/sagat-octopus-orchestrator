# connection contract

## Overview

Private broker topology and socket transport implementation supporting the public client.

Source directory: [`vendor/rskafka/src/connection`](../../../../rskafka/src/connection). This contract is outside the verified upstream snapshot.

## Interface

- Internal BrokerTopology/Broker types provide broker lookup and metadata-driven updates. Transport implements Tokio AsyncRead/AsyncWrite over TCP and feature-enabled TLS, optionally routing through SOCKS5. [vendor/rskafka/src/connection/topology.rs:BrokerTopology] [vendor/rskafka/src/connection/topology.rs:Broker] [vendor/rskafka/src/connection/transport.rs:Transport] [vendor/rskafka/src/connection/transport.rs:Transport::connect]
- Only ConnectionError and selected SASL configuration types are publicly re-exported; the connection and transport modules themselves remain private. [vendor/rskafka/src/lib.rs:ConnectionError] [vendor/rskafka/src/client/mod.rs:SaslConfig]
- [transport](transport/README.md) — SASL credential/configuration plumbing, with configuration types re-exported through the public client API.

## Invariants

- Topology keys entries by broker ID, inserts new IDs, and replaces only changed host/port endpoints; absent metadata entries are not removed. [vendor/rskafka/src/connection/topology.rs:BrokerTopology::update]
- Without transport-tls only plain transport exists; with it TLS is applied only when configured. Without transport-socks5 the proxy argument is ignored; with it a supplied proxy invokes SOCKS5 negotiation. [vendor/rskafka/src/connection/transport.rs:Transport::wrap_tls] [vendor/rskafka/src/connection/transport.rs:Transport::connect_tcp]

## Side Effects

- Topology additions/endpoint changes emit tracing events; transport opens sockets and may perform SOCKS5 and TLS handshakes. [vendor/rskafka/src/connection/topology.rs:BrokerTopology::update] [vendor/rskafka/src/connection/transport.rs:Transport::connect] [vendor/rskafka/src/connection/transport.rs:Transport::connect_tcp] [vendor/rskafka/src/connection/transport.rs:Transport::wrap_tls]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
