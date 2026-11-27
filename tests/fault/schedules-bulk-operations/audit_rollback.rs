//! An audit record exists only for an effect that committed (`T018-071`, `FR-018-044`, `NFR-018-006`, `G2031`): the record
//! of a schedule change, of the creation and the cancellation of a batch job or a backfill, of a trigger-now, of a step of
//! the pass, of the pause a revoked authority makes and of the effect of a batch item is written in the transaction of the
//! effect, so an operation killed between two of its statements leaves no audit record and no other trace, and sent again
//! leaves exactly one; an operation the platform refuses after it began (a stale revision, a name in use, a limit) leaves
//! none either. The connection is killed inside PostgreSQL after each writing statement in turn (`tripwire.rs`), as in the
//! other crash matrices of this suite; the digest compared is that of every table, `schedules` and `audit_records`
//! included, except the tables a cut-off try writes again on purpose in a transaction of its own (the retry bookkeeping of
//! a refused admission or a deferred item, [`Matrix::bookkeeping`]). The reset request has its own matrix
//! (`cancel_source.rs`, `reset.rs`), which compares the same digests. An operation that has a key (the creation of a
//! batch job, a trigger-now) is sent a third time, and answers the first.
//!
//! A time the pass skips (`SKIP` next to an open run) is recorded by its action row alone: the step writes no audit record,
//! and the matrix says so (`records: 0`), so that a record added to it later has to be added in the transaction.
//!
//! **Not covered here**: the handover command (outside, D2013). Real PostgreSQL, the real services.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{ActionView, BackfillRequest, ScheduleRecord};
use sagat_domain::schedules::{Note, Overlap};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::batch::{create, runs_started, service};
use crate::support::digests::{all_table_digests, changed};
use crate::support::schedules::{Fixture, fixture, fixture_from, hourly, some_hours_ago, t0};
use crate::tripwire::Tripwire;

/// The audit records of `action`, or all of them for `None`.
async fn audits(f: &Fixture, action: Option<&str>) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE $1::text IS NULL OR action = $1")
        .bind(action)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the audit trail")
}

/// What a matrix checks of its operation.
#[derive(Clone, Copy)]
struct Matrix {
    /// What the messages call the operation.
    name: &'static str,
    /// The action of the audit record the effect leaves; `None` counts every record (an effect that is not audited).
    action: Option<&'static str>,
    /// How many records of `action` one effect leaves.
    records: i64,
    /// The tables a cut-off try writes again, after the cut and in a transaction of its own: the retry bookkeeping of a
    /// refused admission (`schedules`: the time stays owed) or of a deferred item (`batch_items`, `batch_jobs`). They are
    /// left out of the digests; the operation itself (`run`) answers an error while its effect has not committed.
    bookkeeping: &'static [&'static str],
    /// A count of the effect (a statement with no parameter): as it was after a cut, one more once the operation is sent
    /// again, and no more after a replay of its key.
    effects: Option<&'static str>,
}

impl Matrix {
    /// An operation that leaves one record of `action` and nothing to keep after a cut.
    const fn audited(action: &'static str) -> Self {
        Self {
            name: action,
            action: Some(action),
            records: 1,
            bookkeeping: &[],
            effects: None,
        }
    }
}

/// The digests of every table but `left_out`.
async fn digests_but(f: &Fixture, left_out: &[&str]) -> BTreeMap<String, String> {
    let mut digests = all_table_digests(&f.w.db.pool).await;
    digests.retain(|table, _| !left_out.contains(&table.as_str()));
    digests
}

async fn effects(f: &Fixture, count: Option<&'static str>) -> Option<i64> {
    match count {
        None => None,
        Some(sql) => Some(
            sqlx::query_scalar(sql)
                .fetch_one(&f.w.db.pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}")),
        ),
    }
}

