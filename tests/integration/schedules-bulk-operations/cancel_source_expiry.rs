//! The expiry of a reset that waits for its source (`T018-057`, `FR-018-038`, D2059 to D2068, the owner decision
//! OD-2026-11-08-1): an entry still `PENDING_SOURCE` a whole window after its request, with its source still open, ends
//! `FAILED` with the closed reason `SOURCE_DID_NOT_END`; no run is made, its place in the namespace is freed, the source
//! stays as the request left it. An ended source wins, however late. Real PostgreSQL, the real run service and the real
//! completer; the window is read on a clock the test sets (the pass is given the database's clock in a process), so
//! nothing here sleeps. The races and the crashes of the expiry are in the fault suite
//! (`tests/fault/schedules-bulk-operations/cancel_source_expiry.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::clock::Clock as _;
use sagat_application::error::{AppError, StoreError};
use sagat_application::identity::RequestContext;
use sagat_application::operations::MonitorStore as _;
use sagat_application::reset_completion::{
    Completed, Completion, DueReset, Expiry, PendingResetStore as _, ResetCompleter,
    ResetCompletions, ResetExpiry, SOURCE_DID_NOT_END,
};
use sagat_application::runs::{PendingReset, ResetState, RunStore as _};
use sagat_application::schedules::ShardFence;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{ResetId, RunId, RunStatus};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, ResetPass, ResetRound, ResetSettings};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::business_ids::key;
use crate::cancel_source::{
    Completions, Open, asking, audited, completed_steps, digest_of, end, entries, made_from,
    open_chain, open_chain_tagged, outcome, plant, request,
};
use crate::firing::{Fixture, TestClock, fixture};
use crate::support::engine::{activity, document, with};

type Expiring = ResetCompleter<PgRunStore, Completions, PgIdentityStore, TestClock>;

