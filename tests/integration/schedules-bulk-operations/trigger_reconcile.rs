//! The reconciliation of trigger-owned schedules (`T018-060`, `FR-018-017`): it covers what publication did not (an older
//! control plane that publishes during the upgrade, the triggers older than the feature), repairs drift, keeps going when
//! one entry is stuck, never starves another tenant, and runs under the lease of the shard of the trigger. Real PostgreSQL,
//! a real keeper, a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration as StdDuration;

use sagat_application::clock::Clock as _;
use sagat_application::error::StoreError;
use sagat_application::schedule_triggers::{
    Change, Drift, DriftPage, DriftWindow, HandoverState, TriggerReconciler, TriggerScheduleStore,
};
use sagat_application::schedules::{ScheduleState, ShardFence};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgRunStore, PgScheduleStore};
use sagat_scheduler::{
    Keeper, KeeperSettings, Placement, TriggerPass, TriggerRound, TriggerSettings,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::schedules::TestClock;
use crate::support::trigger_schedules::{Triggers, cron_workflow, triggers};

/// What an older control plane does when it publishes: it writes the trigger and knows nothing of schedules.
pub(crate) async fn old_control_plane_publishes(
    t: &Triggers,
    like: &str,
    id: Uuid,
    expression: &str,
    zone: &str,
) {
    sqlx::query(
        "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
         SELECT $2, tenant_id, namespace_id, workflow_id, version_id, 'cron', $3, $4 \
           FROM workflow_triggers WHERE id = (SELECT t.id FROM workflow_triggers t JOIN workflows w \
                                              ON w.id = t.workflow_id WHERE w.name = $1 \
                                              ORDER BY t.id LIMIT 1)",
    )
    .bind(like)
    .bind(id)
    .bind(expression)
    .bind(zone)
    .execute(&t.w.db.pool)
    .await
    .expect("an older control plane publishes");
}

/// One replica: the keeper that holds its shards and the pass that works on them.
pub(crate) struct Replica {
    pub(crate) placement: Placement,
    pub(crate) keeper: Keeper<PgRunStore>,
    pub(crate) pass: TriggerPass<Counting, SpecCalendar, TestClock>,
    pub(crate) drifts: Arc<AtomicU32>,
}

/// The store, counting how many times a round asks how the schedules differ from the triggers.
#[derive(Clone)]
pub(crate) struct Counting {
    pub(crate) inner: PgScheduleStore,
    pub(crate) drifts: Arc<AtomicU32>,
}

impl TriggerScheduleStore for Counting {
    async fn handover(&self) -> Result<HandoverState, StoreError> {
        self.inner.handover().await
    }

    async fn drift(&self, window: &DriftWindow) -> Result<DriftPage, StoreError> {
        self.drifts.fetch_add(1, Ordering::SeqCst);
        self.inner.drift(window).await
    }

    async fn convergence(
        &self,
    ) -> Result<sagat_application::schedule_triggers::Convergence, StoreError> {
        self.inner.convergence().await
    }

    async fn legacy_fires(
        &self,
        since: Option<OffsetDateTime>,
    ) -> Result<Vec<(Uuid, OffsetDateTime)>, StoreError> {
        self.inner.legacy_fires(since).await
    }

    async fn apply(&self, change: &Change, fence: Option<&ShardFence>) -> Result<(), StoreError> {
        self.inner.apply(change, fence).await
    }
}

pub(crate) fn replica(t: &Triggers, settings: TriggerSettings) -> Replica {
    let placement = Placement::new(format!("replica-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        t.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    let drifts = Arc::new(AtomicU32::new(0));
    let store = Counting {
        inner: t.store.clone(),
        drifts: Arc::clone(&drifts),
    };
    Replica {
        placement: placement.clone(),
        keeper,
        pass: TriggerPass::new(
            TriggerReconciler::new(store, SpecCalendar, t.clock.clone()),
            placement,
            settings,
        ),
        drifts,
    }
}

impl Replica {
    /// The same replica after a restart: a new pass (nothing remembered) over the leases it holds.
    pub(crate) fn restarted(
        &self,
        t: &Triggers,
        settings: TriggerSettings,
    ) -> TriggerPass<Counting, SpecCalendar, TestClock> {
        TriggerPass::new(
            TriggerReconciler::new(
                Counting {
                    inner: t.store.clone(),
                    drifts: Arc::clone(&self.drifts),
                },
                SpecCalendar,
                t.clock.clone(),
            ),
            self.placement.clone(),
            settings,
        )
    }

    pub(crate) async fn keep(&self) {
        self.keeper.round().await.expect("keeper round");
    }
}

#[tokio::test]
async fn a_trigger_an_older_control_plane_published_gets_its_schedule_with_the_next_time_the_legacy_pass_planned()
 {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    let old = Uuid::now_v7();
    old_control_plane_publishes(&t, "base", old, "15 * * * *", "UTC").await;
    assert!(
        t.schedule_of(old).await.is_none(),
        "an older control plane makes no schedule"
    );
    // the legacy pass planned it for a time of its own
    let planned =
        OffsetDateTime::from_unix_timestamp(OffsetDateTime::now_utc().unix_timestamp() + 600)
            .expect("instant");
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = $2 WHERE id = $1")
        .bind(old)
        .bind(planned)
        .execute(&t.w.db.pool)
        .await
        .expect("planned");

    let found = t.drift().await;
    assert!(
        found
            .iter()
            .any(|d| matches!(d, Drift::Missing(entry) if entry.trigger == old)),
        "the trigger has no schedule: {found:?}"
    );
    t.reconcile().await;
    let schedule = t.schedule_of(old).await.expect("it has one now");
    assert_eq!(schedule.state, ScheduleState::Active);
    assert_eq!(
        (schedule.next_action_at, schedule.next_actual_at),
        (Some(planned), Some(planned)),
        "in LEGACY the two passes keep one tick between them: the schedule takes the trigger's next time"
    );
    assert_eq!(schedule.spec.cron()[0].expression(), "15 * * * *");
    assert!(t.drift().await.is_empty(), "nothing differs now");
}

#[tokio::test]
async fn after_the_handover_the_next_time_of_a_new_schedule_comes_from_the_calendar_not_from_a_trigger_nobody_plans()
 {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    t.hand_over().await;
    let old = Uuid::now_v7();
    old_control_plane_publishes(&t, "base", old, "15 * * * *", "UTC").await;
    // a trigger whose next time is long past: nothing moves it any more
    sqlx::query(
        "UPDATE workflow_triggers SET next_fire_at = now() - interval '300 days' WHERE id = $1",
    )
    .bind(old)
    .execute(&t.w.db.pool)
    .await
    .expect("stale");
    t.clock.set(OffsetDateTime::now_utc());
    t.reconcile().await;
    let schedule = t.schedule_of(old).await.expect("schedule");
    let next = schedule.next_action_at.expect("planned");
    assert!(
        next > t.clock.now(),
        "the first time after now, not a time 300 days ago"
    );
    assert!(next < t.clock.now() + Duration::hours(1) + Duration::seconds(1));
}

#[tokio::test]
async fn a_trigger_an_older_control_plane_dropped_loses_its_schedule_and_the_waiting_actions_are_dropped()
 {
    let t = triggers().await;
    t.publish(cron_workflow("old-drops", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("old-drops").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    sqlx::query(
        "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         VALUES ($1, $2, 'SPEC', to_timestamp(3600), 'SCHEDULED', 1, 'BUFFERED', now())",
    )
    .bind(schedule.tenant_id.as_uuid())
    .bind(schedule.id.as_uuid())
    .execute(&t.w.db.pool)
    .await
    .expect("a waiting action");
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("the older control plane drops the trigger");

    assert!(
        matches!(t.drift().await.as_slice(), [Drift::Orphan { trigger: id, .. }] if *id == trigger)
    );
    t.reconcile().await;
    assert_eq!(t.read(&schedule).await.state, ScheduleState::Deleted);
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM schedule_actions WHERE schedule_id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("outcome");
    assert_eq!(outcome, "DROPPED_DELETED");
    assert!(t.drift().await.is_empty());
}

#[tokio::test]
async fn a_schedule_whose_spec_drifted_from_its_trigger_is_brought_back_and_a_missing_next_time_is_planned()
 {
    let t = triggers().await;
    t.publish(cron_workflow("drifts", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("drifts").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");

    // the expression and zone of the trigger are not the schedule's (the same id, edited by hand)
    sqlx::query("UPDATE workflow_triggers SET cron = '30 * * * *', timezone = 'Europe/Lisbon' WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("drift");
    assert!(matches!(t.drift().await.as_slice(), [Drift::Respec { .. }]));
    t.reconcile().await;
    let fixed = t.read(&schedule).await;
    assert_eq!(fixed.revision, 2, "a new revision, the history stays");
    assert_eq!(fixed.spec.cron()[0].expression(), "30 * * * *");
    assert_eq!(fixed.spec.timezone(), "Europe/Lisbon");
    assert!(fixed.next_action_at.is_some());
    assert!(t.drift().await.is_empty());

    // an active schedule with no next time is planned (legacy: from the trigger's next time)
    let planned =
        OffsetDateTime::from_unix_timestamp(OffsetDateTime::now_utc().unix_timestamp() + 1_200)
            .expect("instant");
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = $2 WHERE id = $1")
        .bind(trigger)
        .bind(planned)
        .execute(&t.w.db.pool)
        .await
        .expect("planned");
    sqlx::query("UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL WHERE id = $1")
        .bind(schedule.id.as_uuid())
        .execute(&t.w.db.pool)
        .await
        .expect("lost its time");
    assert!(matches!(
        t.drift().await.as_slice(),
        [Drift::NoNextTime { .. }]
    ));
    t.reconcile().await;
    assert_eq!(t.read(&schedule).await.next_action_at, Some(planned));

    // a paused schedule has no next time and is not drift
    t.hand_over().await;
    t.service
        .pause(
            &t.w.admin,
            schedule.id,
            sagat_domain::schedules::Note::new("hold").expect("note"),
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("pause");
    assert!(t.drift().await.is_empty(), "paused is not drift");
}

#[tokio::test]
async fn a_round_asks_once_for_every_trigger_of_its_shards_and_makes_the_schedules_under_its_leases()
 {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    let mut ids = Vec::new();
    for n in 0..30 {
        let id = Uuid::now_v7();
        ids.push(id);
        old_control_plane_publishes(&t, "base", id, &format!("{} * * * *", n + 1), "UTC").await;
    }
    let r = replica(&t, TriggerSettings::default());
    assert_eq!(
        r.pass.round().await,
        TriggerRound::default(),
        "a replica that holds no shard does nothing"
    );
    assert_eq!(r.drifts.load(Ordering::SeqCst), 0);

    r.keep().await;
    // a tenant is given at most 20 entries of a round, so a tenant with a great many does not hold the others back
    let round = r.pass.round().await;
    assert_eq!(round.drift, 20, "{round:?}");
    assert_eq!(round.created + round.repaired, 20, "{round:?}");
    assert_eq!(
        r.drifts.load(Ordering::SeqCst),
        1,
        "one query for the round, not one per trigger"
    );
    let round = r.pass.round().await;
    assert_eq!(
        (round.created, round.repaired),
        (10, 1),
        "the rest, and the schedule of the trigger publication made, which is planned: {round:?}"
    );
    assert_eq!(r.drifts.load(Ordering::SeqCst), 2, "again one query");
    for id in ids {
        assert!(t.schedule_of(id).await.is_some());
    }
    assert_eq!(r.pass.round().await.drift, 0, "nothing differs now");
}

#[tokio::test]
async fn a_stuck_entry_does_not_take_the_place_of_the_others_and_a_tenant_with_many_does_not_starve_another()
 {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    // a trigger whose zone this build cannot evaluate (a stored value another build wrote): it sorts first in its tenant
    let stuck = Uuid::from_u128(1);
    old_control_plane_publishes(&t, "base", stuck, "5 * * * *", "Mars/Olympus_Mons").await;
    for n in 0..4 {
        old_control_plane_publishes(
            &t,
            "base",
            Uuid::now_v7(),
            &format!("{} * * * *", n + 10),
            "UTC",
        )
        .await;
    }
    // the other tenant has one
    let other = t.w.other_admin.clone();
    let theirs =
        t.w.defs
            .create_workflow(
                &other,
                "payments",
                "theirs",
                &serde_json::to_vec(&cron_workflow("theirs", "0 * * * *", "UTC")).expect("json"),
                &format!("k-{}", Uuid::now_v7()),
                &crate::support::engine::ctx(),
            )
            .await
            .expect("create");
    t.w.defs
        .publish(
            &other,
            theirs.workflow.id(),
            theirs.workflow.draft_revision(),
            None,
            &format!("p-{}", Uuid::now_v7()),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("publish");

    // one entry a tenant a round: the stuck one is the first of its tenant's, every round, unless it is left out
    let r = replica(
        &t,
        TriggerSettings {
            batch: 10,
            per_tenant: 1,
            scan: 2_000,
        },
    );
    r.keep().await;
    let first = r.pass.round().await;
    assert_eq!(
        first.failed, 1,
        "the stuck entry fails and is said so: {first:?}"
    );
    assert!(
        first.repaired + first.created >= 1,
        "the other tenant's entry is done in the same round: {first:?}"
    );
    let their_trigger: Uuid = sqlx::query_scalar(
        "SELECT t.id FROM workflow_triggers t JOIN workflows w ON w.id = t.workflow_id WHERE w.name = 'theirs'",
    )
    .fetch_one(&t.w.db.pool)
    .await
    .expect("theirs");
    let theirs_planned: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT next_action_at FROM schedules WHERE trigger_id = $1")
            .bind(their_trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("their schedule");
    assert!(theirs_planned.is_some(), "the other tenant was not starved");

    // the stuck entry is left out of the next rounds, so the others of its tenant get their turn, one a round
    let mut made = 0;
    for _ in 0..6 {
        let round = r.pass.round().await;
        assert_eq!(round.failed, 0, "the stuck entry is left out: {round:?}");
        made += round.created;
    }
    assert_eq!(
        made, 4,
        "every other entry of the tenant was made, one a round"
    );
    assert!(
        t.schedule_of(stuck).await.is_none(),
        "and the stuck one is still waiting for a build that can evaluate it"
    );
}

#[tokio::test]
async fn a_schedule_somebody_holds_fails_its_entry_after_a_bounded_wait_and_the_others_go_on() {
    let t = triggers().await;
    t.publish(cron_workflow("held", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("held").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let old = Uuid::now_v7();
    old_control_plane_publishes(&t, "held", old, "20 * * * *", "UTC").await;
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("dropped by an older control plane");

    // an operator's transaction holds the row of the schedule that has to be removed
    let mut held = t.w.db.pool.begin().await.expect("hold");
    sqlx::query("SELECT id FROM schedules WHERE id = $1 FOR UPDATE")
        .bind(schedule.id.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the row");

    let r = replica(&t, TriggerSettings::default());
    r.keep().await;
    let round = r.pass.round().await;
    assert_eq!(
        round.failed, 1,
        "the removal waited a bounded time and failed: {round:?}"
    );
    assert_eq!(
        round.created, 1,
        "and the entry behind it was made in the same round: {round:?}"
    );
    assert!(t.schedule_of(old).await.is_some());
    held.rollback().await.expect("release");

    // once the lock is gone the entry (after its pause) is removed; a round that is asked to ignore the pause shows it
    let again = t.reconcile().await;
    assert_eq!(again, 1);
    assert_eq!(t.read(&schedule).await.state, ScheduleState::Deleted);
}

#[tokio::test]
async fn a_stale_lease_commits_nothing() {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let old = Uuid::now_v7();
    old_control_plane_publishes(&t, "base", old, "25 * * * *", "UTC").await;
    let drift = t.drift().await;
    let entry = drift
        .iter()
        .find_map(|d| match d {
            Drift::Missing(entry) => Some(entry.clone()),
            _ => None,
        })
        .expect("a missing schedule");
    // a lease of a shard nobody holds: the transaction that would write refuses
    let lease = sagat_application::shards::Lease {
        shard: sagat_domain::sharding::Shard::of(old),
        generation: 999,
    };
    let fence = ShardFence {
        owner: "nobody".to_owned(),
        lease,
    };
    let refused = t
        .store
        .apply(&Change::Create { entry, next: None }, Some(&fence))
        .await;
    assert!(
        matches!(refused, Err(StoreError::StateConflict(_))),
        "{refused:?}"
    );
    assert!(t.schedule_of(old).await.is_none(), "nothing was written");
}

/// A measurement, not a test: how long the one query of a round and the convergence count take with 10,000 and 100,000 cron
/// triggers that have no schedule, and how long a round of 20 writes takes. Run it by hand:
/// `cargo test -p sagat-tests --test integration_schedules_bulk_operations -- --ignored --nocapture convergence_time`. The
/// environment is stated by whoever runs it (machine, PostgreSQL version, load); the numbers go in D2047.
#[tokio::test]
#[ignore = "a measurement: run by hand with --ignored --nocapture"]
async fn convergence_time_of_the_reconciliation_with_ten_thousand_and_a_hundred_thousand_triggers()
{
    use sagat_application::schedule_triggers::TriggerScheduleStore as _;
    let t = triggers().await;
    t.publish(cron_workflow("seed", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let mut have = 0_i64;
    for target in [10_000_i64, 100_000] {
        sqlx::query(
            "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
             SELECT gen_random_uuid(), s.tenant_id, s.namespace_id, s.workflow_id, s.version_id, 'cron', \
                    (n % 60)::text || ' * * * *', 'UTC' \
               FROM (SELECT * FROM workflow_triggers LIMIT 1) s CROSS JOIN generate_series(1, $1) AS n",
        )
        .bind(target - have)
        .execute(&t.w.db.pool)
        .await
        .expect("seed the triggers");
        have = target;
        sqlx::query("ANALYZE workflow_triggers")
            .execute(&t.w.db.pool)
            .await
            .expect("analyze");
        sqlx::query("ANALYZE schedules")
            .execute(&t.w.db.pool)
            .await
            .expect("analyze");
        let started = std::time::Instant::now();
        let drift = t
            .store
            .drift(&crate::support::trigger_schedules::window())
            .await
            .expect("drift")
            .drift;
        let drift_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        let convergence = t.store.convergence().await.expect("convergence");
        let convergence_ms = started.elapsed().as_millis();
        let r = replica(&t, TriggerSettings::default());
        r.keep().await;
        let started = std::time::Instant::now();
        let mut writes = 0;
        for _ in 0..25 {
            let round = r.pass.round().await;
            writes += round.created + round.repaired;
        }
        let round_ms = started.elapsed().as_millis();
        println!(
            "{target} triggers without a schedule: the drift query returned {} rows in {drift_ms} ms; the convergence count is {} in {convergence_ms} ms; \
             25 rounds wrote {writes} schedules in {round_ms} ms ({:.0} writes/s unpaced)",
            drift.len(),
            convergence.unconverged,
            f64::from(writes) / (round_ms as f64 / 1000.0),
        );
    }
}