/// Kills the operation after each of its writing statements in turn. `prepare` makes what one try needs (a schedule to
/// change, runs to cancel); `run` is the operation. Every try is on state of its own.
async fn die_at_every_write<S, T, P, PFut, R, RFut>(
    f: &Fixture,
    action: &'static str,
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
    die_at_every_write_with(f, Matrix::audited(action), prepare, run, replay_identity).await;
}

/// [`die_at_every_write`] under the checks of `matrix`.
async fn die_at_every_write_with<S, T, P, PFut, R, RFut>(
    f: &Fixture,
    matrix: Matrix,
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
    let action = matrix.name;
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
    assert_eq!(
        tables.contains("audit_records"),
        matrix.records > 0,
        "{action} writes its audit record in its transaction ({} expected): {tables:?}",
        matrix.records
    );
    println!(
        "{action}: {} writing statements in one transaction, on {} tables",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let state = prepare(2 + *position).await;
        let digests = digests_but(f, matrix.bookkeeping).await;
        let recorded = audits(f, matrix.action).await;
        let effected = effects(f, matrix.effects).await;

        wire.arm(*position).await;
        let killed = run(state.clone()).await;
        wire.disarm().await;

        assert!(
            killed.is_err(),
            "{action}: cut off at write {position} ({table}), the operation answered {:?}",
            killed.map(|_| ())
        );
        let after = digests_but(f, matrix.bookkeeping).await;
        assert_eq!(
            after,
            digests,
            "{action}: cut off at write {position} ({table}), a table changed: {:?}",
            changed(&digests, &after)
        );
        assert_eq!(
            audits(f, matrix.action).await,
            recorded,
            "{action}: cut off at write {position} ({table}), an audit record was left for an effect that did not commit"
        );
        assert_eq!(
            effects(f, matrix.effects).await,
            effected,
            "{action}: cut off at write {position} ({table}), the effect committed"
        );

        // sent again: the effect commits once, and so does its record
        let again = run(state.clone())
            .await
            .unwrap_or_else(|e| panic!("{action}: write {position} ({table}), sent again: {e:?}"));
        assert_eq!(
            audits(f, matrix.action).await,
            recorded + matrix.records,
            "{action}: write {position} ({table}), one record for the one effect"
        );
        assert_eq!(
            effects(f, matrix.effects).await,
            effected.map(|n| n + 1),
            "{action}: write {position} ({table}), the effect commits once"
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
                audits(f, matrix.action).await,
                recorded + matrix.records,
                "{action}: write {position} ({table}), a replay leaves no second record"
            );
            assert_eq!(
                effects(f, matrix.effects).await,
                effected.map(|n| n + 1),
                "{action}: write {position} ({table}), a replay has no second effect"
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

// --- the pass (G2031) -------------------------------------------------------------------------------------------------------

/// A schedule created at `start`, its first time due: what one try of the pass acts on.
async fn due_schedule(
    f: &Fixture,
    start: OffsetDateTime,
    name: &str,
    policies: &serde_json::Value,
    target: Option<serde_json::Value>,
) -> ScheduleRecord {
    f.clock.set(start);
    let schedule = f.create(name, &hourly(), policies, target).await;
    f.clock
        .set(start + Duration::hours(1) + Duration::seconds(5));
    schedule
}

/// One step of the pass on `schedule`, as the next pass takes it: the clock moves on six minutes first, past the backoff a
/// cut-off try asked for (a refused admission keeps its time owed for 5 seconds to 5 minutes, `retry_delay`). A step that
/// did not act is an error: its time is still owed.
async fn pass_step(f: &Fixture, schedule: &ScheduleRecord) -> Result<(), AppError> {
    f.clock.advance(Duration::minutes(6));
    match f.firing.fire(&f.due(schedule), None).await? {
        Fired::Acted => Ok(()),
        Fired::Refused(reason) => Err(AppError::StateConflict(reason)),
        Fired::Idle => Err(AppError::StateConflict("the pass found nothing due")),
    }
}

#[tokio::test]
async fn a_time_the_pass_starts_that_dies_at_any_write_leaves_no_record_and_the_next_pass_leaves_one()
 {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a time the pass starts",
            action: Some("run.started"),
            records: 1,
            // a refused admission keeps the time owed: `defer_schedule`, in a transaction of its own
            bookkeeping: &["schedules"],
            effects: Some(
                "SELECT count(*) FROM schedule_actions WHERE kind = 'SCHEDULED' AND outcome = 'STARTED'",
            ),
        },
        |n| async move {
            due_schedule(fx, start, &format!("audit-pass-start-{n}"), &json!({}), None).await
        },
        move |schedule| async move { pass_step(fx, &schedule).await },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_time_the_pass_skips_that_dies_at_any_write_leaves_nothing_and_the_next_pass_skips_it_once()
 {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a time the pass skips",
            // the skip is recorded by its action row: no audit record of any kind
            action: None,
            records: 0,
            bookkeeping: &[],
            effects: Some(
                "SELECT count(*) FROM schedule_actions WHERE kind = 'SCHEDULED' AND outcome = 'SKIPPED_OVERLAP'",
            ),
        },
        |n| async move {
            let schedule = due_schedule(
                fx,
                start,
                &format!("audit-pass-skip-{n}"),
                &json!({"overlap": "SKIP"}),
                None,
            )
            .await;
            // the run of the first time is open when the second falls due
            fx.drain(&schedule).await;
            fx.clock
                .set(start + Duration::hours(2) + Duration::seconds(5));
            schedule
        },
        move |schedule| async move { pass_step(fx, &schedule).await },
        None,
    )
    .await;
}

/// A person's open run holds the business id the action of `name` takes at `nominal` (`FAIL`).
async fn hold_the_id(f: &Fixture, name: &str, nominal: OffsetDateTime) {
    use sagat_application::runs::{BusinessStart, StartOptions, StartRun};
    use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};

    let id = format!(
        "{name}-{}",
        nominal
            .format(&time::format_description::well_known::Rfc3339)
            .expect("a time")
    );
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow: f.workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key: &format!("holder-{name}"),
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(&id).expect("an identifier"),
                    conflict: ConflictPolicy::Fail,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                ..StartOptions::default()
            },
            &ctx(),
        )
        .await
        .expect("the holder of the id");
}