fn completer_at(f: &Fixture, clock: &TestClock) -> Expiring {
    ResetCompleter::new(
        f.w.store.clone(),
        Completions(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock.clone(),
    )
}

/// The window of the engine's default, as the clock arithmetic of the tests wants it.
pub(crate) fn window() -> Duration {
    Duration::seconds(
        i64::try_from(ResetSettings::default().expiry.as_secs()).expect("a day in seconds"),
    )
}

/// An engine that holds every shard, whose pass reads the clock the test sets.
pub(crate) struct Engine {
    pub(crate) clock: TestClock,
    pub(crate) pass: ResetPass<PgRunStore, Completions, PgIdentityStore, TestClock>,
    keeper: Keeper<PgRunStore>,
}

impl Engine {
    /// The process stops: it lets go of its shards, as a replica that leaves does, so that the next one can take them.
    async fn stops(self) {
        self.keeper.leave().await;
    }
}

pub(crate) async fn engine_with(f: &Fixture, name: &str, settings: ResetSettings) -> Engine {
    let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("keeper");
    // a test of a pass that holds no shard would prove nothing
    assert_eq!(
        placement.owned().iter().count(),
        Shard::all().count(),
        "the engine holds every shard"
    );
    let clock = TestClock::at(OffsetDateTime::now_utc());
    Engine {
        pass: ResetPass::new(completer_at(f, &clock), placement, settings),
        clock,
        keeper,
    }
}

async fn engine(f: &Fixture, name: &str) -> Engine {
    engine_with(f, name, ResetSettings::default()).await
}

/// An entry of the *other* tenant, waiting for a source of theirs that stays open (`other_tenant_entry` of the
/// completion suite asks a source that has nothing in flight to be cancelled, which ends it at once).
async fn open_entry_of_the_other_tenant(f: &Fixture) -> ResetId {
    let workflow =
        f.w.publish_as_other_tenant(document(
            &format!("cancel-source-expiry-other-{}", Uuid::now_v7()),
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let run =
        f.w.start_as_other_tenant(workflow, &json!({"k": 2}))
            .await
            .expect("a run of the other tenant");
    let record =
        f.w.store
            .get(f.w.other_admin.tenant_id(), run)
            .await
            .expect("get")
            .expect("the run of the other tenant");
    assert!(!record.status.is_terminal(), "it stays open");
    let id = ResetId::generate();
    sqlx::query(
        "INSERT INTO run_resets \
         (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, reapply, business_id_policy, \
          source_policy, state, actor, reason, created_at, reapply_report, acknowledged_reexecution) \
         VALUES ($1, $2, $3, $4, 1, $5, 'NONE', 'KEEP', 'CANCEL_SOURCE', 'PENDING_SOURCE', $6, $7, now(), '[]', true)",
    )
    .bind(f.w.other_admin.tenant_id().as_uuid())
    .bind(f.w.other_namespace.id().as_uuid())
    .bind(id.as_uuid())
    .bind(run.as_uuid())
    .bind(vec![7u8; 32])
    .bind(f.w.other_admin.subject().as_str())
    .bind(crate::cancel_source::WHY)
    .execute(&f.w.db.pool)
    .await
    .expect("an entry of the other tenant");
    id
}

/// When the request was made, as the register keeps it.
pub(crate) async fn requested_at(f: &Fixture, id: ResetId) -> OffsetDateTime {
    sqlx::query_scalar("SELECT created_at FROM run_resets WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry")
}

/// The records `run.reset` of the source, oldest first, as `(actor, reason)`.
async fn records(f: &Fixture, source: RunId) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT actor, coalesce(reason, '') FROM audit_records \
         WHERE action = 'run.reset' AND resource_id = $1 ORDER BY created_at, id",
    )
    .bind(source.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

pub(crate) fn counts(round: &ResetRound) -> (usize, usize, u32, u32, u32, u32) {
    (
        round.due,
        round.lapsed,
        round.created,
        round.expired,
        round.failed_entries,
        round.errors,
    )
}

/// A request for the open source of `open`, accepted.
pub(crate) async fn asked(f: &Fixture, open: &Open) -> ResetId {
    request(f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request")
        .reset
        .id
}

// ------------------------------------------------------------------------------------------------- the expiry

#[tokio::test]
async fn an_entry_whose_source_is_still_open_a_whole_window_after_its_request_fails_makes_no_run_and_leaves_the_source_as_it_was()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    assert_eq!(f.w.store.sample().await.expect("sample").resets_pending, 1);
    let status = f.w.run_status(open.run).await;
    assert_eq!(status, RunStatus::Cancelling, "the request cancelled it");
    let journal = f.w.journal(open.run).await;
    let engine = engine(&f, "engine-one").await;

    engine.clock.set(asked_at + window());
    let round = engine.pass.round().await;
    assert_eq!(counts(&round), (0, 1, 0, 1, 0, 0), "{round:?}");
    assert!(round.did_work());

    let (state, reason, new_run, completed) = outcome(&f, id).await;
    assert_eq!(
        (state.as_str(), reason.as_deref(), new_run, completed),
        ("FAILED", Some(SOURCE_DID_NOT_END), None, true)
    );
    let completed_at: OffsetDateTime =
        sqlx::query_scalar("SELECT completed_at FROM run_resets WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("completed_at");
    assert_eq!(
        completed_at,
        engine.clock.now(),
        "it ended at the time of the pass"
    );
    assert!(made_from(&f, open.run).await.is_empty(), "no run is made");
    // the source is as the request left it: cancelling, with the journal it had
    assert_eq!(f.w.run_status(open.run).await, status);
    assert_eq!(f.w.journal(open.run).await, journal);
    // the place is free: nothing waits any more, and nothing is late
    let sample = f.w.store.sample().await.expect("sample");
    assert_eq!((sample.resets_pending, sample.overdue.resets), (0, 0.0));

    // the source ends later: nothing is made for an entry that ended, and the pass has nothing to do
    end(&f, &open).await;
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelled);
    let after = engine.pass.round().await;
    assert_eq!(counts(&after), (0, 0, 0, 0, 0, 0), "{after:?}");
    assert!(!after.did_work());
    assert!(made_from(&f, open.run).await.is_empty());
    assert_eq!(outcome(&f, id).await.0, "FAILED");
    assert_eq!(entries(&f, open.run).await, 1, "the register keeps it");
}

#[tokio::test]
async fn the_place_an_entry_held_is_free_for_a_new_request_at_the_limit() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'pending_resets', 1, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of one");
    let one = open_chain_tagged(&f, "1").await;
    let two = open_chain_tagged(&f, "2").await;
    let first = asked(&f, &one).await;

    let refused = request(&f, two.run, asking(two.after_a, true), &key()).await;
    assert!(
        matches!(&refused, Err(AppError::QuotaExceeded(what)) if *what == sagat_domain::quota::QuotaResource::PendingResets.limit_name()),
        "the namespace is full: {refused:?}"
    );
    assert_eq!(entries(&f, two.run).await, 0);

    let engine = engine(&f, "engine-one").await;
    engine.clock.set(requested_at(&f, first).await + window());
    assert_eq!(engine.pass.round().await.expired, 1);

    let made = request(&f, two.run, asking(two.after_a, true), &key())
        .await
        .expect("the place is free again");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert_eq!(entries(&f, two.run).await, 1);
}

#[tokio::test]
async fn an_entry_inside_its_window_waits_and_the_instant_the_window_is_over_it_expires() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;

    // a day, to the microsecond
    let engine = engine(&f, "engine-one").await;
    engine
        .clock
        .set(asked_at + window() - Duration::microseconds(1));
    let inside = engine.pass.round().await;
    assert_eq!(counts(&inside), (0, 0, 0, 0, 0, 0), "{inside:?}");
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    engine.clock.set(asked_at);
    assert_eq!(engine.pass.round().await.lapsed, 0, "at the request");
    engine.clock.set(asked_at - Duration::days(30));
    assert_eq!(
        engine.pass.round().await.lapsed,
        0,
        "a clock far behind the request expires nothing"
    );
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");

    engine.clock.set(asked_at + window());
    let over = engine.pass.round().await;
    assert_eq!(counts(&over), (0, 1, 0, 1, 0, 0), "{over:?}");
    assert_eq!(outcome(&f, id).await.1.as_deref(), Some(SOURCE_DID_NOT_END));
}

#[tokio::test]
async fn the_window_is_the_one_the_deployment_set_and_an_engine_with_another_one_does_not_expire_what_is_not_its_own()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    let hour = Duration::hours(1);

    // the deployment that kept the default: an hour and a second after the request is nothing to it
    let day = engine(&f, "engine-day").await;
    let past_an_hour = asked_at + hour + Duration::seconds(1);
    day.clock.set(past_an_hour);
    assert_eq!(day.pass.round().await.lapsed, 0, "a day is not over");
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    day.stops().await;

    // the one that set an hour: the same entry, at the same time, is over its window
    let short = engine_with(
        &f,
        "engine-short",
        ResetSettings {
            expiry: StdDuration::from_secs(3_600),
            ..ResetSettings::default()
        },
    )
    .await;
    short.clock.set(past_an_hour - Duration::seconds(2));
    assert_eq!(short.pass.round().await.lapsed, 0, "an hour less a second");
    short.clock.set(past_an_hour);
    assert_eq!(short.pass.round().await.expired, 1, "an hour is over");
    assert_eq!(outcome(&f, id).await.0, "FAILED");
}

