//! Signatures of events that cross the platform's boundary (`FR-004-004`, `FR-004-006`): the ones a
//! provider sends to an endpoint, and the ones the platform sends to a subscriber.
//!
//! A signature covers the instant, the identity of the event, what the sender says the event is
//! (its type, subject and correlation key) and its body, so a captured request cannot be replayed
//! later (the instant), cannot be given another identity (the id), cannot be given another meaning
//! (the envelope: the type picks the triggers that fire and the correlation key the waits that wake)
//! and cannot be given another payload (the body). The scheme is HMAC-SHA256 over
//! `"{label}\n{timestamp}\n{event_id}\n{event_type}\n{subject}\n{correlation_key}\n"` followed by the
//! raw body, written `v1=<hex>`; an absent subject or key is the empty string. No covered field can
//! contain a newline (they are checked when an event is made), so the pieces cannot be shifted into
//! one another. The label separates the two directions, so a delivery the platform sent can never
//! be passed off as an event a provider sent with the same secret.
//!
//! More than one signature may be sent (comma separated) while a secret is being rotated; the
//! request is accepted if any of them verifies against any secret in force. How many are looked at,
//! and how long the header may be, is bounded **before** anything is computed, because the header is
//! written by whoever knows the URL.

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The only version of the scheme.
pub const VERSION: &str = "v1";

/// Signatures a header may carry: the current secret, the one before it, and room for a provider
/// that signs with a third while it moves.
pub const MAX_SIGNATURES: usize = 4;

/// Longest signature header looked at: `MAX_SIGNATURES` of `v1=` and 64 hex digits, with separators.
pub const MAX_HEADER_BYTES: usize = 512;

/// Shortest secret that signs or verifies anything. A shorter one is an accident waiting to be
/// guessed from a single captured request.
pub const MIN_SECRET_BYTES: usize = 16;

/// Which way an event travels. It is part of what is signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A provider to the platform.
    Inbound,
    /// The platform to a subscriber.
    Outbound,
}

impl Direction {
    const fn label(self) -> &'static str {
        match self {
            Self::Inbound => "octopus-event-in-v1",
            Self::Outbound => "octopus-event-out-v1",
        }
    }
}

/// What a signature covers besides the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Covered<'a> {
    /// Seconds since the epoch.
    pub timestamp: i64,
    pub event_id: &'a str,
    /// As the sender wrote it: empty when the request names none (the endpoint's default applies
    /// after the signature has been checked, and is not something the sender signed).
    pub event_type: &'a str,
    pub subject: &'a str,
    pub correlation_key: &'a str,
}

/// Why a request was not accepted. The reason is for metrics and logs; a caller is told only that
/// the signature is not valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// The header is not `v1=<hex>[,v1=<hex>...]` within the bounds above.
    #[error("the signature header is malformed")]
    Malformed,
    /// The timestamp is further from the clock than the endpoint allows.
    #[error("the timestamp is outside the allowed window")]
    Stale,
    /// No signature verifies under any secret in force.
    #[error("the signature does not verify")]
    Mismatch,
}

impl SignatureError {
    /// A closed label for counters.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::Stale => "stale",
            Self::Mismatch => "mismatch",
        }
    }
}

/// Whether a secret may sign or verify anything.
#[must_use]
pub const fn usable(secret: &[u8]) -> bool {
    secret.len() >= MIN_SECRET_BYTES
}

