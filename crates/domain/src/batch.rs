//! Batch operations (feature 018): one action applied to many runs, one run per transaction, at a
//! durable rate. This module holds the action set and its parameters, the selector, the limits, the
//! state machines of a job and of an item, the counters, the rate slot and the retry of a refused
//! item. Selecting and applying are the store's and the executor's work.
//!
//! A batch never holds two run locks and never undoes anything: cancelling a job stops new items and
//! marks the rest `CANCELLED`, and the items already applied stay applied (FR-018-032).

use std::fmt;
use std::str::FromStr;

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::business_ids::BusinessId;
use crate::execution::RunStatus;

pub const MAX_RUN_IDS: usize = 1000;
pub const MAX_ITEMS_HARD: u32 = 100_000;
pub const DEFAULT_MAX_ITEMS: u32 = 10_000;
pub const MIN_REASON_CHARS: usize = 8;
pub const MAX_REASON_CHARS: usize = 2000;
pub const MAX_ORIGIN_CHARS: usize = 64;
/// 200 items a second, in thousandths of an item.
pub const MAX_RATE_MILLI: u32 = 200_000;
pub const DEFAULT_RATE_MILLI: u32 = 20_000;
pub const DEFAULT_ITEM_RETRIES: u32 = 10;
pub const MAX_ITEM_RETRIES: u32 = 50;
const FIRST_BACKOFF_SECONDS: u64 = 5;
const MAX_BACKOFF_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BatchError {
    #[error("the action needs, or refuses, a step id, a signal, an update or a repin")]
    ActionParameters,
    #[error("a reason is 8 to 2000 characters")]
    BadReason,
    #[error("the selector is malformed or over its ceiling")]
    SelectorInvalid,
    #[error("a job selects 1 to 100,000 items")]
    BadLimits,
    #[error("a rate is 0.001 to 200 items a second")]
    BadRate,
    #[error("that is not a transition of the state machine")]
    IllegalTransition,
    #[error("the counters cannot make that move")]
    CounterInvariant,
}

// --- actions ----------------------------------------------------------------------------------

/// What a job does to each selected run. The set only grows (D1021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Cancel,
    Signal,
    Update,
    Rerun,
    RetryStep,
    Pause,
    Resume,
    MarkManualReview,
    /// Only when feature 019 is enabled; otherwise `BATCH_ACTION_UNAVAILABLE`.
    Repin,
}

/// Which parameters a create request carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParamsGiven {
    pub step_id: bool,
    pub signal: bool,
    pub update: bool,
    pub repin: bool,
}

impl Action {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancel => "CANCEL",
            Self::Signal => "SIGNAL",
            Self::Update => "UPDATE",
            Self::Rerun => "RERUN",
            Self::RetryStep => "RETRY_STEP",
            Self::Pause => "PAUSE",
            Self::Resume => "RESUME",
            Self::MarkManualReview => "MARK_MANUAL_REVIEW",
            Self::Repin => "REPIN",
        }
    }

    /// Whether the action exists only with the worker deployments of feature 019.
    #[must_use]
    pub const fn needs_feature_019(self) -> bool {
        matches!(self, Self::Repin)
    }

    /// Each action takes exactly its own parameters: a step id for `RETRY_STEP` and
    /// `MARK_MANUAL_REVIEW`, a signal, an update or a repin target for the others that have one, and
    /// nothing for the rest.
    ///
    /// # Errors
    /// [`BatchError::ActionParameters`] when one is missing or one too many is given.
    pub fn check_params(self, given: ParamsGiven) -> Result<(), BatchError> {
        let wanted = match self {
            Self::Cancel | Self::Pause | Self::Resume | Self::Rerun => ParamsGiven::default(),
            Self::RetryStep | Self::MarkManualReview => ParamsGiven {
                step_id: true,
                ..ParamsGiven::default()
            },
            Self::Signal => ParamsGiven {
                signal: true,
                ..ParamsGiven::default()
            },
            Self::Update => ParamsGiven {
                update: true,
                ..ParamsGiven::default()
            },
            Self::Repin => ParamsGiven {
                repin: true,
                ..ParamsGiven::default()
            },
        };
        if given == wanted {
            Ok(())
        } else {
            Err(BatchError::ActionParameters)
        }
    }
}

