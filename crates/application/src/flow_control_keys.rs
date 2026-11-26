//! What an operator and a run's reader see of the live keys of flow control (feature 022, FR-022-022, FR-022-048;
//! T022-026, T022-078): the run-level permit of one run (`Run.concurrency`) and its rate-limit slot (`Run.rateLimit`,
//! with its position and estimate in a single-run read), and the concurrency keys and rate-limit windows of a namespace
//! (`GET /namespaces/{namespaceId}/concurrency-keys` and `.../rate-limit-keys`; plan "Visibility of the rate gate").
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
use sagat_domain::workflow::{GateLevel, RatePeriod, ResolvedGroup};
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::keyed_digest::KeyRing;

/// The run-level concurrency permit of a run as its rows say now: held, or waited for until a queue deadline. `None` on
/// a run whose definition declares no run-level `concurrency`, and on one that no longer holds or waits (ended,
/// timed out, dequeued): the journal keeps those facts.
///
/// Its key's label is hidden when it is read ([`Self::label`] is `None`); only [`Self::reveal_label`], which the run
/// service calls for a caller who may reveal personal data in the run's namespace, shows it (fail closed, D022-060).
#[derive(Clone, PartialEq, Eq)]
pub struct RunPermit {
    pub pool: GatePool,
    pub group: ResolvedGroup,
    pub state: RunGateState,
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
        state: RunGateState,
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

/// Prints the key as its label only, revealed or not, never the digest (NFR-022-002: logs).
impl std::fmt::Debug for RunPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunPermit")
            .field("pool", &self.pool)
            .field("group", &self.group)
            .field("state", &self.state)
            .field("since", &self.since)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Whether a run's start holds what its gate gives (a permit, or a slot that still counts) or waits for it. Gate-neutral
/// (task T022-112, decision D022-068 item 6): each body names the state in its gate's words (`HOLDING` for a permit,
/// `GRANTED` for a slot, `WAITING` for both), so this type carries no word of either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunGateState {
    /// The permit is held, or the slot was granted and still counts.
    Held,
    /// Queued; the run is dequeued and fails with its gate's wait timeout at `deadline` if not granted before.
    Waiting { deadline: OffsetDateTime },
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
    /// Whether the page lists keys (`items`) or groups (`groups`): keys only to a caller who may reveal personal data
    /// in the namespace, groups to every other (decision D022-086).
    pub view: KeyView,
    pub items: Vec<ConcurrencyKeyView>,
    pub groups: Vec<ConcurrencyGroupView>,
    pub queued_runs: u64,
    /// The digest key version the database had active in the page's snapshot, which seals its cursor (D022-059); `None`
    /// before any version was activated (then no key exists either).
    pub cursor_version: Option<u16>,
    /// The group view's own place to resume at, `None` after its last group: the place after its last group, or a group
    /// it read in part (decision D022-086 item 5). The key view resumes after its last item.
    pub next: Option<KeyPosition>,
}

/// The run-level rate-limit slot of a run as its rows say now (`Run.rateLimit`, FR-022-048, decision D022-067 item 1):
/// granted while it counts (less than its window since its grant), or waited for until a queue deadline. `None` on a
/// run whose definition declares no run-level `rateLimit`, and on one whose slot no longer counts or that no longer
/// waits (timed out, dequeued): the journal keeps those facts. Its label is hidden until [`Self::reveal_label`], as a
/// permit's; its position and estimate are set only by a single-run read ([`Self::place`]).
#[derive(Clone, PartialEq, Eq)]
pub struct RunRateSlot {
    pub pool: GatePool,
    pub group: ResolvedGroup,
    pub state: RunGateState,
    /// The database clock of the statement that granted the slot or buffered the run.
    pub since: OffsetDateTime,
    /// The effective limit the row was decided with.
    pub limit: u32,
    pub per: RatePeriod,
    /// The run's rank among the eligible waiters of its key (1 is next), in a single-run read of a waiting run.
    pub position: Option<u32>,
    /// When enough counted grants age out for that rank, or the read's instant when there is room; in a single-run read
    /// of a waiting run, and `None` when it cannot be computed (D022-014).
    pub estimated_grant_at: Option<OffsetDateTime>,
    label: Option<KeyLabel>,
    digest: [u8; 32],
}

