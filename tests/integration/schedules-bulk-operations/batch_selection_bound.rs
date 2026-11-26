//! The time bound of a batch job's selection (`FR-018-028`, `FR-018-029`, G2403, D2405): a selector of run ids takes
//! exactly the runs it names, whatever their recorded creation, and a filter selector takes the runs created at or before
//! the instant the job was created (`selection_bound_at`).
//!
//! A backward step of the database clock between a run's commit and the creation of a job makes the run's `created_at`
//! later than the job's bound. These tests make that state by moving `created_at` an hour away from the database's clock,
//! which no step of the clock reaches: the virtual machine a laptop runs PostgreSQL in steps it back by up to two seconds
//! (G2403), so no test here compares two readings of that clock that are closer than an hour.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::batch::{Action, JobState, Selector};
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::batch_executor::{
    create, everything, executor, items, job, live, select, service, settle, status_of, write,
};
use crate::batch_security::run_in_another_namespace;
use crate::firing::{Fixture, fixture};
use crate::support::engine::chain;

/// Moves the recorded creation of `runs` an hour after the database's clock: what a clock that stepped back between their
/// commit and a job's creation leaves, with a margin no step reaches.
async fn recorded_an_hour_ahead(f: &Fixture, runs: &[Uuid]) {
    sqlx::query(
        "UPDATE runs SET created_at = clock_timestamp() + interval '1 hour' WHERE id = ANY($1)",
    )
    .bind(runs)
    .execute(&f.w.db.pool)
    .await
    .expect("record the runs as created an hour ahead");
}

/// Moves the recorded creation of `runs` an hour back: inside the bound of any job created after it.
async fn recorded_an_hour_back(f: &Fixture, runs: &[Uuid]) {
    sqlx::query("UPDATE runs SET created_at = created_at - interval '1 hour' WHERE id = ANY($1)")
        .bind(runs)
        .execute(&f.w.db.pool)
        .await
        .expect("record the runs as created an hour back");
}

/// Applies the items of a job that has been selected until it has nothing more to do.
async fn run_to_the_end(
    f: &Fixture,
    exec: &crate::batch_executor::Executor,
    job_: &sagat_application::batch::JobRecord,
) {
    for _ in 0..4 {
        f.clock.advance(Duration::seconds(1));
        settle(exec, job_).await;
    }
}

async fn item_runs(f: &Fixture, id: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("items")
}

#[tokio::test]
async fn a_job_that_names_a_run_by_id_selects_it_whatever_the_database_clock_says_of_its_creation()
{
    let f = fixture().await;
    let run = f.w.start(f.workflow, &json!({})).await.as_uuid();
    recorded_an_hour_ahead(&f, &[run]).await;
    let selector = Selector::run_ids(vec![run]).expect("selector");

    let preview = service(&f)
        .preview(&f.w.admin, f.w.namespace.id(), &selector, None)
        .await
        .expect("preview");
    assert_eq!(
        (preview.matched, preview.sample.clone()),
        (1, vec![run]),
        "the preview of a list of ids counts the run it names"
    );

    live(&f);
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, selector), "named-run-clock-back").await;
    let created_at: time::OffsetDateTime =
        sqlx::query_scalar("SELECT created_at FROM runs WHERE id = $1")
            .bind(run)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the run's recorded creation");
    assert!(
        created_at > job_.selection_bound_at,
        "the state of G2403: the run is recorded as created after the job's bound"
    );

    select(&f, &exec, &job_).await;
    run_to_the_end(&f, &exec, &job_).await;

    let done = job(&f, job_.id).await;
    assert_eq!(
        (done.state, done.counts.selected, done.counts.succeeded),
        (JobState::Completed, 1, 1),
        "the job selects the run it names, not nothing"
    );
    assert_eq!(item_runs(&f, job_.id).await, vec![run]);
    assert_eq!(status_of(&f, run).await, "CANCELLED");
}

#[tokio::test]
async fn a_filter_selector_still_leaves_out_a_run_recorded_after_its_bound() {
    let f = fixture().await;
    let inside = f.w.start(f.workflow, &json!({})).await.as_uuid();
    recorded_an_hour_back(&f, &[inside]).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, everything()), "filter-bound").await;
    // a run committed after the job was created, recorded an hour ahead so that no step of the clock brings it inside
    let late = f.w.start(f.workflow, &json!({})).await.as_uuid();
    recorded_an_hour_ahead(&f, &[late]).await;

    let preview = service(&f)
        .preview(&f.w.admin, f.w.namespace.id(), &everything(), None)
        .await
        .expect("preview");
    assert_eq!(
        (preview.matched, preview.sample.clone()),
        (1, vec![inside]),
        "the preview of a filter is bounded by the instant it reads"
    );

    select(&f, &exec, &job_).await;
    run_to_the_end(&f, &exec, &job_).await;

    let done = job(&f, job_.id).await;
    assert_eq!(
        (done.state, done.counts.selected, done.counts.succeeded),
        (JobState::Completed, 1, 1)
    );
    assert_eq!(
        item_runs(&f, job_.id).await,
        vec![inside],
        "a filter takes only the runs created at or before the job's bound"
    );
    assert_eq!(status_of(&f, inside).await, "CANCELLED");
    assert_eq!(
        status_of(&f, late).await,
        "RUNNING",
        "the run after the bound is not touched"
    );
}

#[tokio::test]
async fn an_id_selector_still_refuses_the_runs_of_another_namespace_or_tenant_whatever_their_creation()
 {
    let f = fixture().await;
    let own = f.w.start(f.workflow, &json!({})).await.as_uuid();
    let elsewhere = run_in_another_namespace(&f).await;
    let foreign_workflow = f.w.publish_as_other_tenant(chain("foreign")).await;
    let foreign =
        f.w.start_as_other_tenant(foreign_workflow, &json!({}))
            .await
            .expect("a run of another tenant")
            .as_uuid();
    // the run of this namespace is recorded after any bound and the others inside every bound: what refuses the others is
    // their tenant and their namespace, not the time
    recorded_an_hour_ahead(&f, &[own]).await;
    recorded_an_hour_back(&f, &[elsewhere, foreign]).await;
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
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, selector), "named-runs-elsewhere").await;
    select(&f, &exec, &job_).await;
    run_to_the_end(&f, &exec, &job_).await;

    let done = job(&f, job_.id).await;
    assert_eq!(
        (done.state, done.counts.selected, done.counts.succeeded),
        (JobState::Completed, 1, 1),
        "the other ids never became items, not even skipped ones that would tell they exist"
    );
    assert_eq!(item_runs(&f, job_.id).await, vec![own]);
    assert_eq!(items(&f, job_.id).await.len(), 1);
    assert_eq!(status_of(&f, own).await, "CANCELLED");
    for untouched in [elsewhere, foreign] {
        assert_ne!(
            status_of(&f, untouched).await,
            "CANCELLED",
            "a run of another namespace or tenant is not cancelled by this job"
        );
    }
}
