//! The scheduler's half of a collection's operator chain (design 042, section 3): the call of the evaluator in a
//! blocking task, the per-replica memory of entries that did not commit (their re-entries are counted, and one that
//! waited for storage quota rests before the next try), and the element counters.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sagat_application::collection_chain::{
    ChainAnswer, ChainCeilings, ChainFailure, ChainInput, StageTally,
};
use sagat_application::execution::{ExpressionEvaluator, RunData};
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::identity::TenantId;
use sagat_domain::operators::OperatorChain;
use sagat_domain::workflow::Expression;

/// The longest a collection that waited for storage quota rests before its entry is tried again (FR-042-009).
pub(crate) const MAX_QUOTA_REST: Duration = Duration::from_secs(30);

/// How long an entry that did not commit is remembered; past it the next entry is not counted as a re-entry.
const FORGET_AFTER: Duration = Duration::from_secs(600);

/// Entries remembered before the oldest are forgotten.
const REMEMBERED: usize = 4_096;

/// Entries one tenant may hold of [`REMEMBERED`].
const REMEMBERED_PER_TENANT: usize = 512;

/// What the chain runs over, owned so it can move into the blocking task.
pub(crate) enum OwnedInput {
    Items(Expression),
    Elements(Vec<Vec<u8>>),
}

/// Runs the chain in a blocking task, so a long chain holds no runtime worker; the task checks its own deadline
/// between elements, since it cannot be aborted from outside. The run data goes in and comes back, unchanged.
///
/// A panic of the evaluator is resumed here, as it would be from a synchronous call. `None` when the runtime is
/// shutting down and the task never ran: nothing was decided, and the run is left for the next pass.
pub(crate) async fn apply<E: ExpressionEvaluator + 'static>(
    permits: &tokio::sync::Semaphore,
    evaluator: Arc<E>,
    input: OwnedInput,
    chain: OperatorChain,
    data: RunData,
    ceilings: ChainCeilings,
) -> Option<(Result<ChainAnswer, ChainFailure>, RunData)> {
    // At most `MAX_CONCURRENT_CHAINS` chains of this replica are evaluated at once: each may hold two stage lists of
    // 4 MiB as values and bytes. The permit is held until the evaluation has answered.
    let Ok(_permit) = permits.acquire().await else {
        return None;
    };
    // The blocking thread logs to the caller's subscriber, inside the caller's span.
    let span = tracing::Span::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let task = tokio::task::spawn_blocking(move || {
        let _default = tracing::dispatcher::set_default(&dispatch);
        let _entered = span.enter();
        let answer = match &input {
            OwnedInput::Items(items) => {
                evaluator.collection_chain(ChainInput::Items(items), &chain, &data, &ceilings)
            }
            OwnedInput::Elements(elements) => {
                evaluator.collection_chain(ChainInput::Elements(elements), &chain, &data, &ceilings)
            }
        };
        (answer, data)
    });
    match task.await {
        Ok(answered) => Some(answered),
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(_) => None,
    }
}

/// Counts what each stage of a committed chain kept, dropped and produced (FR-042-025). Only the entry that commits
/// counts, so an evaluation that was discarded and done again is counted once.
pub(crate) fn count_elements(stages: &[StageTally]) {
    for stage in stages {
        for (outcome, count) in stage.outcomes() {
            metrics::counter!(
                "operator_elements_total",
                "surface" => "collection",
                "operator" => stage.operator.as_str(),
                "outcome" => outcome,
            )
            .increment(count);
        }
    }
}

/// Why an entry of a chained collection did not commit: the `reason` label of `collection_chain_reentries_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Uncommitted {
    /// The store answered that the run's content quota is full, and the run could not be stopped yet.
    StorageQuota,
    /// The shard lease moved between the read and the entry.
    Fence,
    /// The run changed between the read and the entry.
    Revision,
    /// The blob source could not be read right now.
    SourceUnavailable,
}

impl Uncommitted {
    pub(crate) const fn reason(self) -> &'static str {
        match self {
            Self::StorageQuota => "storage_quota",
            Self::Fence => "fence",
            Self::Revision => "revision",
            Self::SourceUnavailable => "source_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Remembered {
    /// Why the last try did not commit; taken by the next try, which counts it.
    reason: Option<Uncommitted>,
    noted: Instant,
    /// When the last wait for storage quota ends, and how long it was.
    rest: Option<(Instant, Duration)>,
}

/// What the next entry of a chained collection may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Try now; `Some` when an earlier try did not commit, with its reason.
    Enter(Option<Uncommitted>),
    /// The collection waited for storage quota and rests until then: no evaluation this pass.
    Rest,
}