impl RunRateSlot {
    /// A slot as its rows say, its label hidden and its place unread.
    #[must_use]
    pub const fn new(
        pool: GatePool,
        group: ResolvedGroup,
        digest: [u8; 32],
        state: RunGateState,
        since: OffsetDateTime,
        (limit, per): (u32, RatePeriod),
    ) -> Self {
        Self {
            pool,
            group,
            state,
            since,
            limit,
            per,
            position: None,
            estimated_grant_at: None,
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

    /// Sets the place a single-run read computed for a waiting run, and the limit its grant is decided with (the lower
    /// of its filed limit and the namespace's current ceiling, decision D022-075 item 3).
    pub fn place(&mut self, place: RatePlace) {
        self.position = Some(place.position);
        self.estimated_grant_at = place.estimated_grant_at;
        self.limit = place.limit;
    }
}

/// Prints the key as its label only, revealed or not, never the digest (NFR-022-002: logs).
impl std::fmt::Debug for RunRateSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRateSlot")
            .field("pool", &self.pool)
            .field("group", &self.group)
            .field("state", &self.state)
            .field("since", &self.since)
            .field("limit", &self.limit)
            .field("per", &self.per)
            .field("position", &self.position)
            .field("estimated_grant_at", &self.estimated_grant_at)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Where a waiting run stands in its key's queue at one instant `t` (plan "Visibility of the rate gate", D022-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatePlace {
    /// `a + 1`, with `a` the waiters ahead of it that the pass visits (not past their deadline, their run live and not
    /// paused, in the queue's order).
    pub position: u32,
    pub estimated_grant_at: Option<OffsetDateTime>,
    /// The limit the grant routine decides the waiter with.
    pub limit: u32,
}

/// What one statement read of a waiting run's place at one instant `t` (plan "Visibility of the rate gate", D022-014).
/// The statement guarantees that `kth_lease` counts at `t` (`kth_lease > t - per`), which [`estimated_grant`] relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceReading {
    /// The statement's instant (`statement_timestamp()`).
    pub t: OffsetDateTime,
    /// The key's grants that count over the waiter's window `per` at `t`.
    pub counted: u64,
    /// The waiters ahead of it that the pass visits (not past their deadline, their run running or waiting and not
    /// paused, in the queue's order).
    pub ahead: u64,
    /// The waiter's own effective limit.
    pub limit: u32,
    /// The lease instant of the `k`-th oldest counted grant for `k = counted + ahead - limit + 1`, when
    /// `1 <= k <= counted`.
    pub kth_lease: Option<OffsetDateTime>,
    pub per: RatePeriod,
    /// The waiter's own queue deadline.
    pub deadline: OffsetDateTime,
    /// Whether the pass skips the waiter's run: paused, or no longer running or waiting (an orphan row).
    pub skipped: bool,
}

impl PlaceReading {
    /// The place this reading gives: the position `ahead + 1` and the estimate of [`estimated_grant`].
    #[must_use]
    pub fn place(&self) -> RatePlace {
        RatePlace {
            position: u32::try_from(self.ahead.saturating_add(1)).unwrap_or(u32::MAX),
            estimated_grant_at: estimated_grant(self),
            limit: self.limit,
        }
    }
}

/// The estimate of a waiter's grant from what one statement read at `t` (plan "Visibility of the rate gate"). The grant
/// routine grants it when `counted + ahead < limit`; a grant to a waiter ahead moves one from `ahead` to `counted`, so
/// the sum falls only when a counted grant ages out, at its lease plus `per` (the half-open window). `t` when `k <= 0`
/// (room now), the `k`-th lease plus `per` when `1 <= k <= counted`, `None` when `k > counted` (beyond one window).
/// `None` too when the pass will not grant this waiter (decision D022-068 item 1): its run is skipped (paused or ended),
/// its deadline is at or before `t`, or the instant is at or after its deadline (the pass grants only while
/// `deadline > now()`, so at the deadline exactly the waiter times out first).
#[must_use]
pub fn estimated_grant(reading: &PlaceReading) -> Option<OffsetDateTime> {
    // a deadline at or before `t` needs no test of its own: every instant below is at or after `t` (a counted lease is
    // after `t - per`)
    debug_assert!(
        reading
            .kth_lease
            .is_none_or(|lease| lease + reading.per.duration() > reading.t),
        "a counted lease counts at the reading's instant"
    );
    // the domain's one rule of the estimate (task T022-071), in microseconds on the database clock
    let micros = |at: OffsetDateTime| i64::try_from(at.unix_timestamp_nanos() / 1_000).ok();
    let estimate =
        sagat_domain::execution::window::estimate_us(sagat_domain::execution::window::Reading {
            at_us: micros(reading.t)?,
            counted: reading.counted,
            ahead: reading.ahead,
            limit: reading.limit,
            kth_lease_us: reading.kth_lease.and_then(micros),
            per: reading.per,
            deadline_us: micros(reading.deadline)?,
            skipped: reading.skipped,
        })?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(estimate) * 1_000).ok()
}

