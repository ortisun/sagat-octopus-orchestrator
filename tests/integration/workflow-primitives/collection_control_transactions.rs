//! Native control projections fail closed and share atomic journal/projection/outbox boundaries.
use super::collection_controls::at_control;
use super::collection_finish_atomic::snapshot;
use sagat_application::{
    execution::PathExecutionStore,
    runs::{Lock, RunStore},
    scope_frontier::ScopeFrontierStore,
};
use sagat_domain::{
    execution::{ExecutionInstanceId, InstancePath},
    workflow::NodeId,
};
use serde_json::json;

#[tokio::test]
async fn control_choice_and_selected_task_roll_back_together_if_the_outbox_fails() {
    let _server = crate::support::hold_server();
    for whole in [false, true] {
        let name = format!("collection-control-outbox-{whole}");
        let flow = at_control(
            &name,
            json!({
                "id":"choose", "type":"condition", "expression":"item > 0",
                "ifTrue":"accept", "ifFalse":"reject"
            }),
            json!([1]),
        )
        .await;
        let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
        sqlx::raw_sql(
            "CREATE FUNCTION reject_control_task() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION 'test-only scoped control outbox failure' \
               USING ERRCODE='23514',CONSTRAINT='scoped_control_forced_abort'; END $$; \
             CREATE TRIGGER reject_control_task BEFORE INSERT ON event_outbox \
               FOR EACH ROW WHEN (NEW.event_type='task.ready' AND NEW.routing_key='line.charge') \
               EXECUTE FUNCTION reject_control_task()",
        )
        .execute(&flow.api.db.pool)
        .await
        .expect("install owned outbox fault");
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        let result = if whole {
            flow.scheduler
                .advance_collection_from_run(flow.tenant, flow.run, &owner)
                .await
        } else {
            flow.scheduler
                .advance_collection(flow.tenant, flow.run, &owner)
                .await
        };
        let error = result.expect_err("the selected task reaches the injected native fault");
        assert!(
            error
                .to_string()
                .contains("test-only scoped control outbox failure"),
            "{error}"
        );
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before,
            "choice, routing, task, journal, quota, audit and telemetry all roll back"
        );
        sqlx::raw_sql(
            "DROP TRIGGER reject_control_task ON event_outbox; DROP FUNCTION reject_control_task()",
        )
        .execute(&flow.api.db.pool)
        .await
        .expect("remove owned outbox fault");
        let result = if whole {
            flow.scheduler
                .advance_collection_from_run(flow.tenant, flow.run, &owner)
                .await
        } else {
            flow.scheduler
                .advance_collection(flow.tenant, flow.run, &owner)
                .await
        }
        .expect("retry from unchanged state");
        assert_eq!(result, sagat_scheduler::CollectionAdvance::BodyProgress);
        flow.drive().await;
        let selected = flow.poll().await;
        assert_eq!(selected.len(), 1);
        assert_eq!(crate::support::scope_flow::node(&selected[0]), "accept");
        flow.complete(&selected[0], json!({"ok":true})).await;
        flow.drive().await;
        assert_eq!(flow.run_status().await, "SUCCEEDED");
        flow.verify().await;
    }
}

