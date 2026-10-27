//! Internal immutable structured-codec fixtures, not public activation or released-profile proof.
use super::scoped_worker::{self, Fixture, StructuredJsonIrCodec};
use sagat_application::{
    execution::{
        Actor, DefinitionCache, Effects, ExpressionEvaluator, PathExecutionStore, PathExecutionTx,
        RunTx,
    },
    runs::{Lock, RunStore},
};
use sagat_domain::{
    execution::{self, ExecutionInstanceId, InstancePath, StepStatus},
    sharding::Shard,
    workflow::NodeId,
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::json;
use std::sync::Arc;

mod cancellation;
mod guards;
mod receipts;
mod timeouts;

fn wait(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("items").expect("owner"), index)
            .expect("path"),
        NodeId::parse("b").expect("wait"),
    )
}

async fn pending_wait(name: &str) -> (Fixture, PgRunStore) {
    pending_wait_with_route(name, true).await
}

async fn pending_wait_with_route(name: &str, timeout_route: bool) -> (Fixture, PgRunStore) {
    pending_wait_with_timeout(name, timeout_route, 60000).await
}

async fn pending_wait_with_timeout(
    name: &str,
    timeout_route: bool,
    timeout_ms: u64,
) -> (Fixture, PgRunStore) {
    pending_wait_document(name, wait_document(name, timeout_route, timeout_ms)).await
}

fn wait_document(name: &str, timeout_route: bool, timeout_ms: u64) -> serde_json::Value {
    let mut document = scoped_worker::structured_workflow(name);
    document["bodies"]["line"]["nodes"][1] = json!({"id":"b","type":"waitEvent","eventType":"payment.confirmed","correlationExpr":"item.sku","timeoutMs":60000,"lookbackMs":1000,"onTimeout":"returned","next":"returned"});
    document["bodies"]["line"]["nodes"][1]["timeoutMs"] = json!(timeout_ms);
    if !timeout_route {
        document["bodies"]["line"]["nodes"][1]
            .as_object_mut()
            .expect("wait object")
            .remove("onTimeout");
    }
    document["bodies"]["line"]["nodes"][2]["result"] = json!("item");
    document
}

async fn pending_wait_document(name: &str, document: serde_json::Value) -> (Fixture, PgRunStore) {
    let fixture = scoped_worker::fixture_with_document(name, document).await;
    pending_wait_fixture(fixture).await
}

async fn pending_wait_fixture(fixture: Fixture) -> (Fixture, PgRunStore) {
    let work = scoped_worker::poll(&fixture, 2).await;
    assert_eq!(work.len(), 2);
    for task in work {
        fixture
            .workers
            .complete(
                &fixture.session.principal,
                &crate::support::engine::task_ref(&task),
                b"{}",
                &format!("scope-event-source-{}", scoped_worker::index(&task)),
            )
            .await
            .expect("real scoped activity completion");
    }
    let runtime = fixture.world.db.runtime_pool().await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("runtime role");
    assert_eq!(role, "sagat_app");
    let store = PgRunStore::new(
        runtime,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            wait(0).path().clone(),
        )
        .await
        .expect("real pending path")
        .expect("path");
    assert_eq!(
        snapshot
            .state
            .local_instance(wait(0).node())
            .expect("actual routed wait")
            .status(),
        StepStatus::Pending
    );
    assert!(
        scoped_worker::poll(&fixture, 8).await.is_empty(),
        "a pending event wait creates no worker work"
    );
    (fixture, store)
}

