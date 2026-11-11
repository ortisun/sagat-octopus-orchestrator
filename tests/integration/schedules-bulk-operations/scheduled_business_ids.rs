//! The business-id prefix of a schedule's action (`T018-016`, `FR-018-016`, `FR-018-018..021`, `SC-018-006`): the id a run
//! of a schedule gets, what an action does when the id is held or remembered, and that the claim is made in the
//! transaction that creates the run, under the schedule's row. Real PostgreSQL, the real run service and firing, a clock the
//! test moves by hand (`tests/support/schedules.rs`).
//!
//! The id is `<prefix>-<nominal time>`, the nominal time RFC 3339 in UTC to the whole second (`order-2026-10-28T13:00:00Z`).
//! Customer text: the tests look at it only in the tables that hold it (the binding and the run).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::identity::RequestContext;
use sagat_application::runs::StartRun;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{BackfillRequest, DueBackfill, ScheduleRecord};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::schedules::{Ceilings, Overlap, RemainingActions};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

use crate::business_ids::{
    business, cancel, holder_of, key, runs_of, scheduler_of, start, succeed,
};
use crate::firing::{Fixture, fixture, hourly, outcomes_of, t0};
use crate::support::engine::ctx;
use crate::support::schedules::fixture_with;

/// The target of a schedule whose runs are started under `prefix`.
fn target(f: &Fixture, prefix: &str, conflict: &str, reuse: &str) -> Value {
    json!({
        "workflowId": f.workflow.to_string(),
        "businessIdPrefix": prefix,
        "businessIdConflictPolicy": conflict,
        "businessIdReusePolicy": reuse
    })
}

/// The id the action of the nominal time `t0 + hours` makes (the clock of the fixture is a whole hour).
fn id_at(prefix: &str, hours: i64) -> String {
    format!("{prefix}-2026-10-28T{:02}:00:00Z", 12 + hours)
}

/// What the table of the decisions says, oldest nominal time first: the outcome, its reason and the run it names.
async fn decisions(
    f: &Fixture,
    schedule: &ScheduleRecord,
) -> Vec<(String, Option<String>, Option<Uuid>)> {
    sqlx::query_as(
        "SELECT outcome, reason, run_id FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at, origin",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the decisions")
}

async fn counters(f: &Fixture, schedule: &ScheduleRecord) -> Value {
    sqlx::query_scalar("SELECT counters FROM schedules WHERE id = $1")
        .bind(schedule.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the counters")
}

async fn bindings(f: &Fixture) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM business_ids")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count")
}

fn conflict(reason: &str) -> (String, Option<String>, Option<Uuid>) {
    (
        "BUSINESS_ID_CONFLICT".to_owned(),
        Some(reason.to_owned()),
        None,
    )
}

// --- the write ----------------------------------------------------------------------------------

#[tokio::test]
async fn a_schedule_may_name_a_business_id_prefix_with_its_policies_and_reads_them_back() {
    let f = fixture().await;
    let schedule = f
        .create(
            "with-a-prefix",
            &hourly(),
            &json!({}),
            Some(target(&f, "order", "USE_EXISTING", "REJECT_DUPLICATE")),
        )
        .await;
    let read = f.read(&schedule).await;
    let id = read.target.business_id().expect("the prefix is kept");
    assert_eq!(id.prefix(), "order");
    assert_eq!(id.conflict(), ConflictPolicy::UseExisting);
    assert_eq!(id.reuse(), ReusePolicy::RejectDuplicate);

    // a schedule made without one has none, and an update that leaves it out takes it away
    let plain = f.create("without", &hourly(), &json!({}), None).await;
    assert!(f.read(&plain).await.target.business_id().is_none());
    let replacement = f.write_of(
        "with-a-prefix",
        &hourly(),
        &json!({}),
        Some(json!({"workflowId": f.workflow.to_string()})),
    );
    let updated = f
        .service
        .update(
            &f.w.admin,
            schedule.id,
            replacement,
            1,
            &RequestContext::default(),
        )
        .await
        .expect("update");
    assert!(updated.target.business_id().is_none());
}

// --- the id a run gets --------------------------------------------------------------------------

#[tokio::test]
async fn each_run_a_schedule_starts_is_bound_to_the_prefix_a_dash_and_the_nominal_time() {
    let f = fixture().await;
    let schedule = f
        .create(
            "orders",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            Some(target(&f, "order", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    let first = f.runs(&schedule).await[0];
    assert_eq!(runs_of(&f, &id_at("order", 1)).await, [first]);
    assert_eq!(holder_of(&f, &id_at("order", 1)).await, Some(first));

    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs_of(&f, &id_at("order", 2)).await, [runs[1]]);
    assert_eq!(bindings(&f).await, 2, "one binding for each run");
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 2)]));
}

