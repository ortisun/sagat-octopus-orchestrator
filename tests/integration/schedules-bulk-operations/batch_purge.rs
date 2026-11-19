//! The retention of batch jobs and their items (`T018-051`, the purge half; `G2317`, `FR-018-033`): a job that has
//! finished and whose retention has passed goes with its items, in bounded calls, the items first and the job last; a
//! job that has not finished, or is inside its window, never goes. The runtime role may run the purge and may not
//! delete a row itself. Real PostgreSQL, both database roles.
//!
//! Jobs are written by SQL with the ages the case needs (the guard of the table refuses to move a job's own times, and
//! the database keeps the clock the window is read against, which a test does not move). Where a case needs a job the
//! platform made (its key, its audit record, its selector as stored), the job is made by the service and then aged with
//! the guard disabled by the owner, said where it is.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::batch::{JobRecord, MIN_RETENTION, NamedPayload};
use sagat_application::error::AppError;
use sagat_application::identity::{PageRequest, RequestContext};
use sagat_application::maintenance::{BatchPurged, MaintenanceStore};
use sagat_domain::batch::{Action, Filter, JobState, Selector};
use sagat_domain::quota::QuotaResource;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sagat_scheduler::{BATCH_PURGE_JOBS, BATCH_PURGE_ROWS};
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use sqlx::Connection as _;
use uuid::Uuid;

use crate::batch_executor::{create, everything, service, write};
use crate::counted::{recorded, series};
use crate::firing::{Fixture, fixture};
use crate::migration::undo_to;
use crate::pending_resets_migration::code;
use crate::support::bounded::within;
use crate::support::logs::capture_with_spans;

/// Hours in a day: the ages below are written in hours so that a window can end an hour ago or in an hour.
const DAY: i32 = 24;
const INSUFFICIENT_PRIVILEGE: &str = "42501";
const RESTRICT_VIOLATION: &str = "23001";

/// A job as written by SQL. Defaults to a job that finished long ago and whose window ended ten days ago.
#[derive(Clone)]
struct Job {
    tenant: Uuid,
    namespace: Uuid,
    id: Uuid,
    state: &'static str,
    /// Hours since the job was created.
    created_ago: i32,
    /// Hours until the window of the job ends (`expires_at`); negative when it has ended.
    expires_in: i32,
    /// Hours since the job ended; none for a job that has not.
    finished_ago: Option<i32>,
    items: i32,
    selector: Value,
    reason: String,
}

fn job(f: &Fixture) -> Job {
    Job {
        tenant: f.w.admin.tenant_id().as_uuid(),
        namespace: f.w.namespace.id().as_uuid(),
        id: Uuid::new_v4(),
        state: "COMPLETED",
        created_ago: 40 * DAY,
        expires_in: -10 * DAY,
        finished_ago: Some(39 * DAY),
        items: 3,
        selector: json!({"filter": {}}),
        reason: "stop the runs that are stuck".to_owned(),
    }
}

/// A job that has not finished: however old, it has no end.
fn unfinished(f: &Fixture, state: &'static str) -> Job {
    Job {
        state,
        created_ago: 400 * DAY,
        expires_in: -370 * DAY,
        finished_ago: None,
        ..job(f)
    }
}

async fn seed(f: &Fixture, job: &Job) -> Uuid {
    seed_on(&f.w.db.pool, job).await
}

async fn seed_on(pool: &sqlx::PgPool, job: &Job) -> Uuid {
    let failure = (job.state == "FAILED").then_some("AUTHORITY_REVOKED");
    sqlx::query(
        "INSERT INTO batch_jobs \
         (tenant_id, namespace_id, id, action, selector, reason, created_by, state, failure_reason, max_items, \
          rate_milli, selected, selection_bound_at, selection_horizon, next_slot_at, created_at, finished_at, expires_at) \
         VALUES ($1, $2, $3, 'CANCEL', $4, $5, 'alice', $6, $7, 100000, 20000, $8, \
                 clock_timestamp() - make_interval(hours => $9), pg_current_xact_id(), clock_timestamp(), \
                 clock_timestamp() - make_interval(hours => $9), \
                 CASE WHEN $10::int IS NULL THEN NULL ELSE clock_timestamp() - make_interval(hours => $10) END, \
                 clock_timestamp() + make_interval(hours => $11))",
    )
    .bind(job.tenant)
    .bind(job.namespace)
    .bind(job.id)
    .bind(&job.selector)
    .bind(&job.reason)
    .bind(job.state)
    .bind(failure)
    .bind(job.items)
    .bind(job.created_ago)
    .bind(job.finished_ago)
    .bind(job.expires_in)
    .execute(pool)
    .await
    .expect("seed the job");
    sqlx::query(
        "INSERT INTO batch_items (tenant_id, job_id, ordinal, run_id, state, updated_at) \
         SELECT $1, $2, g, gen_random_uuid(), 'SUCCEEDED', clock_timestamp() FROM generate_series(1, $3) g",
    )
    .bind(job.tenant)
    .bind(job.id)
    .bind(job.items)
    .execute(pool)
    .await
    .expect("seed the items");
    job.id
}

/// `n` jobs that ended and whose window ended, with no item, in one statement.
async fn seed_empty_jobs(f: &Fixture, n: i32) {
    sqlx::query(
        "INSERT INTO batch_jobs \
         (tenant_id, namespace_id, id, action, selector, reason, created_by, state, max_items, rate_milli, selected, \
          selection_bound_at, selection_horizon, next_slot_at, created_at, finished_at, expires_at) \
         SELECT $1, $2, gen_random_uuid(), 'CANCEL', '{\"filter\": {}}', 'stop the runs that are stuck', 'alice', 'COMPLETED', \
                100000, 20000, 0, clock_timestamp() - interval '40 days', pg_current_xact_id(), clock_timestamp(), \
                clock_timestamp() - interval '40 days', clock_timestamp() - interval '39 days', clock_timestamp() - interval '10 days' \
         FROM generate_series(1, $3)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the empty jobs");
}

/// The content the tenant of the fixture is charged for, in bytes.
async fn content_usage(f: &Fixture) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT coalesce(content_bytes, 0)::bigint FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .fetch_optional(&f.w.db.pool)
    .await
    .expect("usage")
    .unwrap_or(0)
}

/// `(jobs, items)` in the tables.
async fn rows(f: &Fixture) -> (i64, i64) {
    let jobs = sqlx::query_scalar("SELECT count(*) FROM batch_jobs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("jobs");
    let items = sqlx::query_scalar("SELECT count(*) FROM batch_items")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("items");
    (jobs, items)
}

async fn items_of(f: &Fixture, tenant: Uuid, id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM batch_items WHERE tenant_id = $1 AND job_id = $2")
        .bind(tenant)
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("items of the job")
}

async fn state_of(f: &Fixture, tenant: Uuid, id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT state FROM batch_jobs WHERE tenant_id = $1 AND id = $2")
        .bind(tenant)
        .bind(id)
        .fetch_optional(&f.w.db.pool)
        .await
        .expect("state of the job")
}

async fn job_ids(f: &Fixture) -> BTreeSet<Uuid> {
    sqlx::query_scalar("SELECT id FROM batch_jobs")
        .fetch_all(&f.w.db.pool)
        .await
        .expect("ids")
        .into_iter()
        .collect()
}

/// No item whose job is gone (the foreign key says so; the purge must not be the one case that finds a way round it).
async fn no_orphan_item(f: &Fixture) {
    let orphans: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM batch_items i WHERE NOT EXISTS \
         (SELECT 1 FROM batch_jobs j WHERE j.tenant_id = i.tenant_id AND j.id = i.job_id)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("orphans");
    assert_eq!(orphans, 0, "an item without its job");
}

async fn purge(f: &Fixture, jobs: u32, rows: u32) -> BatchPurged {
    f.w.store.purge_batch_jobs(jobs, rows).await.expect("purge")
}

fn gone(jobs: u64, items: u64) -> BatchPurged {
    BatchPurged { jobs, items }
}

/// A tenant of its own with `n` namespaces; returns the tenant and the namespaces.
async fn tenant_with_namespaces(f: &Fixture, n: usize) -> (Uuid, Vec<Uuid>) {
    let (tenant, workspace) = (Uuid::new_v4(), Uuid::new_v4());
    let short = tenant.simple().to_string()[..12].to_owned();
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())")
        .bind(tenant)
        .bind(format!("tn-{short}"))
        .execute(&f.w.db.pool)
        .await
        .expect("tenant");
    sqlx::query(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES ($1, $2, 'main', now())",
    )
    .bind(workspace)
    .bind(tenant)
    .execute(&f.w.db.pool)
    .await
    .expect("workspace");
    let mut namespaces = Vec::new();
    for slug in 0..n {
        namespaces.push(extra_namespace(f, tenant, workspace, &format!("ns-{slug}")).await);
    }
    (tenant, namespaces)
}

async fn extra_namespace(f: &Fixture, tenant: Uuid, workspace: Uuid, slug: &str) -> Uuid {
    let namespace = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES ($1, $2, $3, $4, 'prod', now())",
    )
    .bind(namespace)
    .bind(tenant)
    .bind(workspace)
    .bind(slug)
    .execute(&f.w.db.pool)
    .await
    .expect("namespace");
    namespace
}

// --- what goes and what stays -------------------------------------------------------------------------

/// Catches: a purge that leaves the items (an orphan cannot exist: the foreign key refuses the job's delete), one that stops
/// at the state `COMPLETED`, and a count that is not what went.
#[tokio::test]
async fn a_finished_job_past_its_retention_goes_with_its_items_in_each_of_the_three_terminal_states()
 {
    let f = fixture().await;
    for state in ["COMPLETED", "CANCELLED", "FAILED"] {
        seed(&f, &Job { state, ..job(&f) }).await;
    }
    assert_eq!(rows(&f).await, (3, 9));

    assert_eq!(purge(&f, 100, 10_000).await, gone(3, 9));

    assert_eq!(rows(&f).await, (0, 0));
    assert_eq!(
        purge(&f, 100, 10_000).await,
        BatchPurged::default(),
        "and a second call finds nothing"
    );
}

