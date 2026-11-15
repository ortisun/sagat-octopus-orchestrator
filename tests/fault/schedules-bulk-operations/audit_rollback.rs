//! An audit record exists only for an effect that committed (`T018-071`, `FR-018-044`, `NFR-018-006`): the record of a
//! schedule change, of the creation and the cancellation of a batch job is written in the transaction of the effect, so
//! an operation killed between two of its statements leaves no audit record and no other trace, and sent again leaves
//! exactly one; an operation the platform refuses after it began (a stale revision, a name in use, a limit) leaves none
//! either. The connection is killed inside PostgreSQL after each writing statement in turn (`tripwire.rs`), as in the
//! other crash matrices of this suite; the digest compared is that of every table, `schedules` and `audit_records`
//! included. The reset request has its own matrix (`cancel_source.rs`, `reset.rs`), which compares the same digests. An operation
//! that has a key (the creation of a batch job) is sent a third time, and answers the first.
//!
//! **Not covered here** (`G2031`): the firing of a schedule (its audit record `run.started` is in the crash matrix of `firing.rs`,
//! which compares the digests, but a decision with no run, such as a skip, is not checked for its record), the creation and the
//! cancellation of a backfill, trigger-now, `revoke_authority` (the audit record and the pause of the schedule in one
//! transaction), the item audit of the batch executor, and the handover command. Real PostgreSQL, the real services.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::future::Future;

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_domain::schedules::Note;
use serde_json::json;
use uuid::Uuid;

use crate::batch::{create, runs_started, service};
use crate::support::digests::all_table_digests;
use crate::support::schedules::{Fixture, fixture, hourly};
use crate::tripwire::Tripwire;

async fn audits(f: &Fixture, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = $1")
        .bind(action)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the audit trail")
}

/// Kills the operation after each of its writing statements in turn. `prepare` makes what one try needs (a schedule to
/// change, runs to cancel); `run` is the operation. Every try is on state of its own.
async fn die_at_every_write<S, T, P, PFut, R, RFut>(
    f: &Fixture,
    action: &str,
    prepare: P,
    run: R,
    replay_identity: Option<fn(&T) -> String>,
) where
    S: Clone,
    P: Fn(usize) -> PFut,
    PFut: Future<Output = S>,
    R: Fn(S) -> RFut,
    RFut: Future<Output = Result<T, AppError>>,
{
    let wire = Tripwire::install(&f.w.db.pool).await;
    // the first operation of a tenant makes writes that later ones do not; one is made first
    let warm = prepare(0).await;
    run(warm.clone())
        .await
        .unwrap_or_else(|e| panic!("{action}: warm-up: {e:?}"));

    let probe = prepare(1).await;
    wire.watch().await;
    run(probe.clone())
        .await
        .unwrap_or_else(|e| panic!("{action}: probe: {e:?}"));
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "{action} is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    assert!(
        tables.contains("audit_records"),
        "{action} writes its audit record: {tables:?}"
    );
    println!(
        "{action}: {} writing statements in one transaction, on {} tables",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let state = prepare(2 + *position).await;
        let digests = all_table_digests(&f.w.db.pool).await;
        let recorded = audits(f, action).await;

        wire.arm(*position).await;
        let killed = run(state.clone()).await;
        wire.disarm().await;

        assert!(
            killed.is_err(),
            "{action}: cut off at write {position} ({table}), the operation answered {:?}",
            killed.map(|_| ())
        );
        assert_eq!(
            all_table_digests(&f.w.db.pool).await,
            digests,
            "{action}: cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(
            audits(f, action).await,
            recorded,
            "{action}: cut off at write {position} ({table}), an audit record was left for an effect that did not commit"
        );

        // sent again: the effect commits once, and so does its record
        let again = run(state.clone())
            .await
            .unwrap_or_else(|e| panic!("{action}: write {position} ({table}), sent again: {e:?}"));
        assert_eq!(
            audits(f, action).await,
            recorded + 1,
            "{action}: write {position} ({table}), one record for the one effect"
        );

        // and a third time, for an operation with a key: the answer of the first, and no second record
        if let Some(identity) = replay_identity {
            let third = run(state.clone()).await.unwrap_or_else(|e| {
                panic!("{action}: write {position} ({table}), a replay of the key: {e:?}")
            });
            assert_eq!(
                identity(&third),
                identity(&again),
                "{action}: write {position} ({table}), the replay answers the first"
            );
            assert_eq!(
                audits(f, action).await,
                recorded + 1,
                "{action}: write {position} ({table}), a replay leaves no second record"
            );
        }
    }
}

/// Room for the jobs a matrix leaves in progress: the deployment ceiling first, then the limit of the namespace.
async fn room_for_jobs(f: &Fixture, n: i64) {
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'batch_jobs', $2, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', $3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs");
    // the active jobs of the tenant are limited too: its limit and its ceiling
    for statement in [
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'tenant_batch_jobs', $2, 'operator')",
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_batch_jobs', $2, 'admin')",
    ] {
        sqlx::query(statement)
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(n)
            .execute(&f.w.db.pool)
            .await
            .expect("room for the jobs of the tenant");
    }
}

fn ctx() -> RequestContext {
    RequestContext::default()
}