type Key = (TenantId, RunId, ExecutionInstanceId);

/// The per-replica memory of chained entries that did not commit (G042-012: a hint; a lost entry costs one more
/// evaluation, never a wrong one, since every evaluation over the same run data gives the same output).
#[derive(Debug)]
pub(crate) struct Reentries {
    entries: Mutex<BTreeMap<Key, Remembered>>,
}

impl Reentries {
    pub(crate) const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn key(tenant: TenantId, run: RunId, owner: &ExecutionInstanceId) -> Key {
        (tenant, run, owner.clone())
    }

    /// Whether to try the entry now. A re-entry is counted by the reason the earlier try did not commit.
    pub(crate) fn admit(
        &self,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
        now: Instant,
    ) -> Admission {
        let Ok(mut entries) = self.entries.lock() else {
            return Admission::Enter(None);
        };
        match entries.get_mut(&Self::key(tenant, run, owner)) {
            Some(Remembered {
                rest: Some((until, _)),
                ..
            }) if now < *until => Admission::Rest,
            Some(remembered) if now.duration_since(remembered.noted) < FORGET_AFTER => {
                // Counted once: the note's reason is taken by this try; the rest, if any, stays for the next note.
                let reason = remembered.reason.take();
                if let Some(reason) = reason {
                    metrics::counter!(
                        "collection_chain_reentries_total",
                        "reason" => reason.reason(),
                    )
                    .increment(1);
                }
                Admission::Enter(reason)
            }
            _ => Admission::Enter(None),
        }
    }