/// Catches: the state predicate dropped from the function (the guard of the table would then refuse the delete and the call
/// would fail, so the test would see an error and not a count), a state left out of the three that are terminal, and a job
/// whose age is mistaken for its end.
#[tokio::test]
async fn a_job_that_has_not_finished_is_never_purged_whatever_its_age() {
    let f = fixture().await;
    let mut kept = Vec::new();
    for state in ["SELECTING", "RUNNING", "CANCELLING"] {
        kept.push(seed(&f, &unfinished(&f, state)).await);
    }

    assert_eq!(purge(&f, 100, 10_000).await, BatchPurged::default());

    assert_eq!(rows(&f).await, (3, 9), "every job and every item is there");
    // and the guard of the table still refuses the owner the delete of one that has not finished
    for id in kept {
        let refused = sqlx::query("DELETE FROM batch_jobs WHERE id = $1")
            .bind(id)
            .execute(&f.w.db.pool)
            .await;
        assert_eq!(code(refused), RESTRICT_VIOLATION);
    }
}

/// The window ends at `expires_at`, the instant the job was told it was kept until. Catches: a window read a day early or late
/// (a job that expires in an hour goes, or one that expired an hour ago stays), and one read against `created_at`.
#[tokio::test]
async fn a_job_inside_its_window_is_kept_with_every_item_and_one_past_it_by_an_hour_goes() {
    let f = fixture().await;
    let inside = seed(
        &f,
        &Job {
            created_ago: 30 * DAY,
            expires_in: 1,
            finished_ago: Some(20 * DAY),
            ..job(&f)
        },
    )
    .await;
    let past = seed(
        &f,
        &Job {
            created_ago: 30 * DAY + 1,
            expires_in: -1,
            finished_ago: Some(20 * DAY),
            ..job(&f)
        },
    )
    .await;

    assert_eq!(purge(&f, 100, 10_000).await, gone(1, 3));

    assert_eq!(job_ids(&f).await, BTreeSet::from([inside]));
    assert_eq!(
        items_of(&f, f.w.admin.tenant_id().as_uuid(), inside).await,
        3,
        "all of its items"
    );
    assert_eq!(
        state_of(&f, f.w.admin.tenant_id().as_uuid(), past).await,
        None
    );
}

/// A job that ends after its own window (it waited for an engine, or ran long) is kept for the least retention after it ended,
/// and the number is the one `MIN_RETENTION` names. Catches: a floor that is not in the function, one that is not the constant's,
/// and one off by an hour either way.
#[tokio::test]
async fn a_job_that_ended_after_its_window_is_kept_for_the_least_retention_after_it_ended() {
    let f = fixture().await;
    let floor = i32::try_from(MIN_RETENTION.whole_hours()).expect("hours");
    let late = seed(
        &f,
        &Job {
            created_ago: 40 * DAY,
            expires_in: -10 * DAY,
            finished_ago: Some(1),
            ..job(&f)
        },
    )
    .await;
    let just_inside = seed(
        &f,
        &Job {
            finished_ago: Some(floor - 1),
            ..job(&f)
        },
    )
    .await;
    let just_past = seed(
        &f,
        &Job {
            finished_ago: Some(floor + 1),
            ..job(&f)
        },
    )
    .await;

    assert_eq!(purge(&f, 100, 10_000).await, gone(1, 3));

    assert_eq!(job_ids(&f).await, BTreeSet::from([late, just_inside]));
    assert_eq!(
        state_of(&f, f.w.admin.tenant_id().as_uuid(), just_past).await,
        None
    );
}

// --- bounded, and going on where it stopped --------------------------------------------------------------

/// A job of 5,000 items with a call that may remove 1,200 rows: five calls, none over the bound, the job there and finished
/// until the last one takes it. Catches: a bound that is not kept (the whole job in one call), a job removed before its items,
/// the job row removed by a call that had no row left, and a call that does not go on where the last one stopped.
#[tokio::test]
async fn a_job_of_five_thousand_items_is_purged_over_several_calls_and_ends_fully() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    let id = seed(
        &f,
        &Job {
            items: 5_000,
            ..job(&f)
        },
    )
    .await;

    let mut calls = Vec::new();
    // bounded: a purge that never ends a job (a mutant that does not) must fail the test, not keep it calling for ever
    for _ in 0..10 {
        let one = purge(&f, 10, 1_200).await;
        if one.rows() == 0 {
            break;
        }
        assert!(
            one.rows() <= 1_200,
            "a call never removes more than it is given: {one:?}"
        );
        calls.push(one);
        let removed: u64 = calls.iter().map(|c| c.items).sum();
        if calls.len() < 5 {
            assert_eq!(
                items_of(&f, tenant, id).await,
                5_000 - i64::try_from(removed).expect("count"),
                "the items it has left"
            );
            assert_eq!(
                state_of(&f, tenant, id).await.as_deref(),
                Some("COMPLETED"),
                "a job that was cut short keeps its end"
            );
            no_orphan_item(&f).await;
        }
    }

    assert_eq!(
        calls,
        vec![
            gone(0, 1_200),
            gone(0, 1_200),
            gone(0, 1_200),
            gone(0, 1_200),
            gone(1, 200)
        ]
    );
    assert_eq!(rows(&f).await, (0, 0));
}

/// The job row needs a row of the budget of its own. Catches: a job removed by a call that had spent its budget on items, and a
/// job left for another call when there was room for it.
#[tokio::test]
async fn the_job_goes_only_when_the_budget_has_a_row_left_for_it() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    let exact = seed(
        &f,
        &Job {
            items: 4,
            ..job(&f)
        },
    )
    .await;

    assert_eq!(
        purge(&f, 10, 4).await,
        gone(0, 4),
        "the budget went on the items"
    );
    assert_eq!(items_of(&f, tenant, exact).await, 0);
    assert_eq!(
        state_of(&f, tenant, exact).await.as_deref(),
        Some("COMPLETED"),
        "a job with no items left, still there"
    );
    assert_eq!(
        purge(&f, 10, 4).await,
        gone(1, 0),
        "the next call takes the job"
    );

    seed(
        &f,
        &Job {
            items: 4,
            ..job(&f)
        },
    )
    .await;
    assert_eq!(
        purge(&f, 10, 5).await,
        gone(1, 4),
        "four items and the job in five rows"
    );
    assert_eq!(rows(&f).await, (0, 0));
}

/// Catches: a bound on jobs that is not kept (each job holds a row lock until the call ends).
#[tokio::test]
async fn a_call_removes_at_most_the_jobs_it_is_given() {
    let f = fixture().await;
    for _ in 0..5 {
        seed(
            &f,
            &Job {
                items: 0,
                ..job(&f)
            },
        )
        .await;
    }

    assert_eq!(purge(&f, 2, 1_000).await, gone(2, 0));
    assert_eq!(purge(&f, 2, 1_000).await, gone(2, 0));
    assert_eq!(purge(&f, 2, 1_000).await, gone(1, 0));
    assert_eq!(rows(&f).await, (0, 0));
}

/// The function is the security boundary, so it holds its own ceilings whatever it is asked: 1,000 jobs and 100,000 rows in one call
/// (the adapter's `i32::MAX` is only the largest the type holds). A caller that asks for everything, as the runtime role can by calling
/// the function itself, gets the ceiling and no more. Catches: a function that takes the bound it is given (the first removes all
/// 1,100 jobs, the second the job too).
#[tokio::test]
async fn a_call_that_asks_for_more_than_the_function_allows_gets_its_ceilings_and_no_more() {
    let f = fixture().await;
    seed_empty_jobs(&f, 1_100).await;

    assert_eq!(
        purge(&f, u32::MAX, u32::MAX).await,
        gone(1_000, 0),
        "1,000 jobs"
    );
    assert_eq!(rows(&f).await, (100, 0));
    assert_eq!(purge(&f, u32::MAX, u32::MAX).await, gone(100, 0));
}

/// The ceiling on rows: a job of exactly 100,000 items (the most a job holds) is emptied by a call that asks for everything and its row
/// stays, because the call has no row of the ceiling left for it; the next call takes it. Catches: no ceiling on rows (the call
/// removes the 100,000 items and the job).
#[tokio::test]
async fn a_call_that_asks_for_everything_removes_at_most_a_hundred_thousand_rows() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    let id = seed(
        &f,
        &Job {
            items: 100_000,
            ..job(&f)
        },
    )
    .await;

    assert_eq!(purge(&f, u32::MAX, u32::MAX).await, gone(0, 100_000));
    assert_eq!(state_of(&f, tenant, id).await.as_deref(), Some("COMPLETED"));
    assert_eq!(purge(&f, u32::MAX, u32::MAX).await, gone(1, 0));
}

/// The time of a call is linear in its rows. A statement that joins the chunk to the items of the job by a nested loop is quadratic: 5,000
/// rows of one job took 3.2 s on this machine under load, which is most of the engine's 5 s statement timeout, and a call that times out
/// rolls back and is made again identically. 20,000 rows of one job are removed here within 20 s (about half a second when linear; about
/// fifty seconds when quadratic). Catches: a delete of the chunk that is not a range of the job's index.
#[tokio::test]
async fn a_call_over_one_large_job_takes_time_in_proportion_to_its_rows() {
    let f = fixture().await;
    seed(
        &f,
        &Job {
            items: 20_000,
            ..job(&f)
        },
    )
    .await;

    let started = std::time::Instant::now();
    let one = purge(&f, 1, 20_000).await;
    let took = started.elapsed();

    assert_eq!(one, gone(0, 20_000));
    assert!(
        took < StdDuration::from_secs(20),
        "20,000 item rows of one job took {took:?}"
    );
}