impl FromStr for Action {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "CANCEL" => Self::Cancel,
            "SIGNAL" => Self::Signal,
            "UPDATE" => Self::Update,
            "RERUN" => Self::Rerun,
            "RETRY_STEP" => Self::RetryStep,
            "PAUSE" => Self::Pause,
            "RESUME" => Self::Resume,
            "MARK_MANUAL_REVIEW" => Self::MarkManualReview,
            "REPIN" => Self::Repin,
            _ => return Err(()),
        })
    }
}

/// Why the operator ran the batch: 8 to 2000 characters, kept in the audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Justification(String);

impl Justification {
    /// # Errors
    /// [`BatchError::BadReason`] outside 8 to 2000 characters.
    pub fn new(value: &str) -> Result<Self, BatchError> {
        if !(MIN_REASON_CHARS..=MAX_REASON_CHARS).contains(&value.chars().count()) {
            return Err(BatchError::BadReason);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// --- selection --------------------------------------------------------------------------------

/// The structured filter. Every field is optional and the filters AND together; an empty filter
/// selects every run created at or before the job, up to `maxItems` (the contract requires no field).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub workflow_id: Option<Uuid>,
    pub version_id: Option<Uuid>,
    pub statuses: Vec<RunStatus>,
    pub origin: Option<String>,
    pub schedule_id: Option<Uuid>,
    pub created_after: Option<OffsetDateTime>,
    pub created_before: Option<OffsetDateTime>,
    /// Customer text: kept out of logs by the `Debug` below.
    pub business_id_prefix: Option<String>,
}

impl fmt::Debug for Filter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Filter")
            .field("workflow_id", &self.workflow_id)
            .field("version_id", &self.version_id)
            .field("statuses", &self.statuses)
            .field("schedule_id", &self.schedule_id)
            .field("created_after", &self.created_after)
            .field("created_before", &self.created_before)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorKind {
    RunIds(Vec<Uuid>),
    Filter(Filter),
}

/// What a job selects, checked. Feature 020's query selector joins when 020 is enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(SelectorKind);

impl Selector {
    /// # Errors
    /// [`BatchError::SelectorInvalid`] unless there are 1 to 1,000 distinct ids.
    pub fn run_ids(ids: Vec<Uuid>) -> Result<Self, BatchError> {
        if ids.is_empty() || ids.len() > MAX_RUN_IDS {
            return Err(BatchError::SelectorInvalid);
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        if sorted.windows(2).any(|w| w[0] == w[1]) {
            return Err(BatchError::SelectorInvalid);
        }
        Ok(Self(SelectorKind::RunIds(ids)))
    }

    /// # Errors
    /// [`BatchError::SelectorInvalid`] for a repeated status, a creation range that is empty or
    /// backwards, an origin outside 1 to 64 visible ASCII characters, or a business-id prefix that is
    /// not 1 to 200 visible ASCII characters.
    pub fn filter(filter: Filter) -> Result<Self, BatchError> {
        let mut statuses: Vec<&str> = filter.statuses.iter().map(|s| s.as_str()).collect();
        statuses.sort_unstable();
        if statuses.windows(2).any(|w| w[0] == w[1]) {
            return Err(BatchError::SelectorInvalid);
        }
        if let (Some(after), Some(before)) = (filter.created_after, filter.created_before)
            && before <= after
        {
            return Err(BatchError::SelectorInvalid);
        }
        if let Some(origin) = &filter.origin
            && (origin.is_empty()
                || origin.chars().count() > MAX_ORIGIN_CHARS
                || !origin.chars().all(|c| c.is_ascii_graphic()))
        {
            return Err(BatchError::SelectorInvalid);
        }
        if let Some(prefix) = &filter.business_id_prefix
            && BusinessId::new(prefix).is_err()
        {
            return Err(BatchError::SelectorInvalid);
        }
        Ok(Self(SelectorKind::Filter(filter)))
    }

    #[must_use]
    pub const fn kind(&self) -> &SelectorKind {
        &self.0
    }
}

// --- limits and rate --------------------------------------------------------------------------

/// How fast a job applies items, in thousandths of an item a second: 0.001 to 200 items a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate(u32);

impl Rate {
    pub const MAX: Self = Self(MAX_RATE_MILLI);

    /// # Errors
    /// [`BatchError::BadRate`] outside 1 to 200,000 thousandths.
    pub fn from_milli(milli: u32) -> Result<Self, BatchError> {
        if !(1..=MAX_RATE_MILLI).contains(&milli) {
            return Err(BatchError::BadRate);
        }
        Ok(Self(milli))
    }

    /// # Errors
    /// [`BatchError::BadRate`] outside 1 to 200 items a second.
    pub fn per_second(items: u32) -> Result<Self, BatchError> {
        Self::from_milli(items.checked_mul(1000).ok_or(BatchError::BadRate)?)
    }

    #[must_use]
    pub const fn milli(self) -> u32 {
        self.0
    }

    /// The time one item takes at this rate.
    #[must_use]
    pub fn interval(self) -> Duration {
        Duration::microseconds(i64::from(1_000_000_000 / self.0))
    }
}

impl Default for Rate {
    fn default() -> Self {
        Self(DEFAULT_RATE_MILLI)
    }
}

/// What a job may ask for, and how a refused item is retried. The effective value is the lower of
/// the request, the namespace and the deployment, never above the hard ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_items: u32,
    pub rate: Rate,
    /// Refusals (quota, rate, lock wait, full queue) an item survives.
    pub item_retries: u32,
    /// How long after its first refusal an item is still retried.
    pub retry_window: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_items: DEFAULT_MAX_ITEMS,
            rate: Rate::default(),
            item_retries: DEFAULT_ITEM_RETRIES,
            retry_window: Duration::hours(1),
        }
    }
}

