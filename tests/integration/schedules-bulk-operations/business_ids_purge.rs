//! The retention of business identifier bindings (`T018-034`, `FR-018-018`): a binding whose holder
//! closed and whose window passed, or whose holder is gone, is a row that holds nothing and is
//! purged; a binding whose holder is open, or closed inside its window, is never. The purge is one
//! bounded call of a function the runtime role may run, since the role may not delete from the
//! table, and housekeeping makes it. Real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::maintenance::MaintenanceStore;
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use uuid::Uuid;

use crate::business_id_lookup::lookup;
use crate::business_ids::{business, cancel, holder_of, key, start};
use crate::firing::{Fixture, fixture};

fn held(id: &str) -> sagat_application::runs::StartOptions {
    business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate)
}

/// A run under `id` that ended `minutes` ago, with a window of `window_s` seconds.
async fn ended(f: &Fixture, id: &str, minutes: i32, window_s: i32) {
    let started = start(f, held(id), &key()).await.expect("the start");
    cancel(f, started.run.info.run_id).await;
    sqlx::query("UPDATE business_ids SET reuse_window_s = $2 WHERE business_id = $1")
        .bind(id)
        .bind(window_s)
        .execute(&f.w.db.pool)
        .await
        .expect("a window");
    sqlx::query(
        "UPDATE runs SET finished_at = clock_timestamp() - $2 * interval '1 minute' WHERE id = $1",
    )
    .bind(started.run.info.run_id.as_uuid())
    .bind(minutes)
    .execute(&f.w.db.pool)
    .await
    .expect("a finish");
}

async fn bindings(f: &Fixture) -> Vec<String> {
    sqlx::query_scalar("SELECT business_id FROM business_ids ORDER BY business_id")
        .fetch_all(&f.w.db.pool)
        .await
        .expect("bindings")
}

async fn orphan(f: &Fixture, id: &str) {
    sqlx::query(
        "INSERT INTO business_ids \
         (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
         VALUES ($1, $2, $3, $4, $5, now(), 2592000)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(id)
    .bind(Uuid::now_v7())
    .bind(f.workflow.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("an orphan");
}

#[tokio::test]
async fn only_a_binding_that_holds_nothing_is_purged() {
    let f = fixture().await;
    // open, closed inside its window, closed past it, and a holder that is gone
    start(&f, held("open-1"), &key()).await.expect("open");
    ended(&f, "inside-1", 2, 3_600).await;
    ended(&f, "past-1", 5, 60).await;
    orphan(&f, "gone-1").await;

    let removed = f.w.store.purge_business_ids(100).await.expect("purge");

    assert_eq!(removed, 2);
    assert_eq!(bindings(&f).await, ["inside-1", "open-1"]);
    // what is kept still reads, and what went reads as free
    lookup(&f, "open-1").await.expect("still held");
    lookup(&f, "inside-1").await.expect("still remembered");
    assert_eq!(holder_of(&f, "past-1").await, None);
    assert_eq!(f.w.store.purge_business_ids(100).await.expect("again"), 0);
}

#[tokio::test]
async fn a_purge_is_bounded_and_goes_on_where_it_stopped() {
    let f = fixture().await;
    for n in 0..5 {
        orphan(&f, &format!("gone-{n}")).await;
    }

    assert_eq!(f.w.store.purge_business_ids(2).await.expect("a few"), 2);
    assert_eq!(bindings(&f).await.len(), 3);
    assert_eq!(f.w.store.purge_business_ids(2).await.expect("a few"), 2);
    assert_eq!(f.w.store.purge_business_ids(2).await.expect("the rest"), 1);
    assert!(bindings(&f).await.is_empty());
}

#[tokio::test]
async fn housekeeping_purges_the_bindings_that_are_free_again() {
    let f = fixture().await;
    start(&f, held("open-2"), &key()).await.expect("open");
    ended(&f, "past-2", 5, 60).await;
    orphan(&f, "gone-2").await;

    f.w.scheduler.housekeeping().await;

    assert_eq!(bindings(&f).await, ["open-2"]);
}

#[tokio::test]
async fn the_runtime_role_may_run_the_purge_and_may_not_delete_a_binding_itself() {
    let f = fixture().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app')")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("role");
    if !exists {
        return;
    }
    let (may_run, may_delete): (bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('sagat_app', 'business_ids_purge(integer)', 'EXECUTE'), \
                has_table_privilege('sagat_app', 'business_ids', 'DELETE')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("privileges");
    assert!(may_run, "the role runs the bounded purge");
    assert!(!may_delete, "and has no DELETE of its own on the table");
}
