//! Control admission preserves the native slot, revision and journal transaction boundaries.
use super::collection_finish_atomic::snapshot;
use crate::support::scope_flow::{Flow, collection_document};
use sagat_application::{
    execution::{DefinitionCache, Engine},
    maintenance::{MaintenanceStore, Window},
    primitives::{
        CollectionAdmission, CollectionScopeId, CollectionStore, CollectionTransition,
        PrimitiveStoreError, SchedulerAuthority,
    },
};
use sagat_domain::{
    execution::ExecutionInstanceId,
    identity::{NamespaceId, Subject},
    workflow::NodeId,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{MIGRATOR, PgRunStore, migrate};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

fn owner() -> ExecutionInstanceId {
    ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"))
}

async fn entered(name: &str) -> Flow {
    let flow = Flow::start(name, collection_document(name, vec![
        json!({"id":"choose", "type":"condition", "expression":"item > 0", "ifTrue":"returned", "ifFalse":"returned"}),
        json!({"id":"returned", "type":"return", "result":"item"}),
    ], 2), json!([1, -1])).await;
    assert_eq!(
        flow.scheduler
            .advance_collection(flow.tenant, flow.run, &owner())
            .await
            .expect("enter only"),
        sagat_scheduler::CollectionAdvance::Entered
    );
    flow
}

pub(super) async fn admission(flow: &Flow, index: u32) -> CollectionAdmission {
    let row = sqlx::query(
        "SELECT r.namespace_id,r.trace_id,r.run_version,s.scope_id,sh.owner,sh.generation,clock_timestamp() AS now \
         FROM runs r JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
         JOIN scheduler_shards sh ON sh.shard=run_shard(r.id) WHERE r.tenant_id=$1 AND r.id=$2",
    ).bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("native authority");
    let scope = CollectionScopeId::new(row.get("scope_id"));
    let transition = CollectionTransition::new(
        flow.tenant,
        NamespaceId::new(row.get("namespace_id")),
        flow.run,
        u64::try_from(row.get::<i64, _>("run_version")).expect("version"),
        owner(),
        SchedulerAuthority::for_run(
            row.get::<String, _>("owner"),
            flow.run,
            u64::try_from(row.get::<i64, _>("generation")).expect("generation"),
        ),
        Subject::engine(),
        row.get("trace_id"),
        row.get("now"),
        None,
    );
    CollectionAdmission::scheduler_from_item(
        transition,
        scope,
        index,
        ExecutionInstanceId::in_path(
            owner()
                .path()
                .child(owner().node().clone(), index)
                .expect("path"),
            NodeId::parse("choose").expect("entry"),
        ),
        flow.api
            .executions
            .collection_item(flow.tenant, flow.run, scope, index)
            .await
            .expect("captured item")
            .expect("membership"),
        1,
    )
    .expect("command")
}

#[tokio::test]
async fn scheduler_admission_rolls_back_on_journal_failure_and_replays_without_a_fake_outbox() {
    let flow = entered("control-entry-atomic").await;
    let command = admission(&flow, 0).await;
    sqlx::raw_sql("CREATE FUNCTION reject_control_admission() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only control admission abort' USING ERRCODE='23514'; END $$; CREATE TRIGGER reject_control_admission BEFORE INSERT ON run_events FOR EACH ROW WHEN (NEW.event_type='collection.iteration_admitted') EXECUTE FUNCTION reject_control_admission()")
        .execute(&flow.api.db.pool).await.expect("owned journal fault");
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert!(
        flow.api
            .executions
            .admit_collection_member(&command)
            .await
            .expect_err("journal fault")
            .to_string()
            .contains("test-only control admission abort")
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        before
    );
    sqlx::raw_sql("DROP TRIGGER reject_control_admission ON run_events; DROP FUNCTION reject_control_admission()")
        .execute(&flow.api.db.pool).await.expect("remove owned fault");
    let committed = flow
        .api
        .executions
        .admit_collection_member(&command)
        .await
        .expect("admit");
    assert!(committed.applied);
    let after = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert_eq!(
        after["journal"].as_array().expect("journal").len(),
        before["journal"].as_array().expect("journal").len() + 1
    );
    assert_eq!(after["outbox"], before["outbox"]);
    assert!(
        !flow
            .api
            .executions
            .admit_collection_member(&command)
            .await
            .expect("exact replay")
            .applied
    );
    let mut mismatch = command.clone();
    mismatch.item_digest = sagat_domain::workflow::Digest::new([7; 32]);
    assert_eq!(
        flow.api.executions.admit_collection_member(&mismatch).await,
        Err(PrimitiveStoreError::DuplicateMismatch)
    );
    assert_eq!(
        flow.api
            .executions
            .admit_collection_activity(&command)
            .await,
        Err(PrimitiveStoreError::InvalidTransition)
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        after
    );
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn control_admission_honors_revision_and_capacity_while_accepted_work_continues_cold() {
    let flow = entered("control-entry-capacity-cold").await;
    let first = admission(&flow, 0).await;
    let stale = admission(&flow, 1).await;
    flow.api
        .executions
        .admit_collection_member(&first)
        .await
        .expect("first admitted");
    assert!(matches!(
        flow.api.executions.admit_collection_member(&stale).await,
        Err(PrimitiveStoreError::RevisionConflict { .. })
    ));
    let response = flow
        .api
        .as_user(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit":1}))
        .send()
        .await
        .expect("reduce capacity");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let next = admission(&flow, 1).await;
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert_eq!(
        flow.api.executions.admit_collection_member(&next).await,
        Err(PrimitiveStoreError::AtCapacity)
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        before
    );
    // A new cache, store and scheduler have no in-memory admission hint or placement ownership.
    let cold = PgRunStore::new(
        flow.api.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let candidates = cold
        .collection_candidates(&Window::everything(32))
        .await
        .expect("durable discovery");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.run_id == flow.run && candidate.instance() == owner())
    );
    let placement = Placement::new(first.transition.scheduler().owner());
    Keeper::new(
        cold.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("renew exact owner's authority from the database");
    let scheduler = Scheduler::new(
        cold,
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    assert_eq!(
        scheduler
            .advance_collection(flow.tenant, flow.run, &owner())
            .await
            .expect("accepted cold choice despite reduction"),
        sagat_scheduler::CollectionAdvance::BodyProgress
    );
    assert_eq!(
        scheduler
            .advance_collection(flow.tenant, flow.run, &owner())
            .await
            .expect("accepted checkpoint despite reduction"),
        sagat_scheduler::CollectionAdvance::Checkpointed { index: 0 }
    );
    let active: i32 = sqlx::query_scalar(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1",
    )
    .bind(flow.tenant.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("accepted slot released");
    assert_eq!(active, 0);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

async fn disguised_scheduler(name: &str) -> Flow {
    let flow = entered(name).await;
    flow.api
        .executions
        .admit_collection_member(&admission(&flow, 0).await)
        .await
        .expect("real scheduler admission");
    sqlx::query("UPDATE collection_iterations SET entry_dispatch='ACTIVITY',activity='fake',outbox_id=$3 WHERE tenant_id=$1 AND run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).bind(uuid::Uuid::new_v4()).execute(&flow.api.db.pool).await.expect("CHECK-valid contradictory projection");
    flow
}

#[tokio::test]
async fn a_disguised_scheduler_membership_is_rejected_by_the_full_projection_reader() {
    use sagat_application::runs::{Lock, RunStore};
    use sagat_application::{execution::PathExecutionStore, scope_frontier::ScopeFrontierStore};
    let flow = disguised_scheduler("control-entry-disguised-read").await;
    let result = flow
        .api
        .executions
        .begin(flow.tenant, flow.run, Lock::Wait)
        .await;
    assert!(
        result.is_err(),
        "the pinned control entry cannot restore an ACTIVITY dispatch"
    );
    let path = owner()
        .path()
        .child(owner().node().clone(), 0)
        .expect("member path");
    assert!(
        flow.api
            .executions
            .load_path(flow.tenant, flow.run, path)
            .await
            .is_err(),
        "bounded reader rejects the disguised dispatch"
    );
    assert!(
        flow.api
            .executions
            .collection_frontier(flow.tenant, flow.run, &owner())
            .await
            .is_err(),
        "frontier rejects the disguised dispatch"
    );
}

#[tokio::test]
async fn immutable_control_admission_evidence_blocks_down_even_if_the_projection_is_disguised() {
    let flow = disguised_scheduler("control-entry-disguised-down").await;
    let mut connection = flow
        .api
        .db
        .pool
        .acquire()
        .await
        .expect("dedicated migrator");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        MIGRATOR.undo(&mut *connection, 20_261_220_000_000),
    )
    .await
    .expect("bounded downgrade");
    connection.close().await.expect("close migrator session");
    assert!(
        result.is_err(),
        "immutable admission evidence must prevent unsafe rollback"
    );
}

#[tokio::test]
async fn retained_scheduler_memberships_refuse_downgrade_even_after_completion() {
    let flow = entered("control-entry-downgrade").await;
    let command = admission(&flow, 0).await;
    flow.api
        .executions
        .admit_collection_member(&command)
        .await
        .expect("admit");
    for terminal in [false, true] {
        if terminal {
            flow.drive().await;
            assert_eq!(flow.run_status().await, "SUCCEEDED");
        }
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        // SQLx 0.9 leaves its session advisory lock held on a refused migration. Close the
        // dedicated acquired connection rather than returning that locked session to the pool.
        let mut connection = flow
            .api
            .db
            .pool
            .acquire()
            .await
            .expect("dedicated migrator");
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            MIGRATOR.undo(&mut *connection, 20_261_220_000_000),
        )
        .await
        .expect("bounded retained-evidence refusal")
        .expect_err("retained scheduler evidence");
        connection
            .close()
            .await
            .expect("release refused migrator session lock");
        assert!(
            error
                .to_string()
                .contains("scheduler-owned collection admission evidence is retained"),
            "{error}"
        );
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
        migrate(&flow.api.db.pool)
            .await
            .expect("failed downgrade preserves profile");
    }
}

#[tokio::test]
async fn cancelling_a_pending_control_releases_its_slot_without_worker_ownership() {
    let flow = entered("control-entry-cancel").await;
    flow.api
        .executions
        .admit_collection_member(&admission(&flow, 0).await)
        .await
        .expect("pending control");
    let (status, _) = flow
        .operate(
            json!({"action":"CANCEL", "reason":"cancel before evaluating the admitted control"}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "CANCELLED");
    assert!(flow.poll().await.is_empty());
    let counts: (i64,i64,i32) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND slot_held), \
         (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'), \
         active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1",
    ).bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("no ownership leak");
    assert_eq!(counts, (0, 0, 0));
    flow.verify().await;
}

#[tokio::test]
async fn activity_memberships_backfill_and_downgrade_but_dispatch_contradictions_are_refused() {
    use crate::support::scope_flow::activity;
    let flow = Flow::start(
        "control-entry-backfill",
        collection_document(
            "control-entry-backfill",
            vec![
                activity("work", "line.notify", "returned", json!({})),
                json!({"id":"returned", "type":"return", "result":"item"}),
            ],
            1,
        ),
        json!([1]),
    )
    .await;
    flow.drive().await;
    MIGRATOR
        .undo(&flow.api.db.pool, 20_261_220_000_000)
        .await
        .expect("activity-only profile reverses");
    let original: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(m) FROM collection_iterations m WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("old row without dispatch");
    assert!(original.get("entry_dispatch").is_none());
    migrate(&flow.api.db.pool)
        .await
        .expect("backfill retained activity membership");
    let restored: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(m)-'entry_dispatch' FROM collection_iterations m WHERE tenant_id=$1 AND run_id=$2 AND entry_dispatch='ACTIVITY'")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("default backfilled");
    assert_eq!(restored, original);
    for statement in [
        "UPDATE collection_iterations SET entry_dispatch='SCHEDULER' WHERE tenant_id=$1 AND run_id=$2",
        "UPDATE collection_iterations SET activity=NULL WHERE tenant_id=$1 AND run_id=$2",
        "UPDATE collection_iterations SET entry_dispatch='UNKNOWN' WHERE tenant_id=$1 AND run_id=$2",
    ] {
        // A fixed test-only SQL vocabulary exercises the closed storage dispatch contract.
        let error = sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .execute(&flow.api.db.pool)
            .await
            .expect_err("contradictory dispatch is rejected by PostgreSQL");
        let database = error.as_database_error().expect("native constraint");
        assert_eq!(database.code().as_deref(), Some("23514"));
        assert_eq!(
            database.constraint(),
            Some("collection_iterations_entry_dispatch")
        );
    }
    let work = flow.poll().await;
    assert_eq!(work.len(), 1);
    flow.complete(&work[0], json!({})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn public_scheduler_entries_remain_outside_the_frozen_ordinary_leaf_profile() {
    use sagat_application::leaf_certification::{
        CertifyLeafCollectionSuccess, LeafCollectionPathIdentity, LeafCollectionSuccessError,
        LeafCollectionSuccessStore,
    };
    use sagat_domain::workflow::{CollectionSuccessShapeError, LeafCollectionSuccessShape};
    let _server = crate::support::hold_server();
    for kind in ["condition", "switch", "timer"] {
        let name = format!("control-entry-frozen-{kind}");
        let control = if kind == "condition" {
            json!({"id":"choose","type":"condition","expression":"item > 0","ifTrue":"returned","ifFalse":"returned"})
        } else if kind == "switch" {
            json!({"id":"choose","type":"switch","cases":[{"id":"positive","when":"item > 0","target":"returned"}],"default":"returned"})
        } else {
            json!({"id":"choose","type":"timer","durationMs":1,"next":"returned"})
        };
        let document = collection_document(
            &name,
            vec![
                control,
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            1,
        );
        let parsed = JsonIrCodec
            .parse_structured(&serde_json::to_vec(&document).expect("document"))
            .expect("current public shape");
        assert_eq!(
            LeafCollectionSuccessShape::check(&parsed.definition, &owner()),
            Err(CollectionSuccessShapeError::UnsupportedBody)
        );
        let flow = Flow::start(&name, document, json!([1])).await;
        flow.drive().await;
        assert_eq!(flow.run_status().await, "SUCCEEDED");
        // Reconstruct current authority from the same native bounded command helper.
        let a = admission(&flow, 0).await;
        let command = CertifyLeafCollectionSuccess {
            identity: LeafCollectionPathIdentity::new(
                flow.tenant,
                a.transition.namespace_id(),
                flow.run,
                a.entry.path().clone(),
            )
            .expect("leaf identity"),
            expected_run_revision: a.transition.expected_run_version(),
            scheduler: a.transition.scheduler().clone(),
        };
        let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
        for pool in [&flow.api.db.pool, &flow.api.api_pool] {
            let store = sagat_infra_postgres::PgLeafCollectionSuccessStore::new(
                pool.clone(),
                DefinitionCache::new(Arc::new(JsonIrCodec), 8),
            );
            assert_eq!(
                store.certify_leaf_collection_success(&command).await,
                Err(LeafCollectionSuccessError::UnsupportedShape(
                    CollectionSuccessShapeError::UnsupportedBody
                ))
            );
        }
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
        flow.verify().await;
    }
}

#[tokio::test]
async fn scheduler_entry_rejects_wrong_dispatch_membership_tenant_and_shard_authority() {
    use sagat_application::primitives::CollectionEntryDispatch;
    use sagat_domain::{
        identity::TenantId,
        workflow::{ActivityName, Digest},
    };
    let flow = entered("control-entry-refusals").await;
    let valid = admission(&flow, 0).await;
    let transition = |tenant, scheduler, revision| {
        CollectionTransition::new(
            tenant,
            valid.transition.namespace_id(),
            flow.run,
            revision,
            owner(),
            scheduler,
            Subject::engine(),
            valid.transition.trace_id().map(str::to_owned),
            valid.transition.occurred_at(),
            None,
        )
    };
    let mut wrong_kind = valid.clone();
    wrong_kind.dispatch = CollectionEntryDispatch::Activity {
        activity: ActivityName::parse("line.notify").expect("activity"),
        outbox_id: uuid::Uuid::new_v4(),
    };
    let mut wrong_item = valid.clone();
    wrong_item.item_digest = Digest::new([7; 32]);
    let mut wrong_reservation = valid.clone();
    wrong_reservation.reserved_instances = 2;
    let mut wrong_authority = valid.clone();
    wrong_authority.transition = transition(
        flow.tenant,
        SchedulerAuthority::for_run(
            valid.transition.scheduler().owner(),
            flow.run,
            valid.transition.scheduler().lease().generation + 1,
        ),
        valid.transition.expected_run_version(),
    );
    let mut wrong_tenant = valid.clone();
    wrong_tenant.transition = transition(
        TenantId::new(uuid::Uuid::new_v4()),
        valid.transition.scheduler().clone(),
        valid.transition.expected_run_version(),
    );
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    for (command, expected) in [
        (wrong_kind, PrimitiveStoreError::InvalidTransition),
        (wrong_item, PrimitiveStoreError::InvalidTransition),
        (wrong_reservation, PrimitiveStoreError::InvalidTransition),
        (wrong_authority, PrimitiveStoreError::SchedulerFenceLost),
        (wrong_tenant, PrimitiveStoreError::Missing),
    ] {
        assert_eq!(
            flow.api.executions.admit_collection_member(&command).await,
            Err(expected)
        );
        assert_eq!(
            snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
            before
        );
    }
    assert!(
        flow.api
            .executions
            .admit_collection_member(&valid)
            .await
            .expect("valid command after refusals")
            .applied
    );
    flow.drive().await;
    flow.verify().await;
}

#[tokio::test]
async fn a_current_revision_admission_cannot_release_new_work_while_the_run_is_paused() {
    let flow = entered("control-entry-pause-admission").await;
    let (status, _) = flow
        .operate(json!({"action":"PAUSE","reason":"pause before new member admission"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    let command = admission(&flow, 0).await;
    let before = snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await;
    assert_eq!(
        flow.api.executions.admit_collection_member(&command).await,
        Err(PrimitiveStoreError::InvalidTransition)
    );
    assert_eq!(
        snapshot(&flow.api.db.pool, flow.tenant.as_uuid(), flow.run).await,
        before
    );
    let (status, _) = flow
        .operate(json!({"action":"RESUME","reason":"resume explicit admission"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}