impl Limits {
    /// # Errors
    /// [`BatchError::BadLimits`] unless `max_items` is 1 to 100,000. The retry limits take their
    /// defaults.
    pub fn new(max_items: u32, rate: Rate) -> Result<Self, BatchError> {
        if !(1..=MAX_ITEMS_HARD).contains(&max_items) {
            return Err(BatchError::BadLimits);
        }
        Ok(Self {
            max_items,
            rate,
            ..Self::default()
        })
    }

    /// Held inside the hard ceilings: 100,000 items, 50 retries within 24 hours.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            max_items: self.max_items.clamp(1, MAX_ITEMS_HARD),
            rate: self.rate,
            item_retries: self.item_retries.min(MAX_ITEM_RETRIES),
            retry_window: self
                .retry_window
                .clamp(Duration::seconds(1), Duration::hours(24)),
        }
    }
}

/// The slot after the one an item used. It starts from the later of the slot and now, so a job that
/// ran late never catches up in a burst, and two items are never closer than one interval.
#[must_use]
pub fn next_slot(slot: OffsetDateTime, now: OffsetDateTime, rate: Rate) -> OffsetDateTime {
    slot.max(now) + rate.interval()
}

// --- retry ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Try again after this long.
    After(Duration),
    /// The item fails: too many refusals, or too long since the first.
    Exhausted,
}