/// One rate-limit window row of a namespace, read without its lock at the page's instant (FR-022-048, D022-067).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitKeyView {
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    /// `None` unless the reader may reveal personal data in the namespace; the service sets it.
    pub label: Option<KeyLabel>,
    /// Grants that count over the row's longest period: unleased, or leased less than that period ago.
    pub grants_in_window: u32,
    pub unleased_grants: u32,
    pub waiters: u32,
    pub oldest_wait_since: Option<OffsetDateTime>,
    /// When somebody waits and no grant is unleased: the first instant a counted grant stops counting for one of the
    /// waiters (each over its own window).
    pub next_slot_at: Option<OffsetDateTime>,
    /// Where a page that ends with this key resumes.
    pub position: KeyPosition,
}

/// A page of rate-limit windows, oldest wait first, and the namespace's queued runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitKeyPage {
    /// As [`ConcurrencyKeyPage::view`].
    pub view: KeyView,
    pub items: Vec<RateLimitKeyView>,
    pub groups: Vec<RateLimitGroupView>,
    pub queued_runs: u64,
    /// The digest key version the database had active in the page's snapshot, which seals its cursor.
    pub cursor_version: Option<u16>,
    /// As [`ConcurrencyKeyPage::next`].
    pub next: Option<KeyPosition>,
}

/// What a page of a key listing lists (FR-022-022, FR-022-057, decisions D022-061 and D022-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyView {
    /// One row per key, by its label, with its whole instants: for a caller who may reveal personal data in the
    /// namespace, which covers every level a published key may read (decision D022-085 item 6).
    Keys,
    /// One row per pool, scope and group, its keys summed, with whole instants: for every other caller. A group is the
    /// definition's name or a declared slug, already on each run, so a group row tells nothing a run reader lacks and
    /// links no run to another (decision D022-061 (b)).
    Groups,
}

/// The most key rows a page of a group view reads (decision D022-086 item 5): the per-key page's bound. A group with more
/// keys is summed across the pages that read it.
pub const GROUP_PAGE_KEY_ROWS: u32 = 200;

/// One group of a namespace's concurrency keys, summed over its keys across the pages that read it, each page one
/// snapshot (decision D022-086 item 5). It never says how many keys the group has: that count tells which runs share a
/// key (batch 4 reviews, security Major 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyGroupView {
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    pub holders: u64,
    pub waiters: u64,
    pub oldest_wait_since: Option<OffsetDateTime>,
    pub oldest_holder_since: Option<OffsetDateTime>,
    /// Whether an earlier page began its sums: they then mix the snapshots of the pages that read it (decision D022-101).
    pub spans_pages: bool,
    /// Where a page that ends with this group resumes (its digest is all zero: groups are ordered by pool, scope and
    /// name).
    pub position: KeyPosition,
}

/// One group of a namespace's rate-limit windows, summed over its windows across the pages that read it, each at its
/// page's instant (decision D022-086 item 5). Neither how many windows it has nor when its next slot frees: the first
/// tells which runs share a key, the second links a waiting run to a granted one (batch 4 reviews, security Major 1 and
/// minor 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitGroupView {
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    pub grants_in_window: u64,
    pub unleased_grants: u64,
    pub waiters: u64,
    pub oldest_wait_since: Option<OffsetDateTime>,
    /// Whether an earlier page began its sums: they then mix the snapshots of the pages that read it (decision D022-101).
    pub spans_pages: bool,
    pub position: KeyPosition,
}

/// What a page of a group view has read of a group it did not finish (decision D022-086 item 5): its sums (holders and
/// waiters of a concurrency group; counted grants, unleased grants and waiters of a rate group) and its oldest instants
/// in microseconds since the epoch (the oldest wait; the oldest holder of a concurrency group). Carried in the sealed
/// cursor, never shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupPartial {
    pub sums: [u64; 3],
    pub oldest_us: [Option<i64>; 2],
}

impl GroupPartial {
    /// Adds what one key row adds.
    pub fn add(&mut self, sums: [u64; 3], oldest_us: [Option<i64>; 2]) {
        for (total, value) in self.sums.iter_mut().zip(sums) {
            *total = total.saturating_add(value);
        }
        for (oldest, value) in self.oldest_us.iter_mut().zip(oldest_us) {
            *oldest = match (*oldest, value) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
    }

    /// An instant of [`Self::oldest_us`].
    #[must_use]
    pub fn instant(&self, index: usize) -> Option<OffsetDateTime> {
        self.oldest_us
            .get(index)
            .copied()
            .flatten()
            .and_then(|us| OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000).ok())
    }
}

/// Which listing a place belongs to: the cursor of one is refused by every other (decision D022-067 item 5), the key
/// and group views of one listing included (decision D022-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyListing {
    Concurrency,
    RateLimit,
    ConcurrencyGroups,
    RateLimitGroups,
}