#[tokio::test]
async fn a_restart_of_the_engine_does_not_start_the_window_again() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;

    // the first process looks at it early and is gone
    let first = engine(&f, "engine-before").await;
    first.clock.set(asked_at + Duration::hours(23));
    assert_eq!(first.pass.round().await.lapsed, 0);
    first.stops().await;
    // the second one starts a minute before the window is over: it holds nothing about the entry but the register
    let second = engine(&f, "engine-after").await;
    second
        .clock
        .set(asked_at + window() - Duration::seconds(60));
    assert_eq!(second.pass.round().await.expired, 0);
    // and expires it when the window, counted from the request, is over, not a window after it started
    second.clock.set(asked_at + window());
    let round = second.pass.round().await;
    assert_eq!(counts(&round), (0, 1, 0, 1, 0, 0), "{round:?}");
}

#[tokio::test]
async fn a_source_that_has_ended_completes_the_entry_however_late_and_it_is_never_expired() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    end(&f, &open).await;

    let engine = engine(&f, "engine-one").await;
    engine.clock.set(asked_at + Duration::days(10));
    let round = engine.pass.round().await;
    // the entry is due, and is not also found past its window: an ended source wins
    assert_eq!(counts(&round), (1, 0, 1, 0, 0, 0), "{round:?}");
    let (state, reason, new_run, _) = outcome(&f, id).await;
    assert_eq!((state.as_str(), reason), ("CREATED", None));
    assert!(new_run.is_some());
    assert_eq!(made_from(&f, open.run).await.len(), 1, "the run is made");
    assert!(
        records(&f, open.run)
            .await
            .iter()
            .all(|(_, reason)| !reason.contains(SOURCE_DID_NOT_END))
    );
    // and nothing is left for the expiry to look at
    let idle = engine.pass.round().await;
    assert_eq!(counts(&idle), (0, 0, 0, 0, 0, 0));
}