/// The oldest window first, and the job a call cut short is finished before the next is begun, so the order of the work is a
/// queue and nothing waits behind a job that never ends. The three jobs are written so that the orders a mutant could take all
/// disagree with the window's: by `created_at` and by `finished_at` the first is the job with the *middle* window, by the id it is the
/// middle one and (descending) the second is the newest, and by the window reversed it is the newest. Catches: an order by any of
/// them, a call that leaves a job it began for one it did not, and a budget that is not carried from one job to the next within a call.
#[tokio::test]
async fn jobs_go_oldest_window_first_and_a_job_cut_short_is_finished_before_the_next_is_begun() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    // (id, created, expired, ended), in days ago: the windows end in the order oldest, middle, newest; the creations in the order
    // middle, newest, oldest; the ends in the order middle, newest, oldest; the ids in the order middle, newest, oldest
    let mut ids = Vec::new();
    for (id, created, expired, ended) in [(3_u128, 45, 30, 20), (1, 60, 20, 55), (2, 50, 15, 30)] {
        ids.push(
            seed(
                &f,
                &Job {
                    id: Uuid::from_u128(id),
                    created_ago: created * DAY,
                    expires_in: -expired * DAY,
                    finished_ago: Some(ended * DAY),
                    items: 10,
                    ..job(&f)
                },
            )
            .await,
        );
    }
    let (oldest, middle, newest) = (ids[0], ids[1], ids[2]);

    assert_eq!(purge(&f, 10, 6).await, gone(0, 6));
    assert_eq!(
        items_of(&f, tenant, oldest).await,
        4,
        "the oldest window is first"
    );
    assert_eq!(items_of(&f, tenant, middle).await, 10);

    assert_eq!(
        purge(&f, 10, 6).await,
        gone(1, 5),
        "the last four items and the job, and the one row left goes to the next job"
    );
    assert_eq!(state_of(&f, tenant, oldest).await, None);
    assert_eq!(items_of(&f, tenant, middle).await, 9);
    assert_eq!(items_of(&f, tenant, newest).await, 10);

    assert_eq!(purge(&f, 10, 100).await, gone(2, 19));
    assert_eq!(rows(&f).await, (0, 0));
}

// --- tenants and namespaces ----------------------------------------------------------------------------

/// Two tenants with two namespaces each, and in each a job past its retention, one inside its window and one that is running.
/// Only the four that are over go. A job id may be the same in two tenants (the key is the tenant's and the id): the items of
/// the one that goes are its own. Catches: a purge that reads one tenant or namespace only, one that joins the items by the id of the
/// job alone, and one that lets a state or a window of another tenant decide.
#[tokio::test]
async fn only_the_old_finished_jobs_of_every_tenant_and_namespace_go_and_never_the_items_of_the_same_id_elsewhere()
 {
    let f = fixture().await;
    let first = (
        f.w.admin.tenant_id().as_uuid(),
        f.w.namespace.id().as_uuid(),
    );
    let workspace: Uuid = sqlx::query_scalar("SELECT workspace_id FROM namespaces WHERE id = $1")
        .bind(first.1)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("workspace");
    let first_b = extra_namespace(&f, first.0, workspace, "second-namespace").await;
    let (second, second_namespaces) = tenant_with_namespaces(&f, 2).await;

    let mut over = BTreeSet::new();
    let mut staying = BTreeSet::new();
    for (tenant, namespace) in [
        (first.0, first.1),
        (first.0, first_b),
        (second, second_namespaces[0]),
        (second, second_namespaces[1]),
    ] {
        let base = Job {
            tenant,
            namespace,
            ..job(&f)
        };
        over.insert(seed(&f, &base).await);
        staying.insert(
            seed(
                &f,
                &Job {
                    id: Uuid::new_v4(),
                    created_ago: 5 * DAY,
                    expires_in: 25 * DAY,
                    finished_ago: Some(4 * DAY),
                    ..base.clone()
                },
            )
            .await,
        );
        staying.insert(
            seed(
                &f,
                &Job {
                    id: Uuid::new_v4(),
                    ..Job {
                        state: "RUNNING",
                        finished_ago: None,
                        ..base
                    }
                },
            )
            .await,
        );
    }
    // the same id in two tenants: the one of the first is over, the one of the second is running
    let shared = Uuid::new_v4();
    seed(
        &f,
        &Job {
            id: shared,
            items: 3,
            ..job(&f)
        },
    )
    .await;
    seed(
        &f,
        &Job {
            tenant: second,
            namespace: second_namespaces[0],
            id: shared,
            items: 5,
            ..unfinished(&f, "RUNNING")
        },
    )
    .await;
    assert_eq!(rows(&f).await, (4 * 3 + 2, 4 * 3 * 3 + 8));

    assert_eq!(purge(&f, 100, 10_000).await, gone(5, 15));

    let left: BTreeSet<Uuid> = job_ids(&f).await;
    assert_eq!(left, staying.iter().copied().chain([shared]).collect());
    assert!(over.is_disjoint(&left));
    assert_eq!(
        items_of(&f, second, shared).await,
        5,
        "the items of the job of the same id in the other tenant are untouched"
    );
    assert_eq!(
        state_of(&f, second, shared).await.as_deref(),
        Some("RUNNING")
    );
    assert_eq!(state_of(&f, first.0, shared).await, None);
    no_orphan_item(&f).await;
}

/// A tenant that has a job of the same id: the chunk of items a call takes is the items of its own job, so a budget that cuts the job
/// short cuts it where it said. Catches: a chunk read by the id of the job alone (it lists the ordinals of both tenants, twice each,
/// so a chunk of three removes two, the call takes that for the end of the items and deletes the job, which the foreign key refuses).
#[tokio::test]
async fn the_chunk_of_a_job_is_its_own_items_when_another_tenant_has_a_job_of_the_same_id() {
    let f = fixture().await;
    let (second, namespaces) = tenant_with_namespaces(&f, 1).await;
    let shared = Uuid::new_v4();
    let tenant = f.w.admin.tenant_id().as_uuid();
    seed(
        &f,
        &Job {
            id: shared,
            items: 4,
            ..job(&f)
        },
    )
    .await;
    seed(
        &f,
        &Job {
            tenant: second,
            namespace: namespaces[0],
            id: shared,
            items: 4,
            ..unfinished(&f, "RUNNING")
        },
    )
    .await;

    assert_eq!(purge(&f, 10, 3).await, gone(0, 3));
    assert_eq!(items_of(&f, tenant, shared).await, 1);
    assert_eq!(purge(&f, 10, 3).await, gone(1, 1));

    assert_eq!(state_of(&f, tenant, shared).await, None);
    assert_eq!(
        items_of(&f, second, shared).await,
        4,
        "the other tenant's job is whole"
    );
    no_orphan_item(&f).await;
}

// --- the role, the locks and the order of the work --------------------------------------------------------