#[tokio::test]
async fn a_time_whose_business_id_is_held_that_dies_at_any_write_leaves_no_record_and_the_next_pass_leaves_one()
 {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "the decision a held business id makes",
            action: Some("business_id.conflict"),
            records: 1,
            // the decision is recorded inside the start, whose refusal keeps the time owed
            bookkeeping: &["schedules"],
            effects: Some(
                "SELECT count(*) FROM schedule_actions WHERE outcome = 'BUSINESS_ID_CONFLICT'",
            ),
        },
        |n| async move {
            let name = format!("audit-pass-held-{n}");
            let target = json!({
                "workflowId": fx.workflow.to_string(),
                "businessIdPrefix": name.as_str(),
                "businessIdConflictPolicy": "FAIL"
            });
            let schedule = due_schedule(fx, start, &name, &json!({}), Some(target)).await;
            hold_the_id(fx, &name, start + Duration::hours(1)).await;
            schedule
        },
        move |schedule| async move { pass_step(fx, &schedule).await },
        None,
    )
    .await;
}

#[tokio::test]
async fn the_pause_a_revoked_authority_makes_that_dies_at_any_write_leaves_no_record_and_the_next_pass_leaves_one()
 {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "the pause of a revoked authority",
            action: Some("schedule.auto_paused"),
            records: 1,
            bookkeeping: &[],
            // the action AUTHORITY_REVOKED and the pause, on the same schedule: both commit with the record or none does
            effects: Some(
                "SELECT count(*) FROM schedules s JOIN schedule_actions a ON a.schedule_id = s.id \
                 WHERE s.state = 'PAUSED' AND a.outcome = 'AUTHORITY_REVOKED'",
            ),
        },
        |n| async move {
            let author =
                fx.w.member(
                    &format!("audit-revoked-{n}"),
                    sagat_domain::identity::Role::Developer,
                )
                .await;
            fx.clock.set(start);
            let schedule = fx
                .create_as(
                    &author,
                    &format!("audit-pass-revoked-{n}"),
                    &hourly(),
                    &json!({}),
                    None,
                )
                .await;
            sqlx::query(
                "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2",
            )
            .bind(schedule.tenant_id.as_uuid())
            .bind(&schedule.authorized_by)
            .execute(&fx.w.db.pool)
            .await
            .expect("the author can no longer start runs");
            fx.clock
                .set(start + Duration::hours(1) + Duration::seconds(5));
            schedule
        },
        move |schedule| async move { pass_step(fx, &schedule).await },
        None,
    )
    .await;
}