#[tokio::test]
async fn a_source_that_ends_after_the_entry_has_lapsed_but_before_the_pass_looks_is_completed_and_not_expired()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    let engine = engine(&f, "engine-one").await;
    engine.clock.set(asked_at + window());

    // the pass found the entry lapsed (its source was open) and the source ends before the step is made: the step reads
    // the source again, under the lock, and leaves the entry to be completed
    let found = DueReset {
        tenant_id: f.w.admin.tenant_id(),
        id,
    };
    end(&f, &open).await;
    let completer = completer_at(&f, &engine.clock);
    let step = completer
        .expire(&found, asked_at, None)
        .await
        .expect("the step");
    assert_eq!(step, Completed::Idle, "the source has ended");
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    assert_eq!(
        completer.step(&found, None).await.expect("the completion"),
        Completed::Created
    );
}

#[tokio::test]
async fn a_source_in_manual_review_is_open_and_its_entry_expires_leaving_the_run_for_an_operator() {
    let f = fixture().await;
    let source = crate::batch_actions::run_with_a_stuck_compensation(&f.w).await;
    assert_eq!(f.w.run_status(source).await, RunStatus::ManualReview);
    let made = request(&f, source, asking(1, true), &key())
        .await
        .expect("the request");
    let asked_at = requested_at(&f, made.reset.id).await;

    let engine = engine(&f, "engine-one").await;
    engine.clock.set(asked_at + window());
    let round = engine.pass.round().await;
    assert_eq!(counts(&round), (0, 1, 0, 1, 0, 0), "{round:?}");
    assert_eq!(
        outcome(&f, made.reset.id).await.1.as_deref(),
        Some(SOURCE_DID_NOT_END)
    );
    assert_eq!(
        f.w.run_status(source).await,
        RunStatus::ManualReview,
        "the run still waits for an operator"
    );

    // the operator resolves it later: the run ends, and the entry that ended makes nothing
    f.w.review(
        source,
        "reserve~c",
        sagat_application::runs::ReviewAction::Skip,
        "the provider is gone for good",
    )
    .await
    .expect("skip the compensation");
    assert!(f.w.run_status(source).await.is_terminal());
    assert_eq!(counts(&engine.pass.round().await), (0, 0, 0, 0, 0, 0));
    assert!(made_from(&f, source).await.is_empty());
}

// ------------------------------------------------------------------------------------- bounded, fair, per tenant

/// `n` entries planted on the open source of `open`, in the order of their request.
async fn planted(f: &Fixture, open: &Open, n: usize) -> Vec<ResetId> {
    let digest = digest_of(f, open.run, usize::try_from(open.after_a).expect("small")).await;
    let mut ids = Vec::new();
    for _ in 0..n {
        ids.push(
            plant(
                f,
                open.run,
                i64::try_from(open.after_a).expect("small"),
                &digest,
                true,
            )
            .await,
        );
    }
    ids
}

pub(crate) async fn latest_request(f: &Fixture, ids: &[ResetId]) -> OffsetDateTime {
    let mut latest = OffsetDateTime::UNIX_EPOCH;
    for id in ids {
        latest = latest.max(requested_at(f, *id).await);
    }
    latest
}

async fn states(f: &Fixture, ids: &[ResetId]) -> Vec<String> {
    let mut states = Vec::new();
    for id in ids {
        states.push(outcome(f, *id).await.0);
    }
    states
}

#[tokio::test]
async fn a_round_expires_at_most_its_batch_oldest_first_and_the_rest_wait_for_the_next() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let ids = planted(&f, &open, 5).await;
    let engine = engine_with(
        &f,
        "engine-one",
        ResetSettings {
            batch: 2,
            per_tenant: 10,
            ..ResetSettings::default()
        },
    )
    .await;
    engine.clock.set(latest_request(&f, &ids).await + window());

    let first = engine.pass.round().await;
    assert_eq!(counts(&first), (0, 2, 0, 2, 0, 0), "{first:?}");
    assert_eq!(
        states(&f, &ids).await,
        [
            "FAILED",
            "FAILED",
            "PENDING_SOURCE",
            "PENDING_SOURCE",
            "PENDING_SOURCE"
        ],
        "the oldest two"
    );
    assert_eq!(engine.pass.round().await.expired, 2);
    assert_eq!(engine.pass.round().await.expired, 1);
    let done = engine.pass.round().await;
    assert_eq!(counts(&done), (0, 0, 0, 0, 0, 0), "{done:?}");
    assert!(states(&f, &ids).await.iter().all(|state| state == "FAILED"));
}

