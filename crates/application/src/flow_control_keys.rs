//! What an operator and a run's reader see of the live keys of flow control (feature 022, FR-022-022; T022-026): the
//! run-level permit of one run (`Run.concurrency`) and the concurrency keys of a namespace with their holders, waiters
//! and oldest ages (`GET /namespaces/{namespaceId}/concurrency-keys`).
//!
//! Both are read from the same rows the gate decides with (`concurrency_semaphores`, `concurrency_holders`,
//! `concurrency_waiters`), so a run's permit and the listing of its key never disagree about what they read. A key is
//! shown only as its 12-hex label (D1168), and only to a caller who may reveal personal data in the namespace (FR-022-057,
//! D022-060: until T022-104 classifies what a key reads, every key is taken to read `pii`); its value never reaches this
//! layer. The reads take no lock: each is a snapshot of one statement, and a key's counts may move between two pages.

use std::fmt::Write as _;
use std::future::Future;

use sagat_domain::execution::{GatePool, KeyLabel};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{GateLevel, ResolvedGroup};
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::keyed_digest::KeyRing;

/// The run-level concurrency permit of a run as its rows say now: held, or waited for until a queue deadline. `None` on
/// a run whose definition declares no run-level `concurrency`, and on one that no longer holds or waits (ended,
/// timed out, dequeued): the journal keeps those facts.
///
/// Its key's label is hidden when it is read ([`Self::label`] is `None`); only [`Self::reveal_label`], which the run
/// service calls for a caller who may reveal personal data in the run's namespace, shows it (fail closed, D022-060).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPermit {
    pub pool: GatePool,
    pub group: ResolvedGroup,
    pub state: RunPermitState,
    /// The database clock of the statement that acquired the permit (holding) or queued the run (waiting), read after
    /// the key's lock and before its commit.
    pub since: OffsetDateTime,
    label: Option<KeyLabel>,
    digest: [u8; 32],
}

impl RunPermit {
    /// A permit as its rows say, its label hidden.
    #[must_use]
    pub const fn new(
        pool: GatePool,
        group: ResolvedGroup,
        digest: [u8; 32],
        state: RunPermitState,
        since: OffsetDateTime,
    ) -> Self {
        Self {
            pool,
            group,
            state,
            since,
            label: None,
            digest,
        }
    }

    /// The key's label, if it was revealed to this reader.
    #[must_use]
    pub const fn label(&self) -> Option<&KeyLabel> {
        self.label.as_ref()
    }

    /// Shows the key's label: for a caller who may reveal personal data in the run's namespace.
    pub fn reveal_label(&mut self) {
        self.label = Some(KeyLabel::of_digest(&self.digest));
    }
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
    /// The key's label, `None` unless the reader may reveal personal data in the namespace (FR-022-057, D022-060): the
    /// store leaves it `None` and the service sets it from [`KeyPosition`]'s digest.
    pub label: Option<KeyLabel>,
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
/// It carries the key's digest, not only its label: two keys of one group may share a label (48 bits), and a position
/// that could not tell them apart would skip or repeat one at a page boundary. A caller never sees it: the cursor is the
/// position sealed (encrypted and authenticated) for the namespace under the deployment's flow-control key
/// ([`Self::seal`], D022-059), so the cursor of a page whose labels are hidden discloses no label either.
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
    /// The cursor a caller is given: the position sealed for this namespace of this tenant. `None` when this process
    /// holds no flow-control key (it cannot then hand out a cursor; the service refuses the page, D022-059).
    #[must_use]
    pub fn seal(&self, ring: &KeyRing, tenant: TenantId, namespace: NamespaceId) -> Option<String> {
        ring.seal_cursor(tenant, namespace, self.encode().as_bytes())
    }