// --- trigger-now (G2031) ----------------------------------------------------------------------------------------------------

async fn trigger_now(
    f: &Fixture,
    schedule: &ScheduleRecord,
    overlap: Option<Overlap>,
    key: &str,
) -> Result<ActionView, AppError> {
    f.firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest { overlap, key },
            &ctx(),
        )
        .await
}

#[tokio::test]
async fn a_trigger_now_that_starts_a_run_and_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one()
 {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a trigger-now that starts a run",
            effects: Some(
                "SELECT count(*) FROM schedule_actions WHERE kind = 'TRIGGERED' AND outcome = 'STARTED'",
            ),
            ..Matrix::audited("schedule.triggered")
        },
        |n| async move {
            let schedule = fx
                .create(&format!("audit-trigger-start-{n}"), &hourly(), &json!({}), None)
                .await;
            (schedule, format!("audit-trigger-start-key-{n:04}"))
        },
        move |(schedule, key): (ScheduleRecord, String)| async move {
            trigger_now(fx, &schedule, None, &key).await
        },
        Some(|action: &ActionView| {
            format!("{} {} {:?}", action.nominal_at, action.outcome, action.run_id)
        }),
    )
    .await;
}

#[tokio::test]
async fn a_trigger_now_that_is_skipped_and_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one()
 {
    let f = fixture().await;
    let fx = &f;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a trigger-now that is skipped",
            effects: Some(
                "SELECT count(*) FROM schedule_actions WHERE kind = 'TRIGGERED' AND outcome = 'SKIPPED_OVERLAP'",
            ),
            ..Matrix::audited("schedule.triggered")
        },
        |n| async move {
            let schedule = fx
                .create(
                    &format!("audit-trigger-skip-{n}"),
                    &hourly(),
                    &json!({"overlap": "SKIP"}),
                    None,
                )
                .await;
            // a run of the schedule is open: one asked for now under ALLOW_ALL
            let open = trigger_now(
                fx,
                &schedule,
                Some(Overlap::AllowAll),
                &format!("audit-trigger-open-key-{n:04}"),
            )
            .await
            .expect("an open run");
            assert_eq!(open.outcome, "STARTED");
            // the action under test is asked for a second later: another instant, another action
            fx.clock.advance(Duration::seconds(1));
            (schedule, format!("audit-trigger-skip-key-{n:04}"))
        },
        move |(schedule, key): (ScheduleRecord, String)| async move {
            trigger_now(fx, &schedule, None, &key).await
        },
        Some(|action: &ActionView| {
            format!("{} {} {:?}", action.nominal_at, action.outcome, action.run_id)
        }),
    )
    .await;
}

// --- backfills (G2031) ------------------------------------------------------------------------------------------------------

/// Room for the backfills a matrix leaves in progress: the deployment ceiling first, then the limit of the namespace.
async fn room_for_backfills(f: &Fixture, n: i64) {
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'schedule_backfills', $2, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'schedule_backfills', $3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(n)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the backfills");
}

