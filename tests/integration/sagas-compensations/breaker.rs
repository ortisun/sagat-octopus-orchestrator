//! The circuit breaker of compensations: a provider that is down is not called again by every run
//! that has something to undo (FR-003-003).

use std::time::Duration;

use sagat_application::execution::BreakerPolicy;
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use sagat_domain::identity::Role;
use serde_json::json;

use crate::settlement;
use crate::support::engine::{
    Session, World, document, effect_activities, subject_of, world_with_breaker,
};

/// Opens after two failures in a row, for a minute (twice as long after each failure past that).
const POLICY: BreakerPolicy = BreakerPolicy {
    threshold: 2,
    open_for: Duration::from_secs(60),
    max_open: Duration::from_secs(600),
};

async fn breaker_world(policy: BreakerPolicy) -> World {
    world_with_breaker(WorkerSettings::default(), policy).await
}

/// One worker for the forward work and one for the compensations, so that fetching the one never
/// leases (and drops) a task of the other.
struct Pair {
    forward: Session,
    undo: Session,
}

async fn pair(w: &World) -> Pair {
    Pair {
        forward: w
            .session(&["act.reserve", "act.submit", "act.notify"])
            .await,
        undo: w.session(&["undo.reserve", "undo.submit"]).await,
    }
}

/// A run of `settlement` whose `reserve` completed and whose `submit` failed for good, so the
/// compensation of `reserve` is ready.
async fn needing_an_undo(
    w: &World,
    p: &Pair,
    workflow: sagat_domain::definition::WorkflowId,
) -> RunId {
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task_of(&p.forward, "act.reserve").await;
    w.complete(&p.forward, &reserve, &json!({}))
        .await
        .expect("reserve");
    let submit = w.one_task_of(&p.forward, "act.submit").await;
    w.fail(&p.forward, &submit, false)
        .await
        .expect("fails for good");
    run
}

/// The oldest ready compensation is leased and fails with a transient error, and its retry falls
/// due again at once. Returns the run it belonged to.
async fn fail_one_undo(w: &World, p: &Pair) -> RunId {
    let mut tasks = w.poll(&p.undo, 1).await;
    assert_eq!(tasks.len(), 1, "a compensation is ready");
    let undo = tasks.remove(0);
    w.fail_with(&p.undo, &undo, "PROVIDER_DOWN", true)
        .await
        .expect("the provider is down");
    w.age_retry(undo.run_id, "reserve~c").await;
    w.scheduler.tick().await;
    undo.run_id
}

async fn breaker_row(w: &World) -> Option<(i32, bool)> {
    sqlx::query_as(
        "SELECT failures, COALESCE(opened_until > clock_timestamp(), false) FROM compensation_breakers \
         WHERE tenant_id = $1 AND activity = 'undo.reserve'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_optional(&w.db.pool)
    .await
    .expect("breaker")
}

/// Pulls the end of the window into the past: the breaker is half open.
async fn let_the_window_pass(w: &World) {
    sqlx::query(
        "UPDATE compensation_breakers SET opened_until = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("age the breaker");
}

#[tokio::test]
async fn consecutive_failures_open_the_breaker_and_other_runs_wait_without_losing_anything() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    let first = needing_an_undo(&w, &p, workflow).await;
    let second = needing_an_undo(&w, &p, workflow).await;

    // Two failed attempts in a row, whichever run they belong to.
    fail_one_undo(&w, &p).await;
    assert_eq!(
        breaker_row(&w).await,
        Some((1, false)),
        "one failure is below the threshold"
    );
    fail_one_undo(&w, &p).await;
    assert_eq!(
        breaker_row(&w).await,
        Some((2, true)),
        "open after the second failure"
    );

    // The provider is left alone: nothing is leased although both undos are ready.
    for run in [first, second] {
        assert_eq!(
            w.step_status(run, "reserve~c").await,
            Some(StepStatus::Ready)
        );
    }
    assert!(
        w.poll(&p.undo, 8).await.is_empty(),
        "the breaker holds the compensations back"
    );
    // Neither run is failing: they wait, still compensating, with their retry budget intact.
    assert_eq!(w.run_status(first).await, RunStatus::Compensating);
    assert_eq!(w.run_status(second).await, RunStatus::Compensating);
    w.assert_consistent(first).await;
}

#[tokio::test]
async fn forward_work_and_other_providers_are_not_held_back() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    needing_an_undo(&w, &p, workflow).await;
    needing_an_undo(&w, &p, workflow).await;
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    assert_eq!(breaker_row(&w).await.map(|r| r.1), Some(true));

    // A new run still gets its forward work, whatever the state of `undo.reserve`.
    let fresh = w.start(workflow, &json!({})).await;
    let task = w.one_task(&p.forward).await;
    assert_eq!(
        (task.activity.as_str(), task.run_id),
        ("act.reserve", fresh)
    );
}