/// Catches: a `DELETE` granted to the runtime role on either table (the function is the only way a job goes), and a function the
/// role may not run.
#[tokio::test]
async fn the_runtime_role_may_run_the_purge_and_may_not_delete_a_job_or_an_item_itself() {
    let f = fixture().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app')")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("role");
    if !exists {
        return;
    }
    let (may_run, jobs, items): (bool, bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('sagat_app', 'batch_jobs_purge(integer, integer)', 'EXECUTE'), \
                has_table_privilege('sagat_app', 'batch_jobs', 'DELETE'), \
                has_table_privilege('sagat_app', 'batch_items', 'DELETE')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("privileges");
    assert!(may_run, "the role runs the bounded purge");
    assert!(
        !jobs && !items,
        "and has no DELETE of its own on either table"
    );

    // and not only on paper: the role itself is refused, and the function it runs does the work
    let id = seed(&f, &job(&f)).await;
    let runtime = f.w.db.runtime_pool().await;
    for statement in [
        "DELETE FROM batch_items WHERE job_id = $1",
        "DELETE FROM batch_jobs WHERE id = $1",
    ] {
        let refused = sqlx::query(statement).bind(id).execute(&runtime).await;
        assert_eq!(code(refused), INSUFFICIENT_PRIVILEGE, "{statement}");
    }
    let (removed_jobs, removed_items): (i64, i64) =
        sqlx::query_as("SELECT jobs_removed, items_removed FROM batch_jobs_purge($1, $2)")
            .bind(10_i32)
            .bind(100_i32)
            .fetch_one(&runtime)
            .await
            .expect("the role runs the function");
    assert_eq!((removed_jobs, removed_items), (1, 3));
}

/// Holds `sql` (a row lock on what `id` names) in a transaction of the owner; returns it with its backend.
async fn hold(
    f: &Fixture,
    sql: &'static str,
    id: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut held = f.w.db.pool.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("take the lock");
    (held, pid)
}

/// Waits until a backend runs a statement that names `what` while it waits for a lock `holder` holds (`pg_stat_activity`, not a
/// time), at most two minutes.
async fn blocked_on(f: &Fixture, holder: i32, what: &str, stage: &str) {
    within(stage, StdDuration::from_secs(120), async {
        loop {
            let waiting: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock' AND strpos(query, $2) > 0 \
                 LIMIT 1",
            )
            .bind(holder)
            .bind(what)
            .fetch_optional(&f.w.db.pool)
            .await
            .expect("look at the backends");
            if waiting.is_some() {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// A job another transaction holds is skipped, not waited for, so a purge is never stopped by one job and the others behind it are
/// reached; the job it skipped goes at the next call. Catches: the lock taken without `SKIP LOCKED` (the call waits for the holder
/// and the stage times out).
#[tokio::test]
async fn a_job_another_transaction_holds_is_skipped_and_the_ones_behind_it_are_purged() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id().as_uuid();
    let held_job = seed(
        &f,
        &Job {
            created_ago: 60 * DAY,
            expires_in: -30 * DAY,
            ..job(&f)
        },
    )
    .await;
    let free = seed(&f, &job(&f)).await;
    let (held, _) = hold(
        &f,
        "SELECT 1 FROM batch_jobs WHERE id = $1 FOR UPDATE",
        held_job,
    )
    .await;

    let one = within(
        "a purge past a job that is held",
        StdDuration::from_secs(60),
        f.w.store.purge_batch_jobs(10, 1_000),
    )
    .await
    .expect("purge");

    assert_eq!(one, gone(1, 3));
    assert_eq!(state_of(&f, tenant, free).await, None);
    assert_eq!(
        items_of(&f, tenant, held_job).await,
        3,
        "the held job is whole"
    );
    held.rollback().await.expect("release");
    assert_eq!(
        purge(&f, 10, 1_000).await,
        gone(1, 3),
        "and goes when it is free"
    );
}

/// What a purge holds while it works is the job it works on: another job can be read and cancelled while one is being purged, and so
/// can the job itself be read. The purge is held in the middle (an item of its job is locked by a transaction of the test) and
/// watched in `pg_stat_activity`. Catches: a lock on more than the job (a table lock, or a lock on every candidate), and a read that
/// takes the lock of the row.
#[tokio::test]
async fn a_purge_in_flight_does_not_block_the_read_or_the_cancel_of_another_job() {
    let f = fixture().await;
    let ctx = RequestContext::default();
    let live = seed(&f, &job(&f)).await;
    let other = create(&f, write(Action::Cancel, everything()), "purge-other-job").await;
    let (held, holder) = hold(
        &f,
        "SELECT 1 FROM batch_items WHERE job_id = $1 FOR UPDATE",
        live,
    )
    .await;
    let running = tokio::spawn({
        let w = Arc::clone(&f.w);
        async move { w.store.purge_batch_jobs(10, 1_000).await }
    });
    blocked_on(
        &f,
        holder,
        "batch_jobs_purge",
        "the purge waits for an item",
    )
    .await;

    let read = within(
        "the read of another job",
        StdDuration::from_secs(30),
        service(&f).get(&f.w.admin, other.id),
    )
    .await
    .expect("another job reads while a purge is in flight");
    assert_eq!(read.id, other.id);
    let cancelled = within(
        "the cancel of another job",
        StdDuration::from_secs(30),
        service(&f).cancel(&f.w.admin, other.id, "no longer needed", &ctx),
    )
    .await
    .expect("another job is cancelled while a purge is in flight");
    assert_eq!(cancelled.id, other.id);
    within(
        "the read of the job being purged",
        StdDuration::from_secs(30),
        service(&f).get(&f.w.admin, live),
    )
    .await
    .expect("and the job being purged still reads");

    held.rollback().await.expect("release the item");
    assert_eq!(
        running.await.expect("the task").expect("the purge"),
        gone(1, 3)
    );
}

/// Two calls at once (two replicas run the housekeeping) share the work and never both remove a row, wait for each other, or fail:
/// every job and item goes once. Catches: a lock order that deadlocks, a count that is wrong when a row was skipped, and a job that
/// is lost between the two.
#[tokio::test]
async fn two_purges_at_the_same_time_remove_every_row_once_and_never_deadlock() {
    let f = fixture().await;
    for _ in 0..12 {
        seed(
            &f,
            &Job {
                items: 40,
                ..job(&f)
            },
        )
        .await;
    }
    let worker = |f: &Fixture| {
        let w = Arc::clone(&f.w);
        tokio::spawn(async move {
            let (mut jobs, mut items) = (0, 0);
            // bounded, as the purge of five thousand items: the totals say what a purge that does not end the jobs did
            for _ in 0..100 {
                let one = w.store.purge_batch_jobs(3, 50).await.expect("purge");
                if one.rows() == 0 {
                    break;
                }
                jobs += one.jobs;
                items += one.items;
            }
            (jobs, items)
        })
    };
    let (a, b) = (worker(&f), worker(&f));
    let ((a_jobs, a_items), (b_jobs, b_items)) = (a.await.expect("a"), b.await.expect("b"));

    assert_eq!((a_jobs + b_jobs, a_items + b_items), (12, 480));
    assert_eq!(rows(&f).await, (0, 0));
}

// --- what a person and the platform see ------------------------------------------------------------------

/// A job past its window that has not been purged yet still reads (nothing is taken early), and a purged one reads as an id nobody
/// has: not found, for the job, its items and its cancellation, and it is not in the list. Catches: a purge that leaves the job or
/// its items readable, a read that refuses a job that is past its window and not yet purged (a job would go before the purge
/// said so), and a purged job that is answered as anything but unknown.
#[tokio::test]
async fn a_purged_job_reads_as_unknown_and_one_that_is_not_yet_purged_still_reads() {
    let f = fixture().await;
    let ctx = RequestContext::default();
    let id = seed(&f, &job(&f)).await;
    let page = PageRequest::default();
    let admin = &f.w.admin;
    let ns = f.w.namespace.id();

    assert_eq!(service(&f).get(admin, id).await.expect("reads").id, id);
    assert_eq!(
        service(&f)
            .items(admin, id, None, &page)
            .await
            .expect("items read")
            .items
            .len(),
        3
    );
    assert_eq!(
        service(&f)
            .list(admin, ns, None, &page)
            .await
            .expect("list")
            .items
            .len(),
        1
    );

    assert_eq!(purge(&f, 10, 100).await, gone(1, 3));

    assert!(matches!(
        service(&f).get(admin, id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        service(&f).items(admin, id, None, &page).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        service(&f)
            .cancel(admin, id, "no longer needed", &ctx)
            .await,
        Err(AppError::NotFound)
    ));
    assert!(
        service(&f)
            .list(admin, ns, None, &page)
            .await
            .expect("list")
            .items
            .is_empty()
    );
}

/// What a person reads of a job that a call of the purge cut short, pinned so that a change is deliberate: the job is past its window
/// and not yet gone, so it reads (`200`) with the counters it has always had (they are the job's own, `selected` 10) and the items it has
/// left, which start at the ordinal where the call stopped. It is the one visible in-between state of a purge; the pass finishes the job in
/// the same pass while its calls use their bounds, so it lasts a moment, not an interval of the housekeeping (the runbook, section 6).
/// Catches: a purge that hides a job it has begun (a read would answer `404` while the job is still there), and one that changes the
/// counters of a job it cut short.
#[tokio::test]
async fn a_job_a_call_cut_short_still_reads_with_the_items_it_has_left() {
    let f = fixture().await;
    let id = seed(
        &f,
        &Job {
            items: 10,
            ..job(&f)
        },
    )
    .await;
    assert_eq!(purge(&f, 10, 4).await, gone(0, 4));

    let read = service(&f)
        .get(&f.w.admin, id)
        .await
        .expect("the job still reads");
    assert_eq!(read.state, JobState::Completed);
    assert_eq!(read.counts.selected, 10, "the counters are the job's own");
    let page = service(&f)
        .items(&f.w.admin, id, None, &PageRequest::default())
        .await
        .expect("its items read");
    let ordinals: Vec<u32> = page.items.iter().map(|item| item.ordinal).collect();
    assert_eq!(ordinals, vec![5, 6, 7, 8, 9, 10], "the items it has left");
}

/// The same over HTTP: the read of the job answers `200` with its own counters, and the list of its items starts at the ordinal where
/// the call stopped. Catches: as the test of the store, for the routes a person uses.
#[tokio::test]
async fn a_job_a_call_cut_short_still_reads_over_http_with_the_items_it_has_left() {
    use reqwest::Method;

    use crate::support::worker::platform;

    let p = platform().await;
    let namespace = p.namespace_id().await;
    let id = seed_on(
        &p.api.db.pool,
        &Job {
            tenant: p.tenant.as_uuid(),
            namespace: namespace.parse().expect("a namespace id"),
            id: Uuid::new_v4(),
            state: "COMPLETED",
            created_ago: 40 * DAY,
            expires_in: -10 * DAY,
            finished_ago: Some(39 * DAY),
            items: 10,
            selector: json!({"filter": {}}),
            reason: "stop the runs that are stuck".to_owned(),
        },
    )
    .await;
    let cut = p
        .api
        .execution_store
        .purge_batch_jobs(10, 4)
        .await
        .expect("purge");
    assert_eq!(cut, gone(0, 4));

    let route = format!("/namespaces/{namespace}/batch-operations/{id}");
    let (status, read) = p.rest(Method::GET, &route, None).await;
    assert_eq!(status, 200, "{read}");
    assert_eq!(read["state"], "COMPLETED", "{read}");
    assert_eq!(read["counts"]["selected"], 10, "{read}");
    let (status, items) = p.rest(Method::GET, &format!("{route}/items"), None).await;
    assert_eq!(status, 200, "{items}");
    let ordinals: Vec<i64> = items["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["ordinal"].as_i64())
        .collect();
    assert_eq!(ordinals, vec![5, 6, 7, 8, 9, 10], "{items}");
}

/// The list of a namespace goes on across a purge: the cursor is the id of the last job of the page, and a job that went, the
/// cursor's own included, is not in the page after it and does not stop it. Catches: a cursor that needs its own row.
#[tokio::test]
async fn the_list_of_jobs_goes_on_across_the_purge() {
    let f = fixture().await;
    let admin = &f.w.admin;
    let ns = f.w.namespace.id();
    // newest first: 6 5 | 4 3 | 2 1; the jobs 5, 4 and 2 are over
    let ids: Vec<Uuid> = (1..=6_u128).map(|n| Uuid::from_u128(n << 64 | n)).collect();
    for (n, id) in ids.iter().enumerate() {
        let over = matches!(n + 1, 2 | 4 | 5);
        seed(
            &f,
            &Job {
                id: *id,
                expires_in: if over { -10 * DAY } else { 10 * DAY },
                ..job(&f)
            },
        )
        .await;
    }
    let page = PageRequest {
        limit: Some(2),
        cursor: None,
    };
    let first = service(&f)
        .list(admin, ns, None, &page)
        .await
        .expect("first");
    assert_eq!(
        first.items.iter().map(|j| j.id).collect::<Vec<_>>(),
        vec![ids[5], ids[4]]
    );
    let cursor = first.next_cursor.expect("another page");

    assert_eq!(purge(&f, 10, 100).await, gone(3, 9));

    let second = service(&f)
        .list(
            admin,
            ns,
            None,
            &PageRequest {
                limit: Some(2),
                cursor: Some(cursor),
            },
        )
        .await
        .expect("second");
    assert_eq!(
        second.items.iter().map(|j| j.id).collect::<Vec<_>>(),
        vec![ids[2], ids[0]]
    );
    assert_eq!(second.next_cursor, None);
}

/// A purge frees nothing the active-job limits count, and takes nothing from them: only the jobs that are active count and a job that
/// ended never did. The content a job charged (its selector and its reason) is given back with it, in the transaction that
/// removes it (ADR-0027), exactly: the usage after the purge is what it was before the three jobs plus what the active job made since
/// charged. Catches: a purge that moves the count of active jobs, one that leaves the charge of what it removed, and one that gives
/// back more than it was charged (a double refund, which the tenant would spend).
#[tokio::test]
async fn a_purge_leaves_the_limits_of_active_jobs_alone_and_gives_back_the_content_it_charged() {
    let f = fixture().await;
    let usage = |f: &Fixture| {
        let pool = f.w.db.pool.clone();
        let tenant = f.w.admin.tenant_id().as_uuid();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(content_bytes, 0)::bigint FROM tenant_retained_usage WHERE tenant_id = $1",
            )
            .bind(tenant)
            .fetch_optional(&pool)
            .await
            .expect("usage")
            .unwrap_or(0)
        }
    };
    let before = usage(&f).await;
    seed(&f, &unfinished(&f, "RUNNING")).await;
    let running = usage(&f).await;
    for _ in 0..3 {
        seed(
            &f,
            &Job {
                reason: "a reason that weighs something ".repeat(8),
                selector: json!({"filter": {"businessIdPrefix": "weighs-something-"}}),
                ..job(&f)
            },
        )
        .await;
    }
    let with_three = usage(&f).await;
    assert!(
        with_three > running,
        "the three jobs are charged: {running} then {with_three}"
    );
    let new_job = |key: &'static str| {
        let (f, key) = (&f, key);
        async move {
            service(f)
                .create(
                    &f.w.admin,
                    f.w.namespace.id(),
                    write(Action::Cancel, everything()),
                    key,
                    &RequestContext::default(),
                )
                .await
        }
    };
    new_job("purge-limit-one")
        .await
        .expect("a second active job: the finished ones did not count");
    let refused = new_job("purge-limit-two").await;
    assert!(
        matches!(&refused, Err(AppError::QuotaExceeded(what)) if *what == QuotaResource::BatchJobs.limit_name()),
        "two active jobs a namespace: {refused:?}"
    );
    // what making the second active job charged: the purge must leave that, and give back what the three charged and nothing else
    let with_second = usage(&f).await;
    let second_job = with_second - with_three;

    assert_eq!(purge(&f, 10, 1_000).await, gone(3, 9));

    let refused = new_job("purge-limit-three").await;
    assert!(
        matches!(&refused, Err(AppError::QuotaExceeded(what)) if *what == QuotaResource::BatchJobs.limit_name()),
        "a purge frees no slot of the active jobs: {refused:?}"
    );
    let after = usage(&f).await;
    assert_eq!(
        after,
        running + second_job,
        "exactly what the three jobs charged is given back: {with_three} charged, {with_second} with the second active job"
    );
    assert!(
        after >= before,
        "and the tenant's usage never goes below what it was before any job: {before} then {after}"
    );
    // the active jobs are as they were
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM batch_jobs WHERE state IN ('SELECTING', 'RUNNING', 'CANCELLING')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("active");
    assert_eq!(active, 2);
}

/// The read of a purged job over HTTP answers what the read of an id nobody has answers: the same status, the same code and the same
/// details, for the job, its items and its cancellation, and nothing of the job is in the body. Before the purge a job past its
/// window still reads, as ever. Catches: a purge that leaves the job (the read answers `200`), a read that answers a purged job
/// another way than an unknown one (a person could tell that it had existed), and a route whose answer was never looked at over HTTP.
#[tokio::test]
async fn a_purged_job_answers_404_over_http_exactly_as_an_id_nobody_has_does() {
    use reqwest::Method;

    use crate::support::worker::platform;

    let p = platform().await;
    let namespace = p.namespace_id().await;
    let id = seed_on(
        &p.api.db.pool,
        &Job {
            tenant: p.tenant.as_uuid(),
            namespace: namespace.parse().expect("a namespace id"),
            id: Uuid::new_v4(),
            state: "COMPLETED",
            created_ago: 40 * DAY,
            expires_in: -10 * DAY,
            finished_ago: Some(39 * DAY),
            items: 3,
            selector: json!({"filter": {"businessIdPrefix": "canary-http-"}}),
            reason: "stop the runs that are stuck".to_owned(),
        },
    )
    .await;
    let nobody = Uuid::new_v4();
    let route =
        |id: Uuid, tail: &str| format!("/namespaces/{namespace}/batch-operations/{id}{tail}");
    let cancel_body = json!({"reason": "no longer needed"});

    let (status, read) = p.rest(Method::GET, &route(id, ""), None).await;
    assert_eq!(
        status, 200,
        "a job past its window and not yet purged still reads: {read}"
    );
    let (status, items) = p.rest(Method::GET, &route(id, "/items"), None).await;
    assert_eq!(status, 200, "{items}");
    assert_eq!(items["items"].as_array().map(Vec::len), Some(3), "{items}");

    let purged = p
        .api
        .execution_store
        .purge_batch_jobs(10, 100)
        .await
        .expect("purge");
    assert_eq!(purged, gone(1, 3));

    for (method, tail, body) in [
        (Method::GET, "", None),
        (Method::GET, "/items", None),
        (Method::POST, "/cancel", Some(&cancel_body)),
    ] {
        let (status, answer) = p.rest(method.clone(), &route(id, tail), body).await;
        let (unknown_status, unknown) = p.rest(method.clone(), &route(nobody, tail), body).await;
        assert_eq!(status, 404, "{method} {tail}: {answer}");
        assert_eq!(status, unknown_status, "{method} {tail}");
        assert_eq!(answer["code"], "NOT_FOUND", "{method} {tail}: {answer}");
        assert_eq!(answer["code"], unknown["code"], "{method} {tail}");
        assert_eq!(answer["details"], unknown["details"], "{method} {tail}");
        assert_ne!(
            answer["details"]["reason"], "ROUTE_NOT_FOUND",
            "the route is there and the job is not: {answer}"
        );
        let text = answer.to_string();
        assert!(
            !text.contains(&id.to_string()) && !text.contains("canary-http-"),
            "{method} {tail}: the answer names the job that went: {answer}"
        );
    }
}

/// A bound that is not at least 1 removes nothing, and a call with no bound removes nothing either (the function is `STRICT`: a caller
/// that leaves the bound out does not lift it). Catches: a function that is not strict (`NULL` makes the limit of the chunk and of the
/// loop disappear and the call removes every job that is over).
#[tokio::test]
async fn a_bound_that_is_not_at_least_one_removes_nothing_and_a_call_without_a_bound_answers_nothing()
 {
    let f = fixture().await;
    for _ in 0..3 {
        seed(&f, &job(&f)).await;
    }

    assert_eq!(purge(&f, 0, 1_000).await, BatchPurged::default());
    assert_eq!(purge(&f, 10, 0).await, BatchPurged::default());
    for call in [
        "SELECT jobs_removed, items_removed FROM batch_jobs_purge(NULL, NULL)",
        "SELECT jobs_removed, items_removed FROM batch_jobs_purge(10, NULL)",
        "SELECT jobs_removed, items_removed FROM batch_jobs_purge(NULL, 1000)",
    ] {
        let answered: Vec<(i64, i64)> = sqlx::query_as(call)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("the call");
        assert!(answered.is_empty(), "{call}: {answered:?}");
    }

    assert_eq!(rows(&f).await, (3, 9), "nothing was removed");
}

/// The window a job is stamped is 30 days by default and is held between the least (`MIN_RETENTION`, 14 days) and the most (90) the
/// service allows, whatever `with_retention` is given: the purge reads the stamp, so a window under the floor would be a job that the
/// purge keeps longer than its `expiresAt` says. Catches: a floor that is the old one day, and a ceiling that is not held.
#[tokio::test]
async fn the_window_a_job_is_stamped_is_thirty_days_and_is_held_between_the_least_and_the_most() {
    use sagat_application::batch::{DEFAULT_RETENTION, MAX_RETENTION};
    use time::Duration;

    let f = fixture().await;
    // room for every job the cases create: the deployment ceiling first, then the limit
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, 'batch_jobs', 16, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) VALUES ($1, $2, 'batch_jobs', 16, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs");
    let stamped = |retention: Option<Duration>, key: &'static str| {
        let f = &f;
        async move {
            let service = match retention {
                Some(retention) => service(f).with_retention(retention),
                None => service(f),
            };
            let made = service
                .create(
                    &f.w.admin,
                    f.w.namespace.id(),
                    write(Action::Cancel, everything()),
                    key,
                    &RequestContext::default(),
                )
                .await
                .expect("create");
            made.expires_at - made.created_at
        }
    };

    assert_eq!(stamped(None, "window-default").await, DEFAULT_RETENTION);
    assert_eq!(
        stamped(Some(Duration::days(1)), "window-one-day").await,
        MIN_RETENTION,
        "one day is raised to the least"
    );
    assert_eq!(
        stamped(Some(Duration::days(45)), "window-forty-five").await,
        Duration::days(45)
    );
    assert_eq!(
        stamped(Some(Duration::days(400)), "window-too-long").await,
        MAX_RETENTION,
        "a year is cut to the most"
    );
}

/// A job that carries a payload (a `SIGNAL`, or an `UPDATE` with arguments) holds it by reference: `batch_jobs.payload_digest` names a
/// row of `payloads` (`batch_jobs_payload_fk`), content of the tenant, up to 256 KiB. The purge removes the job and its items and **not
/// that row**: nothing in this feature collects the payloads that nothing references, which is the registry of feature 020 (G2316), so
/// the payload stays, and so does its charge to the tenant. This test pins what stays so that 020 meets it, not discovers it (G2325):
/// the row is there after the purge, the tenant's usage is not refunded for it, and is refunded for the job's own selector and reason.
/// Catches: a purge that deletes the payload (it would break another job that shares the digest, which payloads are by content), and one
/// that leaves what the job itself charged.
#[tokio::test]
async fn the_payload_a_signal_job_holds_outlives_the_job_and_stays_charged() {
    let f = fixture().await;
    let body = format!(r#"{{"ticket":"{}"}}"#, "x".repeat(4_000)).into_bytes();
    let before = content_usage(&f).await;
    let mut request = write(Action::Signal, everything());
    request.signal = Some(NamedPayload {
        name: "approve".to_owned(),
        body: body.clone(),
    });
    let made = create(&f, request, "payload-residue-key").await;
    let digest = made.payload_digest.expect("the job carries its payload");
    let with_job = content_usage(&f).await;
    assert!(
        with_job >= before + i64::try_from(body.len()).expect("size"),
        "the payload and the job are charged: {before} then {with_job}"
    );
    aged(&f, made.id, 40, 30, 39).await;

    assert_eq!(purge(&f, 10, 100).await, gone(1, 0));

    assert_eq!(rows(&f).await, (0, 0), "the job is gone");
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(digest.as_slice())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the payload");
    assert_eq!(kept, 1, "the payload outlives the job (G2325)");
    let after = content_usage(&f).await;
    assert!(
        after >= before + i64::try_from(body.len()).expect("size"),
        "the payload is still charged: {before}, with the job {with_job}, after the purge {after}"
    );
    assert!(
        after < with_job,
        "and what the job charged itself (its selector and its reason) is given back: {with_job} then {after}"
    );
}

// --- the key of the request that made a job ---------------------------------------------------------------

/// Ages a job the service made as if it were `created_days` old and ended `ended_days` ago with a window of `window_days`, and the
/// key of its request as if it were as old as the job. The owner disables the guard of the table for the statement (the guard
/// refuses to move what a job is, and a test cannot move the clock of the database), and enables it again.
async fn aged(f: &Fixture, id: Uuid, created_days: i32, window_days: i32, ended_days: i32) {
    // three statements of their own: the update leaves the deferred trigger events of the table until it commits, and the table
    // cannot be altered while it has any
    sqlx::query("ALTER TABLE batch_jobs DISABLE TRIGGER batch_jobs_guard")
        .execute(&f.w.db.pool)
        .await
        .expect("disable the guard for this test");
    sqlx::query(
        "UPDATE batch_jobs SET state = 'COMPLETED', \
                created_at = clock_timestamp() - make_interval(days => $2), \
                expires_at = clock_timestamp() - make_interval(days => $2) + make_interval(days => $3), \
                finished_at = clock_timestamp() - make_interval(days => $4) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(created_days)
    .bind(window_days)
    .bind(ended_days)
    .execute(&f.w.db.pool)
    .await
    .expect("age the job");
    sqlx::query("ALTER TABLE batch_jobs ENABLE ALWAYS TRIGGER batch_jobs_guard")
        .execute(&f.w.db.pool)
        .await
        .expect("enable the guard again");
    sqlx::query(
        "UPDATE command_dedupe SET created_at = clock_timestamp() - make_interval(days => $2) WHERE resource_id = $1::text",
    )
    .bind(id)
    .bind(created_days)
    .execute(&f.w.db.pool)
    .await
    .expect("age the key");
}

/// The same request, under the same key, as many times as the case asks.
async fn ask(f: &Fixture) -> Result<JobRecord, AppError> {
    service(f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            write(Action::Cancel, everything()),
            "purge-replay-key",
            &RequestContext::default(),
        )
        .await
}

/// The key of the request that made a job is remembered for seven days (`SchedulerSettings::idempotency_retention`) and a job is kept
/// for fourteen after it ended at least, so a replay of the key never meets a job that was purged: it finds the job, or, once the
/// key is forgotten, it is a request of its own. It reads the default memory of a key only (the engine has no setting for it): a
/// configuration that raised it above the floor is G2322, which no test here reaches. Catches: a floor that is shorter than the
/// memory of the key (the replay would find a key whose job is gone and answer an internal error, as every replay of a resource that
/// went does), and a replay that answers a job that is not the one it made.
#[tokio::test]
async fn a_replay_of_the_key_of_a_job_finds_the_job_while_it_is_kept_and_is_a_new_request_once_both_are_gone()
 {
    let remembered = i64::try_from(
        sagat_scheduler::SchedulerSettings::default()
            .idempotency_retention
            .as_secs(),
    )
    .expect("seconds");
    assert!(
        MIN_RETENTION.whole_seconds() > remembered,
        "the least a job is kept is longer than the memory of its key"
    );

    let f = fixture().await;
    let first = ask(&f).await.expect("the job");

    // four days old, ended three days ago with a window of one day: past its window, not past its floor; its key is remembered
    aged(&f, first.id, 4, 1, 3).await;
    f.w.scheduler.housekeeping().await;
    assert_eq!(
        ask(&f).await.expect("the replay").id,
        first.id,
        "the key is remembered and so is the job"
    );

    // fifteen days old and ended fourteen days ago and a bit: the key (seven days) is forgotten and then the job goes
    aged(&f, first.id, 20, 1, 15).await;
    f.w.scheduler.housekeeping().await;
    assert_eq!(rows(&f).await.0, 0, "the job went");
    let replay = ask(&f).await.expect("a request of its own");
    assert_ne!(replay.id, first.id, "not the job that went");
    assert_eq!(replay.state, JobState::Selecting);
}

// --- the time of one call ------------------------------------------------------------------------------------

/// `jobs` jobs that ended and whose window ended, each with `items` items, a selector of about 65,000 bytes (the most the table
/// takes) and a reason of 2,000 characters (the most it takes): what the quota trigger measures at the delete of each job row.
async fn seed_heavy(f: &Fixture, jobs: i32, items: i32) {
    sqlx::query(
        "INSERT INTO batch_jobs \
         (tenant_id, namespace_id, id, action, selector, reason, created_by, state, max_items, rate_milli, selected, \
          selection_bound_at, selection_horizon, next_slot_at, created_at, finished_at, expires_at) \
         SELECT $1, $2, gen_random_uuid(), 'CANCEL', \
                jsonb_build_object('filter', jsonb_build_object('businessIdPrefix', repeat('a', 65000))), repeat('r', 2000), \
                'alice', 'COMPLETED', 100000, 20000, $4, clock_timestamp() - interval '40 days', pg_current_xact_id(), \
                clock_timestamp(), clock_timestamp() - interval '40 days', clock_timestamp() - interval '39 days', \
                clock_timestamp() - interval '10 days' \
         FROM generate_series(1, $3)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(jobs)
    .bind(items)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the heavy jobs");
    sqlx::query(
        "INSERT INTO batch_items (tenant_id, job_id, ordinal, run_id, state, updated_at) \
         SELECT j.tenant_id, j.id, g, gen_random_uuid(), 'SUCCEEDED', clock_timestamp() \
         FROM batch_jobs j CROSS JOIN generate_series(1, $1) g",
    )
    .bind(items)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the items");
}

/// A measurement, not a test of a result: how long one call of the purge takes on this machine, for the engine's statement timeout
/// (5 s, `PoolConfig::statement_timeout`) and for a lock held to the end of a call. Run it with
/// `cargo test -p sagat-tests --test integration_schedules_bulk_operations -- --ignored --nocapture one_call_of_the_purge`.
/// Three cases, three runs each, printed as milliseconds: 5,000 item rows of one job; 100 jobs of the most a row weighs with their
/// items (4,900 items and 100 job rows); and the largest call the function allows (100,000 rows).
#[tokio::test]
#[ignore = "a measurement: prints the time of one call of the purge"]
async fn one_call_of_the_purge_measured() {
    let f = fixture().await;
    let time =
        |label: &'static str, jobs: i32, items: i32, call: (u32, u32), expect: BatchPurged| {
            let f = &f;
            async move {
                let mut took = Vec::new();
                for _ in 0..3 {
                    seed_heavy(f, jobs, items).await;
                    let started = std::time::Instant::now();
                    let one =
                        f.w.store
                            .purge_batch_jobs(call.0, call.1)
                            .await
                            .expect("purge");
                    took.push(started.elapsed().as_millis());
                    assert_eq!(one, expect, "{label}");
                    // what is left of a job the call cut short goes at the next calls, outside the time
                    while purge(f, 1_000, 100_000).await.rows() > 0 {}
                }
                eprintln!("MEASURED {label}: {took:?} ms (three runs)");
            }
        };
    time(
        "one job of 5,000 items, call of 5,000 rows",
        1,
        5_000,
        (100, 5_000),
        gone(0, 5_000),
    )
    .await;
    time(
        "100 jobs of 49 items with the heaviest rows, call of 5,000 rows",
        100,
        49,
        (100, 5_000),
        gone(100, 4_900),
    )
    .await;
    time(
        "one job of 100,000 items, the largest call (100,000 rows)",
        1,
        100_000,
        (100, 100_000),
        gone(0, 100_000),
    )
    .await;
}

// --- housekeeping, the metric and what is said ---------------------------------------------------------------

/// The page: what a housekeeping call purges is counted by kind once it has committed, over several calls of the function when the
/// job is large (12,003 rows with the bound of 5,000), and the jobs that stay are not. Catches: a pass that makes one call only
/// (the job of 12,000 items is left with its last 7,000), counters that are not counted or are counted under another word, and a
/// label that is not one of the two.
#[test]
fn housekeeping_purges_what_is_over_in_rounds_and_counts_it_by_kind() {
    let page = recorded(|| async {
        let f = fixture().await;
        seed(
            &f,
            &Job {
                items: 12_000,
                ..job(&f)
            },
        )
        .await;
        seed(&f, &job(&f)).await;
        seed(
            &f,
            &Job {
                items: 0,
                ..job(&f)
            },
        )
        .await;
        seed(
            &f,
            &Job {
                created_ago: 5 * DAY,
                expires_in: 25 * DAY,
                finished_ago: Some(4 * DAY),
                ..job(&f)
            },
        )
        .await;
        seed(&f, &unfinished(&f, "RUNNING")).await;

        f.w.scheduler.housekeeping().await;

        assert_eq!(
            rows(&f).await,
            (2, 6),
            "the one inside its window and the one that runs"
        );
    });
    assert_eq!(
        series(&page, "batch_purged_total", &[("kind", "job")]),
        Some(3.0),
        "{page}"
    );
    assert_eq!(
        series(&page, "batch_purged_total", &[("kind", "item")]),
        Some(12_003.0),
        "{page}"
    );
    for line in page.lines().filter(|l| l.starts_with("batch_purged_total")) {
        let labels = &line[line.find('{').map_or(0, |i| i + 1)..line.rfind('}').unwrap_or(0)];
        assert!(
            labels == "kind=\"job\"" || labels == "kind=\"item\"",
            "a label of the series is not one of the two closed words: `{line}`"
        );
    }
}

/// One pass of the housekeeping removes 250 jobs that have no items: three calls (100, 100 and 50 jobs), and the pass goes on while a
/// call used its bound on jobs, though its rows are few. Catches: a pass that goes on only for rows (the first call removes 100 rows,
/// under its budget of rows, and the pass ends with 150 jobs left for the next interval).
#[tokio::test]
async fn one_pass_removes_two_hundred_and_fifty_jobs_that_have_no_items() {
    // the case needs more jobs than a call may take
    const _: () = assert!(BATCH_PURGE_JOBS < 250);
    let f = fixture().await;
    seed_empty_jobs(&f, 250).await;

    f.w.scheduler.housekeeping().await;

    assert_eq!(rows(&f).await, (0, 0));
}

/// A job whose items are exactly the rows of one call: the first call removes them all and no row is left for the job's own, and a pass
/// that ends there leaves a job with no items and a read that finds it empty until the next interval. The pass goes on, because the
/// call used its whole row budget, and the second call removes the job. Catches: a continuation that wants more than the budget (`>`
/// for `>=`), and a pass that stops at the first call that is full.
#[tokio::test]
async fn one_pass_finishes_a_job_whose_items_were_exactly_one_call() {
    let f = fixture().await;
    seed(
        &f,
        &Job {
            items: i32::try_from(BATCH_PURGE_ROWS).expect("rows"),
            ..job(&f)
        },
    )
    .await;

    f.w.scheduler.housekeeping().await;

    assert_eq!(
        rows(&f).await,
        (0, 0),
        "the job went in the pass that began it"
    );
}

/// `increase` does not count the first sample of a series: the three series exist at zero once a scheduler does, before any purge.
/// Catches: a counter created by its first increment (the page has no such line before it).
#[test]
fn the_series_of_the_purge_exist_at_zero_before_the_first_purge() {
    let page = recorded(|| async {
        let _f = fixture().await;
    });
    for kind in ["job", "item"] {
        assert_eq!(
            series(&page, "batch_purged_total", &[("kind", kind)]),
            Some(0.0),
            "the series `{kind}` is on the page at zero before any purge:\n{page}"
        );
    }
    let failures = page
        .lines()
        .find(|line| line.starts_with("batch_purge_failures_total "))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse::<f64>().ok());
    assert_eq!(failures, Some(0.0), "{page}");
}

/// Runs the housekeeping once over a job that is over (its reason is `reason`) and a binding with no holder, after `break_it` has made
/// the purge fail, inside a capture of the logs and the metrics; returns what was logged and the page of the metrics. Checks that the
/// failed call removed nothing (it rolled back) and that the rest of the housekeeping went on (the binding was purged after it).
fn housekeeping_over_a_broken_purge(
    reason: &str,
    break_it: &'static [&'static str],
) -> (String, String) {
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
    tracing::warn!(probe = "purge-probe", "the capture is on");
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            seed(&f, &Job { reason: reason.to_owned(), ..job(&f) }).await;
            for statement in break_it {
                sqlx::query(*statement)
                    .execute(&f.w.db.pool)
                    .await
                    .expect("break the purge");
            }
            sqlx::query(
                "INSERT INTO business_ids \
                 (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
                 VALUES ($1, $2, 'orphan-after-purge-failure', $3, $4, now(), 2592000)",
            )
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(f.w.namespace.id().as_uuid())
            .bind(Uuid::now_v7())
            .bind(f.workflow.as_uuid())
            .execute(&f.w.db.pool)
            .await
            .expect("a binding with no holder");

            f.w.scheduler.housekeeping().await;

            assert_eq!(rows(&f).await, (1, 3), "the failed purge removed nothing");
            let bindings: i64 = sqlx::query_scalar("SELECT count(*) FROM business_ids")
                .fetch_one(&f.w.db.pool)
                .await
                .expect("bindings");
            assert_eq!(bindings, 0, "the rest of the housekeeping went on");
        });
    });
    drop(guard);
    let said = logs.text();
    assert!(
        said.contains("purge-probe"),
        "the log capture was listening"
    );
    assert!(
        said.contains("could not purge batch jobs"),
        "the failure is logged: {said}"
    );
    let page = metrics.render();
    assert_eq!(
        page.lines()
            .find(|line| line.starts_with("batch_purge_failures_total "))
            .and_then(|line| line.rsplit(' ').next())
            .and_then(|value| value.parse::<f64>().ok()),
        Some(1.0),
        "the failure is counted once: {page}"
    );
    (said, page)
}

