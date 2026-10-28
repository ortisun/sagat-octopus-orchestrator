//! Native timer projection, journal and prepared-deadline boundaries use real PostgreSQL.
use super::{
    collection_finish_atomic::snapshot,
    collection_timers::{age, pending_timer, waiting_timer},
};
use crate::support::scope_flow::Flow;
use sagat_application::{
    execution::{
        Actor, DefinitionCache, Effects, Engine, PathExecutionStore, PathExecutionTx, RunTx,
    },
    primitives::SchedulerAuthority,
    runs::{Lock, RunStore},
};
use sagat_domain::{
    execution::{ExecutionInstanceId, InstancePath},
    workflow::NodeId,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use serde_json::json;
use std::{sync::Arc, time::Duration};

fn timer() -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("owner"), 0)
            .expect("path"),
        NodeId::parse("delay").expect("timer"),
    )
}
async fn authority(flow: &Flow) -> SchedulerAuthority {
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("real authority");
    SchedulerAuthority::for_run(
        owner,
        flow.run,
        u64::try_from(generation).expect("generation"),
    )
}

#[tokio::test]
async fn a_due_timer_fires_while_paused_and_cold_resume_releases_its_route_once() {
    use sagat_application::maintenance::{MaintenanceStore, Window};
    use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
    let flow = waiting_timer("collection-timer-pause-cold").await;
    let (status, _) = flow
        .operate(json!({"action":"PAUSE","reason":"pause an accepted timer"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    age(&flow).await;
    flow.drive().await;
    let timer_state:(String,bool,Option<String>,Option<time::OffsetDateTime>)=sqlx::query_as("SELECT status,routed,waiting_kind,due_at FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='delay'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("paused fired state");
    assert_eq!(timer_state, ("SUCCEEDED".into(), false, None, None));
    assert_eq!(flow.run_status().await, "PAUSED");
    assert!(flow.poll().await.is_empty());
    let (status, _) = flow
        .operate(json!({"action":"RESUME","reason":"resume deferred timer routing"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    let store = PgRunStore::new(
        flow.api.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    assert!(
        store
            .collection_candidates(&Window::everything(32))
            .await
            .expect("cold durable routing candidate")
            .iter()
            .any(|c| c.run_id == flow.run)
    );
    let authority = authority(&flow).await;
    let placement = Placement::new(authority.owner());
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("renew cold replica authority");
    let scheduler = Scheduler::new(
        store,
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    for _ in 0..10 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0);
        if !report.did_work() {
            break;
        }
    }
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let counts:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE event_type='timer.fired'),count(*) FILTER(WHERE event_type='step.routed' AND node='delay') FROM run_events WHERE tenant_id=$1 AND run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("one fire and route");
    assert_eq!(counts, (1, 1));
    flow.verify().await;
}

#[tokio::test]
async fn a_prepared_timer_fire_refuses_a_deadline_changed_without_a_run_revision() {
    let _server = crate::support::hold_server();
    for full in [false, true] {
        let flow = waiting_timer(&format!("collection-timer-deadline-{full}")).await;
        age(&flow).await;
        let identity = timer();
        let authority = authority(&flow).await;
        let snapshot_path = if full {
            None
        } else {
            flow.api
                .executions
                .load_path(flow.tenant, flow.run, identity.path().clone())
                .await
                .expect("prepare bounded read")
        };
        let mut full_tx = if full {
            flow.api
                .executions
                .begin(flow.tenant, flow.run, Lock::Wait)
                .await
                .expect("prepare full read")
        } else {
            None
        };
        sqlx::query("UPDATE steps SET due_at=clock_timestamp()+interval '1 hour' WHERE tenant_id=$1 AND run_id=$2 AND node='delay'")
            .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).execute(&flow.api.db.pool).await.expect("owner fixture changes only the deadline");
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        if let Some(mut tx) = full_tx.take() {
            assert!(
                tx.holds_shard(authority.owner(), authority.lease())
                    .await
                    .expect("full authority")
            );
            let result = Engine::new(JsonEvaluator)
                .run_addressed(
                    &mut tx,
                    Vec::new(),
                    Effects::by(Actor::System("timer-test"), None),
                    |definition, state, now| {
                        Ok((
                            (),
                            sagat_domain::execution::fire_timer_instance(
                                definition, state, &identity, now,
                            )?,
                        ))
                    },
                )
                .await;
            assert!(
                result.is_err(),
                "full writer must compare the prepared deadline"
            );
            drop(tx);
        } else {
            let mut tx = flow
                .api
                .executions
                .begin_path(snapshot_path.expect("bounded snapshot"), Lock::Wait)
                .await
                .expect("same Run revision locks")
                .expect("run");
            assert!(
                tx.holds_shard(authority.owner(), authority.lease())
                    .await
                    .expect("bounded authority")
            );
            let now = tx.now();
            let progress = {
                let snapshot = tx.snapshot();
                sagat_domain::execution::fire_timer_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    &identity,
                    now,
                )
                .expect("prepared snapshot was due")
            };
            let result = tx
                .persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(Actor::System("timer-test"), None),
                )
                .await;
            assert!(
                result.is_err(),
                "bounded writer must compare the prepared deadline"
            );
            drop(tx);
        }
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
        age(&flow).await;
        flow.drive().await;
        assert_eq!(flow.run_status().await, "SUCCEEDED");
        flow.verify().await;
    }
}

#[tokio::test]
async fn orphan_worker_metadata_on_a_timer_is_rejected_by_all_three_readers() {
    use sagat_application::scope_frontier::ScopeFrontierStore;
    let flow = waiting_timer("collection-timer-orphan-metadata").await;
    sqlx::query("UPDATE steps SET reported_lease_token_hash=decode(repeat('07',32),'hex') WHERE tenant_id=$1 AND run_id=$2 AND node='delay'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).execute(&flow.api.db.pool).await.expect("explicit orphan worker metadata");
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert!(
        flow.api
            .executions
            .begin(flow.tenant, flow.run, Lock::Wait)
            .await
            .is_err()
    );
    assert!(
        flow.api
            .executions
            .load_path(flow.tenant, flow.run, timer().path().clone())
            .await
            .is_err()
    );
    let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
    assert!(
        flow.api
            .executions
            .collection_frontier(flow.tenant, flow.run, &owner)
            .await
            .is_err()
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        before
    );
}

async fn clock_write(
    flow: &Flow,
    full: bool,
    fire: bool,
) -> Result<(), sagat_application::execution::EngineError> {
    use sagat_application::execution::EngineError;
    let identity = timer();
    let authority = authority(flow).await;
    if full {
        let mut tx = flow
            .api
            .executions
            .begin(flow.tenant, flow.run, Lock::Wait)
            .await?
            .expect("run");
        assert!(tx.holds_shard(authority.owner(), authority.lease()).await?);
        Engine::new(JsonEvaluator)
            .run_addressed(
                &mut tx,
                Vec::new(),
                Effects::by(Actor::System("timer-atomic-test"), None),
                |definition, state, now| {
                    let progress = if fire {
                        sagat_domain::execution::fire_timer_instance(
                            definition, state, &identity, now,
                        )?
                    } else {
                        sagat_domain::execution::start_timer_instance(
                            definition, state, &identity, now,
                        )?
                    };
                    Ok(((), progress))
                },
            )
            .await?;
        tx.commit().await?;
    } else {
        let snapshot = flow
            .api
            .executions
            .load_path(flow.tenant, flow.run, identity.path().clone())
            .await?
            .expect("path");
        let mut tx = flow
            .api
            .executions
            .begin_path(snapshot, Lock::Wait)
            .await?
            .expect("run");
        assert!(tx.holds_shard(authority.owner(), authority.lease()).await?);
        let now = tx.now();
        let progress = {
            let snapshot = tx.snapshot();
            if fire {
                sagat_domain::execution::fire_timer_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    &identity,
                    now,
                )
            } else {
                sagat_domain::execution::start_timer_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    &identity,
                    now,
                )
            }
        }
        .map_err(EngineError::Execution)?;
        tx.persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("timer-atomic-test"), None),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[tokio::test]
async fn timer_start_and_fire_roll_back_every_projection_on_journal_failure_in_both_writers() {
    use crate::support::scope_flow::collection_document;
    let _server = crate::support::hold_server();
    for full in [false, true] {
        for fire in [false, true] {
            let name = format!("collection-timer-journal-{full}-{fire}");
            let flow = if fire {
                let flow = waiting_timer(&name).await;
                age(&flow).await;
                flow
            } else {
                let flow=Flow::start(&name,collection_document(&name,vec![
                    json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                    json!({"id":"returned","type":"return","result":"item"}),
                ],1),json!([1])).await;
                let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
                assert_eq!(
                    flow.scheduler
                        .advance_collection(flow.tenant, flow.run, &owner)
                        .await
                        .expect("enter"),
                    sagat_scheduler::CollectionAdvance::Entered
                );
                assert_eq!(
                    flow.scheduler
                        .advance_collection(flow.tenant, flow.run, &owner)
                        .await
                        .expect("admit Pending timer"),
                    sagat_scheduler::CollectionAdvance::Admitted { index: 0 }
                );
                flow
            };
            sqlx::raw_sql("CREATE FUNCTION reject_timer_journal() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only timer journal abort' USING ERRCODE='23514'; END $$; CREATE TRIGGER reject_timer_journal BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type IN ('timer.started','timer.fired')) EXECUTE FUNCTION reject_timer_journal()")
                .execute(&flow.api.db.pool).await.expect("owned native journal fault");
            let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
            let error = clock_write(&flow, full, fire)
                .await
                .expect_err("the actual clock transition reaches the fault");
            assert!(
                error.to_string().contains("test-only timer journal abort"),
                "{error}"
            );
            assert_eq!(
                snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
                before
            );
            sqlx::raw_sql("DROP TRIGGER reject_timer_journal ON run_events; DROP FUNCTION reject_timer_journal()")
                .execute(&flow.api.db.pool).await.expect("remove owned fault");
            clock_write(&flow, full, fire)
                .await
                .expect("retry unchanged clock transition");
            if !fire {
                age(&flow).await;
            }
            flow.drive().await;
            assert_eq!(flow.run_status().await, "SUCCEEDED");
            flow.verify().await;
        }
    }
}

async fn observe_run_waiter(flow: &Flow, holder: (i32, time::OffsetDateTime)) {
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
        .expect("exact run-lock wait edge");
        if queued {
            break;
        }
        tokio::task::yield_now().await;
    }
}

async fn cancellation_and_timer_order(cancel_first: bool) {
    let flow = waiting_timer(&format!("collection-timer-cancel-order-{cancel_first}")).await;
    age(&flow).await;
    let holder_pool = flow.api.db.runtime_pool_with(1).await;
    let holder: (i32, time::OffsetDateTime) = sqlx::query_as(
        "SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(&holder_pool)
    .await
    .expect("owned single-connection holder");
    let store = PgRunStore::new(
        holder_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let mut tx = store
        .begin(flow.tenant, flow.run, Lock::Wait)
        .await
        .expect("holder transaction")
        .expect("run");
    let authority = authority(&flow).await;
    assert!(
        tx.holds_shard(authority.owner(), authority.lease())
            .await
            .expect("holder authority")
    );
    if cancel_first {
        Engine::new(JsonEvaluator)
            .run(
                &mut tx,
                Effects::by(Actor::System("timer-race-test"), None),
                |definition, state, now| {
                    Ok((
                        (),
                        sagat_domain::execution::stop_run(
                            definition,
                            state,
                            sagat_domain::execution::StopReason::Cancelled,
                            None,
                            now,
                        )?,
                    ))
                },
            )
            .await
            .expect("prepare real uncommitted cancellation");
        let timer_tick = flow.scheduler.tick();
        let release = async {
            observe_run_waiter(&flow, holder).await;
            tx.commit()
                .await
                .expect("cancellation commits before queued timer mutation");
        };
        let (report, ()) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(timer_tick, release)
        })
        .await
        .expect("bounded cancellation-first run-lock proof");
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.timers_fired, 0, "stale prepared timer is skipped");
    } else {
        let identity = timer();
        Engine::new(JsonEvaluator)
            .run_addressed(
                &mut tx,
                Vec::new(),
                Effects::by(Actor::System("timer-race-test"), None),
                |definition, state, now| {
                    Ok((
                        (),
                        sagat_domain::execution::fire_timer_instance(
                            definition, state, &identity, now,
                        )?,
                    ))
                },
            )
            .await
            .expect("prepare real uncommitted timer fire and routing");
        let cancel =
            flow.operate(json!({"action":"CANCEL","reason":"cancel after committed timer fire"}));
        let release = async {
            observe_run_waiter(&flow, holder).await;
            tx.commit()
                .await
                .expect("timer commits before queued public cancellation");
        };
        let ((status, _), ()) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(cancel, release)
        })
        .await
        .expect("bounded timer-first run-lock proof");
        assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "CANCELLED");
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT \
        (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='timer.fired'), \
        (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND slot_held), \
        (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready')")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("one winning transition, no slot/task leak");
    assert_eq!(counts, (i64::from(!cancel_first), 0, 0));
    assert!(flow.poll().await.is_empty());
    flow.verify().await;
}