impl KeyListing {
    const fn tag(self) -> &'static str {
        match self {
            Self::Concurrency => "k1",
            Self::RateLimit => "r1",
            Self::ConcurrencyGroups => "kg1",
            Self::RateLimitGroups => "rg1",
        }
    }

    /// The listing of this gate in `view`.
    #[must_use]
    pub const fn of(self, view: KeyView) -> Self {
        match (self, view) {
            (Self::Concurrency | Self::ConcurrencyGroups, KeyView::Keys) => Self::Concurrency,
            (Self::Concurrency | Self::ConcurrencyGroups, KeyView::Groups) => {
                Self::ConcurrencyGroups
            }
            (Self::RateLimit | Self::RateLimitGroups, KeyView::Keys) => Self::RateLimit,
            (Self::RateLimit | Self::RateLimitGroups, KeyView::Groups) => Self::RateLimitGroups,
        }
    }
}

/// How long a key listing's cursor is read after its page sealed it (decision D022-100): a cursor of a group view carries
/// the sums of a group read in part, which a later page must not add to rows read much later, and a page boundary kept
/// for long would tell whether a group still crosses the bound of a page.
pub const KEY_CURSOR_LIFETIME: time::Duration = time::Duration::minutes(15);

/// The position a cursor names, the view of the listing that wrote it (decision D022-086), and the instant its page
/// sealed it, in whole seconds (decision D022-100).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingPosition {
    pub view: KeyView,
    pub position: KeyPosition,
    pub sealed_at: OffsetDateTime,
}

impl ListingPosition {
    /// Whether the cursor has outlived [`KEY_CURSOR_LIFETIME`] at `now` (decision D022-100). A cursor sealed ahead of
    /// `now` (another process whose clock is ahead) is read: processes' clocks differ by far less than the lifetime.
    #[must_use]
    pub fn expired(&self, now: OffsetDateTime) -> bool {
        now - self.sealed_at > KEY_CURSOR_LIFETIME
    }
}

/// The place of a key in the listing's order: oldest wait first (a key nobody waits for last), then pool, scope, group
/// and digest, a total order since those five are the row's key within a namespace (D022-055).
///
/// It carries the key's digest, not only its label: two keys of one group may share a label (48 bits), and a position
/// that could not tell them apart would skip or repeat one at a page boundary. A caller never sees it: the cursor is the
/// position sealed (encrypted and authenticated) for the namespace under the deployment's flow-control key
/// ([`Self::seal`], D022-059), so the cursor of a page whose labels are hidden discloses no label either.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyPosition {
    /// The oldest wait of the key in microseconds since the epoch, `None` for a key nobody waits for.
    pub oldest_wait_us: Option<i64>,
    pub pool: GatePool,
    pub scope: GateLevel,
    pub group: ResolvedGroup,
    pub digest: [u8; 32],
    /// In a group view, a group read in part: `digest` is then the last key read and these its sums so far (decision
    /// D022-086 item 5). `None` in a key view and for a group read whole.
    pub partial: Option<GroupPartial>,
}

/// Prints the key as its label, never the digest (NFR-022-002: logs).
impl std::fmt::Debug for KeyPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPosition")
            .field("oldest_wait_us", &self.oldest_wait_us)
            .field("pool", &self.pool)
            .field("scope", &self.scope)
            .field("group", &self.group)
            .field("key", &KeyLabel::of_digest(&self.digest))
            .field("partial", &self.partial)
            .finish()
    }
}

impl KeyPosition {
    /// The cursor a caller is given: the position sealed for this namespace of this tenant under `version`, the digest
    /// key version the database has active, with `sealed_at`, the instant of its page, in whole seconds (decision
    /// D022-100). `None` when this process does not hold that version (it cannot then hand out a cursor; the service
    /// refuses the page, D022-059).
    #[must_use]
    pub fn seal(
        &self,
        listing: KeyListing,
        ring: &KeyRing,
        version: u16,
        tenant: TenantId,
        namespace: NamespaceId,
        sealed_at: OffsetDateTime,
    ) -> Option<String> {
        let plain = format!("{}.{}", sealed_at.unix_timestamp(), self.encode(listing));
        ring.seal_cursor(version, tenant, namespace, plain.as_bytes())
    }

