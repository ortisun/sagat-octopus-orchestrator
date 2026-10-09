//! Signing and verifying the way a provider and a receiver do, for the suites of feature 004.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::signing::{Covered, Direction, sign, verify};

/// How a provider signs an event: the type it names is covered, no subject and no key.
pub fn inbound(
    secret: &str,
    timestamp: i64,
    event_id: &str,
    event_type: &str,
    body: &[u8],
) -> String {
    inbound_with(secret, timestamp, event_id, event_type, "", "", body)
}

/// How a provider signs an event that also names a subject and a correlation key.
pub fn inbound_with(
    secret: &str,
    timestamp: i64,
    event_id: &str,
    event_type: &str,
    subject: &str,
    correlation_key: &str,
    body: &[u8],
) -> String {
    sign(
        Direction::Inbound,
        secret.as_bytes(),
        &Covered {
            timestamp,
            event_id,
            event_type,
            subject,
            correlation_key,
        },
        body,
    )
    .expect("a secret long enough to sign with")
}

/// Whether a receiver that holds `secret` accepts what the platform sent: the headers it came with
/// (lower-case names, as the test receiver records them) and the body.
pub fn delivery_verifies(secret: &str, headers: &BTreeMap<String, String>, body: &[u8]) -> bool {
    let timestamp: i64 = headers["x-octopus-timestamp"].parse().expect("timestamp");
    verify(
        Direction::Outbound,
        &[secret.as_bytes()],
        &headers["x-octopus-signature"],
        &Covered {
            timestamp,
            event_id: &headers["x-octopus-event-id"],
            event_type: &headers["x-octopus-event-type"],
            subject: "",
            correlation_key: "",
        },
        body,
        timestamp,
        Duration::from_secs(300),
    )
    .is_ok()
}