    /// Remembers that an entry did not commit. A wait for storage quota rests one pass interval the first time, then
    /// twice as long as the time before, up to [`MAX_QUOTA_REST`].
    pub(crate) fn note(
        &self,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
        reason: Uncommitted,
        interval: Duration,
        now: Instant,
    ) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() >= REMEMBERED {
            entries.retain(|_, remembered| now.duration_since(remembered.noted) < FORGET_AFTER);
            if entries.len() >= REMEMBERED
                && let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, remembered)| remembered.noted)
                    .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest);
            }
        }
        let key = Self::key(tenant, run, owner);
        if !entries.contains_key(&key)
            && entries
                .keys()
                .filter(|(held, _, _)| *held == tenant)
                .count()
                >= REMEMBERED_PER_TENANT
            && let Some(oldest) = entries
                .iter()
                .filter(|((held, _, _), _)| *held == tenant)
                .min_by_key(|(_, remembered)| remembered.noted)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        let previous = entries.get(&key).and_then(|remembered| remembered.rest);
        let rest = (reason == Uncommitted::StorageQuota).then(|| {
            let length = previous
                .map_or(interval, |(_, length)| length.saturating_mul(2))
                .clamp(interval.min(MAX_QUOTA_REST), MAX_QUOTA_REST);
            (now + length, length)
        });
        entries.insert(
            key,
            Remembered {
                reason: Some(reason),
                noted: now,
                rest,
            },
        );
    }

    /// Forgets the collection: its entry committed, or the run was stopped.
    pub(crate) fn forget(&self, tenant: TenantId, run: RunId, owner: &ExecutionInstanceId) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&Self::key(tenant, run, owner));
        }
    }

    /// The rest the collection is in, if any: for tests.
    #[cfg(test)]
    fn rest(&self, tenant: TenantId, run: RunId, owner: &ExecutionInstanceId) -> Option<Duration> {
        self.entries
            .lock()
            .ok()?
            .get(&Self::key(tenant, run, owner))?
            .rest
            .map(|(_, length)| length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::workflow::NodeId;

    fn owner() -> ExecutionInstanceId {
        ExecutionInstanceId::root(NodeId::parse("lines").expect("node"))
    }

    #[test]
    fn a_wait_for_storage_quota_rests_from_one_interval_doubling_up_to_30_seconds() {
        let memory = Reentries::new();
        let (tenant, run, owner) = (TenantId::generate(), RunId::generate(), owner());
        let interval = Duration::from_millis(500);
        let start = Instant::now();
        assert_eq!(
            memory.admit(tenant, run, &owner, start),
            Admission::Enter(None),
            "a first entry is not a re-entry"
        );
        let mut now = start;
        let mut lengths = Vec::new();
        for _ in 0..9 {
            memory.note(
                tenant,
                run,
                &owner,
                Uncommitted::StorageQuota,
                interval,
                now,
            );
            let length = memory.rest(tenant, run, &owner).expect("a rest");
            lengths.push(length.as_millis());
            assert_eq!(
                memory.admit(tenant, run, &owner, now + length - Duration::from_millis(1)),
                Admission::Rest,
                "no evaluation before the rest is over"
            );
            now += length;
            assert_eq!(
                memory.admit(tenant, run, &owner, now),
                Admission::Enter(Some(Uncommitted::StorageQuota)),
                "the re-entry after the rest is counted with its reason"
            );
        }
        assert_eq!(
            lengths,
            [
                500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000
            ]
        );
        memory.forget(tenant, run, &owner);
        assert_eq!(
            memory.admit(tenant, run, &owner, now),
            Admission::Enter(None)
        );
    }

    #[test]
    fn other_reasons_do_not_rest_and_are_held_per_collection() {
        let memory = Reentries::new();
        let (tenant, run) = (TenantId::generate(), RunId::generate());
        let other = RunId::generate();
        let now = Instant::now();
        memory.note(
            tenant,
            run,
            &owner(),
            Uncommitted::Fence,
            Duration::from_millis(500),
            now,
        );
        assert_eq!(
            memory.admit(tenant, run, &owner(), now),
            Admission::Enter(Some(Uncommitted::Fence))
        );
        assert_eq!(
            memory.admit(tenant, other, &owner(), now),
            Admission::Enter(None)
        );
        assert_eq!(
            memory.admit(tenant, run, &owner(), now + FORGET_AFTER),
            Admission::Enter(None),
            "an old note is forgotten"
        );
    }

    #[test]
    fn the_memory_is_bounded() {
        let memory = Reentries::new();
        let tenant = TenantId::generate();
        let now = Instant::now();
        for _ in 0..(REMEMBERED + 10) {
            memory.note(
                tenant,
                RunId::generate(),
                &owner(),
                Uncommitted::Revision,
                Duration::from_millis(500),
                now,
            );
        }
        assert!(memory.entries.lock().expect("lock").len() <= REMEMBERED);
    }

    /// Review of S1: a re-entry is counted once, by the try that follows the note, not by every pass after it.
    #[test]
    fn a_re_entry_is_counted_once() {
        let (metrics, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(60));
        let memory = Reentries::new();
        let (tenant, run) = (TenantId::generate(), RunId::generate());
        let now = Instant::now();
        metrics::with_local_recorder(&recorder, || {
            memory.note(
                tenant,
                run,
                &owner(),
                Uncommitted::Fence,
                Duration::from_millis(500),
                now,
            );
            assert_eq!(
                memory.admit(tenant, run, &owner(), now),
                Admission::Enter(Some(Uncommitted::Fence))
            );
            assert_eq!(
                memory.admit(tenant, run, &owner(), now),
                Admission::Enter(None),
                "the second try after one note is not a re-entry again"
            );
        });
        assert!(
            metrics
                .render()
                .contains("collection_chain_reentries_total{reason=\"fence\"} 1"),
            "{}",
            metrics.render()
        );
    }

    /// Review of S1: one tenant cannot fill the memory and push every other tenant's notes out.
    #[test]
    fn a_tenant_holds_at_most_its_share_of_the_memory() {
        let memory = Reentries::new();
        let (busy, quiet) = (TenantId::generate(), TenantId::generate());
        let now = Instant::now();
        let quiet_run = RunId::generate();
        memory.note(
            quiet,
            quiet_run,
            &owner(),
            Uncommitted::Revision,
            Duration::from_millis(500),
            now,
        );
        for _ in 0..REMEMBERED {
            memory.note(
                busy,
                RunId::generate(),
                &owner(),
                Uncommitted::Revision,
                Duration::from_millis(500),
                now,
            );
        }
        let entries = memory.entries.lock().expect("lock");
        assert!(
            entries
                .keys()
                .filter(|(tenant, _, _)| *tenant == busy)
                .count()
                <= REMEMBERED_PER_TENANT
        );
        assert!(
            entries.contains_key(&(quiet, quiet_run, owner())),
            "the quiet tenant's note stays"
        );
    }
}