fn mac(
    direction: Direction,
    secret: &[u8],
    covered: &Covered<'_>,
    body: &[u8],
) -> Option<HmacSha256> {
    let pieces = [
        covered.event_id,
        covered.event_type,
        covered.subject,
        covered.correlation_key,
    ];
    if !usable(secret) || pieces.iter().any(|piece| piece.contains('\n')) {
        return None;
    }
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(direction.label().as_bytes());
    mac.update(b"\n");
    mac.update(covered.timestamp.to_string().as_bytes());
    for piece in pieces {
        mac.update(b"\n");
        mac.update(piece.as_bytes());
    }
    mac.update(b"\n");
    mac.update(body);
    Some(mac)
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// The header value that signs `body` with `secret`, or `None` when the secret is too short to sign
/// anything or a covered field contains a newline.
#[must_use]
pub fn sign(
    direction: Direction,
    secret: &[u8],
    covered: &Covered<'_>,
    body: &[u8],
) -> Option<String> {
    let digest = mac(direction, secret, covered, body)?
        .finalize()
        .into_bytes();
    Some(format!("{VERSION}={}", encode_hex(&digest)))
}

/// Looks at the header and the clock only: the cheap part of verifying, which a caller does before
/// it reads or opens a single secret. Returns the signatures the header carries.
///
/// # Errors
/// [`SignatureError::Malformed`] for a header that is too long, carries too many signatures or is
/// not `v1=<64 hex digits>` lists; [`SignatureError::Stale`] for a timestamp further than `skew`
/// from `now` in either direction.
pub fn precheck(
    header: &str,
    timestamp: i64,
    now: i64,
    skew: Duration,
) -> Result<Vec<Vec<u8>>, SignatureError> {
    if header.len() > MAX_HEADER_BYTES || header.split(',').count() > MAX_SIGNATURES {
        return Err(SignatureError::Malformed);
    }
    let candidates: Vec<Vec<u8>> = header
        .split(',')
        .map(str::trim)
        .map(|part| {
            part.strip_prefix(VERSION)
                .and_then(|rest| rest.strip_prefix('='))
                .and_then(decode_hex)
                .filter(|bytes| bytes.len() == 32)
        })
        .collect::<Option<_>>()
        .ok_or(SignatureError::Malformed)?;
    let window = i64::try_from(skew.as_secs()).unwrap_or(i64::MAX);
    if now.saturating_sub(timestamp).abs() > window {
        return Err(SignatureError::Stale);
    }
    Ok(candidates)
}

/// Checks a request.
///
/// `now` and `covered.timestamp` are seconds since the epoch; the request is refused when they
/// differ by more than `skew` in either direction. The comparison of each candidate against each
/// secret is constant-time; how many are compared depends only on the header and the number of
/// secrets, and the body is hashed once per secret, not once per candidate.
///
/// # Errors
/// [`SignatureError`].
pub fn verify(
    direction: Direction,
    secrets: &[&[u8]],
    header: &str,
    covered: &Covered<'_>,
    body: &[u8],
    now: i64,
    skew: Duration,
) -> Result<(), SignatureError> {
    let candidates = precheck(header, covered.timestamp, now, skew)?;
    let mut valid = false;
    for secret in secrets {
        let Some(base) = mac(direction, secret, covered, body) else {
            continue;
        };
        for candidate in &candidates {
            valid |= base.clone().verify_slice(candidate).is_ok();
        }
    }
    if valid {
        Ok(())
    } else {
        Err(SignatureError::Mismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"a-secret-of-some-length";
    const BODY: &[u8] = br#"{"orderId":7}"#;
    const SKEW: Duration = Duration::from_secs(300);
    const IN: Direction = Direction::Inbound;

    fn covered(timestamp: i64) -> Covered<'static> {
        Covered {
            timestamp,
            event_id: "evt-1",
            event_type: "invoice.paid",
            subject: "subject/1",
            correlation_key: "ord-7",
        }
    }

    fn signed(secret: &[u8], covered: &Covered<'_>) -> String {
        sign(IN, secret, covered, BODY).expect("a usable secret")
    }

    #[test]
    fn a_signature_verifies_for_what_it_covers_and_nothing_else() {
        let header = signed(SECRET, &covered(1_000));
        assert!(header.starts_with("v1="));
        assert_eq!(
            verify(IN, &[SECRET], &header, &covered(1_000), BODY, 1_000, SKEW),
            Ok(())
        );
        let base = covered(1_000);
        let changed: Vec<(&str, Covered<'_>)> = vec![
            ("timestamp", covered(1_001)),
            (
                "event id",
                Covered {
                    event_id: "evt-2",
                    ..base
                },
            ),
            (
                "event type",
                Covered {
                    event_type: "invoice.voided",
                    ..base
                },
            ),
            (
                "subject",
                Covered {
                    subject: "subject/2",
                    ..base
                },
            ),
            (
                "correlation key",
                Covered {
                    correlation_key: "ord-8",
                    ..base
                },
            ),
            (
                "absent type",
                Covered {
                    event_type: "",
                    ..base
                },
            ),
        ];
        for (what, other) in changed {
            assert_eq!(
                verify(IN, &[SECRET], &header, &other, BODY, 1_000, SKEW),
                Err(SignatureError::Mismatch),
                "a signature must not survive a change of the {what}"
            );
        }
        assert_eq!(
            verify(
                IN,
                &[b"another-secret-of-length"],
                &header,
                &base,
                BODY,
                1_000,
                SKEW
            ),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify(
                IN,
                &[SECRET],
                &header,
                &base,
                br#"{"orderId":8}"#,
                1_000,
                SKEW
            ),
            Err(SignatureError::Mismatch),
            "or of the body"
        );
    }

    #[test]
    fn the_fields_cannot_be_shifted_into_one_another() {
        // Without separators, type "a" and subject "bc" would sign like type "ab" and subject "c".
        let a = Covered {
            event_type: "a",
            subject: "bc",
            ..covered(1_000)
        };
        let b = Covered {
            event_type: "ab",
            subject: "c",
            ..covered(1_000)
        };
        assert_ne!(signed(SECRET, &a), signed(SECRET, &b));
        // A newline inside a field would let it pose as two; such a field cannot be signed at all.
        let smuggled = Covered {
            subject: "x\nord-7",
            correlation_key: "",
            ..covered(1_000)
        };
        assert_eq!(sign(IN, SECRET, &smuggled, BODY), None);
    }

    #[test]
    fn what_the_platform_sends_cannot_pass_for_what_a_provider_sent() {
        let delivery = sign(Direction::Outbound, SECRET, &covered(1_000), BODY).expect("signs");
        assert_ne!(delivery, signed(SECRET, &covered(1_000)));
        assert_eq!(
            verify(IN, &[SECRET], &delivery, &covered(1_000), BODY, 1_000, SKEW),
            Err(SignatureError::Mismatch),
            "a delivery replayed to an inbound endpoint"
        );
        assert_eq!(
            verify(
                Direction::Outbound,
                &[SECRET],
                &delivery,
                &covered(1_000),
                BODY,
                1_000,
                SKEW
            ),
            Ok(())
        );
    }

    #[test]
    fn a_secret_too_short_to_mean_anything_signs_and_verifies_nothing() {
        assert!(!usable(b"short"));
        assert!(usable(&[b'x'; MIN_SECRET_BYTES]));
        assert_eq!(sign(IN, b"short", &covered(1_000), BODY), None);
        // Even a signature made with it somewhere else does not verify here.
        let forged = {
            use hmac::{Hmac, KeyInit, Mac};
            let mut mac = Hmac::<Sha256>::new_from_slice(b"short").expect("any length");
            mac.update(b"anything");
            format!("v1={}", encode_hex(&mac.finalize().into_bytes()))
        };
        assert_eq!(
            verify(IN, &[b"short"], &forged, &covered(1_000), BODY, 1_000, SKEW),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn a_timestamp_outside_the_window_is_stale_in_either_direction() {
        let header = signed(SECRET, &covered(1_000));
        let check = |now| verify(IN, &[SECRET], &header, &covered(1_000), BODY, now, SKEW);
        assert_eq!(check(1_300), Ok(()));
        assert_eq!(check(1_301), Err(SignatureError::Stale));
        assert_eq!(check(700), Ok(()));
        assert_eq!(
            check(699),
            Err(SignatureError::Stale),
            "a timestamp from the future is as suspicious as one from the past"
        );
    }

    #[test]
    fn staleness_is_judged_before_the_signature_so_a_replay_costs_nothing() {
        let header = signed(SECRET, &covered(1_000));
        assert_eq!(
            verify(
                IN,
                &[b"wrong-secret-of-length"],
                &header,
                &covered(1_000),
                BODY,
                100_000,
                SKEW
            ),
            Err(SignatureError::Stale)
        );
    }

    #[test]
    fn a_malformed_header_is_refused_without_looking_at_the_secrets() {
        for bad in [
            "",
            "v1=",
            "v1=zz",
            "v2=00",
            "v1=0011",
            "abc",
            &format!("v1={}", "0".repeat(63)),
        ] {
            assert_eq!(
                verify(IN, &[SECRET], bad, &covered(1_000), BODY, 1_000, SKEW),
                Err(SignatureError::Malformed),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_header_is_bounded_before_a_single_hash_is_computed() {
        let one = format!("v1={}", "0".repeat(64));
        let several = |n: usize| vec![one.as_str(); n].join(",");
        // Up to the bound is fine (and, being wrong, a mismatch).
        assert_eq!(
            precheck(&several(MAX_SIGNATURES), 1_000, 1_000, SKEW).map(|c| c.len()),
            Ok(MAX_SIGNATURES)
        );
        // One more is refused outright, as is a header that is simply long, or all separators.
        for header in [
            several(MAX_SIGNATURES + 1),
            several(6_000),
            ",".repeat(100_000),
            "v1=".repeat(5_000),
        ] {
            assert_eq!(
                precheck(&header, 1_000, 1_000, SKEW),
                Err(SignatureError::Malformed),
                "{} bytes",
                header.len()
            );
        }
        assert!(several(MAX_SIGNATURES).len() <= MAX_HEADER_BYTES);
    }

    #[test]
    fn during_a_rotation_any_signature_against_any_secret_in_force_is_enough() {
        let (old_secret, new_secret): (&[u8], &[u8]) =
            (b"old-secret-of-length", b"new-secret-of-length");
        let old = signed(old_secret, &covered(1_000));
        let new = signed(new_secret, &covered(1_000));
        let both = format!("{old}, {new}");
        for header in [&old, &new, &both] {
            assert_eq!(
                verify(
                    IN,
                    &[new_secret, old_secret],
                    header,
                    &covered(1_000),
                    BODY,
                    1_000,
                    SKEW
                ),
                Ok(())
            );
        }
        assert_eq!(
            verify(IN, &[new_secret], &old, &covered(1_000), BODY, 1_000, SKEW),
            Err(SignatureError::Mismatch),
            "a secret no longer in force verifies nothing"
        );
        assert_eq!(
            verify(IN, &[], &both, &covered(1_000), BODY, 1_000, SKEW),
            Err(SignatureError::Mismatch),
            "no secret, no signature"
        );
    }

    #[test]
    fn the_known_vectors_stay_the_same() {
        // What a provider's (or a receiver's) own implementation must reproduce; they are in the
        // contract. HMAC-SHA256 keyed with "a-secret-of-some-length", computed independently.
        let covered = Covered {
            timestamp: 1_700_000_000,
            event_id: "evt_1",
            event_type: "invoice.paid",
            subject: "subject/1",
            correlation_key: "ord-7",
        };
        assert_eq!(
            sign(Direction::Inbound, SECRET, &covered, b"{}").as_deref(),
            Some("v1=552996eb858bb3ef71451dbeeb116324e3094fcb4c711cb56cf759d3e061eaec")
        );
        let delivery = Covered {
            event_type: "run.finished",
            subject: "",
            correlation_key: "",
            ..covered
        };
        assert_eq!(
            sign(Direction::Outbound, SECRET, &delivery, b"{}").as_deref(),
            Some("v1=20ba088d876051a345b4a53b1b10fe90648eda985a33e90733c8798cb7a87b5e")
        );
    }
}