#[tokio::test]
async fn the_entries_of_one_tenant_cannot_fill_a_round_of_expiries_and_starve_another_tenant() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let ours = planted(&f, &open, 3).await;
    let theirs = open_entry_of_the_other_tenant(&f).await;
    let engine = engine_with(
        &f,
        "engine-one",
        ResetSettings {
            batch: 100,
            per_tenant: 1,
            ..ResetSettings::default()
        },
    )
    .await;
    let mut all = ours.clone();
    all.push(theirs);
    engine.clock.set(latest_request(&f, &all).await + window());

    let round = engine.pass.round().await;
    assert_eq!(counts(&round), (0, 2, 0, 2, 0, 0), "{round:?}");
    assert_eq!(
        states(&f, &all).await,
        ["FAILED", "PENDING_SOURCE", "PENDING_SOURCE", "FAILED"],
        "one of ours (the oldest) and the other tenant's"
    );
    assert_eq!(engine.pass.round().await.expired, 1);
    assert_eq!(engine.pass.round().await.expired, 1);
    assert!(states(&f, &all).await.iter().all(|state| state == "FAILED"));
}

#[tokio::test]
async fn the_expiry_of_an_entry_is_audited_in_its_own_tenant_and_touches_no_other() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let theirs = open_entry_of_the_other_tenant(&f).await;
    let other_tenant = f.w.other_admin.tenant_id();
    let engine = engine(&f, "engine-one").await;

    // a caller that names the entry of another tenant under its own reaches nothing
    let ours = f.w.admin.tenant_id();
    let at = requested_at(&f, theirs).await + window();
    engine.clock.set(at);
    let stray = DueReset {
        tenant_id: ours,
        id: theirs,
    };
    assert_eq!(
        completer_at(&f, &engine.clock)
            .expire(&stray, at, None)
            .await
            .expect("not an error"),
        Completed::Idle,
        "the register of another tenant is not read"
    );
    let audit = AuditRecord::by(
        &Principal::new(ours, Subject::engine(), Role::Operator),
        AuditAction::RunReset,
        ResourceType::Run,
        open.run.to_string(),
        None,
        at,
    );
    let wrong =
        f.w.store
            .expire_reset(&ResetExpiry {
                tenant_id: ours,
                id: theirs,
                requested_by: at,
                at,
                fence: None,
                audit: &audit,
            })
            .await;
    assert!(
        matches!(wrong, Err(StoreError::MissingReference)),
        "{wrong:?}"
    );
    assert_eq!(outcome(&f, theirs).await.0, "PENDING_SOURCE");

    // the round expires both, each in its own tenant
    engine
        .clock
        .set(latest_request(&f, &[id, theirs]).await + window());
    assert_eq!(engine.pass.round().await.expired, 2);
    let tenants: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT tenant_id, reason FROM audit_records WHERE action = 'run.reset' AND reason LIKE '%failed SOURCE_DID_NOT_END' \
         ORDER BY reason",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the audit trail");
    assert_eq!(tenants.len(), 2, "{tenants:?}");
    for (tenant, reason) in tenants {
        let (expected, entry) = if reason.contains(&id.to_string()) {
            (ours.as_uuid(), id)
        } else {
            (other_tenant.as_uuid(), theirs)
        };
        assert_eq!(tenant, expected, "{reason}");
        assert!(reason.contains(&entry.to_string()), "{reason}");
    }
}

