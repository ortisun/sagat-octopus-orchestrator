//! The lookup of a business identifier (`T018-034`, `FR-018-022`): who holds it, whether that
//! execution is still open, how it ended, and until when a closed holder is remembered. Free,
//! unbound and purged are all the same answer: not found. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::runs::BusinessBinding;
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy};
use sagat_domain::execution::RunStatus;
use sagat_domain::identity::{NamespaceId, Principal, Role, TenantId};
use time::Duration;
use uuid::Uuid;

use crate::business_ids::{business, cancel, key, start};
use crate::firing::{Fixture, fixture};
use crate::support::engine::subject_of;

pub(crate) async fn lookup(f: &Fixture, id: &str) -> Result<BusinessBinding, AppError> {
    f.w.runs
        .business_binding(
            &f.w.admin,
            f.w.namespace.id(),
            &BusinessId::new(id).expect("an identifier"),
        )
        .await
}

fn held(id: &str) -> sagat_application::runs::StartOptions {
    business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate)
}

#[tokio::test]
async fn an_open_holder_is_read_with_the_run_that_carries_it() {
    let f = fixture().await;
    let started = start(&f, held("look-1"), &key()).await.expect("the start");
    let run = started.run.info.run_id;

    let binding = lookup(&f, "look-1").await.expect("the binding");

    assert_eq!(binding.business_id.as_str(), "look-1");
    assert_eq!(binding.execution_id, run);
    assert_eq!(binding.current_run_id, run);
    assert!(binding.open);
    assert_eq!(binding.last_status, None);
    assert_eq!(binding.reusable_after, None);
    assert_eq!(binding.workflow_id, f.workflow);
    let age = time::OffsetDateTime::now_utc() - binding.bound_at;
    assert!(age < Duration::minutes(5), "bound just now: {age}");
}

#[tokio::test]
async fn a_closed_holder_says_how_it_ended_and_until_when_it_is_remembered() {
    let f = fixture().await;
    let started = start(&f, held("look-2"), &key()).await.expect("the start");
    let run = started.run.info.run_id;
    cancel(&f, run).await;
    let finished: time::OffsetDateTime =
        sqlx::query_scalar("SELECT finished_at FROM runs WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("when it ended");

    let binding = lookup(&f, "look-2").await.expect("remembered");

    assert!(!binding.open);
    assert_eq!(binding.last_status, Some(RunStatus::Cancelled));
    assert_eq!(binding.execution_id, run);
    assert_eq!(
        binding.reusable_after,
        Some(finished + Duration::days(30)),
        "the default window is 30 days from the end"
    );
}

#[tokio::test]
async fn an_identifier_that_has_been_free_for_longer_than_its_window_is_not_found() {
    let f = fixture().await;
    let started = start(&f, held("look-3"), &key()).await.expect("the start");
    cancel(&f, started.run.info.run_id).await;
    lookup(&f, "look-3").await.expect("remembered at first");

    sqlx::query("UPDATE business_ids SET reuse_window_s = 60 WHERE business_id = 'look-3'")
        .execute(&f.w.db.pool)
        .await
        .expect("a short window");
    sqlx::query(
        "UPDATE runs SET finished_at = clock_timestamp() - interval '2 minutes' WHERE id = $1",
    )
    .bind(started.run.info.run_id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("an old finish");

    assert_eq!(lookup(&f, "look-3").await.err(), Some(AppError::NotFound));
}

#[tokio::test]
async fn an_unbound_identifier_a_gone_holder_and_another_namespace_are_not_found() {
    let f = fixture().await;
    assert_eq!(
        lookup(&f, "never-bound").await.err(),
        Some(AppError::NotFound)
    );

    // a binding whose holder was purged by a retention policy holds nothing
    sqlx::query(
        "INSERT INTO business_ids \
         (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
         VALUES ($1, $2, 'orphan-look', $3, $4, now(), 2592000)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(Uuid::now_v7())
    .bind(f.workflow.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("an orphan");
    assert_eq!(
        lookup(&f, "orphan-look").await.err(),
        Some(AppError::NotFound)
    );

    // the same identifier is another one in another namespace
    start(&f, held("look-4"), &key()).await.expect("the start");
    let elsewhere =
        f.w.runs
            .business_binding(
                &f.w.admin,
                NamespaceId::generate(),
                &BusinessId::new("look-4").expect("an identifier"),
            )
            .await;
    assert_eq!(elsewhere.err(), Some(AppError::NotFound));
}

#[tokio::test]
async fn a_viewer_reads_a_binding_as_it_reads_a_run() {
    let f = fixture().await;
    start(&f, held("look-5"), &key()).await.expect("the start");
    let viewer = f.w.member("reader", Role::Viewer).await;
    let id = BusinessId::new("look-5").expect("an identifier");

    let read =
        f.w.runs
            .business_binding(&viewer, f.w.namespace.id(), &id)
            .await;

    assert!(read.is_ok(), "a viewer reads runs: {read:?}");
}

#[tokio::test]
async fn the_binding_of_a_tenant_is_not_read_by_another_even_with_its_namespace_id() {
    let f = fixture().await;
    start(&f, held("look-6"), &key()).await.expect("the start");
    let stranger = Principal::new(TenantId::generate(), subject_of("mallory"), Role::Admin);

    let read =
        f.w.runs
            .business_binding(
                &stranger,
                f.w.namespace.id(),
                &BusinessId::new("look-6").expect("an identifier"),
            )
            .await;

    assert_eq!(read.err(), Some(AppError::NotFound));
}