/// The text of a database error can quote a value, and a job holds customer text. A purge that fails is counted and logged by its
/// SQLSTATE and the kind of the error and by nothing else, and the rest of the housekeeping goes on (a binding with no holder is
/// purged after it). The failure is made by a trigger of the test whose message quotes the reason of the job, a canary. Catches: an
/// error read with its text (the storage layer logs the text of an error that is not read without it), a failure that is not counted,
/// one that is counted twice, a log line without the kind, and a failure that ends the housekeeping.
#[test]
fn a_failure_of_the_purge_is_counted_and_logged_by_its_sqlstate_and_kind_and_the_housekeeping_goes_on()
 {
    let canary = format!("canary-purge-failure-{}", Uuid::now_v7().simple());
    let (said, page) = housekeeping_over_a_broken_purge(
        &format!("stop {canary}"),
        &[
            "CREATE FUNCTION fault_quote_reason() RETURNS trigger LANGUAGE plpgsql AS $f$ \
             BEGIN RAISE EXCEPTION 'the job says: %', OLD.reason; END $f$",
            "CREATE TRIGGER aaa_fault_quote BEFORE DELETE ON batch_jobs FOR EACH ROW EXECUTE FUNCTION fault_quote_reason()",
        ],
    );
    assert!(
        said.contains("sqlstate=\"P0001\"") && said.contains("kind=\"internal\""),
        "the failure is logged with its SQLSTATE and its kind: {said}"
    );
    assert!(
        !said.contains(&canary),
        "the text of the error reached a log line"
    );
    assert!(
        !page.contains(&canary),
        "the text of the error reached a metric"
    );
}