/// What to do with an item refused for `attempts` times, the first at `first_refused_at`. The wait
/// starts at 5 seconds and doubles to 5 minutes, the same bounded backoff as a refused schedule
/// action.
#[must_use]
pub fn retry(
    attempts: u32,
    first_refused_at: OffsetDateTime,
    now: OffsetDateTime,
    limits: &Limits,
) -> Retry {
    let limits = limits.clamped();
    if attempts > limits.item_retries || now - first_refused_at >= limits.retry_window {
        return Retry::Exhausted;
    }
    let doublings = attempts.saturating_sub(1).min(16);
    let seconds = FIRST_BACKOFF_SECONDS
        .saturating_mul(1u64 << doublings)
        .min(MAX_BACKOFF_SECONDS);
    Retry::After(Duration::seconds(
        i64::try_from(seconds).unwrap_or(i64::MAX),
    ))
}

// --- the job ----------------------------------------------------------------------------------

/// Why a job failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// The creator lost a permission of the action while the job ran.
    AuthorityRevoked,
    SelectionFailed,
}

impl FailureReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityRevoked => "AUTHORITY_REVOKED",
            Self::SelectionFailed => "SELECTION_FAILED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Selecting,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobEvent {
    /// The selection reached `maxItems` or the end of the matching runs.
    SelectionFinished,
    /// Every selected item has ended.
    ItemsDone,
    CancelRequested,
    /// The items that were waiting are marked `CANCELLED` and the in-flight one has committed.
    CancelDrained,
    Failed(FailureReason),
}

impl JobState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Selecting => "SELECTING",
            Self::Running => "RUNNING",
            Self::Cancelling => "CANCELLING",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::Failed => "FAILED",
        }
    }

    /// Whether the job still counts against the active-job ceiling of its namespace and tenant.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Selecting | Self::Running | Self::Cancelling)
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !self.is_active()
    }

    /// Items are applied only while the job runs: not before the selection ends, not once a cancel
    /// is asked.
    #[must_use]
    pub const fn accepts_new_items(self) -> bool {
        matches!(self, Self::Running)
    }

    /// # Errors
    /// [`BatchError::IllegalTransition`] when the state machine has no such edge. A cancelling job
    /// does not fail: the cancel wins, and a finished job cannot be cancelled.
    pub const fn on(self, event: JobEvent) -> Result<Self, BatchError> {
        match (self, event) {
            (Self::Selecting, JobEvent::SelectionFinished) => Ok(Self::Running),
            (Self::Running, JobEvent::ItemsDone) => Ok(Self::Completed),
            (Self::Selecting | Self::Running, JobEvent::CancelRequested) => Ok(Self::Cancelling),
            (Self::Cancelling, JobEvent::CancelDrained) => Ok(Self::Cancelled),
            (Self::Selecting | Self::Running, JobEvent::Failed(_)) => Ok(Self::Failed),
            (
                Self::Selecting
                | Self::Running
                | Self::Cancelling
                | Self::Completed
                | Self::Cancelled
                | Self::Failed,
                JobEvent::SelectionFinished
                | JobEvent::ItemsDone
                | JobEvent::CancelRequested
                | JobEvent::CancelDrained
                | JobEvent::Failed(_),
            ) => Err(BatchError::IllegalTransition),
        }
    }
}

impl FromStr for JobState {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "SELECTING" => Self::Selecting,
            "RUNNING" => Self::Running,
            "CANCELLING" => Self::Cancelling,
            "COMPLETED" => Self::Completed,
            "CANCELLED" => Self::Cancelled,
            "FAILED" => Self::Failed,
            _ => return Err(()),
        })
    }
}

// --- the item ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    Pending,
    /// Refused for now (quota, rate, lock wait, full queue); retried with a bounded backoff.
    Deferred,
    Succeeded,
    Skipped,
    Failed,
    Cancelled,
}

impl ItemState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Skipped | Self::Failed | Self::Cancelled
        )
    }

    #[must_use]
    pub const fn can_become(self, to: Self) -> bool {
        match (self, to) {
            (Self::Pending, Self::Pending) => false,
            (Self::Pending | Self::Deferred, _) => true,
            (Self::Succeeded | Self::Skipped | Self::Failed | Self::Cancelled, _) => false,
        }
    }
}