// --- schedule changes ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_schedule_created_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write(
        fx,
        "schedule.created",
        |n| async move { format!("audit-create-{n}") },
        move |name| async move {
            fx.try_create(fx.write_of(&name, &hourly(), &json!({}), None))
                .await
        },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_schedule_updated_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write(
        fx,
        "schedule.updated",
        |n| async move {
            fx.create(&format!("audit-update-{n}"), &hourly(), &json!({}), None)
                .await
        },
        move |schedule| async move {
            fx.service
                .update(
                    &fx.w.admin,
                    schedule.id,
                    fx.write_of(
                        &format!("{:?}-renamed", schedule.id.as_uuid()),
                        &hourly(),
                        &json!({"overlap": "SKIP"}),
                        None,
                    ),
                    schedule.revision,
                    &ctx(),
                )
                .await
        },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_schedule_paused_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write(
        fx,
        "schedule.paused",
        |n| async move {
            fx.create(&format!("audit-pause-{n}"), &hourly(), &json!({}), None)
                .await
        },
        move |schedule| async move {
            fx.service
                .pause(
                    &fx.w.admin,
                    schedule.id,
                    Note::new("freeze").expect("note"),
                    &ctx(),
                )
                .await
        },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_schedule_resumed_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write(
        fx,
        "schedule.resumed",
        |n| async move {
            let schedule = fx
                .create(&format!("audit-resume-{n}"), &hourly(), &json!({}), None)
                .await;
            fx.service
                .pause(
                    &fx.w.admin,
                    schedule.id,
                    Note::new("freeze").expect("note"),
                    &ctx(),
                )
                .await
                .expect("pause");
            schedule
        },
        move |schedule| async move {
            fx.service
                .resume(
                    &fx.w.admin,
                    schedule.id,
                    Note::new("thaw").expect("note"),
                    &ctx(),
                )
                .await
        },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_schedule_deleted_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write(
        fx,
        "schedule.deleted",
        |n| async move {
            fx.create(&format!("audit-delete-{n}"), &hourly(), &json!({}), None)
                .await
        },
        move |schedule| async move {
            fx.service
                .delete(&fx.w.admin, schedule.id, Some(schedule.revision), &ctx())
                .await
        },
        None,
    )
    .await;
}

// --- batch jobs -----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_batch_job_created_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    room_for_jobs(fx, 64).await;
    let runs = runs_started(fx, 2).await;
    die_at_every_write(
        fx,
        "batch.created",
        |n| async move { format!("audit-batch-create-key-{n:04}") },
        |key| {
            let runs = runs.clone();
            async move {
                // the helper that creates the job expects the creation to succeed: the refusal of a killed try is
                // read from the service instead
                crate::batch::try_create(fx, &runs, &key).await
            }
        },
        Some(|job: &sagat_application::batch::JobRecord| job.id.to_string()),
    )
    .await;
}

#[tokio::test]
async fn a_batch_job_cancelled_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    room_for_jobs(fx, 64).await;
    let runs = runs_started(fx, 2).await;
    die_at_every_write(
        fx,
        "batch.cancelled",
        |n| {
            let runs = runs.clone();
            async move {
                create(
                    fx,
                    &fx.w.admin,
                    &runs,
                    &format!("audit-batch-cancel-key-{n:04}"),
                )
                .await
            }
        },
        move |job| async move {
            service(fx)
                .cancel(
                    &fx.w.admin,
                    job.id,
                    "stop the clean-up of the stuck runs",
                    &ctx(),
                )
                .await
        },
        None,
    )
    .await;
}

// --- an operation the platform refuses after it began ----------------------------------------------------------------------

#[tokio::test]
async fn an_operation_the_platform_refuses_leaves_no_audit_record_and_no_other_trace() {
    let f = fixture().await;
    let schedule = f.create("audit-refused", &hourly(), &json!({}), None).await;
    let other = f
        .create("audit-refused-name", &hourly(), &json!({}), None)
        .await;
    let runs = runs_started(&f, 1).await;
    // a limit of zero for the batch jobs of the namespace: the one change made on purpose, before the digests are taken
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of zero");
    let digests = all_table_digests(&f.w.db.pool).await;
    let audited = audits_in_all(&f).await;

    // a revision that is not the current one
    let stale = f
        .service
        .update(
            &f.w.admin,
            schedule.id,
            f.write_of("audit-refused", &hourly(), &json!({}), None),
            schedule.revision + 7,
            &ctx(),
        )
        .await;
    assert!(
        matches!(stale, Err(AppError::RevisionConflict { .. })),
        "a stale revision is a revision conflict: {stale:?}"
    );

    // a name another schedule holds
    let taken_name = sqlx::query_scalar::<_, String>("SELECT name FROM schedules WHERE id = $1")
        .bind(other.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the name");
    let taken = f
        .try_create(f.write_of(&taken_name, &hourly(), &json!({}), None))
        .await;
    assert!(
        matches!(taken, Err(AppError::Conflict(_))),
        "a name in use is a conflict: {taken:?}"
    );

    // a schedule that is not there
    let missing = f
        .service
        .pause(
            &f.w.admin,
            sagat_domain::schedules::ScheduleId::new(Uuid::now_v7()),
            Note::new("freeze").expect("note"),
            &ctx(),
        )
        .await;
    eprintln!("DEBUG {stale:?} / {taken:?} / {missing:?}");
    assert!(
        matches!(missing, Err(AppError::NotFound)),
        "a schedule that does not exist is not found: {missing:?}"
    );

    // a limit of zero: the job is refused before it is written
    let refused = service(&f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            crate::batch::write_of(&runs),
            "audit-refused-batch-key",
            &ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "a limit of zero refuses: {:?}",
        refused.map(|_| ())
    );

    assert_eq!(
        all_table_digests(&f.w.db.pool).await,
        digests,
        "what the platform refused changed a table"
    );
    assert_eq!(
        audits_in_all(&f).await,
        audited,
        "what the platform refused left an audit record"
    );
}

async fn audits_in_all(f: &Fixture) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the audit trail")
}
