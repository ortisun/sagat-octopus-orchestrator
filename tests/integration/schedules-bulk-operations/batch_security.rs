//! What a batch job can reach and what limits it (`T018-048`, `SC-018-010`, `FR-018-033`,
//! `FR-018-034`, `FR-018-041`, `FR-018-045`): the runs it selects are those of its own namespace
//! whatever ids it names, and a limit of zero admits no job. The role matrix of creation, the
//! revocation in the middle of a job, the ceilings of active jobs and of items, and the isolation of
//! reads are in `batch` and `batch_executor`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::runs::StartRun;
use sagat_domain::batch::{Action, JobState, Selector};
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::batch_executor::{create, due, executor, live, select, service, write};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{chain, ctx};

/// A run of the first tenant in the namespace `billing`, which is not the one the job is made in.
async fn run_in_another_namespace(f: &Fixture) -> Uuid {
    f.w.add_namespace("billing").await;
    let doc = chain("billed");
    let bytes = serde_json::to_vec(&doc).expect("json");
    let created =
        f.w.defs
            .create_workflow(
                &f.w.admin,
                "billing",
                "billed",
                &bytes,
                &format!("create-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("create workflow");
    let workflow = created.workflow.id();
    f.w.defs
        .publish(
            &f.w.admin,
            workflow,
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("publish");
    f.w.runs
        .start(
            &f.w.admin,
            &StartRun {
                namespace: "billing",
                workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &ctx(),
        )
        .await
        .expect("start the run")
        .info
        .run_id
        .as_uuid()
}

async fn status_in_the_database(f: &Fixture, run: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("status")
}

#[tokio::test]
async fn a_job_reaches_only_the_runs_of_its_own_namespace_whatever_ids_it_names() {
    let f = fixture().await;
    let exec = executor(&f);
    let own = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let elsewhere = run_in_another_namespace(&f).await;
    let foreign_workflow = f.w.publish_as_other_tenant(chain("foreign")).await;
    let foreign =
        f.w.start_as_other_tenant(foreign_workflow, &json!({}))
            .await
            .expect("a run of another tenant")
            .as_uuid();
    let selector = Selector::run_ids(vec![own, elsewhere, foreign]).expect("selector");

    let preview = service(&f)
        .preview(&f.w.admin, f.w.namespace.id(), &selector, None)
        .await
        .expect("preview");
    assert_eq!(
        (preview.matched, preview.sample.clone()),
        (1, vec![own]),
        "only the run of this namespace is counted"
    );

    live(&f);
    let job = create(
        &f,
        sagat_application::batch::BatchWrite {
            selector,
            ..write(
                Action::Cancel,
                Selector::filter(sagat_domain::batch::Filter::default()).expect("s"),
            )
        },
        "security-foreign-ids",
    )
    .await;
    select(&exec, &job).await;
    for _ in 0..50 {
        let now = service(&f).get(&f.w.admin, job.id).await.expect("job");
        if matches!(now.state, JobState::Completed) {
            break;
        }
        f.clock.advance(Duration::seconds(1));
        exec.step(&due(&job), None).await.expect("step");
    }

    let done = service(&f).get(&f.w.admin, job.id).await.expect("job");
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (done.counts.selected, done.counts.succeeded),
        (1, 1),
        "the other ids never became items, not even skipped ones that would tell they exist"
    );
    let items: Vec<Uuid> = sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1")
        .bind(job.id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("items");
    assert_eq!(items, vec![own]);
    assert_eq!(status_in_the_database(&f, own).await, "CANCELLED");
    for untouched in [elsewhere, foreign] {
        assert_ne!(
            status_in_the_database(&f, untouched).await,
            "CANCELLED",
            "a run of another namespace or tenant is not cancelled by this job"
        );
    }
}

#[tokio::test]
async fn a_limit_of_zero_admits_no_job_in_the_namespace_or_in_the_tenant() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', 0, 'admin')",
    )
    .bind(tenant)
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of zero for the namespace");

    let refused = service(&f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            write(
                Action::Cancel,
                Selector::run_ids(vec![Uuid::now_v7()]).expect("s"),
            ),
            "zero-namespace",
            &ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "{refused:?}"
    );

    sqlx::query("DELETE FROM quota_limits WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&f.w.db.pool)
        .await
        .expect("lift it");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_batch_jobs', 0, 'admin')",
    )
    .bind(tenant)
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of zero for the tenant");
    let refused = service(&f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            write(
                Action::Cancel,
                Selector::run_ids(vec![Uuid::now_v7()]).expect("s"),
            ),
            "zero-tenant",
            &ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "{refused:?}"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM batch_jobs WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(stored, 0, "a refused job leaves nothing behind");
}