/// Why an item was skipped: the action did not apply to the run as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NotApplicable,
    NotWaiting,
    OperationNotDeclared,
    RunContinued,
    AlreadyTerminal,
    HiddenFromCreator,
    /// A feature-022 test run is never changed by a batch (D1039).
    TestRun,
}

impl SkipReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "NOT_APPLICABLE",
            Self::NotWaiting => "NOT_WAITING",
            Self::OperationNotDeclared => "OPERATION_NOT_DECLARED",
            Self::RunContinued => "RUN_CONTINUED",
            Self::AlreadyTerminal => "ALREADY_TERMINAL",
            Self::HiddenFromCreator => "HIDDEN_FROM_CREATOR",
            Self::TestRun => "TEST_RUN",
        }
    }
}

/// Why an item was deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferCause {
    QuotaExceeded,
    RateLimited,
    ConcurrencyQueueFull,
    LockWait,
}

impl DeferCause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::RateLimited => "RATE_LIMITED",
            Self::ConcurrencyQueueFull => "CONCURRENCY_QUEUE_FULL",
            Self::LockWait => "LOCK_WAIT",
        }
    }
}

// --- counters ---------------------------------------------------------------------------------

/// The counters of a job. They change in the item's own transaction, with the effect, so they equal
/// the outcomes of the items; `pending` is what was selected and has not moved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    selected: u32,
    succeeded: u32,
    skipped: u32,
    failed: u32,
    cancelled: u32,
    deferred: u32,
}

impl Counters {
    /// More items were materialized by a selection round.
    pub fn select(&mut self, items: u32) {
        self.selected = self.selected.saturating_add(items);
    }

    #[must_use]
    pub const fn selected(&self) -> u32 {
        self.selected
    }

    #[must_use]
    pub const fn succeeded(&self) -> u32 {
        self.succeeded
    }

    #[must_use]
    pub const fn skipped(&self) -> u32 {
        self.skipped
    }

    #[must_use]
    pub const fn failed(&self) -> u32 {
        self.failed
    }

    #[must_use]
    pub const fn cancelled(&self) -> u32 {
        self.cancelled
    }

    #[must_use]
    pub const fn deferred(&self) -> u32 {
        self.deferred
    }

    #[must_use]
    pub const fn terminal(&self) -> u32 {
        self.succeeded + self.skipped + self.failed + self.cancelled
    }

    #[must_use]
    pub const fn pending(&self) -> u32 {
        self.selected - self.terminal() - self.deferred
    }

    /// Whether every selected item has ended.
    #[must_use]
    pub const fn is_drained(&self) -> bool {
        self.pending() == 0 && self.deferred == 0
    }

    /// Moves one item from `from` to `to`.
    ///
    /// # Errors
    /// [`BatchError::CounterInvariant`] when the move is not an edge of the item state machine, or
    /// when no item is in `from`. A refused move changes nothing.
    pub fn apply(&mut self, from: ItemState, to: ItemState) -> Result<(), BatchError> {
        if !from.can_become(to) {
            return Err(BatchError::CounterInvariant);
        }
        let available = match from {
            ItemState::Pending => self.pending(),
            ItemState::Deferred => self.deferred,
            ItemState::Succeeded
            | ItemState::Skipped
            | ItemState::Failed
            | ItemState::Cancelled => 0,
        };
        if available == 0 {
            return Err(BatchError::CounterInvariant);
        }
        if from == ItemState::Deferred {
            self.deferred -= 1;
        }
        match to {
            ItemState::Pending => {}
            ItemState::Deferred => self.deferred += 1,
            ItemState::Succeeded => self.succeeded += 1,
            ItemState::Skipped => self.skipped += 1,
            ItemState::Failed => self.failed += 1,
            ItemState::Cancelled => self.cancelled += 1,
        }
        Ok(())
    }
}