    /// The position a cursor [`Self::seal`] wrote for this namespace of this tenant, whatever its age; `None` for
    /// anything else.
    #[must_use]
    pub fn open(
        text: &str,
        listing: KeyListing,
        ring: &KeyRing,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Option<Self> {
        let (_, plain) = Self::unseal(text, ring, tenant, namespace)?;
        Self::decode(&plain, listing)
    }

    /// The position a cursor of either view of the listing of `gate` names (its key view's or its group view's), with
    /// that view and the instant its page sealed it; `None` for anything else. Whether it has expired is
    /// [`ListingPosition::expired`].
    #[must_use]
    pub fn open_either(
        text: &str,
        gate: KeyListing,
        ring: &KeyRing,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Option<ListingPosition> {
        let (sealed_at, plain) = Self::unseal(text, ring, tenant, namespace)?;
        [KeyView::Keys, KeyView::Groups]
            .into_iter()
            .find_map(|view| {
                Self::decode(&plain, gate.of(view)).map(|position| ListingPosition {
                    view,
                    position,
                    sealed_at,
                })
            })
    }

    /// The instant and the position's text a cursor sealed for this namespace of this tenant holds.
    fn unseal(
        text: &str,
        ring: &KeyRing,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Option<(OffsetDateTime, String)> {
        let plain = String::from_utf8(ring.open_cursor(tenant, namespace, text)?).ok()?;
        let (sealed_at, position) = plain.split_once('.')?;
        // whole seconds since the epoch, digits only (no sign)
        if !sealed_at.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let sealed_at = OffsetDateTime::from_unix_timestamp(sealed_at.parse().ok()?).ok()?;
        Some((sealed_at, position.to_owned()))
    }

    /// The place of a group in the group view's order (pool, scope and name; the digest all zero).
    #[must_use]
    pub const fn of_group(pool: GatePool, scope: GateLevel, group: ResolvedGroup) -> Self {
        Self {
            oldest_wait_us: None,
            pool,
            scope,
            group,
            digest: [0; 32],
            partial: None,
        }
    }

    /// The place of a group read in part, up to and including the key `digest` (decision D022-086 item 5).
    #[must_use]
    pub const fn in_group(
        pool: GatePool,
        scope: GateLevel,
        group: ResolvedGroup,
        digest: [u8; 32],
        partial: GroupPartial,
    ) -> Self {
        Self {
            oldest_wait_us: None,
            pool,
            scope,
            group,
            digest,
            partial: Some(partial),
        }
    }

    /// The position's text before it is sealed, tagged with its listing.
    fn encode(&self, listing: KeyListing) -> String {
        let mut text = String::with_capacity(160);
        text.push_str(listing.tag());
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
        if let Some(partial) = &self.partial {
            for sum in partial.sums {
                let _ = write!(text, ".{sum}");
            }
            for oldest in partial.oldest_us {
                match oldest {
                    Some(us) => {
                        let _ = write!(text, ".{us}");
                    }
                    None => text.push_str(".n"),
                }
            }
        }
        text
    }

    /// Reads a position back; `None` for anything [`Self::encode`] did not write.
    fn decode(text: &str, listing: KeyListing) -> Option<Self> {
        let mut parts = text.split('.');
        if parts.next()? != listing.tag() {
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
        let rest: Vec<&str> = parts.collect();
        let partial = match rest.as_slice() {
            [] => None,
            // only a group view carries a group read in part
            [a, b, c, d, e]
                if matches!(
                    listing,
                    KeyListing::ConcurrencyGroups | KeyListing::RateLimitGroups
                ) =>
            {
                let instant = |text: &str| -> Option<Option<i64>> {
                    if text == "n" {
                        return Some(None);
                    }
                    let us = text.parse::<i64>().ok()?;
                    OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000).ok()?;
                    Some(Some(us))
                };
                Some(GroupPartial {
                    sums: [a.parse().ok()?, b.parse().ok()?, c.parse().ok()?],
                    oldest_us: [instant(d)?, instant(e)?],
                })
            }
            _ => return None,
        };
        Some(Self {
            oldest_wait_us,
            pool,
            scope,
            group,
            digest,
            partial,
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

    /// The namespace's rate-limit windows after `after` (from the first when `None`), at most `limit`, optionally of one
    /// group, with the namespace's queued runs (FR-022-048). Every label is `None`: the service decides who sees them.
    fn rate_limit_keys(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&KeyPosition>,
        limit: u32,
    ) -> impl Future<Output = Result<RateLimitKeyPage, StoreError>> + Send;

    /// The namespace's concurrency groups after `after` (a group's place, from the first when `None`), at most `limit`,
    /// optionally the one group named, each summed over its keys in one snapshot, with the namespace's queued runs
    /// (decision D022-086). A page reads its own groups' keys, never another group's.
    fn concurrency_key_groups(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&KeyPosition>,
        limit: u32,
    ) -> impl Future<Output = Result<ConcurrencyKeyPage, StoreError>> + Send;

    /// The same for the rate-limit windows, at the page's instant (decision D022-086).
    fn rate_limit_key_groups(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&KeyPosition>,
        limit: u32,
    ) -> impl Future<Output = Result<RateLimitKeyPage, StoreError>> + Send;

    /// What one statement read, at one instant, of where the run-level rate-limit waiter of `run` stands; `None` when the
    /// run does not wait for a slot. The service derives the place from it ([`PlaceReading::place`]).
    fn rate_place(
        &self,
        tenant: TenantId,
        run: sagat_domain::execution::RunId,
    ) -> impl Future<Output = Result<Option<PlaceReading>, StoreError>> + Send;

    /// Whether the window of the rate-limit key that the run-level slot of `run` took (or waits for) is full now: its
    /// grants that count over the start's window reach the lower of `declared_limit` and the namespace's current
    /// ceiling of that window length. `None` when the run has no row of its slot. Read in its own read-only
    /// transaction; advisory only (decision D022-079: a `CANCEL_SOURCE` reset is refused at its request on it).
    fn start_window_full(
        &self,
        tenant: TenantId,
        run: sagat_domain::execution::RunId,
        declared_limit: u32,
    ) -> impl Future<Output = Result<Option<bool>, StoreError>> + Send;

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

    /// Whether the database has digest key `version` in force now (active, or retiring with rows left): a cursor sealed
    /// under any other version is refused, a forced retirement's included, whatever ring the process holds.
    fn key_version_in_force(
        &self,
        version: u16,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
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
            partial: None,
        }
    }

    /// The instant the tests' cursors are sealed at, 2026-11-25 12:00 UTC.
    fn page_instant() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_795_608_000).expect("an instant")
    }

    /// Decision D022-100 (batch 5 security suspicion 3): a cursor carries the instant of its page, sealed, and expires
    /// fifteen minutes after it; one sealed ahead of the reader's clock is read; an instant that is not whole seconds
    /// written by [`KeyPosition::seal`] is no cursor.
    #[test]
    fn a_cursor_expires_fifteen_minutes_after_its_page() {
        let ring = KeyRing::parse(RING).expect("ring");
        let (tenant, namespace) = (TenantId::generate(), NamespaceId::generate());
        let p = position(Some(1_763_640_000_123_456), "payments.eu");
        let sealed_at = page_instant() + time::Duration::milliseconds(700);
        let cursor = p
            .seal(
                KeyListing::Concurrency,
                &ring,
                1,
                tenant,
                namespace,
                sealed_at,
            )
            .expect("sealed");
        let opened =
            KeyPosition::open_either(&cursor, KeyListing::Concurrency, &ring, tenant, namespace)
                .expect("opened");
        assert_eq!(
            (opened.view, &opened.position, opened.sealed_at),
            (KeyView::Keys, &p, page_instant()),
            "the page's instant, in whole seconds"
        );
        for (now, expired) in [
            (page_instant() - time::Duration::minutes(1), false),
            (page_instant(), false),
            (page_instant() + KEY_CURSOR_LIFETIME, false),
            (
                page_instant() + KEY_CURSOR_LIFETIME + time::Duration::seconds(1),
                true,
            ),
            (page_instant() + time::Duration::hours(1), true),
        ] {
            assert_eq!(opened.expired(now), expired, "{now}");
        }
        // the instant is inside the sealed text: a forged one is no cursor
        for instant in ["-1", "+1", "", "1.5"] {
            let forged = ring
                .seal_cursor(
                    1,
                    tenant,
                    namespace,
                    format!("{instant}.{}", p.encode(KeyListing::Concurrency)).as_bytes(),
                )
                .expect("sealed");
            assert_eq!(
                KeyPosition::open_either(
                    &forged,
                    KeyListing::Concurrency,
                    &ring,
                    tenant,
                    namespace
                ),
                None,
                "{instant:?}"
            );
        }
    }

    /// Decision D022-067 item 5 (review of task T022-078, architecture minor 7): a position written for one listing is
    /// refused by the other, both ways.
    #[test]
    fn a_position_of_one_listing_is_refused_by_the_other() {
        let p = position(Some(1_763_640_000_123_456), "payments.eu");
        for (written, read) in [
            (KeyListing::Concurrency, KeyListing::RateLimit),
            (KeyListing::RateLimit, KeyListing::Concurrency),
        ] {
            assert_eq!(
                KeyPosition::decode(&p.encode(written), read),
                None,
                "{written:?} read as {read:?}"
            );
            assert_eq!(
                KeyPosition::decode(&p.encode(written), written),
                Some(p.clone())
            );
        }
    }

    const MICROSECOND: time::Duration = time::Duration::microseconds(1);

    /// A reading at `t` of a key with `counted` grants that count and `ahead` waiters ahead, limit 3 per hour, whose
    /// `k`-th counted lease is one minute before `t`, and whose deadline is two hours after `t`.
    fn reading(counted: u64, ahead: u64) -> PlaceReading {
        let t = OffsetDateTime::from_unix_timestamp(1_763_640_000).expect("an instant");
        PlaceReading {
            t,
            counted,
            ahead,
            limit: 3,
            kth_lease: Some(t - time::Duration::minutes(1)),
            per: RatePeriod::from_millis(3_600_000).expect("an hour"),
            deadline: t + time::Duration::hours(2),
            skipped: false,
        }
    }

    /// Plan "Visibility of the rate gate": `k = counted + ahead - limit + 1` at each of its edges. `k <= 0` is room now
    /// (`t`); `k = 1` and `k = counted` are the `k`-th lease plus the window; `k = counted + 1` is beyond one window.
    #[test]
    fn the_estimate_is_the_kth_lease_plus_the_window_at_each_edge_of_k() {
        let r = reading(0, 0);
        assert_eq!(estimated_grant(&r), Some(r.t), "k = -2: room now");
        let r = reading(1, 1);
        assert_eq!(estimated_grant(&r), Some(r.t), "k = 0: the last room");
        let lease_plus_hour = |r: &PlaceReading| r.kth_lease.map(|l| l + time::Duration::hours(1));
        let r = reading(2, 1);
        assert_eq!(estimated_grant(&r), lease_plus_hour(&r), "k = 1");
        let r = reading(3, 2);
        assert_eq!(estimated_grant(&r), lease_plus_hour(&r), "k = 3 = counted");
        let r = reading(3, 3);
        assert_eq!(
            estimated_grant(&r),
            None,
            "k = 4 = counted + 1: beyond one window"
        );
        let r = reading(0, 3);
        assert_eq!(
            estimated_grant(&r),
            None,
            "k = 1 with nothing counted: beyond one window"
        );
        let r = PlaceReading {
            kth_lease: None,
            ..reading(2, 1)
        };
        assert_eq!(
            estimated_grant(&r),
            None,
            "a lease the statement did not find"
        );
        let r = PlaceReading {
            counted: u64::MAX,
            ahead: u64::MAX,
            ..reading(0, 0)
        };
        assert_eq!(
            estimated_grant(&r),
            lease_plus_hour(&r),
            "a saturated sum stays within the counted grants"
        );
    }

    /// Decision D022-068 item 1: no estimate for a waiter the pass will not grant: paused, past its own deadline at
    /// `t`, or an instant at or after its deadline; one microsecond either side of each boundary.
    #[test]
    fn no_estimate_for_a_waiter_the_pass_will_not_grant() {
        let r = PlaceReading {
            skipped: true,
            ..reading(0, 0)
        };
        assert_eq!(estimated_grant(&r), None, "skipped");
        let base = reading(0, 0);
        let r = PlaceReading {
            deadline: base.t,
            ..base
        };
        assert_eq!(
            estimated_grant(&r),
            None,
            "its deadline is the read's instant"
        );
        let r = PlaceReading {
            deadline: base.t + MICROSECOND,
            ..base
        };
        assert_eq!(
            estimated_grant(&r),
            Some(base.t),
            "one microsecond before its deadline: room now"
        );
        let base = reading(2, 1);
        let at = base.kth_lease.expect("a lease") + time::Duration::hours(1);
        let r = PlaceReading {
            deadline: at,
            ..base
        };
        assert_eq!(
            estimated_grant(&r),
            None,
            "the instant is its deadline: it times out first"
        );
        let r = PlaceReading {
            deadline: at - MICROSECOND,
            ..base
        };
        assert_eq!(
            estimated_grant(&r),
            None,
            "the instant is after its deadline"
        );
        let r = PlaceReading {
            deadline: at + MICROSECOND,
            ..base
        };
        assert_eq!(
            estimated_grant(&r),
            Some(at),
            "one microsecond before its deadline"
        );
    }

    #[test]
    fn a_position_reads_back_what_it_was_written_from() {
        for p in [
            position(Some(1_763_640_000_123_456), "payments.eu"),
            position(None, "=order-intake/charge"),
            position(Some(0), "=order-intake"),
        ] {
            assert_eq!(
                KeyPosition::decode(&p.encode(KeyListing::Concurrency), KeyListing::Concurrency),
                Some(p)
            );
        }
    }

    #[test]
    fn a_position_it_did_not_write_is_refused() {
        let good = position(None, "payments").encode(KeyListing::Concurrency);
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
            assert_eq!(
                KeyPosition::decode(&bad, KeyListing::Concurrency),
                None,
                "{bad}"
            );
        }
    }

    /// Keys review (2026-11-20), security M2 and architecture M1: the cursor carries no digest a caller can read, is
    /// refused altered, and is refused in another namespace.
    #[test]
    fn a_cursor_is_the_position_sealed_for_its_namespace() {
        let ring = KeyRing::parse(RING).expect("ring");
        let (tenant, namespace) = (TenantId::generate(), NamespaceId::generate());
        let p = position(Some(1_763_640_000_123_456), "payments.eu");
        let cursor = p
            .seal(
                KeyListing::Concurrency,
                &ring,
                1,
                tenant,
                namespace,
                page_instant(),
            )
            .expect("sealed");
        assert!(!cursor.contains(&hex(&p.digest)), "{cursor}");
        assert!(!cursor.contains(&hex(&p.digest[..6])), "{cursor}");
        assert!(
            cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
            "{cursor}"
        );
        assert_eq!(
            KeyPosition::open(&cursor, KeyListing::Concurrency, &ring, tenant, namespace),
            Some(p.clone())
        );
        assert_eq!(
            KeyPosition::open(
                &cursor,
                KeyListing::Concurrency,
                &ring,
                tenant,
                NamespaceId::generate()
            ),
            None
        );
        // the plain position a caller might forge is no cursor
        assert_eq!(
            KeyPosition::open(
                &p.encode(KeyListing::Concurrency),
                KeyListing::Concurrency,
                &ring,
                tenant,
                namespace
            ),
            None
        );
        assert_eq!(
            p.seal(
                KeyListing::Concurrency,
                &KeyRing::default(),
                1,
                tenant,
                namespace,
                page_instant()
            ),
            None
        );
        // what is printed carries the label at most, never the digest
        assert!(!format!("{p:?}").contains(&hex(&p.digest)));
    }

    /// Keys review (2026-11-20, correctness M1): the longest position a listing writes (a derived group of 130 bytes,
    /// `DRAFT_TEST`, `STEP`, the largest instant) seals and opens.
    #[test]
    fn the_longest_position_seals_and_opens() {
        let ring = KeyRing::parse(RING).expect("ring");
        let (tenant, namespace) = (TenantId::generate(), NamespaceId::generate());
        // the longest derived group the grammar accepts (at most 130 bytes)
        let group = (1..=64)
            .rev()
            .flat_map(|slug| (1..=64).rev().map(move |node| (slug, node)))
            .map(|(slug, node)| format!("=a{}/N{}", "b".repeat(slug - 1), "c".repeat(node - 1)))
            .find_map(|text| ResolvedGroup::parse(&text).ok())
            .expect("a derived group");
        assert!(group.as_str().len() >= 120, "{}", group.as_str());
        let p = KeyPosition {
            oldest_wait_us: Some(i64::MAX / 1_000_000 * 1_000),
            pool: GatePool::DraftTest,
            scope: GateLevel::Step,
            group,
            digest: [0xff; 32],
            partial: None,
        };
        assert!(
            p.encode(KeyListing::Concurrency).len() <= crate::keyed_digest::MAX_CURSOR_PLAIN_BYTES
        );
        let cursor = p
            .seal(
                KeyListing::Concurrency,
                &ring,
                1,
                tenant,
                namespace,
                page_instant(),
            )
            .expect("sealed");
        assert_eq!(
            KeyPosition::open(&cursor, KeyListing::Concurrency, &ring, tenant, namespace),
            Some(p.clone())
        );
        // a group view's place of a group read in part, its sums and instants at their longest (decision D022-086 item 5)
        let longest = KeyPosition::in_group(
            p.pool,
            p.scope,
            p.group.clone(),
            p.digest,
            GroupPartial {
                sums: [u64::MAX; 3],
                oldest_us: [Some(i64::MAX / 1_000_000 * 1_000); 2],
            },
        );
        for listing in [KeyListing::ConcurrencyGroups, KeyListing::RateLimitGroups] {
            assert!(
                longest.encode(listing).len() <= crate::keyed_digest::MAX_CURSOR_PLAIN_BYTES,
                "{}",
                longest.encode(listing).len()
            );
            let cursor = longest
                .seal(listing, &ring, 1, tenant, namespace, page_instant())
                .expect("sealed");
            assert_eq!(
                KeyPosition::open(&cursor, listing, &ring, tenant, namespace),
                Some(longest.clone())
            );
        }
        // a key view never carries a group read in part
        assert_eq!(
            KeyPosition::decode(
                &longest.encode(KeyListing::Concurrency),
                KeyListing::Concurrency
            ),
            None
        );
    }

    #[test]
    fn a_permit_hides_its_label_until_it_is_revealed() {
        let mut permit = RunPermit::new(
            GatePool::Ordinary,
            ResolvedGroup::parse("payments").expect("a group"),
            [0xab; 32],
            RunGateState::Held,
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(permit.label(), None);
        permit.reveal_label();
        assert_eq!(permit.label(), Some(&KeyLabel::of_digest(&[0xab; 32])));
    }
}