async fn assert_binding(
    fixture: &Fixture,
    store: &PgRunStore,
    index: u32,
    key: &str,
    now: time::OffsetDateTime,
) {
    let row:(String,String,String,time::OffsetDateTime,time::OffsetDateTime,bool)=sqlx::query_as("SELECT status,event_type,correlation_key,due_at,event_since,activity IS NULL AND ready_at IS NULL AND lease_token_hash IS NULL AND lease_expires_at IS NULL AND leased_by IS NULL AND reported_lease_token_hash IS NULL AND event_scanned_at IS NULL AND output_digest IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_nodes=ARRAY['items']::text[] AND instance_path_indices=ARRAY[$3]::int[]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).bind(i32::try_from(index).expect("index")).fetch_one(&fixture.world.db.pool).await.expect("independent stored binding");
    assert_eq!(
        row,
        (
            "WAITING".into(),
            "payment.confirmed".into(),
            key.into(),
            now + time::Duration::seconds(60),
            now - time::Duration::seconds(1),
            true
        )
    );
    let other:(String,Option<String>,Option<time::OffsetDateTime>)=sqlx::query_as("SELECT status,correlation_key,event_since FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_nodes=ARRAY['items']::text[] AND instance_path_indices=ARRAY[$3]::int[]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).bind(i32::try_from(1-index).expect("sibling")).fetch_one(&fixture.world.db.pool).await.expect("untouched sibling");
    assert_eq!(other, ("PENDING".into(), None, None));
    assert!(scoped_worker::poll(fixture, 8).await.is_empty());
    store
        .verify_run(fixture.world.admin.tenant_id(), fixture.run)
        .await
        .expect("full journal replay");
}

#[tokio::test]
async fn native_start_records_an_exact_wait_after_real_worker_routing() {
    let (fixture, store) = pending_wait("scoped-event-native-start").await;
    let member = wait(0);
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("evaluation snapshot")
        .expect("path");
    let key = JsonEvaluator
        .correlation_key_instance(
            &sagat_domain::workflow::Expression::parse("item.sku").expect("expression"),
            &snapshot.data,
            &member,
        )
        .expect("own captured item");
    assert_eq!(key.as_str(), "first");
    let mut tx = store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("revision-fenced entry")
        .expect("path");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    assert!(
        tx.holds_shard("scoped-worker-test", lease)
            .await
            .expect("native own guard")
    );
    let now = tx.now();
    let progress = {
        let snapshot = tx.snapshot();
        execution::start_event_wait_path_instance(
            &snapshot.definition,
            &mut snapshot.state,
            &member,
            Ok(key),
            now,
        )
        .expect("pure native-clock start")
    };
    tx.persist_addressed(
        &progress.events,
        &[],
        Effects::by(Actor::System("scheduler"), None),
    )
    .await
    .expect("native start storage");
    tx.commit().await.expect("atomic native start");
    assert_binding(&fixture, &store, 0, "first", now).await;
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("reopen wait")
        .expect("path");
    assert_eq!(
        snapshot
            .state
            .local_instance(member.node())
            .expect("stored wait")
            .status(),
        StepStatus::Waiting
    );
}

#[tokio::test]
async fn correlation_failure_start_cannot_commit_without_an_acknowledged_own_shard_guard() {
    let (fixture, store) = pending_wait("scoped-event-unfenced-error").await;
    let member = wait(0);
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("snapshot")
        .expect("path");
    let mut tx = store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("own run lock")
        .expect("path");
    let now = tx.now();
    let progress = {
        let snapshot = tx.snapshot();
        execution::start_event_wait_path_instance(
            &snapshot.definition,
            &mut snapshot.state,
            &member,
            Err(()),
            now,
        )
        .expect("pure failed start")
    };
    let accepted = tx
        .persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("scheduler"), None),
        )
        .await
        .is_ok();
    if accepted {
        tx.commit()
            .await
            .expect("observe wrongly acknowledged native commit");
    } else {
        drop(tx);
    }
    assert!(
        !accepted,
        "even an expression-failure start requires acknowledged own scheduler authority"
    );
}

