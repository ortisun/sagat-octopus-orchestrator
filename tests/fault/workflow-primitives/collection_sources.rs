//! Crash, fence, duplicate and cancellation edges of a collection entered from a blob reference
//! (T014-016/T014-017, D420-D428), against real PostgreSQL. The entry is one transaction after one
//! read of the blob: whatever goes wrong between the two must leave nothing, and a retry must
//! enter once with the same membership.

use reqwest::Method;
use sagat_application::blobs::BoxFuture;
use sagat_application::collection_source::{
    CollectionItemSource, CollectionSourceError, CollectionSourceRequest, SourceItems,
};
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::workflow::NodeId;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::support::blob_collections::{
    Sched, aggregate, blob_source, collection_ir, platform, scheduler, scoped_worker, scopes,
    start_with_blob, status_and_code, upload,
};
use crate::support::worker::Platform;

async fn rows(pool: &PgPool, sql: &'static str, run: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(run)
        .fetch_one(pool)
        .await
        .expect("count")
}

async fn items_of(p: &Platform, run: Uuid) -> i64 {
    rows(
        &p.api.db.pool,
        "SELECT count(*) FROM collection_scope_items item JOIN iteration_scopes scope \
           ON scope.tenant_id = item.tenant_id AND scope.scope_id = item.scope_id \
         WHERE scope.run_id = $1",
        run,
    )
    .await
}

async fn reads_of(p: &Platform, blob: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'blob.read' AND resource_id = $1",
    )
    .bind(blob)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("audit count")
}

async fn started(p: &Platform, name: &str, bytes: &[u8]) -> (Uuid, String) {
    let workflow = p.publish(name, &collection_ir(name, "input", 10)).await;
    let blob = upload(p, bytes).await;
    let id = blob["id"].as_str().expect("id").to_owned();
    (start_with_blob(p, &workflow, &id).await, id)
}