/// An error that carries no SQLSTATE of the closed code the log reads (the adapter keeps the code only for the errors it cannot
/// otherwise name) still says what kind it was: a foreign key refused (an item the call did not remove, here by a trigger that skips
/// the delete) is `sqlstate="none"` and `kind="missing_reference"`, so an operator is not left with `none` and nothing. Catches: a log
/// line without the kind.
#[test]
fn a_failure_that_has_no_sqlstate_to_log_still_says_its_kind() {
    let (said, _page) = housekeeping_over_a_broken_purge(
        "stop what is stuck",
        &[
            "CREATE FUNCTION fault_skip_item() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN RETURN NULL; END $f$",
            "CREATE TRIGGER aaa_fault_skip BEFORE DELETE ON batch_items FOR EACH ROW EXECUTE FUNCTION fault_skip_item()",
        ],
    );
    assert!(
        said.contains("sqlstate=\"none\"") && said.contains("kind=\"missing_reference\""),
        "{said}"
    );
}

/// A job made by the platform holds a selector with customer text (a business-id prefix) and a reason; the purge removes both and logs,
/// counts and audits none of it, and nothing of the database holds it afterwards. The job is made by the service, aged with the guard
/// disabled (`aged`), and purged by the housekeeping inside a capture of the logs, the spans and the metrics. Catches: a purge that
/// says what it removes (a notice of the function, which the storage layer logs, or a line of the pass), a row of the schema that
/// still holds the selector or the reason, and a job that was never removed. **What stays**, said exactly: the body of a `SIGNAL`
/// job (a payload of the tenant, by digest in `payloads`), which nothing of this feature collects (G2325); the test names it and
/// fails if anything else stays or if it goes (that would be a decision of feature 020's registry, not an accident).
#[test]
fn the_prefix_and_the_reason_of_a_purged_job_reach_no_log_span_metric_or_audit_record_and_are_gone_afterwards()
 {
    let canary = format!("canary-prefix-{}", Uuid::now_v7().simple());
    let hex: String = canary.bytes().map(|b| format!("{b:02x}")).collect();
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
    tracing::warn!(probe = "purge-probe", "the capture is on");
    drop(tracing::info_span!("purge_span_probe", probe = "kept"));
    let leaks = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            let selector = Selector::filter(Filter {
                business_id_prefix: Some(canary.clone()),
                ..Filter::default()
            })
            .expect("selector");
            let mut request = write(Action::Cancel, selector);
            request.reason =
                sagat_domain::batch::Justification::new(&format!("stop what carries {canary}")).expect("reason");
            let made = create(&f, request, "purge-canary-key").await;
            // a job that carries a payload: its body is customer content in `payloads`, which the purge does not remove (G2325)
            let mut signal = write(Action::Signal, everything());
            signal.signal = Some(NamedPayload {
                name: "approve".to_owned(),
                body: format!(r#"{{"ticket":"{canary}"}}"#).into_bytes(),
            });
            let with_payload = create(&f, signal, "purge-canary-payload-key").await;
            let holding = |f: &Fixture| {
                let pool = f.w.db.pool.clone();
                let (canary, hex) = (canary.clone(), hex.clone());
                async move {
                    let tables: Vec<String> = sqlx::query_scalar(
                        "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
                    )
                    .fetch_all(&pool)
                    .await
                    .expect("tables");
                    let mut holding = Vec::new();
                    for table in tables {
                        let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                            "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%' OR t::text LIKE '%' || $2 || '%'"
                        )))
                        .bind(&canary)
                        .bind(&hex)
                        .fetch_one(&pool)
                        .await
                        .unwrap_or_else(|error| panic!("search {table}: {error}"));
                        if hits > 0 {
                            holding.push(table);
                        }
                    }
                    holding
                }
            };
            assert_eq!(
                holding(&f).await,
                ["batch_jobs", "payloads"],
                "before the purge the selector and the reason are in the job, and the body of the signal in the payload, and nowhere else"
            );
            aged(&f, made.id, 40, 30, 39).await;
            aged(&f, with_payload.id, 40, 30, 39).await;

            f.w.scheduler.housekeeping().await;

            assert_eq!(rows(&f).await, (0, 0), "the job went");
            holding(&f).await
        })
    });
    drop(guard);
    assert_eq!(
        leaks,
        ["payloads"],
        "after the purge the selector and the reason are in no table; what stays is the body of the signal, in `payloads`, which the \
         purge does not remove (G2325)"
    );
    let said = logs.text();
    assert!(
        said.contains("purge-probe") && said.contains("purge_span_probe"),
        "the log capture was listening, spans included"
    );
    assert!(
        said.contains("purged batch jobs and their items"),
        "the purge said what it did: {said}"
    );
    assert!(
        !said.contains(&canary) && !said.contains(&hex),
        "the prefix or the reason reached a log line or a span"
    );
    let page = metrics.render();
    assert!(
        !page.contains(&canary) && !page.contains(&hex),
        "the prefix or the reason reached a metric"
    );
}

