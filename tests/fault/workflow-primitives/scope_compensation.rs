//! Faults around the compensation of a stopped collection or loop (T014-017, T014-021,
//! FR-014-005, FR-014-014, FR-003-002): duplicate and stale reports, a lost broker delivery, a
//! connection that dies inside each transaction of the compensation, a transaction that cannot
//! write its outbox or its audit, a cancellation racing the stop, and a restart in the middle.
//!
//! A process cannot be killed from inside a test, so a crash is the two ways the database sees a
//! client die: its connection is ended while the transaction waits for the run lock
//! (`pg_terminate_backend`), or the future driving it is dropped (`CutAfter`). Neither claims to
//! cover a crash of the PostgreSQL server itself.

use reqwest::StatusCode;
use sagat_api_grpc::pb::TaskLease;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::scope_flow::{
    CutAfter, Flow, RunHold, assert_no_open_transaction, blocked, iteration, journal, kill_waiters,
    node, pick, reserve_and_charge,
};

/// Two lines; line 0 completed `reserve` and `charge`, line 1 reserved and was declined. The
/// scheduler has not run since: it still has to start the drain, plan and release.
async fn declined(name: &str) -> Flow {
    let flow = Flow::start(
        name,
        reserve_and_charge(name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;
    flow.drive().await;
    let reserves = flow.poll().await;
    for (index, task) in reserves.iter().enumerate() {
        flow.complete(task, json!({"hold": index})).await;
    }
    flow.drive().await;
    let charges = flow.poll().await;
    flow.complete(&charges[0], json!({"charged": 10})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    flow
}

async fn count(flow: &Flow, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("count")
}

/// How many times each compensation step was completed in the journal: exactly once, whatever
/// was duplicated, cut or restarted.
async fn completions_of_compensations(flow: &Flow) -> Vec<(i32, String, i64)> {
    sqlx::query_as(
        "SELECT instance_path_indices[1], node, count(*) FROM run_events \
         WHERE run_id = $1 AND event_type = 'step.completed' AND node LIKE '%~c' \
         GROUP BY 1, 2 ORDER BY 1, 2",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("completions")
}

fn tasks(batch: &[TaskLease]) -> Vec<(u32, String, String)> {
    batch
        .iter()
        .map(|task| (iteration(task), node(task), task.task_kind.clone()))
        .collect()
}

// ---------------------------------------------------------------- duplicate and stale reports

#[tokio::test]
async fn a_compensation_reported_twice_is_applied_once_and_a_stale_report_is_refused() {
    let name = format!("scope-dup-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    flow.drive().await;
    let batch = flow.poll().await;
    assert_eq!(
        tasks(&batch),
        vec![
            (0, "charge~c".into(), "COMPENSATION".into()),
            (1, "reserve~c".into(), "COMPENSATION".into())
        ]
    );
    let refund = &batch[0];

    // The same report twice, with the same key: the second says it was already applied.
    let first = flow
        .try_complete(refund, json!({"undone": true}), "report-1")
        .await
        .expect("first report");
    assert!(!first.already_applied);
    let second = flow
        .try_complete(refund, json!({"undone": true}), "report-1")
        .await
        .expect("a replay is answered");
    assert!(second.already_applied, "the second report changed nothing");

    // A different report for the same attempt is not a replay and is not accepted.
    let other = flow
        .try_complete(refund, json!({"undone": "again"}), "report-2")
        .await;
    assert!(
        other.is_err(),
        "a second completion with another key is refused"
    );

    // A stale attempt: the lease of the release is lost and granted again; the old holder is late.
    let release = &batch[1];
    sqlx::query(
        "UPDATE steps SET lease_expires_at = now() - interval '1 second' \
         WHERE run_id = $1 AND node = 'reserve~c' AND instance_path_indices = ARRAY[1]",
    )
    .bind(flow.run.as_uuid())
    .execute(&flow.api.db.pool)
    .await
    .expect("lose the lease");
    flow.drive().await;
    flow.elapse_backoff().await;
    flow.drive().await;
    let polled = flow.poll().await;
    let again = pick(&polled, 1, "reserve~c");
    assert_eq!(again.attempt, release.attempt + 1);
    assert_eq!(
        again.idempotency_key, release.idempotency_key,
        "the provider is asked again with the same key"
    );
    let late = flow
        .try_complete(release, json!({"undone": true}), "late-report")
        .await;
    assert!(late.is_err(), "the holder of the old attempt cannot report");
    flow.complete(again, json!({"undone": true})).await;
    // The reservation of line 0 was released by the same pass; it is the other task of the poll.
    for task in polled.iter().filter(|task| !std::ptr::eq(*task, again)) {
        flow.complete(task, json!({"undone": true})).await;
    }
    flow.drive().await;

    assert_eq!(flow.run_status().await, "COMPENSATED");
    assert_eq!(
        completions_of_compensations(&flow).await,
        vec![
            (0, "charge~c".into(), 1),
            (0, "reserve~c".into(), 1),
            (1, "reserve~c".into(), 1)
        ],
        "each compensation completed exactly once"
    );
    flow.verify().await;
}

// ----------------------------------------------------------------------- lost broker delivery

#[tokio::test]
async fn a_compensation_whose_hint_never_arrived_is_leased_from_the_database() {
    let name = format!("scope-hint-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    flow.drive().await;
    let hinted: i64 = count(
        &flow,
        "SELECT count(*) FROM event_outbox WHERE aggregate_id = $1 AND event_type = 'task.ready' \
         AND routing_key IN ('line.refund', 'line.release')",
    )
    .await;
    assert_eq!(
        hinted, 2,
        "one hint per released compensation, with its scoped path"
    );
    let path: Value = sqlx::query_scalar(
        "SELECT payload->'instancePath' FROM event_outbox WHERE aggregate_id = $1 \
         AND routing_key = 'line.refund'",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("hint payload");
    assert_eq!(path, json!([{"scope": "lines", "index": 0}]));

    // The relay delivered every hint and the broker lost them: nothing will wake a worker.
    sqlx::query(
        "UPDATE event_outbox SET delivered_at = now() WHERE aggregate_id = $1 \
         AND event_type = 'task.ready'",
    )
    .bind(flow.run.as_uuid())
    .execute(&flow.api.db.pool)
    .await
    .expect("mark the hints delivered");
    sqlx::query("DELETE FROM event_outbox WHERE aggregate_id = $1 AND event_type = 'task.ready'")
        .bind(flow.run.as_uuid())
        .execute(&flow.api.db.pool)
        .await
        .expect("lose the hints");
    // The reconciliation loop finds the ready compensations nobody claimed and announces them again.
    sqlx::query(
        "UPDATE steps SET last_hinted_at = now() - interval '1 hour' \
         WHERE run_id = $1 AND kind = 'COMPENSATION' AND status = 'READY'",
    )
    .bind(flow.run.as_uuid())
    .execute(&flow.api.db.pool)
    .await
    .expect("age the hints");
    flow.drive().await;
    let renewed: Vec<(String, Value)> = sqlx::query_as(
        "SELECT routing_key, payload->'instancePath' FROM event_outbox \
         WHERE aggregate_id = $1 AND event_type = 'task.ready' ORDER BY routing_key",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("renewed hints");
    assert_eq!(
        renewed,
        vec![
            (
                "line.refund".to_owned(),
                json!([{"scope": "lines", "index": 0}])
            ),
            (
                "line.release".to_owned(),
                json!([{"scope": "lines", "index": 1}])
            ),
        ],
        "each lost hint is announced again, naming its iteration"
    );

    // Polling is the claim: the compensations are leased although no hint ever arrived.
    let polled = flow.poll().await;
    assert_eq!(polled.len(), 2);
    for task in &polled {
        flow.complete(task, json!({"undone": true})).await;
    }
    flow.drive().await;
    for task in flow.poll().await {
        flow.complete(&task, json!({"undone": true})).await;
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}

// -------------------------------------------------------------------- a connection that dies

/// Runs `action` while the run row is held, ends the connection it waits with, lets the row go and
/// returns the result of the action. The transaction of the action never committed.
async fn killed_in<F>(flow: &Flow, what: &str, action: F) -> F::Output
where
    F: std::future::Future,
    F::Output: std::fmt::Debug,
{
    let hold = RunHold::take(flow).await;
    let mut action = Box::pin(action);
    tokio::select! {
        answer = &mut action => panic!("{what} finished while the run row was held: {answer:?}"),
        () = blocked(&flow.api.db.pool, 1) => {}
    }
    assert!(kill_waiters(&flow.api.db.pool).await >= 1);
    hold.release().await;
    action.await
}

/// Runs `action` with a trigger that makes the lease of a compensation linger inside its
/// transaction, ends the connection while it lingers and removes the trigger. A poll skips a run
/// whose row is locked, so the lease cannot be made to wait for the run lock like the others.
async fn killed_while_leasing<F: std::future::Future>(flow: &Flow, action: F) -> F::Output {
    sqlx::raw_sql(
        "CREATE FUNCTION linger_on_compensation_lease() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM pg_sleep(120); RETURN NEW; END $$; \
         CREATE TRIGGER linger_on_compensation_lease BEFORE UPDATE ON steps \
         FOR EACH ROW WHEN (NEW.kind = 'COMPENSATION' AND NEW.status = 'LEASED' \
                            AND OLD.status = 'READY') \
         EXECUTE FUNCTION linger_on_compensation_lease()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("install the delay");
    let mut action = Box::pin(action);
    let lingering = async {
        loop {
            let sleeping: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event = 'PgSleep'",
            )
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("sleepers");
            if sleeping >= 1 {
                return;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::select! {
        _ = &mut action => panic!("the lease finished while it lingered"),
        () = lingering => {}
    }
    let killed: i64 = sqlx::query_scalar(
        "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event = 'PgSleep'",
    )
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("terminate");
    assert!(killed >= 1);
    let output = action.await;
    sqlx::raw_sql(
        "DROP TRIGGER linger_on_compensation_lease ON steps; \
         DROP FUNCTION linger_on_compensation_lease()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("remove the delay");
    output
}

#[tokio::test]
async fn a_connection_that_dies_inside_each_transaction_of_the_compensation_loses_nothing() {
    let name = format!("scope-kill-{}", Uuid::now_v7());
    let flow = declined(&name).await;

    // 1. The scheduler plans and releases: its transaction dies; nothing was planned.
    flow.until_planning().await;
    let tick = killed_in(&flow, "the planning tick", flow.scheduler.tick()).await;
    assert!(tick.failed >= 1, "{tick:?}");
    assert_eq!(
        count(&flow, "SELECT count(*) FROM scope_sagas WHERE run_id = $1").await,
        0,
        "a dead planning transaction left no plan"
    );
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM steps WHERE run_id = $1 AND kind = 'COMPENSATION'"
        )
        .await,
        0
    );
    flow.verify().await;
    flow.drive().await;
    assert_eq!(
        count(&flow, "SELECT count(*) FROM scope_sagas WHERE run_id = $1").await,
        1,
        "the plan is committed once the scheduler runs again"
    );
    assert_no_open_transaction(&flow, "the planning transaction").await;

    // 2. A worker leases a compensation: the lease transaction dies; nothing is leased.
    let leased = killed_while_leasing(&flow, flow.try_poll()).await;
    assert!(
        leased.is_err(),
        "the poll whose connection died is an error"
    );
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM steps WHERE run_id = $1 AND kind = 'COMPENSATION' \
             AND status = 'READY' AND attempt = 0"
        )
        .await,
        2,
        "no compensation was leased by the poll that died"
    );
    let batch = flow.poll().await;
    assert_eq!(batch.len(), 2);
    let refund = pick(&batch, 0, "charge~c");
    let release = pick(&batch, 1, "reserve~c");

    // 3. A completion dies: the compensation is still leased; the same report then lands.
    let died = killed_in(
        &flow,
        "the completion",
        flow.try_complete(refund, json!({"undone": 1}), "report-once"),
    )
    .await;
    assert!(died.is_err());
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' \
             AND node = 'charge~c'"
        )
        .await,
        0
    );
    let landed = flow
        .try_complete(refund, json!({"undone": 1}), "report-once")
        .await
        .expect("the retried report is accepted");
    assert!(!landed.already_applied);

    // 4. A failure report dies: the attempt is still the worker's; the report then lands.
    let died = killed_in(
        &flow,
        "the failure report",
        flow.try_fail(release, "PROVIDER_DOWN", true),
    )
    .await;
    assert!(died.is_err());
    flow.try_fail(release, "PROVIDER_DOWN", true)
        .await
        .expect("the retried failure is accepted");
    flow.elapse_backoff().await;
    flow.drive().await;
    for task in flow.poll().await {
        flow.complete(&task, json!({"undone": true})).await;
    }

    // 5. The scheduler ends the scope: that transaction dies; the run is not finished.
    flow.until_finishing().await;
    let tick = killed_in(&flow, "the finishing tick", flow.scheduler.tick()).await;
    assert!(tick.failed >= 1, "{tick:?}");
    assert_ne!(flow.run_status().await, "COMPENSATED");
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'run.finished'"
        )
        .await,
        0,
        "a dead finishing transaction finished nothing"
    );
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    assert_eq!(
        completions_of_compensations(&flow).await,
        vec![
            (0, "charge~c".into(), 1),
            (0, "reserve~c".into(), 1),
            (1, "reserve~c".into(), 1)
        ]
    );
    assert_no_open_transaction(&flow, "the compensation").await;
    flow.verify().await;
}

// ------------------------------------------------------ a future dropped at every await point

#[tokio::test]
async fn the_scheduler_cut_at_every_await_point_of_the_compensation_resumes_to_the_same_end() {
    let name = format!("scope-cut-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    let mut seen_cut_after_commit = false;
    let mut cuts = 0;
    // A tick has hundreds of await points (its passes read before they write), so the cut walks
    // through them in steps that do not line up with the passes.
    for polls in (3..2000).step_by(11) {
        let before = journal(&flow).await.len();
        let finished = CutAfter {
            inner: Box::pin(flow.scheduler.tick()),
            polls,
        }
        .await
        .is_some();
        cuts += 1;
        flow.flush_connections().await;
        assert_no_open_transaction(&flow, "a cut scheduler tick").await;
        flow.verify().await;
        if !finished && journal(&flow).await.len() > before {
            seen_cut_after_commit = true;
        }
        // Workers keep working between the cuts, exactly as they would.
        for task in flow.poll().await {
            flow.complete(&task, json!({"undone": true})).await;
        }
        if flow.run_status().await == "COMPENSATED" {
            break;
        }
    }
    for _ in 0..8 {
        flow.drive().await;
        for task in flow.poll().await {
            flow.complete(&task, json!({"undone": true})).await;
        }
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED", "after {cuts} cuts");
    assert!(
        seen_cut_after_commit,
        "at least one cut landed after a commit and before the tick returned"
    );
    assert_eq!(
        completions_of_compensations(&flow).await,
        vec![
            (0, "charge~c".into(), 1),
            (0, "reserve~c".into(), 1),
            (1, "reserve~c".into(), 1)
        ]
    );
    flow.verify().await;
}

// ------------------------------------------------------------------ transactions that cannot write

#[tokio::test]
async fn a_planning_transaction_that_cannot_write_its_hint_rolls_back_the_whole_plan() {
    let name = format!("scope-outbox-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_compensation_hint() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected outbox failure'; END $$; \
         CREATE TRIGGER reject_compensation_hint BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'task.ready' \
                            AND NEW.routing_key IN ('line.refund', 'line.release')) \
         EXECUTE FUNCTION reject_compensation_hint()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("install the fault");
    let (_, failed) = flow.drive_through_failures().await;
    assert!(failed >= 1, "the planning transaction failed");
    assert_eq!(
        count(&flow, "SELECT count(*) FROM scope_sagas WHERE run_id = $1").await,
        0,
        "no plan without the hints that announce it"
    );
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM steps WHERE run_id = $1 AND kind = 'COMPENSATION'"
        )
        .await,
        0,
        "no compensation step without its plan"
    );
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM run_events WHERE run_id = $1 \
             AND event_type = 'scope.compensation_planned'"
        )
        .await,
        0
    );
    flow.verify().await;
    sqlx::raw_sql(
        "DROP TRIGGER reject_compensation_hint ON event_outbox; \
         DROP FUNCTION reject_compensation_hint()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("remove the fault");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATING");
    for _ in 0..4 {
        for task in flow.poll().await {
            flow.complete(&task, json!({"undone": true})).await;
        }
        flow.drive().await;
    }
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}

#[tokio::test]
async fn an_operator_decision_that_cannot_be_audited_changes_nothing_and_can_be_repeated() {
    let name = format!("scope-audit-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    flow.drive().await;
    // The first release is delivered; the second keeps failing until a person has to decide.
    let batch = flow.poll().await;
    flow.complete(pick(&batch, 0, "charge~c"), json!({"undone": true}))
        .await;
    let mut stuck = pick(&batch, 1, "reserve~c").clone();
    for attempt in 1..=2 {
        flow.fail(&stuck, "PROVIDER_DOWN", true).await;
        if attempt == 1 {
            flow.elapse_backoff().await;
            flow.drive().await;
            let polled = flow.poll().await;
            stuck = pick(&polled, 1, "reserve~c").clone();
            // The reservation of line 0 is released by the same pass and is delivered.
            flow.complete(pick(&polled, 0, "reserve~c"), json!({"undone": true}))
                .await;
        }
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let decision = json!({
        "action": "RETRY_COMPENSATION", "stepId": "reserve~c",
        "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
        "reason": "the provider is back, release the reservation"
    });
    sqlx::raw_sql(
        "CREATE FUNCTION reject_decision_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected audit failure'; END $$; \
         CREATE TRIGGER reject_decision_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW WHEN (NEW.action = 'saga.step_retried') \
         EXECUTE FUNCTION reject_decision_audit()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("install the fault");
    let (status, _) = flow.operate(decision.clone()).await;
    assert!(status.is_server_error(), "{status}");
    assert_eq!(
        count(
            &flow,
            "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'step.retry_requested'"
        )
        .await,
        0,
        "a decision whose audit failed left no journal event"
    );
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    sqlx::raw_sql(
        "DROP TRIGGER reject_decision_audit ON audit_records; DROP FUNCTION reject_decision_audit()",
    )
    .execute(&flow.api.db.pool)
    .await
    .expect("remove the fault");
    let (status, body) = flow.operate(decision).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    for task in flow.poll().await {
        flow.complete(&task, json!({"undone": true})).await;
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}

// -------------------------------------------------------------------- a cancellation racing a stop

#[tokio::test]
async fn a_cancellation_racing_the_scheduler_that_stops_the_scope_for_a_failure_ends_consistently()
{
    // The run is stopped by whichever commits first: the scheduler's drain for the declined charge,
    // or the operator's cancellation. Neither order may lose what completed or undo it twice.
    let mut outcomes = std::collections::BTreeSet::new();
    for attempt in 0..6 {
        let name = format!("scope-race-{attempt}-{}", Uuid::now_v7());
        let flow = declined(&name).await;
        let cancel =
            flow.operate(json!({"action": "CANCEL", "reason": "cancel while it is declined"}));
        let tick = async {
            for _ in 0..attempt {
                tokio::task::yield_now().await;
            }
            flow.scheduler.tick().await
        };
        let (cancelled, ticked) = tokio::join!(cancel, tick);
        assert_eq!(ticked.failed, 0, "{ticked:?}");
        assert!(
            cancelled.0 == StatusCode::ACCEPTED || cancelled.0.is_client_error(),
            "{:?}",
            cancelled.0
        );
        for _ in 0..8 {
            flow.drive().await;
            for task in flow.poll().await {
                flow.complete(&task, json!({"undone": true})).await;
            }
        }
        flow.drive().await;
        let status = flow.run_status().await;
        assert!(
            status == "COMPENSATED" || status == "CANCELLED",
            "the run ends as what stopped it first, not as something else: {status}"
        );
        assert_eq!(
            (cancelled.0 == StatusCode::ACCEPTED),
            (status == "CANCELLED"),
            "the cancel was accepted exactly when it stopped the run first"
        );
        outcomes.insert(status);
        assert_eq!(
            completions_of_compensations(&flow).await,
            vec![
                (0, "charge~c".into(), 1),
                (0, "reserve~c".into(), 1),
                (1, "reserve~c".into(), 1)
            ]
        );
        flow.verify().await;
    }
    assert!(!outcomes.is_empty());
}

// --------------------------------------------------------------------------------------- restart

#[tokio::test]
async fn a_scheduler_replica_that_takes_over_mid_compensation_finishes_it() {
    let name = format!("scope-restart-{}", Uuid::now_v7());
    let flow = declined(&name).await;
    flow.drive().await;
    let batch = flow.poll().await;
    flow.complete(pick(&batch, 0, "charge~c"), json!({"undone": true}))
        .await;
    // The scheduler process is gone: its leases lapse, and a replica with no memory of it takes the
    // shards over. The old process, if it were still alive, would find itself fenced.
    sqlx::query("UPDATE scheduler_shards SET expires_at = now() - interval '1 second'")
        .execute(&flow.api.db.pool)
        .await
        .expect("the old leases lapse");
    let placement = Placement::new(format!("replica-{name}"));
    Keeper::new(
        flow.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("the replica acquires the shards");
    let replica = Scheduler::new(
        flow.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    flow.complete(pick(&batch, 1, "reserve~c"), json!({"undone": true}))
        .await;
    let stale = flow.scheduler.tick().await;
    assert_eq!(stale.collections_advanced, 0, "{stale:?}");
    for _ in 0..40 {
        let report = replica.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        for task in flow.poll().await {
            flow.complete(&task, json!({"undone": true})).await;
        }
        if flow.run_status().await == "COMPENSATED" {
            break;
        }
    }
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}