#[tokio::test]
async fn a_failure_after_the_read_and_before_the_commit_leaves_nothing_and_the_retry_enters_once() {
    let p = platform().await;
    let (run, blob) = started(&p, "blob-fault-commit", br#"[{"v":1},{"v":2},{"v":3}]"#).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;

    sqlx::raw_sql(
        "CREATE FUNCTION reject_blob_entry() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected entry failure'; END $$; \
         CREATE TRIGGER reject_blob_entry BEFORE INSERT ON collection_scope_items \
         FOR EACH ROW EXECUTE FUNCTION reject_blob_entry()",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("install the fault");
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(scopes(&p, run).await, 0, "no scope");
    assert_eq!(items_of(&p, run).await, 0, "no item");
    assert_eq!(
        reads_of(&p, &blob).await,
        0,
        "no audit of a read that was not used"
    );
    let payloads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE convert_from(content, 'UTF8') LIKE '%collectionSource%'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("manifest payloads");
    assert_eq!(payloads, 0, "no manifest payload survived the rollback");
    assert_eq!(status_and_code(&p, run).await, ("RUNNING".to_owned(), None));

    sqlx::raw_sql(
        "DROP TRIGGER reject_blob_entry ON collection_scope_items; DROP FUNCTION reject_blob_entry()",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("remove the fault");
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(scopes(&p, run).await, 1);
    assert_eq!(items_of(&p, run).await, 3);
    assert_eq!(reads_of(&p, &blob).await, 1);

    let mut worker = scoped_worker(&p, "blob-fault-commit-worker", &["src.line"]).await;
    crate::support::blob_collections::run_to_the_end(&p, &scheduler, &mut worker, run).await;
    assert_eq!(aggregate(&p, run).await, json!([1, 2, 3]));
}

/// Takes the shard lease away after the blob has been read, so the entry reaches the store with a
/// stale fence.
struct StealsTheShardAfterReading {
    inner: Arc<dyn CollectionItemSource>,
    pool: PgPool,
}

impl CollectionItemSource for StealsTheShardAfterReading {
    fn materialize<'a>(
        &'a self,
        request: &'a CollectionSourceRequest,
    ) -> BoxFuture<'a, Result<SourceItems, CollectionSourceError>> {
        Box::pin(async move {
            let items = self.inner.materialize(request).await?;
            sqlx::query(
                "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second'",
            )
            .execute(&self.pool)
            .await
            .expect("expire the leases");
            Ok(items)
        })
    }
}

#[tokio::test]
async fn a_scheduler_that_loses_its_shard_after_reading_commits_nothing() {
    let p = platform().await;
    let (run, blob) = started(&p, "blob-fault-fence", br#"[{"v":1},{"v":2}]"#).await;
    let stealing = Arc::new(StealsTheShardAfterReading {
        inner: blob_source(&p),
        pool: p.api.db.pool.clone(),
    });
    let scheduler = scheduler(&p, Some(stealing)).await;

    let report = scheduler.tick().await;
    assert_eq!(report.fenced, 1, "{report:?}");
    assert_eq!(scopes(&p, run).await, 0);
    assert_eq!(items_of(&p, run).await, 0);
    assert_eq!(reads_of(&p, &blob).await, 0);
    assert_eq!(status_and_code(&p, run).await, ("RUNNING".to_owned(), None));
}

#[tokio::test]
async fn a_replica_without_a_current_lease_does_not_even_read_the_blob() {
    let p = platform().await;
    let (run, blob) = started(&p, "blob-fault-nolease", br#"[{"v":1}]"#).await;
    // A placement that never ran its keeper holds no shard.
    let scheduler = sagat_scheduler::Scheduler::new(
        p.api.executions.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_placement(sagat_scheduler::Placement::new(format!(
        "idle-{}",
        Uuid::now_v7()
    )))
    .with_item_source(blob_source(&p));
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("node"));
    let error = scheduler
        .advance_collection(p.tenant, RunId::new(run), &owner)
        .await
        .expect_err("no lease, no transition");
    assert!(
        matches!(
            error,
            sagat_scheduler::CollectionAdvanceError::FenceUnavailable
        ),
        "{error:?}"
    );
    assert_eq!(scopes(&p, run).await, 0);
    assert_eq!(reads_of(&p, &blob).await, 0);
}

#[tokio::test]
async fn two_attempts_to_enter_the_same_collection_leave_one_membership() {
    let p = platform().await;
    let (run, blob) = started(&p, "blob-fault-duplicate", br#"[{"v":1},{"v":2},{"v":3}]"#).await;
    let scheduler: Sched = scheduler(&p, Some(blob_source(&p))).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("node"));
    let (first, second) = tokio::join!(
        scheduler.advance_collection(p.tenant, RunId::new(run), &owner),
        scheduler.advance_collection(p.tenant, RunId::new(run), &owner),
    );
    assert!(
        first.is_ok() || second.is_ok(),
        "one of the two entered: {first:?} / {second:?}"
    );
    assert_eq!(scopes(&p, run).await, 1, "{first:?} / {second:?}");
    assert_eq!(items_of(&p, run).await, 3);
    assert_eq!(
        reads_of(&p, &blob).await,
        1,
        "only the entry that committed is audited"
    );
    // A third attempt, later, is a no-op on the same membership.
    let third = scheduler
        .advance_collection(p.tenant, RunId::new(run), &owner)
        .await;
    assert!(third.is_ok(), "{third:?}");
    assert_eq!(scopes(&p, run).await, 1);
    assert_eq!(items_of(&p, run).await, 3);
}

#[tokio::test]
async fn a_run_cancelled_before_entry_never_reads_the_blob() {
    let p = platform().await;
    let (run, blob) = started(&p, "blob-fault-cancel", br#"[{"v":1},{"v":2}]"#).await;
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "cancel before the collection starts"})),
        )
        .await;
    assert!(status == 200 || status == 202, "{status} {body}");
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    for _ in 0..4 {
        scheduler.tick().await;
    }
    assert_eq!(status_and_code(&p, run).await.0, "CANCELLED");
    assert_eq!(scopes(&p, run).await, 0);
    assert_eq!(reads_of(&p, &blob).await, 0);
}