#[tokio::test]
async fn after_the_window_exactly_one_probe_is_leased_and_its_success_closes_the_breaker() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    for _ in 0..3 {
        needing_an_undo(&w, &p, workflow).await;
    }
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    assert!(w.poll(&p.undo, 8).await.is_empty());

    let_the_window_pass(&w).await;
    // Half open: of three ready compensations one is leased, the probe.
    let probe = w.poll(&p.undo, 8).await;
    assert_eq!(probe.len(), 1, "exactly one probe");
    assert!(
        w.poll(&p.undo, 8).await.is_empty(),
        "nobody else until the probe reports"
    );

    w.complete(&p.undo, &probe[0], &json!({}))
        .await
        .expect("the provider answers");
    assert_eq!(breaker_row(&w).await, None, "a success closes the breaker");
    // Everyone else is let through, now.
    let rest = w.poll(&p.undo, 8).await;
    assert_eq!(rest.len(), 2);
}

#[tokio::test]
async fn a_probe_that_fails_reopens_the_breaker_for_longer() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    needing_an_undo(&w, &p, workflow).await;
    needing_an_undo(&w, &p, workflow).await;
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    let_the_window_pass(&w).await;

    let probe = w.one_task_of(&p.undo, "undo.reserve").await;
    w.fail_with(&p.undo, &probe, "PROVIDER_DOWN", true)
        .await
        .expect("still down");
    let (failures, remaining): (i32, f64) = sqlx::query_as(
        "SELECT failures, extract(epoch FROM opened_until - clock_timestamp())::float8 \
         FROM compensation_breakers",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("breaker");
    assert_eq!(failures, 3);
    // One failure past the threshold: twice the base window (120 s), not the base.
    assert!((100.0..=120.0).contains(&remaining), "{remaining} s left");
}

#[tokio::test]
async fn the_window_never_exceeds_its_ceiling_however_long_the_provider_is_down() {
    let w = breaker_world(BreakerPolicy {
        threshold: 1,
        open_for: Duration::from_secs(60),
        max_open: Duration::from_secs(100),
    })
    .await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    let run = needing_an_undo(&w, &p, workflow).await;
    for _ in 0..4 {
        let_the_window_pass(&w).await;
        let _ = run;
        let undo = w.one_task_of(&p.undo, "undo.reserve").await;
        w.fail_with(&p.undo, &undo, "PROVIDER_DOWN", true)
            .await
            .expect("down");
        w.age_retry(run, "reserve~c").await;
        w.scheduler.tick().await;
    }
    let remaining: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM opened_until - clock_timestamp())::float8 FROM compensation_breakers",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("breaker");
    assert!(remaining <= 100.0, "{remaining} s left, the ceiling is 100");
}

#[tokio::test]
async fn an_attempt_that_runs_out_of_time_counts_like_one_that_failed() {
    let w = breaker_world(BreakerPolicy {
        threshold: 1,
        ..POLICY
    })
    .await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    let run = needing_an_undo(&w, &p, workflow).await;
    let _undo = w.one_task_of(&p.undo, "undo.reserve").await;
    // The worker never answers: its lease runs out.
    w.age_lease(run, "reserve~c").await;
    w.scheduler.tick().await;
    assert_eq!(breaker_row(&w).await, Some((1, true)));
}

