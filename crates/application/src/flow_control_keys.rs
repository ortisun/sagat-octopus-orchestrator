//! What an operator and a run's reader see of the live keys of flow control (feature 022, FR-022-022; T022-026): the
//! run-level permit of one run (`Run.concurrency`) and the concurrency keys of a namespace with their holders, waiters
//! and oldest ages (`GET /namespaces/{namespaceId}/concurrency-keys`).
//!
//! Both are read from the same rows the gate decides with (`concurrency_semaphores`, `concurrency_holders`,
//! `concurrency_waiters`), so a run's permit and the listing of its key never disagree about what they read. A key is
//! shown only as its 12-hex label (D1168); its value never reaches this layer. The reads take no lock: each is a snapshot
//! of one statement, and a key's counts may move between two pages.

use std::fmt::Write as _;
use std::future::Future;

use sagat_domain::execution::{GatePool, GateRef, KeyLabel};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::{GateLevel, ResolvedGroup};
use time::OffsetDateTime;

use crate::error::StoreError;

/// The run-level concurrency permit of a run as its rows say now: held, or waited for until a queue deadline. `None` on
/// a run whose definition declares no run-level `concurrency`, and on one that no longer holds or waits (ended,
/// timed out, dequeued): the journal keeps those facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPermit {
    pub key: GateRef,
    pub state: RunPermitState,
    /// When the permit was acquired (holding) or the run was queued (waiting): the database clock of that commit.
    pub since: OffsetDateTime,
}

/// Whether a run holds its permit or waits for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPermitState {
    Holding,
    /// Queued; the run is dequeued and fails `CONCURRENCY_WAIT_TIMEOUT` at `deadline` if not granted before.
    Waiting {
        deadline: OffsetDateTime,
    },
}

impl RunPermitState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Holding => "HOLDING",
            Self::Waiting { .. } => "WAITING",
        }
    }
}

/// One live concurrency key of a namespace (a `concurrency_semaphores` row), read without its lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyKeyView {
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    pub label: KeyLabel,
    /// The row's counters, which the gate decides with (invariants 1 and 2 of `data-model.md`).
    pub holders: u32,
    pub waiters: u32,
    pub oldest_wait_since: Option<OffsetDateTime>,
    pub oldest_holder_since: Option<OffsetDateTime>,
    /// Where a page that ends with this key resumes.
    pub position: KeyPosition,
}

/// A page of concurrency keys, oldest wait first, and the namespace's queued runs (concurrency and rate-limit waiters
/// together, a run once), counted as the `queued_runs` ceiling counts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyKeyPage {
    pub items: Vec<ConcurrencyKeyView>,
    pub queued_runs: u64,
}

/// The place of a key in the listing's order: oldest wait first (a key nobody waits for last), then pool, scope, group
/// and digest, a total order since those five are the row's key within a namespace (D022-055).
///
/// It carries the key's digest, not only its label: two keys of one group may share a label (48 bits), and a cursor
/// that could not tell them apart would skip or repeat one at a page boundary. The digest is the output of the same
/// keyed function as the label (an HMAC under the deployment's secret key), so it lets nobody test a guessed value
/// either; it is shown to the caller the listing already shows the labels to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPosition {
    /// The oldest wait of the key in microseconds since the epoch, `None` for a key nobody waits for.
    pub oldest_wait_us: Option<i64>,
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    pub digest: [u8; 32],
}

const CURSOR_VERSION: &str = "k1";

impl KeyPosition {
    /// The opaque cursor text: URL-safe (letters, digits, `.` and `-`).
    #[must_use]
    pub fn encode(&self) -> String {
        let mut text = String::with_capacity(160);
        text.push_str(CURSOR_VERSION);
        text.push('.');
        match self.oldest_wait_us {
            Some(us) => {
                let _ = write!(text, "{us}");
            }
            None => text.push('n'),
        }
        let _ = write!(
            text,
            ".{}.{}.{}.{}",
            self.pool.as_str(),
            self.scope.as_str(),
            hex(&self.digest),
            hex(self.group.as_str().as_bytes())
        );
        text
    }

    /// Reads a cursor back; `None` for anything [`Self::encode`] did not write.
    #[must_use]
    pub fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        if parts.next()? != CURSOR_VERSION {
            return None;
        }
        let since = parts.next()?;
        let oldest_wait_us = if since == "n" {
            None
        } else {
            let us = since.parse::<i64>().ok()?;
            // an instant this process can read back (the rows' instants always are)
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000).ok()?;
            Some(us)
        };
        let pool = GatePool::parse(parts.next()?)?;
        let scope = match parts.next()? {
            "RUN" => GateLevel::Run,
            "STEP" => GateLevel::Step,
            _ => return None,
        };
        let digest: [u8; 32] = unhex(parts.next()?)?.try_into().ok()?;
        let group = String::from_utf8(unhex(parts.next()?)?).ok()?;
        let group = ResolvedGroup::parse(&group).ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            oldest_wait_us,
            pool,
            scope,
            group,
            digest,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// What the API reads about the live concurrency keys of a namespace.
pub trait FlowControlKeyReader {
    /// The namespace's concurrency keys after `after` (from the first when `None`), at most `limit`, optionally of one
    /// group, with the namespace's queued runs.
    fn concurrency_keys(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&KeyPosition>,
        limit: u32,
    ) -> impl Future<Output = Result<ConcurrencyKeyPage, StoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(oldest_wait_us: Option<i64>, group: &str) -> KeyPosition {
        KeyPosition {
            oldest_wait_us,
            pool: GatePool::Ordinary,
            scope: GateLevel::Step,
            group: ResolvedGroup::parse(group).expect("a group"),
            digest: [0xab; 32],
        }
    }

    #[test]
    fn a_cursor_reads_back_what_it_was_written_from() {
        for p in [
            position(Some(1_763_640_000_123_456), "payments.eu"),
            position(None, "=order-intake/charge"),
            position(Some(-5), "=order-intake"),
        ] {
            let text = p.encode();
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
                "{text}"
            );
            assert_eq!(KeyPosition::decode(&text), Some(p));
        }
    }

    #[test]
    fn a_cursor_it_did_not_write_is_refused() {
        let good = position(None, "payments").encode();
        for bad in [
            String::new(),
            "k2".to_owned() + &good[2..],
            good.replace("ORDINARY", "OTHER"),
            good.replace(".STEP.", ".NODE."),
            good.clone() + ".00",
            good[..good.len() - 1].to_owned(),
            good.replacen(".n.", ".1x.", 1),
            // an instant out of every calendar's range
            good.replacen(".n.", &format!(".{}.", i64::MAX), 1),
            // a group no row can have
            format!(
                "k1.n.ORDINARY.RUN.{}.{}",
                "ab".repeat(32),
                hex(b"Bad Group")
            ),
            // a digest of the wrong length
            format!("k1.n.ORDINARY.RUN.{}.{}", "ab".repeat(31), hex(b"payments")),
        ] {
            assert_eq!(KeyPosition::decode(&bad), None, "{bad}");
        }
    }
}