/// Five hours before the clock of the fixture: five times of an hourly schedule.
fn five_hours() -> BackfillRequest {
    BackfillRequest {
        start: t0() - Duration::hours(5),
        end: t0(),
        overlap: None,
    }
}

#[tokio::test]
async fn a_backfill_requested_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    room_for_backfills(fx, 64).await;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a backfill requested",
            effects: Some("SELECT count(*) FROM schedule_backfills"),
            ..Matrix::audited("schedule.backfill_requested")
        },
        |n| async move {
            fx.create(
                &format!("audit-backfill-request-{n}"),
                &hourly(),
                &json!({}),
                None,
            )
            .await
        },
        move |schedule| async move {
            fx.service
                .create_backfill(&fx.w.admin, schedule.id, &five_hours(), &ctx())
                .await
        },
        None,
    )
    .await;
}

#[tokio::test]
async fn a_backfill_cancelled_that_dies_at_any_write_leaves_no_record_and_sent_again_leaves_one() {
    let f = fixture().await;
    let fx = &f;
    room_for_backfills(fx, 64).await;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "a backfill cancelled",
            effects: Some("SELECT count(*) FROM schedule_backfills WHERE state = 'CANCELLED'"),
            ..Matrix::audited("schedule.backfill_cancelled")
        },
        |n| async move {
            let schedule = fx
                .create(
                    &format!("audit-backfill-cancel-{n}"),
                    &hourly(),
                    &json!({}),
                    None,
                )
                .await;
            let backfill = fx
                .service
                .create_backfill(&fx.w.admin, schedule.id, &five_hours(), &ctx())
                .await
                .expect("the backfill");
            (schedule, backfill)
        },
        move |(schedule, backfill): (
            ScheduleRecord,
            sagat_application::schedules::BackfillRecord,
        )| async move {
            fx.service
                .cancel_backfill(&fx.w.admin, schedule.id, backfill.id, &ctx())
                .await
        },
        None,
    )
    .await;
}

// --- the item of a batch job (G2031) ----------------------------------------------------------------------------------------

/// One step of the executor on the one item of `job`, as the next step takes it: ten seconds later, past the backoff of an
/// item a cut-off try deferred (5 seconds after its first refusal). It is an error until the item is recorded with its
/// effect: a cut-off try answers `Acted` for the deferral it records in a transaction of its own.
async fn item_step(f: &Fixture, job: &sagat_application::batch::JobRecord) -> Result<(), AppError> {
    use crate::batch_item_transaction::{due, executor, item_state};

    f.clock.advance(Duration::seconds(10));
    executor(f).step(&due(job), None).await?;
    match item_state(f, job.id).await.as_str() {
        "SUCCEEDED" => Ok(()),
        _ => Err(AppError::StateConflict(
            "the item is not recorded with its effect",
        )),
    }
}

#[tokio::test]
async fn the_effect_of_a_batch_item_that_dies_at_any_write_leaves_no_record_and_the_next_step_leaves_one()
 {
    let f = fixture().await;
    let fx = &f;
    room_for_jobs(fx, 64).await;
    die_at_every_write_with(
        fx,
        Matrix {
            name: "the cancel of a batch item",
            action: Some("run.cancel_requested"),
            records: 1,
            // a cut-off effect is a refusal the executor records as a deferral of the item
            bookkeeping: &["batch_items", "batch_jobs"],
            effects: Some("SELECT count(*) FROM batch_items WHERE state = 'SUCCEEDED'"),
        },
        |n| async move {
            crate::batch_item_transaction::one_item_job(
                fx,
                &format!("audit-batch-item-key-{n:04}"),
                None,
            )
            .await
            .0
        },
        move |job| async move { item_step(fx, &job).await },
        None,
    )
    .await;
}