#[tokio::test]
async fn the_expiry_is_a_system_action_that_leaves_the_requester_the_actor_of_the_request_and_writes_no_free_text()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let before = records(&f, open.run).await;
    assert_eq!(before.len(), 1, "the request: {before:?}");
    assert_eq!(before[0].0, f.w.admin.subject().as_str());
    assert!(before[0].1.contains(crate::cancel_source::WHY));

    let engine = engine(&f, "engine-one").await;
    engine.clock.set(requested_at(&f, id).await + window());
    assert_eq!(engine.pass.round().await.expired, 1);

    let after = records(&f, open.run).await;
    assert_eq!(after.len(), 2, "one record more: {after:?}");
    assert_eq!(after[0], before[0], "the request's record is as it was");
    assert_eq!(after[1].0, "system:engine", "the platform ended it");
    assert_eq!(
        after[1].1,
        format!("completes reset {id}: failed {SOURCE_DID_NOT_END}"),
        "the id and a closed word, nothing the requester wrote"
    );
    assert!(!after[1].1.contains(crate::cancel_source::WHY));
    // a pass that runs again writes nothing more
    engine.pass.round().await;
    assert_eq!(records(&f, open.run).await, after);
    // the same words are what `audited` reads by resource
    assert_eq!(audited(&f, "run.reset", open.run).await.len(), 2);
}

// ------------------------------------------------------------------------------------ the store, called directly

/// A transaction that holds the row of the run the way the one that **ends** it does, and not the way a plain lock would:
/// an `UPDATE` that changes no key column takes `FOR NO KEY UPDATE`, which `FOR SHARE` conflicts with and `FOR KEY SHARE`
/// does not (a `SELECT ... FOR UPDATE` conflicts with both, so a test that held the row with it would stay green if the
/// expiry took the weaker lock: D2060, review D1). Uncommitted: the caller lets go with `rollback`.
pub(crate) async fn hold_as_the_end_of_a_run_does(
    f: &Fixture,
    run: RunId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = f.w.db.pool.begin().await.expect("hold the source");
    let updated = sqlx::query("UPDATE runs SET updated_at = updated_at WHERE id = $1")
        .bind(run.as_uuid())
        .execute(&mut *held)
        .await
        .expect("update the source")
        .rows_affected();
    assert_eq!(updated, 1, "the source is held by an uncommitted update");
    held
}

fn expiry<'a>(
    f: &Fixture,
    id: ResetId,
    requested_by: OffsetDateTime,
    audit: &'a AuditRecord,
) -> ResetExpiry<'a> {
    ResetExpiry {
        tenant_id: f.w.admin.tenant_id(),
        id,
        requested_by,
        at: requested_by,
        fence: None,
        audit,
    }
}

fn expiry_record(f: &Fixture, source: RunId, id: ResetId, at: OffsetDateTime) -> AuditRecord {
    AuditRecord::by(
        &Principal::new(f.w.admin.tenant_id(), Subject::engine(), Role::Operator),
        AuditAction::RunReset,
        ResourceType::Run,
        source.to_string(),
        None,
        at,
    )
    .with_reason(format!("completes reset {id}: failed {SOURCE_DID_NOT_END}"))
}

#[tokio::test]
async fn the_store_ends_an_entry_only_when_it_is_past_the_cutoff_and_its_source_is_open() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    let audit = expiry_record(&f, open.run, id, asked_at);

    // a cutoff before the request: the store compares the `created_at` of the entry with the instant it is given (the
    // caller's, D2079: the pass computes it from its database clock) and the entry was requested after it
    let early =
        f.w.store
            .expire_reset(&expiry(
                &f,
                id,
                asked_at - Duration::microseconds(1),
                &audit,
            ))
            .await
            .expect("the store");
    assert_eq!(early, Expiry::NotDue);
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    assert!(
        records(&f, open.run).await.len() == 1,
        "no record was written"
    );

    // the source ended: an ended source wins
    end(&f, &open).await;
    let ended =
        f.w.store
            .expire_reset(&expiry(&f, id, asked_at, &audit))
            .await
            .expect("the store");
    assert_eq!(ended, Expiry::SourceEnded);
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    assert_eq!(records(&f, open.run).await.len(), 1);
}