#[tokio::test]
async fn correlation_failure_start_commits_with_acknowledged_own_shard_guard() {
    let (fixture, store) = pending_wait("scoped-event-fenced-error").await;
    let member = wait(0);
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("snapshot")
        .expect("path");
    let mut tx = store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("own run lock")
        .expect("path");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    assert!(
        tx.holds_shard("scoped-worker-test", lease)
            .await
            .expect("own authority")
    );
    let now = tx.now();
    let progress = {
        let snapshot = tx.snapshot();
        execution::start_event_wait_path_instance(
            &snapshot.definition,
            &mut snapshot.state,
            &member,
            Err(()),
            now,
        )
        .expect("pure failed start")
    };
    tx.persist_addressed(
        &progress.events,
        &[],
        Effects::by(Actor::System("scheduler"), None),
    )
    .await
    .expect("fenced failed start");
    tx.commit().await.expect("atomic failed start");
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("reopen failed start")
        .expect("path");
    assert_eq!(
        snapshot
            .state
            .local_instance(member.node())
            .expect("failure")
            .failure(),
        Some(&execution::FailureCode::expression_error())
    );
}

#[tokio::test]
async fn full_native_start_preserves_the_other_same_name_member_and_replays_cold() {
    let (fixture, store) = pending_wait("scoped-event-full-start").await;
    let member = wait(1);
    let mut tx = store
        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
        .await
        .expect("full entry")
        .expect("run");
    let lease = fixture
        .placement
        .snapshot()
        .lease_of(Shard::of(fixture.run.as_uuid()), std::time::Instant::now())
        .expect("own shard");
    assert!(
        tx.holds_shard("scoped-worker-test", lease)
            .await
            .expect("own authority")
    );
    let now = tx.now();
    let data = tx.data().await.expect("owned data").clone();
    let key = JsonEvaluator
        .correlation_key_instance(
            &sagat_domain::workflow::Expression::parse("item.sku").expect("expression"),
            &data,
            &member,
        )
        .expect("second item");
    assert_eq!(key.as_str(), "second");
    let progress = {
        let (definition, state) = tx.parts();
        execution::start_event_wait_instance(definition, state, &member, Ok(key), now)
            .expect("full start")
    };
    tx.persist_addressed(
        &progress.events,
        &[],
        Effects::by(Actor::System("scheduler"), None),
    )
    .await
    .expect("full native storage");
    tx.commit().await.expect("full start commit");
    let cold = PgRunStore::new(
        fixture.world.db.runtime_pool().await,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    );
    assert_binding(&fixture, &cold, 1, "second", now).await;
}

#[tokio::test]
async fn a_generic_failure_cannot_impersonate_a_clocked_unhandled_timeout() {
    let (fixture, store) = pending_wait_with_route("scoped-event-fake-timeout", false).await;
    let member = wait(0);
    let snapshot = store
        .load_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            member.path().clone(),
        )
        .await
        .expect("snapshot")
        .expect("path");
    let mut tx = store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("entry")
        .expect("path");
    let failed = execution::AddressedRunEvent::in_path(
        member.path().clone(),
        execution::RunEvent::StepFailed {
            node: member.node().clone(),
            code: execution::FailureCode::wait_timed_out(),
            fence: None,
        },
    )
    .expect("fake timeout event");
    let events = {
        let snapshot = tx.snapshot();
        snapshot
            .state
            .apply_addressed(&failed)
            .expect("demonstrate generic reducer shape");
        let mut events = vec![failed];
        events.extend(
            execution::release_pending_path(&snapshot.definition, &mut snapshot.state)
                .expect("local closure")
                .events,
        );
        events
    };
    let accepted = tx
        .persist_addressed(&events, &[], Effects::by(Actor::System("scheduler"), None))
        .await
        .is_ok();
    if accepted {
        tx.commit()
            .await
            .expect("observe wrongly committed timeout impersonation");
    } else {
        drop(tx);
    }
    assert!(
        !accepted,
        "a timeout must use its due clock event and own authority, not generic StepFailed"
    );
}
