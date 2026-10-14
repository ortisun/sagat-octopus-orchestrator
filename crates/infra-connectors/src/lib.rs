//! Adapters between the event gateway and the outside world (`FR-004-005`, `FR-004-006`).
//!
//! * [`webhook`]: outbound HTTP through the guarded egress client, so a subscription can only
//!   reach what the egress policy of its namespace allows.
//! * [`nats`], [`kafka`], [`sqs`]: the consumers that turn what a tenant's broker holds into events.
//!   They share [`ingest`], which fixes the order that matters (the event is in the inbox before the
//!   broker is told the message is done), and are kept running by the [`supervisor`].

pub mod credentials;
pub mod guard;
pub mod ingest;
pub mod kafka;
pub mod nats;
pub mod publish;
pub mod sqs;
pub mod supervisor;
pub mod webhook;