#[tokio::test]
async fn cancellation_committing_ahead_of_the_queued_timer_prevents_its_fire() {
    cancellation_and_timer_order(true).await;
}

#[tokio::test]
async fn a_timer_committing_ahead_of_public_cancellation_fires_once_then_drains() {
    cancellation_and_timer_order(false).await;
}

#[tokio::test]
async fn prepared_timer_writes_reject_lifecycle_fields_changed_without_a_run_revision() {
    let _server = crate::support::hold_server();
    let mut accepted = Vec::new();
    for full in [false, true] {
        for fire in [false, true] {
            let changes: &[&str] = if fire {
                &["choice", "failure"]
            } else {
                &["choice", "routed", "settled"]
            };
            for change in changes {
                let name = format!("timer-prepared-{full}-{fire}-{change}");
                let flow = if fire {
                    let f = waiting_timer(&name).await;
                    age(&f).await;
                    f
                } else {
                    pending_timer(&name).await
                };
                let identity = timer();
                let authority = authority(&flow).await;
                let prepared = if full {
                    None
                } else {
                    flow.api
                        .executions
                        .load_path(flow.tenant, flow.run, identity.path().clone())
                        .await
                        .expect("prepared bounded path")
                };
                let mut full_tx = if full {
                    flow.api
                        .executions
                        .begin(flow.tenant, flow.run, Lock::Wait)
                        .await
                        .expect("prepared full path")
                } else {
                    None
                };
                let statement = match *change {
                    "choice" => {
                        "UPDATE steps SET condition=true WHERE tenant_id=$1 AND run_id=$2 AND node='delay'"
                    }
                    "routed" => {
                        "UPDATE steps SET routed=true WHERE tenant_id=$1 AND run_id=$2 AND node='delay'"
                    }
                    "settled" => {
                        "UPDATE steps SET status='SUCCEEDED',routed=true WHERE tenant_id=$1 AND run_id=$2 AND node='delay'"
                    }
                    "failure" => {
                        "UPDATE steps SET failure_code='EXPRESSION_ERROR' WHERE tenant_id=$1 AND run_id=$2 AND node='delay'"
                    }
                    _ => unreachable!("closed test vocabulary"),
                };
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .bind(flow.tenant.as_uuid())
                    .bind(flow.run.as_uuid())
                    .execute(&flow.api.db.pool)
                    .await
                    .expect("CHECK-valid owner fixture without Run revision");
                let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
                let rejected = if let Some(mut tx) = full_tx.take() {
                    assert!(
                        tx.holds_shard(authority.owner(), authority.lease())
                            .await
                            .expect("authority")
                    );
                    let result = Engine::new(JsonEvaluator)
                        .run_addressed(
                            &mut tx,
                            Vec::new(),
                            Effects::by(Actor::System("timer-invariant-test"), None),
                            |definition, state, now| {
                                let progress = if fire {
                                    sagat_domain::execution::fire_timer_instance(
                                        definition, state, &identity, now,
                                    )?
                                } else {
                                    sagat_domain::execution::start_timer_instance(
                                        definition, state, &identity, now,
                                    )?
                                };
                                Ok(((), progress))
                            },
                        )
                        .await;
                    drop(tx);
                    result.is_err()
                } else {
                    let mut tx = flow
                        .api
                        .executions
                        .begin_path(prepared.expect("snapshot"), Lock::Wait)
                        .await
                        .expect("unchanged Run revision")
                        .expect("run");
                    assert!(
                        tx.holds_shard(authority.owner(), authority.lease())
                            .await
                            .expect("authority")
                    );
                    let now = tx.now();
                    let progress = {
                        let snapshot = tx.snapshot();
                        if fire {
                            sagat_domain::execution::fire_timer_path_instance(
                                &snapshot.definition,
                                &mut snapshot.state,
                                &identity,
                                now,
                            )
                        } else {
                            sagat_domain::execution::start_timer_path_instance(
                                &snapshot.definition,
                                &mut snapshot.state,
                                &identity,
                                now,
                            )
                        }
                    }
                    .expect("the prepared state is valid");
                    let result = tx
                        .persist_addressed(
                            &progress.events,
                            &[],
                            Effects::by(Actor::System("timer-invariant-test"), None),
                        )
                        .await;
                    drop(tx);
                    result.is_err()
                };
                if !rejected {
                    accepted.push((full, fire, *change));
                }
                assert_eq!(
                    snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
                    before
                );
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "prepared writes silently repaired changed timer states: {accepted:?}"
    );
}

#[tokio::test]
async fn an_invented_failed_timer_is_rejected_by_all_readers() {
    use sagat_application::scope_frontier::ScopeFrontierStore;
    let flow = waiting_timer("timer-invented-failed-read").await;
    sqlx::query("UPDATE steps SET status='FAILED',failure_code='EXPRESSION_ERROR',waiting_kind=NULL,due_at=NULL WHERE tenant_id=$1 AND run_id=$2 AND node='delay'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).execute(&flow.api.db.pool).await.expect("deliberate unsupported timer failure projection");
    let whole = flow
        .api
        .executions
        .begin(flow.tenant, flow.run, Lock::Wait)
        .await;
    let whole_error = whole.is_err();
    drop(whole);
    let bounded = flow
        .api
        .executions
        .load_path(flow.tenant, flow.run, timer().path().clone())
        .await
        .is_err();
    let frontier = flow
        .api
        .executions
        .collection_frontier(
            flow.tenant,
            flow.run,
            &ExecutionInstanceId::root(NodeId::parse("lines").expect("owner")),
        )
        .await
        .is_err();
    assert_eq!(
        (whole_error, bounded, frontier),
        (true, true, true),
        "timers have no Failed transition"
    );
}

#[tokio::test]
async fn generic_step_failure_cannot_persist_an_unsupported_timer_failure() {
    let _server = crate::support::hold_server();
    let mut accepted = Vec::new();
    for full in [false, true] {
        let flow = waiting_timer(&format!("timer-invented-failed-write-{full}")).await;
        let identity = timer();
        let authority = authority(&flow).await;
        let event = sagat_domain::execution::AddressedRunEvent::in_path(
            identity.path().clone(),
            sagat_domain::execution::RunEvent::StepFailed {
                node: identity.node().clone(),
                code: sagat_domain::execution::FailureCode::expression_error(),
                fence: None,
            },
        )
        .expect("generic addressed event");
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        let rejected = if full {
            let mut tx = flow
                .api
                .executions
                .begin(flow.tenant, flow.run, Lock::Wait)
                .await
                .expect("full")
                .expect("run");
            assert!(
                tx.holds_shard(authority.owner(), authority.lease())
                    .await
                    .expect("authority")
            );
            let result = Engine::new(JsonEvaluator)
                .run_addressed(
                    &mut tx,
                    Vec::new(),
                    Effects::by(Actor::System("timer-invariant-test"), None),
                    |_, state, _| {
                        state.apply_addressed(&event)?;
                        Ok((
                            (),
                            sagat_domain::execution::AddressedProgress {
                                events: vec![event.clone()],
                            },
                        ))
                    },
                )
                .await;
            drop(tx);
            result.is_err()
        } else {
            let prepared = flow
                .api
                .executions
                .load_path(flow.tenant, flow.run, identity.path().clone())
                .await
                .expect("path")
                .expect("snapshot");
            let mut tx = flow
                .api
                .executions
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("bounded")
                .expect("run");
            assert!(
                tx.holds_shard(authority.owner(), authority.lease())
                    .await
                    .expect("authority")
            );
            tx.snapshot()
                .state
                .apply_addressed(&event)
                .expect("generic reducer has no definition kind");
            let result = tx
                .persist_addressed(
                    &[event],
                    &[],
                    Effects::by(Actor::System("timer-invariant-test"), None),
                )
                .await;
            drop(tx);
            result.is_err()
        };
        if !rejected {
            accepted.push(full);
        }
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
    }
    assert!(
        accepted.is_empty(),
        "native writers accepted an invented timer failure: {accepted:?}"
    );
}

#[tokio::test]
async fn timer_member_progress_matches_the_full_run_oracle() {
    use crate::support::scope_flow::collection_document;
    use sagat_scheduler::CollectionAdvance;
    use serde_json::Value;
    async fn run_case(full: bool) -> (Vec<CollectionAdvance>, Vec<Value>) {
        let name = format!("timer-oracle-{full}");
        let flow = Flow::start(
            &name,
            collection_document(
                &name,
                vec![
                    json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                    json!({"id":"returned","type":"return","result":"item"}),
                ],
                1,
            ),
            json!([1, 2]),
        )
        .await;
        let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
        let mut transitions = Vec::new();
        let mut fired = 0;
        for _ in 0..40 {
            let progress = if full {
                flow.scheduler
                    .advance_collection_from_run(flow.tenant, flow.run, &owner)
                    .await
            } else {
                flow.scheduler
                    .advance_collection(flow.tenant, flow.run, &owner)
                    .await
            }
            .expect("one native collection transition");
            transitions.push(progress);
            if progress == CollectionAdvance::Finished {
                break;
            }
            if progress != CollectionAdvance::Waiting {
                continue;
            }
            assert!(flow.poll().await.is_empty(), "timers own no workers");
            let index:i32=sqlx::query_scalar("SELECT instance_path_indices[1] FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='delay' AND status='WAITING' AND waiting_kind='TIMER'")
                .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("the one waiting member");
            let instance = ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(
                        NodeId::parse("lines").expect("owner"),
                        u32::try_from(index).expect("index"),
                    )
                    .expect("path"),
                NodeId::parse("delay").expect("timer"),
            );
            age(&flow).await;
            let a = authority(&flow).await;
            if full {
                let mut tx = flow
                    .api
                    .executions
                    .begin(flow.tenant, flow.run, Lock::Wait)
                    .await
                    .expect("full begin")
                    .expect("run");
                assert!(
                    tx.holds_shard(a.owner(), a.lease())
                        .await
                        .expect("current authority")
                );
                Engine::new(JsonEvaluator)
                    .run_addressed(
                        &mut tx,
                        Vec::new(),
                        Effects::by(Actor::System("timer-oracle"), None),
                        |definition, state, now| {
                            Ok((
                                (),
                                sagat_domain::execution::fire_timer_instance(
                                    definition, state, &instance, now,
                                )?,
                            ))
                        },
                    )
                    .await
                    .expect("full fire");
                tx.commit().await.expect("full commit");
            } else {
                let snapshot = flow
                    .api
                    .executions
                    .load_path(flow.tenant, flow.run, instance.path().clone())
                    .await
                    .expect("bounded load")
                    .expect("run");
                let mut tx = flow
                    .api
                    .executions
                    .begin_path(snapshot, Lock::Wait)
                    .await
                    .expect("bounded begin")
                    .expect("run");
                assert!(
                    tx.holds_shard(a.owner(), a.lease())
                        .await
                        .expect("current authority")
                );
                let now = tx.now();
                let snapshot = tx.snapshot();
                let progress = sagat_domain::execution::fire_timer_path_instance(
                    &snapshot.definition,
                    &mut snapshot.state,
                    &instance,
                    now,
                )
                .expect("bounded fire");
                tx.persist_addressed(
                    &progress.events,
                    &[],
                    Effects::by(Actor::System("timer-oracle"), None),
                )
                .await
                .expect("bounded persist");
                tx.commit().await.expect("bounded commit");
            }
            fired += 1;
        }
        assert_eq!(transitions.last(), Some(&CollectionAdvance::Finished));
        assert_eq!(fired, 2);
        assert_eq!(flow.run_status().await, "SUCCEEDED");
        flow.verify().await;
        let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('seq',seq,'kind',event_type,'node',node,'nodes',instance_path_nodes,'indices',instance_path_indices,'detail',detail) FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq")
            .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_all(&flow.api.db.pool).await.expect("complete journal");
        let uuid =
            regex::Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
                .expect("UUID pattern");
        let journal = rows
            .into_iter()
            .map(|mut row| {
                if row["kind"] == "timer.started" {
                    let detail = row["detail"].as_object_mut().expect("timer detail");
                    assert_eq!(detail.len(), 1, "only the explicit timer deadline");
                    assert!(detail["due_at_us"].as_i64().is_some_and(|due| due > 0));
                    detail.insert("due_at_us".into(), json!("<database deadline>"));
                }
                serde_json::from_str(&uuid.replace_all(&row.to_string(), "<id>"))
                    .expect("identity/clock normalized event")
            })
            .collect();
        (transitions, journal)
    }
    let _server = crate::support::hold_server();
    assert_eq!(
        run_case(false).await,
        run_case(true).await,
        "exact transition order and journal apart from generated identities and explicit clocks"
    );
}