#[tokio::test]
async fn a_schedule_without_a_prefix_starts_runs_without_a_business_id() {
    let f = fixture().await;
    let schedule = f.create("plain", &hourly(), &json!({}), None).await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    f.drain(&schedule).await;
    let run = f.runs(&schedule).await[0];
    let id: Option<String> = sqlx::query_scalar("SELECT business_id FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("run");
    assert_eq!(id, None);
    assert_eq!(bindings(&f).await, 0);
}

#[tokio::test]
async fn an_update_changes_the_prefix_the_next_actions_use() {
    let f = fixture().await;
    let schedule = f
        .create(
            "renamed",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            Some(target(&f, "old", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    f.drain(&schedule).await;
    let replacement = f.write_of(
        "renamed",
        &hourly(),
        &json!({"overlap": "ALLOW_ALL"}),
        Some(target(&f, "new", "FAIL", "ALLOW_DUPLICATE")),
    );
    f.service
        .update(
            &f.w.admin,
            schedule.id,
            replacement,
            1,
            &RequestContext::default(),
        )
        .await
        .expect("update");
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs_of(&f, &id_at("old", 1)).await, [runs[0]]);
    // the first time after the update is the one the new revision acts on
    assert_eq!(runs_of(&f, &id_at("new", 2)).await, [runs[1]]);
}

// --- an id that is held ---------------------------------------------------------------------------

#[tokio::test]
async fn an_open_holder_under_fail_makes_the_action_a_recorded_conflict_that_starts_nothing() {
    let f = fixture().await;
    let mut write = f.write_of(
        "fails",
        &hourly(),
        &json!({"pauseOnFailure": true}),
        Some(target(&f, "order", "FAIL", "ALLOW_DUPLICATE")),
    );
    write.remaining_actions = Some(RemainingActions::new(5).expect("remaining"));
    let schedule = f.try_create(write).await.expect("create");
    let holder = start(
        &f,
        business(
            &id_at("order", 1),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("a person holds the id the first action would take")
    .run
    .info
    .run_id;

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert!(f.runs(&schedule).await.is_empty(), "nothing was started");
    assert_eq!(
        decisions(&f, &schedule).await,
        [conflict("BUSINESS_ID_IN_USE")],
        "the time is acted on: a recorded decision with its reason, and no run named"
    );
    let after = f.read(&schedule).await;
    assert_eq!(
        after.next_action_at,
        Some(t0() + Duration::hours(2)),
        "the schedule goes on to the next time"
    );
    assert_eq!(
        after.retry_at, None,
        "a held id is not retried: the time is acted on"
    );
    assert_eq!(
        after.remaining_actions,
        Some(5),
        "a refused action uses up no remaining action"
    );
    assert_eq!(
        after.state,
        sagat_application::schedules::ScheduleState::Active
    );
    assert!(after.tracked_runs.is_empty());
    assert_eq!(
        counters(&f, &schedule).await,
        json!({"BUSINESS_ID_CONFLICT": 1})
    );
    assert_eq!(
        holder_of(&f, &id_at("order", 1)).await,
        Some(holder.as_uuid()),
        "the binding is the person's"
    );
    assert_eq!(f.fire(&schedule).await, Fired::Idle);

    // the next time takes its own id and starts
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs_of(&f, &id_at("order", 2)).await, [runs[0]]);
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(4));
}

#[tokio::test]
async fn use_existing_records_the_holder_as_the_result_of_the_action_and_starts_nothing() {
    let f = fixture().await;
    let schedule = f
        .create(
            "joins",
            &hourly(),
            &json!({}),
            Some(target(&f, "order", "USE_EXISTING", "ALLOW_DUPLICATE")),
        )
        .await;
    let holder = start(
        &f,
        business(
            &id_at("order", 1),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("holder")
    .run
    .info
    .run_id;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert!(f.runs(&schedule).await.is_empty(), "no run of the schedule");
    assert_eq!(
        decisions(&f, &schedule).await,
        [(
            "ALREADY_ACTED".to_owned(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
            Some(holder.as_uuid())
        )]
    );
    assert_eq!(bindings(&f).await, 1, "no second binding");
    let after = f.read(&schedule).await;
    assert!(
        after.tracked_runs.is_empty(),
        "a run that is not the schedule's is not tracked: it holds back no overlap rule"
    );
    assert_eq!(after.next_action_at, Some(t0() + Duration::hours(2)));
}

#[tokio::test]
async fn a_closed_holder_inside_its_window_follows_the_reuse_policy_of_the_schedule() {
    let f = fixture().await;
    // (prefix, reuse policy, how the holder ended, what the action became)
    let cases: [(&str, &str, bool, &str); 4] = [
        ("allow", "ALLOW_DUPLICATE", true, "STARTED"),
        ("reject", "REJECT_DUPLICATE", true, "BUSINESS_ID_CONFLICT"),
        ("failed-ok", "ALLOW_DUPLICATE_FAILED_ONLY", false, "STARTED"),
        (
            "failed-no",
            "ALLOW_DUPLICATE_FAILED_ONLY",
            true,
            "BUSINESS_ID_CONFLICT",
        ),
    ];
    let mut schedules = Vec::new();
    for (prefix, reuse, succeeded, _) in cases {
        let schedule = f
            .create(
                prefix,
                &hourly(),
                &json!({}),
                Some(target(&f, prefix, "FAIL", reuse)),
            )
            .await;
        let holder = start(
            &f,
            business(
                &id_at(prefix, 1),
                ConflictPolicy::Fail,
                ReusePolicy::AllowDuplicate,
            ),
            &key(),
        )
        .await
        .expect("holder")
        .run
        .info
        .run_id;
        if succeeded {
            succeed(&f, holder).await;
        } else {
            cancel(&f, holder).await;
        }
        schedules.push((schedule, holder));
    }
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    for ((prefix, _, _, outcome), (schedule, holder)) in cases.iter().zip(&schedules) {
        assert_eq!(f.fire(schedule).await, Fired::Acted, "{prefix}");
        let made = decisions(&f, schedule).await;
        assert_eq!(made.len(), 1, "{prefix}: {made:?}");
        assert_eq!(made[0].0, *outcome, "{prefix}");
        let bound = holder_of(&f, &id_at(prefix, 1)).await;
        if *outcome == "STARTED" {
            let run = f.runs(schedule).await[0];
            assert_eq!(bound, Some(run), "{prefix}: the binding is the new run's");
            assert_ne!(bound, Some(holder.as_uuid()));
        } else {
            assert_eq!(
                made[0].1.as_deref(),
                Some("BUSINESS_ID_REUSE_REJECTED"),
                "{prefix}"
            );
            assert_eq!(made[0].2, None, "{prefix}: a refusal names no run");
            assert!(f.runs(schedule).await.is_empty(), "{prefix}");
            assert_eq!(
                bound,
                Some(holder.as_uuid()),
                "{prefix}: the old holder keeps it"
            );
        }
    }
}

#[tokio::test]
async fn a_waiting_action_that_finds_its_id_held_is_settled_and_never_started() {
    let f = fixture().await;
    let schedule = f
        .create(
            "waits",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            Some(target(&f, "buf", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts
    let first = f.runs(&schedule).await[0];
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits for the open run
    assert_eq!(f.read(&schedule).await.buffered, 1);
    // someone takes the id the waiting action will ask for
    let holder = start(
        &f,
        business(
            &id_at("buf", 2),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("holder")
    .run
    .info
    .run_id;

    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // forgets the run, then releases the waiting action
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUSINESS_ID_CONFLICT", 1), ("STARTED", 1)]),
        "the waiting action became the refused one, not a second row and not a run"
    );
    let after = f.read(&schedule).await;
    assert_eq!(after.buffered, 0, "it does not wait any more");
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        holder_of(&f, &id_at("buf", 2)).await,
        Some(holder.as_uuid())
    );
    assert_eq!(
        f.fire(&schedule).await,
        Fired::Idle,
        "nothing is left to release"
    );
}

#[tokio::test]
async fn a_firing_that_waits_for_a_competing_claim_records_the_conflict_when_it_commits() {
    let f = fixture().await;
    let schedule = f
        .create(
            "races",
            &hourly(),
            &json!({}),
            Some(target(&f, "race", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    // an open run, which a competing transaction binds to the id the action will ask for but has not committed yet
    let holder = start(
        &f,
        business(
            "some-other-id",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("an open run")
    .run
    .info
    .run_id;
    let mut competing =
        f.w.db
            .pool
            .begin()
            .await
            .expect("the competing transaction");
    sqlx::query(
        "INSERT INTO business_ids (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
         SELECT tenant_id, namespace_id, $2, id, workflow_id, now(), 2592000 FROM runs WHERE id = $1",
    )
    .bind(holder.as_uuid())
    .bind(id_at("race", 1))
    .execute(&mut *competing)
    .await
    .expect("the uncommitted claim");

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    let firing = f.fire(&schedule);
    let release = async {
        f.wait_for_a_backend_on_a_lock("the claim waits for the competing one")
            .await;
        competing
            .commit()
            .await
            .expect("the competing claim commits");
    };
    let (fired, ()) = tokio::join!(firing, release);
    assert_eq!(fired, Fired::Acted);
    assert_eq!(
        decisions(&f, &schedule).await,
        [conflict("BUSINESS_ID_IN_USE")]
    );
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(
        holder_of(&f, &id_at("race", 1)).await,
        Some(holder.as_uuid()),
        "the competing claim won; there is one binding of the id"
    );
}

/// A decision that starts no run is still the decision of a subject who may start runs: a revocation that has returned stops it,
/// as it stops the run (`FR-018-015`, `authority::`), also on the path where the id is held and the transaction ends at the claim.
/// Found by the independent security review of this slice: the claim ended the transaction before the membership was looked at
/// again, so the action was recorded as a conflict (or as the holder's) instead of `AUTHORITY_REVOKED`.
#[tokio::test]
async fn a_revocation_that_commits_after_the_action_read_the_membership_stops_an_action_whose_id_is_held_too()
 {
    for policy in ["FAIL", "USE_EXISTING"] {
        let f = fixture().await;
        let author =
            f.w.member("author-held", sagat_domain::identity::Role::Developer)
                .await;
        let schedule = f
            .create_as(
                &author,
                "late-held",
                &hourly(),
                &json!({}),
                Some(target(&f, "late", policy, "ALLOW_DUPLICATE")),
            )
            .await;
        start(
            &f,
            business(
                &id_at("late", 1),
                ConflictPolicy::Fail,
                ReusePolicy::AllowDuplicate,
            ),
            &key(),
        )
        .await
        .expect("a person holds the id");
        f.clock.advance(Duration::hours(1) + Duration::seconds(1));
        let fired = f
            .revoked_while_the_action_waits(&schedule, author.subject().as_str(), f.fire(&schedule))
            .await;
        assert_eq!(fired, Fired::Acted, "{policy}");
        assert_eq!(
            f.outcomes(&schedule).await,
            outcomes_of(&[("AUTHORITY_REVOKED", 1)]),
            "{policy}: the revocation, not a decision about the id"
        );
        assert_eq!(
            f.read(&schedule).await.state,
            sagat_application::schedules::ScheduleState::Paused,
            "{policy}"
        );
        assert!(f.runs(&schedule).await.is_empty(), "{policy}");
    }
}

/// And the plain case, which the pass has always caught before it asks for the run: an author who lost `run:start` before the
/// action reads the membership is `AUTHORITY_REVOKED` whatever holds the id.
#[tokio::test]
async fn an_author_who_lost_the_permission_is_revoked_whatever_holds_the_id() {
    let f = fixture().await;
    let author =
        f.w.member("author-gone", sagat_domain::identity::Role::Developer)
            .await;
    let schedule = f
        .create_as(
            &author,
            "gone-held",
            &hourly(),
            &json!({}),
            Some(target(&f, "gone", "USE_EXISTING", "ALLOW_DUPLICATE")),
        )
        .await;
    start(
        &f,
        business(
            &id_at("gone", 1),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("a person holds the id");
    sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2")
        .bind(schedule.tenant_id.as_uuid())
        .bind(author.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("revoke");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)])
    );
    assert_eq!(
        f.read(&schedule).await.state,
        sagat_application::schedules::ScheduleState::Paused
    );
}
/// Found by the independent correctness review of this slice. A waiting action keeps the last remaining action for itself, so the
/// next time is "exhausted" (`decide`: nothing is left to give) and the schedule has no next time while it waits. When the waiting
/// action then finds its id held it is settled **without using the action up**, so the schedule has an action left and must go on
/// from the first time after now: it must not be left `ACTIVE` with a remaining action and no next time for ever.
#[tokio::test]
async fn a_waiting_action_that_finds_its_id_held_gives_back_the_action_it_kept_and_the_schedule_goes_on()
 {
    let f = fixture().await;
    let mut write = f.write_of(
        "strands",
        &hourly(),
        &json!({"overlap": "BUFFER_ONE"}),
        Some(target(&f, "strand", "FAIL", "ALLOW_DUPLICATE")),
    );
    write.remaining_actions = Some(RemainingActions::new(2).expect("remaining"));
    let schedule = f.try_create(write).await.expect("create");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts: one action left
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits for the open run and keeps that action for itself
    start(
        &f,
        business(
            &id_at("strand", 2),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("a person holds the id of the waiting action");
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 15:00: nothing is left to give while the waiting action keeps the last one
    let waiting = f.read(&schedule).await;
    assert_eq!(
        (
            waiting.remaining_actions,
            waiting.buffered,
            waiting.next_action_at
        ),
        (Some(1), 1, None),
        "the premise: the reservation made the schedule act as exhausted"
    );

    let first = f.runs(&schedule).await[0];
    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // forgets the run, releases the waiting action: its id is held
    let settled = f.read(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUSINESS_ID_CONFLICT", 1), ("STARTED", 1)])
    );
    assert_eq!(settled.buffered, 0);
    assert_eq!(
        settled.remaining_actions,
        Some(1),
        "the action it kept is given back"
    );
    assert_eq!(
        settled.next_action_at,
        Some(t0() + Duration::hours(4)),
        "and the schedule goes on from the first time after now, not never"
    );

    // it does: the next time starts the run it has left
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 2);
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(0));
}
// --- the other ways an action is made ------------------------------------------------------------

#[tokio::test]
async fn trigger_now_takes_the_second_of_the_request_and_a_second_in_the_same_second_finds_the_id_held()
 {
    let f = fixture().await;
    let schedule = f
        .create(
            "now",
            &hourly(),
            &json!({}),
            Some(target(&f, "now", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    let ctx = RequestContext::default();
    let ask = |k: &'static str| TriggerRequest {
        overlap: Some(Overlap::AllowAll),
        key: k,
    };
    // 12:01:40.250: the id has the whole second
    f.clock.set(t0() + Duration::milliseconds(100_250));
    let first = f
        .firing
        .trigger(&f.w.admin, schedule.id, &ask("trigger-key-0001"), &ctx)
        .await
        .expect("trigger");
    assert_eq!(first.outcome, "STARTED");
    let run = first.run_id.expect("the run");
    assert_eq!(runs_of(&f, "now-2026-10-28T12:01:40Z").await, [run]);

    // 12:01:40.750: the same second, the id is held by the run of the first
    f.clock.set(t0() + Duration::milliseconds(100_750));
    let second = f
        .firing
        .trigger(&f.w.admin, schedule.id, &ask("trigger-key-0002"), &ctx)
        .await
        .expect("trigger");
    assert_eq!(second.outcome, "BUSINESS_ID_CONFLICT");
    assert_eq!(second.reason.as_deref(), Some("BUSINESS_ID_IN_USE"));
    assert_eq!(second.run_id, None);
    assert_eq!(second.kind, sagat_domain::schedules::ActionKind::Triggered);
    // a retry of that request is told the same
    let again = f
        .firing
        .trigger(&f.w.admin, schedule.id, &ask("trigger-key-0002"), &ctx)
        .await
        .expect("trigger again");
    assert_eq!(again, second);

    // the next second is another id
    f.clock.set(t0() + Duration::milliseconds(101_250));
    let third = f
        .firing
        .trigger(&f.w.admin, schedule.id, &ask("trigger-key-0003"), &ctx)
        .await
        .expect("trigger");
    assert_eq!(third.outcome, "STARTED");
    assert_eq!(f.runs(&schedule).await.len(), 2);
    assert_eq!(bindings(&f).await, 2);
}

#[tokio::test]
async fn a_backfilled_time_whose_id_is_held_is_counted_as_a_conflict_and_the_backfill_goes_on() {
    let f = fixture().await;
    let schedule = f
        .create(
            "past",
            &hourly(),
            &json!({}),
            Some(target(&f, "back", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    // someone holds the id of the first of the two times of the range
    let holder = start(
        &f,
        business(
            &id_at("back", -2),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("holder")
    .run
    .info
    .run_id;
    let backfill = f
        .service
        .create_backfill(
            &f.w.admin,
            schedule.id,
            &BackfillRequest {
                start: t0() - Duration::hours(2),
                end: t0(),
                overlap: Some(Overlap::AllowAll),
            },
            &RequestContext::default(),
        )
        .await
        .expect("backfill");
    let due = DueBackfill {
        tenant_id: backfill.tenant_id,
        schedule: backfill.schedule,
        id: backfill.id,
    };
    for _ in 0..10 {
        if f.firing.backfill_step(&due, None).await.expect("step") != Fired::Acted {
            break;
        }
    }
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT kind, outcome, reason FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("rows");
    assert_eq!(
        rows,
        [
            (
                "BACKFILL".to_owned(),
                "BUSINESS_ID_CONFLICT".to_owned(),
                Some("BUSINESS_ID_IN_USE".to_owned())
            ),
            ("BACKFILL".to_owned(), "STARTED".to_owned(), None)
        ]
    );
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs_of(&f, &id_at("back", -1)).await, [runs[0]]);
    assert_eq!(
        holder_of(&f, &id_at("back", -2)).await,
        Some(holder.as_uuid())
    );
    let listed = f
        .service
        .backfills(&f.w.admin, schedule.id)
        .await
        .expect("list");
    assert_eq!(listed[0].state.as_str(), "COMPLETED");
    assert_eq!(
        listed[0]
            .counters
            .iter()
            .map(|(k, n)| (k.as_str(), *n))
            .collect::<Vec<_>>(),
        [("BUSINESS_ID_CONFLICT", 1), ("STARTED", 1)]
    );
}

/// `USE_EXISTING` for the other two ways an action is made (found by the independent review: only `FAIL` was tested there): the open run
/// that holds the id answers trigger-now and the step of a backfill, nothing starts, and the decision names it.
#[tokio::test]
async fn use_existing_answers_trigger_now_and_a_backfill_step_with_the_run_that_holds_the_id() {
    let f = fixture().await;
    let schedule = f
        .create(
            "joins-by-hand",
            &hourly(),
            &json!({}),
            Some(target(&f, "hand", "USE_EXISTING", "ALLOW_DUPLICATE")),
        )
        .await;
    let held = |id: String| {
        let f = &f;
        async move {
            start(
                f,
                business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &key(),
            )
            .await
            .expect("holder")
            .run
            .info
            .run_id
            .as_uuid()
        }
    };
    // trigger-now, 12:01:40.250: the id of the second
    let by_hand = held("hand-2026-10-28T12:01:40Z".to_owned()).await;
    f.clock.set(t0() + Duration::milliseconds(100_250));
    let ctx = RequestContext::default();
    let triggered = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: Some(Overlap::AllowAll),
                key: "use-existing-key-0001",
            },
            &ctx,
        )
        .await
        .expect("trigger");
    assert_eq!(triggered.outcome, "ALREADY_ACTED");
    assert_eq!(triggered.reason.as_deref(), Some("BUSINESS_ID_IN_USE"));
    assert_eq!(triggered.run_id, Some(by_hand));
    assert!(f.runs(&schedule).await.is_empty(), "nothing started");

    // a backfill of the 11:00 time, whose id a person holds
    let for_backfill = held(id_at("hand", -1)).await;
    f.clock.set(t0());
    let backfill = f
        .service
        .create_backfill(
            &f.w.admin,
            schedule.id,
            &BackfillRequest {
                start: t0() - Duration::hours(1),
                end: t0(),
                overlap: Some(Overlap::AllowAll),
            },
            &RequestContext::default(),
        )
        .await
        .expect("backfill");
    let due = DueBackfill {
        tenant_id: backfill.tenant_id,
        schedule: backfill.schedule,
        id: backfill.id,
    };
    for _ in 0..10 {
        if f.firing.backfill_step(&due, None).await.expect("step") != Fired::Acted {
            break;
        }
    }
    let rows: Vec<(String, String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT kind, outcome, reason, run_id FROM schedule_actions \
         WHERE schedule_id = $1 AND kind = 'BACKFILL' ORDER BY nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("rows");
    assert_eq!(
        rows,
        [(
            "BACKFILL".to_owned(),
            "ALREADY_ACTED".to_owned(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
            Some(for_backfill)
        )]
    );
    assert!(f.runs(&schedule).await.is_empty(), "nothing started");
}
// --- a time that is acted on twice ----------------------------------------------------------------

/// What a time acted on twice leaves behind: one run, one binding with its holder unchanged, one `STARTED` row and no decision of a
/// conflict with itself. It does **not** show the order of the replay and the claim in `create` (D2193): a claim made first would find
/// the first run as the holder, and the decision it recorded would be refused by the identity of the action's row, with the same answer
/// (`Refused`) and the same state, so no test can tell the two orders apart and none claims to.
#[tokio::test]
async fn a_time_that_is_acted_on_twice_leaves_one_run_one_binding_and_one_row() {
    let f = fixture().await;
    let schedule = f
        .create(
            "again",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            Some(target(&f, "order", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    let run = f.runs(&schedule).await[0];

    // the same time is due again (a pass that did not see the commit): it finds its own key
    sqlx::query("UPDATE schedules SET next_action_at = $2, next_actual_at = $2 WHERE id = $1")
        .bind(schedule.id.as_uuid())
        .bind(t0() + Duration::hours(1))
        .execute(&f.w.db.pool)
        .await
        .expect("rewind");
    assert!(matches!(f.fire(&schedule).await, Fired::Refused(_)));
    assert_eq!(f.runs(&schedule).await, [run], "one run");
    assert_eq!(bindings(&f).await, 1, "one binding");
    assert_eq!(holder_of(&f, &id_at("order", 1)).await, Some(run));
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
    assert_eq!(
        counters(&f, &schedule).await,
        json!({"STARTED": 1}),
        "no decision of a conflict with its own run was counted"
    );
    assert_eq!(
        decisions(&f, &schedule).await,
        [("STARTED".to_owned(), None, Some(run))],
        "the one row is the start"
    );
}

// --- what a waiting action is made under (D2186) --------------------------------------------------

/// A waiting action takes the prefix the schedule has when it is **released**, not the one it had when it was buffered: like the workflow,
/// the version policy and the input, the prefix is the head revision's (`start_run` reads the schedule as it stands), while the action's
/// row keeps the revision it was decided under (`FR-018-014`). The independent review asked which of the two rules holds; this shows it
/// (D2186), so that a change of it is a decision and not a surprise.
#[tokio::test]
async fn a_waiting_action_takes_the_prefix_of_the_head_revision_when_it_is_released() {
    let f = fixture().await;
    let schedule = f
        .create(
            "waits-then-renamed",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            Some(target(&f, "old", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts, under revision 1
    let first = f.runs(&schedule).await[0];
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits for it, under revision 1
    assert_eq!(f.read(&schedule).await.buffered, 1);

    // the schedule is updated while the action waits: revision 2 has another prefix
    let replacement = f.write_of(
        "waits-then-renamed",
        &hourly(),
        &json!({"overlap": "BUFFER_ONE"}),
        Some(target(&f, "new", "FAIL", "ALLOW_DUPLICATE")),
    );
    f.service
        .update(
            &f.w.admin,
            schedule.id,
            replacement,
            1,
            &RequestContext::default(),
        )
        .await
        .expect("update");
    assert_eq!(f.read(&schedule).await.revision, 2);

    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // the waiting action is released
    let released = f.runs(&schedule).await[1];
    assert_eq!(
        runs_of(&f, &id_at("new", 2)).await,
        [released],
        "the run is under the prefix of the head revision, with the nominal time it waited for"
    );
    assert!(
        runs_of(&f, &id_at("old", 2)).await.is_empty(),
        "and not under the prefix it was buffered under"
    );
    let revision: i32 = sqlx::query_scalar(
        "SELECT revision FROM schedule_actions WHERE schedule_id = $1 AND nominal_at = $2",
    )
    .bind(schedule.id.as_uuid())
    .bind(t0() + Duration::hours(2))
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the row of the waiting action");
    assert_eq!(
        revision, 1,
        "the row keeps the revision the action was decided under"
    );
}

// --- the other policies and shapes of a held id (the tests the reviews asked for) ------------------

/// `USE_EXISTING` uses no remaining action up either: the holder stands for the action (`FR-018-012`, `FR-018-019`), and the count is
/// used by the next time that starts a run.
#[tokio::test]
async fn use_existing_uses_no_remaining_action_up() {
    let f = fixture().await;
    let mut write = f.write_of(
        "counted-join",
        &hourly(),
        &json!({}),
        Some(target(&f, "join", "USE_EXISTING", "ALLOW_DUPLICATE")),
    );
    write.remaining_actions = Some(RemainingActions::new(2).expect("remaining"));
    let schedule = f.try_create(write).await.expect("create");
    let holder = start(
        &f,
        business(
            &id_at("join", 1),
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("a person holds the id of 13:00")
    .run
    .info
    .run_id
    .as_uuid();
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00: the holder stands for the action
    let joined = f.read(&schedule).await;
    assert_eq!(
        (
            joined.remaining_actions,
            joined.next_action_at,
            joined.tracked_runs.len()
        ),
        (Some(2), Some(t0() + Duration::hours(2)), 0),
        "nothing is used up, the schedule goes on, and it tracks no run of its own"
    );
    assert_eq!(
        decisions(&f, &schedule).await,
        [(
            "ALREADY_ACTED".to_owned(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
            Some(holder)
        )]
    );
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 is free: it starts a run and uses one up
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(1));
}

/// `BUFFER_ALL` with two waiting actions whose ids are both held: each is settled in place as it comes to be released, one after the
/// other, and the schedule, which acted as exhausted while they kept both remaining actions, is given them back and goes on.
#[tokio::test]
async fn two_waiting_actions_whose_ids_are_held_are_settled_one_after_the_other() {
    let f = fixture().await;
    let mut write = f.write_of(
        "two-wait",
        &hourly(),
        &json!({"overlap": "BUFFER_ALL", "bufferLimit": 5}),
        Some(target(&f, "all", "FAIL", "ALLOW_DUPLICATE")),
    );
    write.remaining_actions = Some(RemainingActions::new(3).expect("remaining"));
    let schedule = f.try_create(write).await.expect("create");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts: two actions left
    let first = f.runs(&schedule).await[0];
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 15:00 waits too
    for hours in [2, 3] {
        start(
            &f,
            business(
                &id_at("all", hours),
                ConflictPolicy::Fail,
                ReusePolicy::AllowDuplicate,
            ),
            &key(),
        )
        .await
        .expect("a person holds the id of a waiting action");
    }
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 16:00: both waiting actions keep the two remaining actions
    let waiting = f.read(&schedule).await;
    assert_eq!(
        (
            waiting.remaining_actions,
            waiting.buffered,
            waiting.next_action_at
        ),
        (Some(2), 2, None),
        "the premise: two waiting actions, two remaining actions, no next time"
    );

    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // forgets the run, then releases the oldest one: its id is held
    let midway = f.read(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUFFERED", 1), ("BUSINESS_ID_CONFLICT", 1), ("STARTED", 1)]),
        "the oldest became the refused decision in place; the other still waits (it is looked at again in five seconds)"
    );
    assert_eq!(
        (
            midway.buffered,
            midway.remaining_actions,
            midway.next_action_at
        ),
        (1, Some(2), Some(t0() + Duration::hours(5))),
        "the schedule is given its time back at the first settling, and the second action keeps its own"
    );
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // the next one is released: its id is held too
    let after = f.read(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUSINESS_ID_CONFLICT", 2), ("STARTED", 1)]),
        "both became the refused decision, in place, one after the other"
    );
    assert_eq!(
        (after.buffered, after.remaining_actions, after.state),
        (
            0,
            Some(2),
            sagat_application::schedules::ScheduleState::Active
        ),
        "nothing waits, nothing was used up"
    );
    assert_eq!(
        after.next_action_at,
        Some(t0() + Duration::hours(5)),
        "the schedule goes on from the first time after the settling"
    );
    assert_eq!(f.runs(&schedule).await.len(), 1, "no run for either");
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 2, "17:00 starts");
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(1));
}

/// The holder of a continuation chain is its first run, and the work is in the segment that is current: `ALREADY_ACTED` names that
/// segment, as `startRun` with `USE_EXISTING` does.
#[tokio::test]
async fn use_existing_names_the_current_segment_of_a_chain_that_holds_the_id() {
    let f = fixture().await;
    let id = id_at("chained", 1);
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": format!("scheduled-chain-{}", Uuid::now_v7())},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = f.w.publish(document).await;
    let initial =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: br#"{"next": {"next": {"segment": 3}}}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &ctx(),
            )
            .await
            .expect("the chain starts under the id of 13:00")
            .run
            .info
            .run_id;
    // the first segment ends and the second is the current one
    let scheduler = scheduler_of(&f).await;
    let session = f.w.session(&["segment.work"]).await;
    let tasks = f.w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 1);
    f.w.complete(&session, tasks.first().expect("a task"), &json!({}))
        .await
        .expect("the first segment works");
    let mut current = initial.as_uuid();
    for _ in 0..8 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        current =
            sqlx::query_scalar("SELECT current_run_id FROM execution_chains WHERE chain_id = $1")
                .bind(initial.as_uuid())
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the chain");
        if current != initial.as_uuid() {
            break;
        }
    }
    assert_ne!(
        current,
        initial.as_uuid(),
        "the premise: the chain moved on to its second segment"
    );

    let schedule = f
        .create(
            "joins-a-chain",
            &hourly(),
            &json!({}),
            Some(target(&f, "chained", "USE_EXISTING", "ALLOW_DUPLICATE")),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    assert_eq!(
        decisions(&f, &schedule).await,
        [(
            "ALREADY_ACTED".to_owned(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
            Some(current)
        )],
        "the action names the segment that is current, not the first run of the chain"
    );
    assert!(f.runs(&schedule).await.is_empty(), "nothing started");
}

// --- a held id under a catch-up ---------------------------------------------------------------------

fn minutely() -> Value {
    json!({"intervals": [{"everyMs": 60_000}]})
}

/// The id the action of the minute `n` after the start of the schedule makes.
fn id_of_minute(prefix: &str, n: u32) -> String {
    format!("{prefix}-2026-10-28T12:{n:02}:00Z")
}

async fn hold(f: &Fixture, id: &str) -> Uuid {
    start(
        f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a person holds the id")
    .run
    .info
    .run_id
    .as_uuid()
}

/// In `ALL`, a time of the recovery whose id is held is acted on like any other: it is the decision of its time and it counts against
/// the actions of the recovery (the ceiling is of acts, not of runs); the other times start, in order, and the rest is one summary.
#[tokio::test]
async fn a_recovery_in_mode_all_settles_the_time_whose_id_is_held_and_starts_the_others() {
    let f = fixture_with(
        t0(),
        Ceilings {
            catchup_actions: 5,
            ..Ceilings::default()
        },
    )
    .await;
    let schedule = f
        .create(
            "backlog-all-held",
            &minutely(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            Some(target(&f, "late", "FAIL", "ALLOW_DUPLICATE")),
        )
        .await;
    let holder = hold(&f, &id_of_minute("late", 2)).await;
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));
    f.drain(&schedule).await;

    let times: Vec<time::OffsetDateTime> = sqlx::query_scalar(
        "SELECT schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY schedule_nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("nominal times");
    assert_eq!(
        times,
        [1, 3, 4, 5].map(|n| t0() + Duration::minutes(n)).to_vec(),
        "minute 2 is the decision of its time, and the recovery still stops at its five acts"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[
            ("BUSINESS_ID_CONFLICT", 1),
            ("COALESCED", 1),
            ("STARTED", 4)
        ])
    );
    assert_eq!(holder_of(&f, &id_of_minute("late", 2)).await, Some(holder));
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(t0() + Duration::minutes(21)),
        "and the schedule goes on after the recovery"
    );
    assert_eq!(f.drain(&schedule).await, 0, "nothing is left to do");
}

/// In `ONE`, the earliest time of the recovery is the only one acted on: when its id is held it is settled and no run starts; every other
/// time is summarized.
#[tokio::test]
async fn a_recovery_in_mode_one_whose_earliest_time_is_held_starts_nothing() {
    let f = fixture_with(
        t0(),
        Ceilings {
            catchup_actions: 5,
            ..Ceilings::default()
        },
    )
    .await;
    let schedule = f
        .create(
            "backlog-one-held",
            &minutely(),
            &json!({"catchupMode": "ONE", "overlap": "ALLOW_ALL"}),
            Some(target(&f, "once", "USE_EXISTING", "ALLOW_DUPLICATE")),
        )
        .await;
    let holder = hold(&f, &id_of_minute("once", 1)).await;
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));
    f.drain(&schedule).await;
    assert!(f.runs(&schedule).await.is_empty(), "no run");
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("ALREADY_ACTED", 1), ("COALESCED", 1)])
    );
    let acted: (Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT reason, run_id FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'ALREADY_ACTED'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the decision");
    assert_eq!(acted, (Some("BUSINESS_ID_IN_USE".to_owned()), Some(holder)));
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(t0() + Duration::minutes(21))
    );
}