#[tokio::test]
async fn the_breaker_belongs_to_one_tenant() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    needing_an_undo(&w, &p, workflow).await;
    needing_an_undo(&w, &p, workflow).await;
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    assert_eq!(breaker_row(&w).await.map(|r| r.1), Some(true));

    // Another tenant's provider with the same operation name is a different provider.
    let theirs = w.publish_as_other_tenant(settlement()).await;
    let their_worker = sagat_domain::identity::Principal::new(
        w.other_admin.tenant_id(),
        subject_of("worker-b"),
        Role::Worker,
    );
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let session = w.session_as(&their_worker, &names, 100).await;
    let run = w
        .start_as_other_tenant(theirs, &json!({}))
        .await
        .expect("start");
    let reserve = w.one_task(&session).await;
    w.complete(&session, &reserve, &json!({}))
        .await
        .expect("reserve");
    let submit = w.one_task(&session).await;
    w.fail(&session, &submit, false).await.expect("fails");
    let undo = w.one_task(&session).await;
    assert_eq!((undo.activity.as_str(), undo.run_id), ("undo.reserve", run));
    w.complete(&session, &undo, &json!({}))
        .await
        .expect("undone");
}

#[tokio::test]
async fn with_the_breaker_off_nothing_is_ever_held_back() {
    let w = breaker_world(BreakerPolicy::OFF).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    let run = needing_an_undo(&w, &p, workflow).await;
    for _ in 0..4 {
        fail_one_undo(&w, &p).await;
    }
    assert_eq!(breaker_row(&w).await, None);
    assert_eq!(w.one_task_of(&p.undo, "undo.reserve").await.run_id, run);
}

#[tokio::test]
async fn an_operators_decision_is_not_a_failure_of_the_provider() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(crate::review::one_shot(None)).await;
    let p = pair(&w).await;
    let run = needing_an_undo(&w, &p, workflow).await;
    // The only attempt of the compensation fails: the run waits for an operator, and the breaker
    // has seen one failure.
    fail_one_undo(&w, &p).await;
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert_eq!(breaker_row(&w).await, Some((1, false)));

    // What the operator decides is not the provider's doing: the counter stays where it was.
    w.review(
        run,
        "reserve~c",
        sagat_application::runs::ReviewAction::Retry,
        "provider is back",
    )
    .await
    .expect("retry");
    assert_eq!(breaker_row(&w).await, Some((1, false)));
    let again = w.one_task_of(&p.undo, "undo.reserve").await;
    w.complete(&p.undo, &again, &json!({}))
        .await
        .expect("works");
    assert_eq!(breaker_row(&w).await, None, "and the success closes it");
}