#[tokio::test]
async fn the_store_never_waits_for_a_run_and_leaves_an_entry_whose_source_is_in_the_hands_of_another_transaction()
 {
    use crate::support::bounded::within;
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let asked_at = requested_at(&f, id).await;
    let audit = expiry_record(&f, open.run, id, asked_at);

    // another transaction holds the row of the source the way the one that ends a run does
    let held = hold_as_the_end_of_a_run_does(&f, open.run).await;
    let busy = within(
        "the expiry does not wait for the row of the source",
        StdDuration::from_secs(60),
        f.w.store.expire_reset(&expiry(&f, id, asked_at, &audit)),
    )
    .await
    .expect("the store");
    assert_eq!(busy, Expiry::SourceBusy);
    assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE");
    assert_eq!(records(&f, open.run).await.len(), 1);
    held.rollback().await.expect("release the source");

    // released, the same call ends the entry
    let done =
        f.w.store
            .expire_reset(&expiry(&f, id, asked_at, &audit))
            .await
            .expect("the store");
    assert_eq!(done, Expiry::Expired);
    assert_eq!(outcome(&f, id).await.1.as_deref(), Some(SOURCE_DID_NOT_END));
    // an entry that ended is not ended again
    let again =
        f.w.store
            .expire_reset(&expiry(&f, id, asked_at, &audit))
            .await;
    assert!(
        matches!(&again, Err(StoreError::StateConflict(why)) if why.contains("completed")),
        "{again:?}"
    );
    assert_eq!(
        records(&f, open.run).await.len(),
        2,
        "one record for the expiry"
    );
}

#[tokio::test]
async fn the_scan_for_expired_entries_is_oldest_first_bounded_per_tenant_and_leaves_out_what_it_is_told_to()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let ours = planted(&f, &open, 3).await;
    let theirs = open_entry_of_the_other_tenant(&f).await;
    let ended = open_chain_tagged(&f, "e").await;
    // a real request, which cancels its source, so that the source ends
    let ended_entry = asked(&f, &ended).await;
    end(&f, &ended).await;
    let mut all = ours.clone();
    all.push(theirs);
    let cutoff = latest_request(&f, &all).await;
    let every: Vec<Shard> = Shard::all().collect();
    let scan = |limit: u32, per_tenant: u32, skipping: Vec<ResetId>, cutoff: OffsetDateTime| {
        let store = f.w.store.clone();
        let every = every.clone();
        async move {
            store
                .expired_resets(&every, limit, per_tenant, &skipping, cutoff)
                .await
                .expect("the scan")
                .into_iter()
                .map(|due| due.id)
                .collect::<Vec<_>>()
        }
    };
    // an entry whose source ended is not lapsed: it is due, and the completion's
    let everything = scan(10, 10, vec![], cutoff).await;
    assert_eq!(everything.len(), 4, "{everything:?}");
    assert!(!everything.contains(&ended_entry));
    assert_eq!(everything[..3], ours[..], "the older ones first");
    // the share of a tenant, the batch, and what is left out
    let one_each = scan(10, 1, vec![], cutoff).await;
    assert_eq!(one_each.len(), 2);
    assert!(one_each.contains(&ours[0]) && one_each.contains(&theirs));
    assert_eq!(scan(2, 10, vec![], cutoff).await, ours[..2]);
    let without_first = scan(10, 1, vec![ours[0]], cutoff).await;
    assert!(without_first.contains(&ours[1]) && !without_first.contains(&ours[0]));
    assert!(scan(10, 10, all.clone(), cutoff).await.is_empty());
    // the cutoff: an entry requested at it is lapsed, one microsecond later is not
    let first = requested_at(&f, ours[0]).await;
    assert!(scan(10, 10, vec![], first).await.contains(&ours[0]));
    assert!(
        !scan(10, 10, vec![], first - Duration::microseconds(1))
            .await
            .contains(&ours[0])
    );
    // and only the shards the replica holds
    let own = Shard::of(ours[0].as_uuid());
    let mine =
        f.w.store
            .expired_resets(&[own], 10, 10, &[], cutoff)
            .await
            .expect("the scan");
    assert!(
        mine.iter().all(|due| Shard::of(due.id.as_uuid()) == own) && !mine.is_empty(),
        "{mine:?}"
    );
}

