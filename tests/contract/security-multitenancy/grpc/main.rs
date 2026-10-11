//! The worker protocol's transport security over the wire (FR-009-003, gap G27): TLS and mutual TLS
//! with real handshakes against a real listener.
#![allow(clippy::expect_used, clippy::panic)]

mod connections;
mod stub;
mod tls;