#[tokio::test]
async fn a_forward_step_named_like_a_held_back_compensation_is_neither_held_back_nor_counted() {
    use crate::support::engine::{activity, document, saga_node, with};
    let w = breaker_world(POLICY).await;
    // The same activity is the compensation in one workflow and plain forward work in another.
    let (tidy, _) = w
        .publish(document(
            "tidy",
            "tx",
            vec![
                saga_node("tx", "reserve", None),
                with(
                    with(activity("reserve"), "compensateWith", json!("act.cleanup")),
                    "next",
                    json!("submit"),
                ),
                activity("submit"),
            ],
        ))
        .await;
    let (cleaner, _) = w
        .publish(document("cleaner", "cleanup", vec![activity("cleanup")]))
        .await;
    let forward = w.session(&["act.reserve", "act.submit"]).await;
    let cleaning = w.session(&["act.cleanup"]).await;
    for _ in 0..2 {
        let run = w.start(tidy, &json!({})).await;
        let reserve = w.one_task_of(&forward, "act.reserve").await;
        assert_eq!(reserve.run_id, run);
        w.complete(&forward, &reserve, &json!({}))
            .await
            .expect("reserve");
        let submit = w.one_task_of(&forward, "act.submit").await;
        w.fail(&forward, &submit, false).await.expect("declined");
    }
    // The provider is down: two attempts at the compensation `act.cleanup` fail in a row.
    for _ in 0..2 {
        let mut tasks = w.poll(&cleaning, 1).await;
        assert_eq!(tasks.len(), 1);
        let undo = tasks.remove(0);
        assert_eq!(undo.kind, sagat_application::worker::TaskKind::Compensation);
        w.fail_with(&cleaning, &undo, "PROVIDER_DOWN", true)
            .await
            .expect("down");
        w.age_retry(undo.run_id, "reserve~c").await;
        w.scheduler.tick().await;
    }
    let key = |w: &World| {
        let pool = w.db.pool.clone();
        let tenant = w.admin.tenant_id().as_uuid();
        async move {
            sqlx::query_as::<_, (i32, bool)>(
                "SELECT failures, COALESCE(opened_until > clock_timestamp(), false) \
                 FROM compensation_breakers WHERE tenant_id = $1 AND activity = 'act.cleanup'",
            )
            .bind(tenant)
            .fetch_optional(&pool)
            .await
            .expect("breaker")
        }
    };
    assert_eq!(
        key(&w).await,
        Some((2, true)),
        "the breaker of the compensation is open"
    );

    // Forward work with that name is not the provider's compensation: it runs, and its success
    // says nothing about the compensations.
    let run = w.start(cleaner, &json!({})).await;
    let tasks = w.poll(&cleaning, 8).await;
    assert_eq!(tasks.len(), 1, "only the forward step is leased: {tasks:?}");
    assert_eq!(tasks[0].run_id, run);
    assert_eq!(tasks[0].kind, sagat_application::worker::TaskKind::Forward);
    w.complete(&cleaning, &tasks[0], &json!({}))
        .await
        .expect("cleaned");
    assert_eq!(
        key(&w).await,
        Some((2, true)),
        "a forward success does not close the breaker"
    );
}

#[tokio::test]
async fn pollers_that_find_the_breaker_half_open_at_once_lease_exactly_one_probe() {
    let w = std::sync::Arc::new(breaker_world(POLICY).await);
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    for _ in 0..8 {
        needing_an_undo(&w, &p, workflow).await;
    }
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    let mut sessions = Vec::new();
    for _ in 0..8 {
        sessions.push(w.session(&["undo.reserve"]).await);
    }
    // Several rounds, because the race is a matter of timing: in each, the window has just passed
    // and eight workers ask for work at the same instant.
    for round in 0..4 {
        let_the_window_pass(&w).await;
        let polls = futures_util::future::join_all(sessions.iter().map(|s| {
            let w = std::sync::Arc::clone(&w);
            async move { w.poll(s, 2).await }
        }))
        .await;
        let leased: Vec<(usize, _)> = polls
            .into_iter()
            .enumerate()
            .flat_map(|(who, tasks)| tasks.into_iter().map(move |task| (who, task)))
            .collect();
        assert_eq!(
            leased.len(),
            1,
            "round {round}: exactly one probe, got {}",
            leased.len()
        );
        // The probe fails, which reopens the breaker for the next round.
        let (who, probe) = &leased[0];
        w.fail_with(&sessions[*who], probe, "PROVIDER_DOWN", true)
            .await
            .expect("the probe reports");
        w.age_retry(probe.run_id, "reserve~c").await;
        w.scheduler.tick().await;
    }
}