// --- the migration -----------------------------------------------------------------------------------------

/// This migration: the one version a renumbering at merge changes in this file.
const THIS: i64 = 20_270_119_000_000;

/// The migration before it, read from the list the migrator embeds: where a rollback of this one alone stops.
fn previous() -> i64 {
    MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version < THIS)
        .map(|m| m.version)
        .max()
        .expect("a migration before this one")
}

#[test]
fn this_migration_is_the_one_the_rollback_of_this_file_starts_from() {
    // a renumbering at merge moves THIS and nothing else of this file: the migration before it is read from the list
    let this: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration() && m.version == THIS)
        .collect();
    assert_eq!(this.len(), 1, "THIS names one forward migration");
    assert!(this[0].description.contains("batch purge"));
    assert!(previous() < THIS);
}

async fn function_and_index(f: &Fixture) -> (i64, i64) {
    let function =
        sqlx::query_scalar("SELECT count(*) FROM pg_proc WHERE proname = 'batch_jobs_purge'")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("function");
    let index = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes WHERE schemaname = current_schema() AND indexname = 'batch_jobs_purge_idx'",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("index");
    (function, index)
}

/// The rollback takes the function and its index and nothing a job or an item holds (what was purged stays purged, what was not is
/// there), is recorded, and applies again with the purge working. Catches: a `down` that refuses (it destroys nothing), one that drops
/// more than the migration added, and an `up` that does not apply again.
#[tokio::test]
async fn the_rollback_drops_the_function_and_its_index_and_loses_no_job_and_applying_again_brings_the_purge_back()
 {
    let f = fixture().await;
    assert_eq!(function_and_index(&f).await, (1, 1));
    seed(&f, &job(&f)).await;
    seed(&f, &unfinished(&f, "RUNNING")).await;
    seed(&f, &job(&f)).await;
    assert_eq!(purge(&f, 1, 1_000).await, gone(1, 3));
    assert_eq!(rows(&f).await, (2, 6));

    undo_to(&f.w.db, previous())
        .await
        .expect("nothing a job or an item holds is lost by the rollback");

    assert_eq!(
        function_and_index(&f).await,
        (0, 0),
        "the function and its index are gone"
    );
    assert_eq!(
        rows(&f).await,
        (2, 6),
        "and every job and item is as it was"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(THIS)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recorded");
    assert_eq!(recorded, 0, "the rollback is recorded");

    migrate(&f.w.db.pool).await.expect("forward again");
    assert_eq!(function_and_index(&f).await, (1, 1));
    assert_eq!(
        purge(&f, 10, 1_000).await,
        gone(1, 3),
        "the purge works again, on what was left"
    );
    assert_eq!(rows(&f).await, (1, 3));
}

/// The index is built the way `20261017000000_scheduler_shards` builds its own: `IF NOT EXISTS`, so an operator can build it
/// `CONCURRENTLY` first on a large database, and a leftover that is `INVALID` (a failed concurrent build, which the planner never
/// uses) is refused with a message that says what to do. Catches: a plain `CREATE INDEX` (the migration fails on an index that is
/// already there), and a migration that accepts an invalid one.
#[tokio::test]
async fn the_index_may_be_built_first_and_an_invalid_leftover_is_refused() {
    let f = fixture().await;
    let build = "CREATE INDEX batch_jobs_purge_idx ON batch_jobs (expires_at, tenant_id, id) \
                 WHERE state IN ('COMPLETED', 'CANCELLED', 'FAILED')";

    // an index an operator built first stays, and the migration applies
    undo_to(&f.w.db, previous())
        .await
        .expect("the rollback of a schema with nothing in the way");
    assert_eq!(function_and_index(&f).await, (0, 0));
    sqlx::query(build)
        .execute(&f.w.db.pool)
        .await
        .expect("an index built first");
    let built: i64 = sqlx::query_scalar("SELECT 'batch_jobs_purge_idx'::regclass::oid::bigint")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("its oid");
    migrate(&f.w.db.pool)
        .await
        .expect("the migration applies over an index that is there");
    let kept: i64 = sqlx::query_scalar("SELECT 'batch_jobs_purge_idx'::regclass::oid::bigint")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("its oid");
    assert_eq!(
        kept, built,
        "the index that was built first is not built again"
    );
    assert_eq!(function_and_index(&f).await, (1, 1));

    // an index that is invalid (a failed concurrent build) is refused, and the message says what to do
    undo_to(&f.w.db, previous())
        .await
        .expect("the rollback drops the index that was built first");
    sqlx::query(build)
        .execute(&f.w.db.pool)
        .await
        .expect("an index built first");
    sqlx::query("UPDATE pg_index SET indisvalid = false WHERE indexrelid = 'batch_jobs_purge_idx'::regclass")
        .execute(&f.w.db.pool)
        .await
        .expect("the leftover of a build that failed");
    // on a connection of its own, which releases the advisory lock of the migrator after the refusal (as `undo_to` does)
    let mut connection = sqlx::PgConnection::connect(&f.w.db.url)
        .await
        .expect("a session for the migrator");
    let refused = MIGRATOR
        .run(&mut connection)
        .await
        .expect_err("an invalid index is refused");
    sqlx::migrate::Migrate::unlock(&mut connection)
        .await
        .expect("release the migration advisory lock after the refusal");
    connection.close().await.expect("close the session");
    assert!(
        format!("{refused:?}").contains("invalid index left by a failed concurrent build"),
        "{refused:?}"
    );
    assert_eq!(
        function_and_index(&f).await.0,
        0,
        "the migration that was refused left no function"
    );

    // drop it, as the message says, and the migration applies
    sqlx::query("DROP INDEX batch_jobs_purge_idx")
        .execute(&f.w.db.pool)
        .await
        .expect("drop the leftover");
    migrate(&f.w.db.pool).await.expect("applies again");
    assert_eq!(function_and_index(&f).await, (1, 1));
}