#[tokio::test]
async fn a_pause_committing_ahead_of_a_prepared_control_refuses_the_stale_choice() {
    use sagat_application::execution::{Actor, DefinitionCache, Effects, Engine, RunTx};
    use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
    use sagat_infra_postgres::PgRunStore;
    use std::{sync::Arc, time::Duration};
    let flow = at_control(
        "collection-control-pause-cas",
        json!({
            "id":"choose", "type":"condition", "expression":"item > 0",
            "ifTrue":"accept", "ifFalse":"reject"
        }),
        json!([1]),
    )
    .await;
    let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    let holder_pool = flow.api.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&holder_pool)
    .await
    .expect("owned single-connection holder identity");
    let holder_store = PgRunStore::new(
        holder_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let mut holder_tx = holder_store
        .begin(flow.tenant, flow.run, Lock::Wait)
        .await
        .expect("holder transaction")
        .expect("run");
    Engine::new(JsonEvaluator)
        .run(
            &mut holder_tx,
            Effects::by(Actor::System("control-race-test"), None),
            |_, state, _| {
                Ok((
                    (),
                    sagat_domain::execution::Progress {
                        events: vec![sagat_domain::execution::pause_run(state)?],
                        pending: None,
                    },
                ))
            },
        )
        .await
        .expect("prepare a real uncommitted pause");
    let advancing = flow
        .scheduler
        .advance_collection(flow.tenant, flow.run, &owner);
    let commit_pause = async {
        loop {
            let queued: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity waiter \
                   JOIN pg_stat_activity holder ON holder.pid=$1 AND holder.backend_start=$2 \
                  WHERE waiter.datname=current_database() AND waiter.wait_event_type='Lock' \
                    AND $1=ANY(pg_blocking_pids(waiter.pid)) \
                    AND waiter.query LIKE '%FROM runs%' AND waiter.query LIKE '%FOR UPDATE%')",
            )
            .bind(holder.0)
            .bind(holder.1)
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("observe exact owned lock edge");
            if queued {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before,
            "neither the uncommitted pause nor the queued prepared choice is visible"
        );
        holder_tx
            .commit()
            .await
            .expect("pause commits before control lock acquisition");
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(advancing, commit_pause)
    })
    .await
    .expect("bounded pause/control lock proof");
    assert_eq!(
        result.expect("stale control is a skipped candidate"),
        sagat_scheduler::CollectionAdvance::Waiting
    );
    let after = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert_eq!(after["run"]["paused"], true);
    assert_eq!(
        after["run"]["run_version"].as_i64(),
        before["run"]["run_version"]
            .as_i64()
            .map(|version| version + 1)
    );
    for field in ["scope", "steps", "iterations", "checkpoints", "usage"] {
        assert_eq!(
            after[field], before[field],
            "{field}: a stale answer produces no control/task/quota transition"
        );
    }
    let (status, _) = flow
        .operate(json!({"action":"RESUME", "reason":"continue after control CAS proof"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    flow.drive().await;
    let selected = flow.poll().await;
    assert_eq!(selected.len(), 1);
    assert_eq!(crate::support::scope_flow::node(&selected[0]), "accept");
    flow.complete(&selected[0], json!({"ok":true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
    drop(holder_store);
    holder_pool.close().await;
}

#[tokio::test]
async fn prepared_control_writers_reject_physical_metadata_added_after_the_snapshot() {
    use sagat_application::execution::{Actor, Effects, Engine, PathExecutionTx};
    use sagat_domain::execution::Resolution;
    use sagat_infra_ir::JsonEvaluator;
    let _server = crate::support::hold_server();
    for whole in [false, true] {
        for timer in [false, true] {
            let flow = at_control(&format!("prepared-control-metadata-{whole}-{timer}"), json!({
                "id":"choose", "type":"condition", "expression":"item > 0", "ifTrue":"accept", "ifFalse":"reject"
            }), json!([1])).await;
            let path = InstancePath::root()
                .child(NodeId::parse("lines").expect("owner"), 0)
                .expect("path");
            let instance = ExecutionInstanceId::in_path(
                path.clone(),
                NodeId::parse("choose").expect("control"),
            );
            let mut full = if whole {
                flow.api
                    .executions
                    .begin(flow.tenant, flow.run, Lock::Wait)
                    .await
                    .expect("prepare full transaction")
            } else {
                None
            };
            let local = if whole {
                None
            } else {
                flow.api
                    .executions
                    .load_path(flow.tenant, flow.run, path)
                    .await
                    .expect("prepare bounded snapshot")
            };
            // Deliberate owner corruption without advancing Run.version, after the decision's read.
            let mutation = if timer {
                "UPDATE steps SET due_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            } else {
                "UPDATE steps SET reported_lease_token_hash=decode(repeat('ab',32),'hex') WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            };
            assert_eq!(
                sqlx::query(sqlx::AssertSqlSafe(mutation))
                    .bind(flow.tenant.as_uuid())
                    .bind(flow.run.as_uuid())
                    .execute(&flow.api.db.pool)
                    .await
                    .expect("owned physical corruption")
                    .rows_affected(),
                1
            );
            let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
            let answer = Resolution::Condition {
                node: NodeId::parse("choose").expect("node"),
                outcome: Ok(true),
            };
            let result = if let Some(mut tx) = full.take() {
                let result = Engine::new(JsonEvaluator)
                    .run_addressed(
                        &mut tx,
                        Vec::new(),
                        Effects::by(Actor::System("control-metadata-test"), None),
                        |definition, state, _| {
                            Ok((
                                (),
                                sagat_domain::execution::resolve_control_instance(
                                    definition, state, &instance, answer,
                                )?,
                            ))
                        },
                    )
                    .await
                    .map_err(|error| error.to_string());
                drop(tx);
                result
            } else {
                let mut tx = flow
                    .api
                    .executions
                    .begin_path(local.expect("bounded snapshot"), Lock::Wait)
                    .await
                    .expect("same revision still locks")
                    .expect("run");
                let events = {
                    let snapshot = tx.snapshot();
                    sagat_domain::execution::resolve_control_path_instance(
                        &snapshot.definition,
                        &mut snapshot.state,
                        &instance,
                        answer,
                    )
                    .expect("the prepared typed state is valid")
                    .events
                };
                let result = tx
                    .persist_addressed(
                        &events,
                        &[],
                        Effects::by(Actor::System("control-metadata-test"), None),
                    )
                    .await
                    .map_err(|error| error.to_string());
                drop(tx);
                result
            };
            let error =
                result.expect_err("physical metadata cannot survive a prepared control write");
            assert!(
                error.contains("step identity"),
                "the native row guard refuses the write: {error}"
            );
            assert_eq!(
                snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
                before,
                "neither writer partially records a choice over malformed physical state"
            );
        }
    }
}

#[tokio::test]
async fn malformed_scoped_control_projections_are_rejected_before_any_transition() {
    let _server = crate::support::hold_server();
    for malformed in [
        "pending-routed",
        "unknown-switch-case",
        "orphan-timer",
        "reported-lease",
    ] {
        let control = if malformed != "unknown-switch-case" {
            json!({"id":"choose", "type":"condition", "expression":"item > 0", "ifTrue":"accept", "ifFalse":"reject"})
        } else {
            json!({"id":"choose", "type":"switch", "cases":[{"id":"positive", "when":"item > 0", "target":"accept"}], "default":"reject"})
        };
        let flow = at_control(
            &format!("malformed-control-{malformed}"),
            control,
            json!([1]),
        )
        .await;
        // Deliberate projection corruption only; no journal event legitimizes either state.
        let mutation = match malformed {
            "pending-routed" => {
                "UPDATE steps SET routed=true WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            }
            "unknown-switch-case" => {
                "UPDATE steps SET status='SUCCEEDED',switch_case_id='undeclared' WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            }
            "orphan-timer" => {
                "UPDATE steps SET due_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            }
            "reported-lease" => {
                "UPDATE steps SET reported_lease_token_hash=decode(repeat('ab',32),'hex') WHERE tenant_id=$1 AND run_id=$2 AND node='choose'"
            }
            _ => unreachable!(),
        };
        let changed = sqlx::query(sqlx::AssertSqlSafe(mutation))
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .execute(&flow.api.db.pool)
            .await
            .expect("corrupt exact scoped control")
            .rows_affected();
        assert_eq!(changed, 1);
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        let path = InstancePath::root()
            .child(NodeId::parse("lines").expect("owner"), 0)
            .expect("path");
        let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
        let path_result = flow
            .api
            .executions
            .load_path(flow.tenant, flow.run, path)
            .await;
        let frontier_result = flow
            .api
            .executions
            .collection_frontier(flow.tenant, flow.run, &owner)
            .await;
        let full_result = flow
            .api
            .executions
            .begin(flow.tenant, flow.run, Lock::Wait)
            .await;
        let rejected = (
            path_result.is_err(),
            frontier_result.is_err(),
            full_result.is_err(),
        );
        drop(full_result);
        assert_eq!(
            rejected,
            (true, true, true),
            "{malformed}: path, frontier, full readers reject corruption"
        );
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
    }
}