#[tokio::test]
async fn a_compensation_held_back_does_not_take_the_place_of_one_that_can_run() {
    use crate::support::engine::{effect_node, parallel_node, saga_node, with};
    let w = breaker_world(BreakerPolicy {
        threshold: 1,
        ..POLICY
    })
    .await;
    // Two independent branches: when `fail` fails, both `a` and `b` are undone, in the same run.
    let (workflow, _) = w
        .publish(document(
            "branches",
            "tx",
            vec![
                saga_node("tx", "split", None),
                with(
                    parallel_node("split", &["a", "b"], "all"),
                    "next",
                    json!("fail"),
                ),
                effect_node("a", None, false),
                effect_node("b", None, false),
                crate::support::engine::activity("fail"),
            ],
        ))
        .await;
    let forward = w.session(&["act.a", "act.b", "act.fail"]).await;
    let run = w.start(workflow, &json!({})).await;
    for task in w.poll(&forward, 8).await {
        w.complete(&forward, &task, &json!({}))
            .await
            .expect("branch");
    }
    let boom = w.one_task(&forward).await;
    w.fail(&forward, &boom, false)
        .await
        .expect("fails for good");

    // The breaker of `undo.a` opens; `undo.b`'s provider is healthy. One worker, one place.
    sqlx::query(
        "INSERT INTO compensation_breakers (tenant_id, namespace_id, activity, failures, opened_until, updated_at) \
         VALUES ($1, $2, 'undo.a', 9, clock_timestamp() + interval '1 hour', clock_timestamp())",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("open the breaker of a");
    let undo = w.session(&["undo.a", "undo.b"]).await;
    let tasks = w.poll(&undo, 1).await;
    assert_eq!(
        tasks.len(),
        1,
        "the compensation that can run is leased although another is held back"
    );
    assert_eq!(tasks[0].activity.as_str(), "undo.b");
    assert_eq!(tasks[0].run_id, run);
    w.complete(&undo, &tasks[0], &json!({}))
        .await
        .expect("undo b");
    assert!(
        w.poll(&undo, 1).await.is_empty(),
        "and the one that is held back stays held"
    );
    assert_eq!(
        w.step_status(run, "a~c").await,
        Some(StepStatus::Ready),
        "a~c loses nothing"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
}

#[tokio::test]
async fn failures_of_attempts_that_were_already_running_do_not_extend_the_window() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    for _ in 0..4 {
        needing_an_undo(&w, &p, workflow).await;
    }
    // Four compensations are leased at once, while the breaker is closed.
    let tasks = w.poll(&p.undo, 4).await;
    assert_eq!(tasks.len(), 4);
    let window = |w: &World| {
        let (pool, tenant) = (w.db.pool.clone(), w.admin.tenant_id().as_uuid());
        async move {
            sqlx::query_as::<_, (i32, i32, bool, Option<time::OffsetDateTime>)>(
                "SELECT failures, level, probing, opened_until FROM compensation_breakers \
                 WHERE tenant_id = $1 AND activity = 'undo.reserve'",
            )
            .bind(tenant)
            .fetch_one(&pool)
            .await
            .expect("breaker")
        }
    };
    for task in &tasks[..2] {
        w.fail_with(&p.undo, task, "PROVIDER_DOWN", true)
            .await
            .expect("down");
    }
    let (failures, level, _, opened) = window(&w).await;
    assert_eq!((failures, level), (2, 0));
    assert!(opened.is_some(), "open after the second failure");

    // The other two were already running: their failures are counted, and change nothing else.
    for task in &tasks[2..] {
        w.fail_with(&p.undo, task, "PROVIDER_DOWN", true)
            .await
            .expect("down");
    }
    let (failures, level, probing, still) = window(&w).await;
    assert_eq!((failures, level, probing), (4, 0, false));
    assert_eq!(
        still, opened,
        "the window did not move, and no probe was needed to reopen it"
    );
}

#[tokio::test]
async fn the_breaker_of_one_namespace_does_not_hold_back_the_same_activity_in_another() {
    let w = breaker_world(POLICY).await;
    let (payments, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    needing_an_undo(&w, &p, payments).await;
    needing_an_undo(&w, &p, payments).await;
    fail_one_undo(&w, &p).await;
    fail_one_undo(&w, &p).await;
    assert_eq!(
        breaker_row(&w).await.map(|r| r.1),
        Some(true),
        "open in `payments`"
    );

    // The same workflow in another environment of the same tenant.
    let staging = w.add_namespace("staging").await;
    let bytes = serde_json::to_vec(&settlement()).expect("json");
    let created = w
        .defs
        .create_workflow(
            &w.admin,
            staging.slug().as_str(),
            "settlement",
            &bytes,
            "create-staging-0001",
            &crate::support::engine::ctx(),
        )
        .await
        .expect("create");
    w.defs
        .publish(
            &w.admin,
            created.workflow.id(),
            1,
            None,
            "publish-staging-0001",
            &crate::support::engine::ctx(),
        )
        .await
        .expect("publish");
    w.bindings
        .bind(
            &w.admin,
            staging.id(),
            w.worker.subject().as_str(),
            &["*".to_owned()],
            &crate::support::engine::ctx(),
        )
        .await
        .expect("bind");
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    let caps = sagat_application::worker::Capabilities {
        activities: names,
        languages: vec![],
        pools: vec![],
        max_inflight: 100,
        protocol_version: "1".into(),
    };
    let registered = w
        .workers
        .register(&w.worker, staging.slug().as_str(), "staging-worker", &caps)
        .await
        .expect("register");
    let theirs = Session {
        principal: w.worker.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    let run = w
        .runs
        .start(
            &w.admin,
            &sagat_application::runs::StartRun {
                namespace: staging.slug().as_str(),
                workflow: created.workflow.id(),
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key: "start-staging-0001",
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("start")
        .info
        .run_id;
    let reserve = w.one_task(&theirs).await;
    w.complete(&theirs, &reserve, &json!({}))
        .await
        .expect("reserve");
    let submit = w.one_task(&theirs).await;
    w.fail(&theirs, &submit, false).await.expect("declined");

    // `payments` is held back; `staging` is not.
    let undo = w.one_task(&theirs).await;
    assert_eq!((undo.activity.as_str(), undo.run_id), ("undo.reserve", run));
    w.complete(&theirs, &undo, &json!({}))
        .await
        .expect("undone");
    assert_eq!(
        breaker_row(&w).await.map(|r| r.1),
        Some(true),
        "and `payments` stays open"
    );
}

#[tokio::test]
async fn the_claim_does_not_walk_the_compensations_an_open_breaker_holds_back() {
    let w = breaker_world(POLICY).await;
    let (workflow, _) = w.publish(settlement()).await;
    let p = pair(&w).await;
    for _ in 0..60 {
        needing_an_undo(&w, &p, workflow).await;
    }
    sqlx::query("ANALYZE steps")
        .execute(&w.db.pool)
        .await
        .expect("analyze");
    sqlx::query(
        "INSERT INTO compensation_breakers (tenant_id, namespace_id, activity, failures, opened_until, updated_at) \
         VALUES ($1, $2, 'undo.reserve', 9, clock_timestamp() + interval '1 hour', clock_timestamp())",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("open");
    // The statement text is built from UUIDs and constants of the test: nothing a user wrote.
    let sql = format!(
        "EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF) {}",
        sagat_infra_postgres::CLAIMABLE_SQL
            .replace("$1", &format!("'{}'", w.admin.tenant_id().as_uuid()))
            .replace("$2", &format!("'{}'", w.namespace.id().as_uuid()))
            .replace("$3::text[]", "ARRAY['undo.reserve']::text[]")
            .replace("$4", "8")
            .replace("$5", "3")
            .replace("$6", "'some-worker'")
    );
    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_all(&w.db.pool)
        .await
        .expect("explain");
    let plan = plan.join("\n");
    // The branch that reads the compensations is gated by a condition on the breaker alone: with
    // the breaker open the planner does not enter it, so the sixty compensations that wait behind
    // it (or sixty thousand) are not read.
    assert!(plan.contains("One-Time Filter"), "{plan}");
    let gated = plan
        .split("One-Time Filter")
        .nth(1)
        .expect("the gated branch");
    assert!(gated.contains("never executed"), "{plan}");
    assert!(!gated.contains("actual rows=60"), "{plan}");
}