    /// The position a cursor [`Self::seal`] wrote for this namespace of this tenant; `None` for anything else.
    #[must_use]
    pub fn open(
        text: &str,
        ring: &KeyRing,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Option<Self> {
        let plain = ring.open_cursor(tenant, namespace, text)?;
        Self::decode(std::str::from_utf8(&plain).ok()?)
    }

    /// The position's text before it is sealed.
    fn encode(&self) -> String {
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

    /// Reads a position back; `None` for anything [`Self::encode`] did not write.
    fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        if parts.next()? != CURSOR_VERSION {
            return None;
        }
        let since = parts.next()?;
        let oldest_wait_us = if since == "n" {
            None
        } else {
            let us = since.parse::<i64>().ok()?;
            // a waiter's instant is never before 1970 and always one PostgreSQL and this process can read back
            if us < 0 {
                return None;
            }
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
    /// group, with the namespace's queued runs. Every label is `None`: the service decides who sees them.
    fn concurrency_keys(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&KeyPosition>,
        limit: u32,
    ) -> impl Future<Output = Result<ConcurrencyKeyPage, StoreError>> + Send;

    /// Whether `subject` may reveal personal data in the namespace now: its role in the tenant may use a reveal grant
    /// and it holds one for the namespace (feature 009's `payload:reveal-pii`). What a key's label needs (FR-022-057,
    /// D022-060). Read without a lock: a label is a pseudonym, not a disclosure that is audited.
    fn reveals_personal_data(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &Subject,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The deployment's flow-control digest keys this process holds, which seal the listing's cursors (D022-059).
    fn flow_control_key_ring(&self) -> Option<&KeyRing>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const RING: &str = "1:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

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
    fn a_position_reads_back_what_it_was_written_from() {
        for p in [
            position(Some(1_763_640_000_123_456), "payments.eu"),
            position(None, "=order-intake/charge"),
            position(Some(0), "=order-intake"),
        ] {
            assert_eq!(KeyPosition::decode(&p.encode()), Some(p));
        }
    }

    #[test]
    fn a_position_it_did_not_write_is_refused() {
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
            // before 1970, and before PostgreSQL's first instant (keys review, security minor 2)
            good.replacen(".n.", ".-1.", 1),
            good.replacen(".n.", ".-300000000000000000.", 1),
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

    /// Keys review (2026-11-20), security M2 and architecture M1: the cursor carries no digest a caller can read, is
    /// refused altered, and is refused in another namespace.
    #[test]
    fn a_cursor_is_the_position_sealed_for_its_namespace() {
        let ring = KeyRing::parse(RING).expect("ring");
        let (tenant, namespace) = (TenantId::generate(), NamespaceId::generate());
        let p = position(Some(1_763_640_000_123_456), "payments.eu");
        let cursor = p.seal(&ring, tenant, namespace).expect("sealed");
        assert!(!cursor.contains(&hex(&p.digest)), "{cursor}");
        assert!(!cursor.contains(&hex(&p.digest[..6])), "{cursor}");
        assert!(
            cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
            "{cursor}"
        );
        assert_eq!(
            KeyPosition::open(&cursor, &ring, tenant, namespace),
            Some(p.clone())
        );
        assert_eq!(
            KeyPosition::open(&cursor, &ring, tenant, NamespaceId::generate()),
            None
        );
        // the plain position a caller might forge is no cursor
        assert_eq!(
            KeyPosition::open(&p.encode(), &ring, tenant, namespace),
            None
        );
        assert_eq!(p.seal(&KeyRing::default(), tenant, namespace), None);
    }

    #[test]
    fn a_permit_hides_its_label_until_it_is_revealed() {
        let mut permit = RunPermit::new(
            GatePool::Ordinary,
            ResolvedGroup::parse("payments").expect("a group"),
            [0xab; 32],
            RunPermitState::Holding,
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(permit.label(), None);
        permit.reveal_label();
        assert_eq!(permit.label(), Some(&KeyLabel::of_digest(&[0xab; 32])));
    }
}