/// The scan that finds the entries past their window reads an index of the entries that wait, never the register as a
/// whole: `run_resets_pending_idx` (`(shard, created_at) WHERE state = 'PENDING_SOURCE'`) or
/// `run_resets_pending_namespace_idx` (`(tenant_id, namespace_id) WHERE state = 'PENDING_SOURCE'`), both partial on the
/// state, so what the scan reads is bounded by the entries that wait (16 a namespace by default, 64 at most), however many
/// the register keeps. With a handful of rows the planner prefers a sequential scan whatever an index is worth, so the
/// test turns that off and asks that the plan can be served by one of them: if the predicate stopped matching the
/// partial indexes (another state, a function of the column) the plan would name a different index or a sequential scan.
/// It does not pick between the two (on this size it takes the namespace one and filters); a measure of the choice at
/// the size of a deployment is not made (G2015).
#[tokio::test]
async fn the_scan_for_expired_entries_can_be_served_by_the_index_of_the_entries_that_wait() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    planted(&f, &open, 3).await;
    let mut tx = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .expect("plans without a sequential scan");
    let shards: Vec<i16> = (0..64).collect();
    let plan: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {}",
        sagat_infra_postgres::EXPIRED_RESETS_SQL
    )))
    .bind(shards)
    .bind(100_i64)
    .bind(10_i64)
    .bind(Vec::<Uuid>::new())
    .bind(OffsetDateTime::now_utc())
    .fetch_all(&mut *tx)
    .await
    .expect("the plan");
    let plan = plan
        .into_iter()
        .map(|(line,)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("using run_resets_pending_idx")
            || plan.contains("using run_resets_pending_namespace_idx")
            || plan.contains("on run_resets_pending_idx")
            || plan.contains("on run_resets_pending_namespace_idx"),
        "{plan}"
    );
    assert!(
        !plan.contains("Seq Scan on run_resets"),
        "the register is not read as a whole:\n{plan}"
    );
}

// -------------------------------------------------------------------------------------------------- telemetry

#[test]
fn what_the_expiry_counts_is_counted_once_it_is_committed_under_the_closed_labels() {
    use crate::counted::{recorded, series};
    const NAME: &str = "workflow_reset_completions_total";
    let page = recorded(|| async {
        let f = fixture().await;
        let lapsed = open_chain_tagged(&f, "1").await;
        let waiting = open_chain_tagged(&f, "2").await;
        let ended = open_chain_tagged(&f, "3").await;
        let first = asked(&f, &lapsed).await;
        let inside = asked(&f, &waiting).await;
        let last = asked(&f, &ended).await;
        end(&f, &ended).await;
        let at = requested_at(&f, first).await + window();
        let clock = TestClock::at(at);
        let completer = completer_at(&f, &clock);
        let due = |id| DueReset {
            tenant_id: f.w.admin.tenant_id(),
            id,
        };
        assert_eq!(
            completer
                .expire(&due(first), at, None)
                .await
                .expect("expired"),
            Completed::Expired
        );
        // an entry that ended already, one inside its window and one whose source ended count nothing
        for (id, by) in [(first, at), (inside, at - window()), (last, at)] {
            assert_eq!(
                completer.expire(&due(id), by, None).await.expect("idle"),
                Completed::Idle
            );
        }
    });
    assert_eq!(
        series(
            &page,
            NAME,
            &[("outcome", "expired"), ("reason", SOURCE_DID_NOT_END)]
        ),
        Some(1.0),
        "{page}"
    );
    // nothing else: no other series, and the labels are the two closed ones
    let lines: Vec<&str> = page.lines().filter(|line| line.starts_with(NAME)).collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = lines[0];
    let inside = &line[line.find('{').expect("labels") + 1..line.rfind('}').expect("labels")];
    let mut keys: Vec<&str> = inside
        .split(',')
        .filter_map(|pair| pair.split('=').next())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["outcome", "reason"], "{line}");
}

/// The expiry never makes a run: the completions it is wired to are never called (a completer that panics if asked).
pub(crate) struct NeverCompletes;

impl ResetCompletions for NeverCompletes {
    async fn complete(
        &self,
        _principal: &Principal,
        _entry: &PendingReset,
        _fence: Option<ShardFence>,
        _ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        panic!("the expiry never creates a run");
    }
}

#[tokio::test]
async fn the_expiry_never_asks_for_a_run_to_be_made() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let id = asked(&f, &open).await;
    let at = requested_at(&f, id).await + window();
    let clock = TestClock::at(at);
    let completer = ResetCompleter::new(
        f.w.store.clone(),
        NeverCompletes,
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock,
    );
    let step = completer
        .expire(
            &DueReset {
                tenant_id: f.w.admin.tenant_id(),
                id,
            },
            at,
            None,
        )
        .await
        .expect("the step");
    assert_eq!(step, Completed::Expired);
    assert_eq!(
        completed_steps(&f, open.run).await.len(),
        1,
        "the journal is the source's"
    );
}
