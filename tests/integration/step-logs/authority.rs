//! Design 041 section 2, task T041-6: the session (T3), the lease (T2) and the authority (T7) of a batch. Each refused
//! batch stores nothing and gets its verdict, with `stored_through_sequence` 0; a caller whose binding or membership
//! is gone gets `AUTHORITY` whatever else was due (D041-050). The audit of an `AUTHORITY` answer is the gRPC handler's
//! (task T041-7, decision D041-146 (2)); the cross-tenant audit of a `LEASE` answer is the ingest's.

use std::time::Duration;

use sagat_application::runs::RunOperation;
use sagat_application::step_logs::{BatchAnswer, StepLogVerdict};
use sagat_application::worker::TaskKind;
use sagat_domain::execution::RunId;
use sagat_domain::identity::Role;
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::StopReason;
use serde_json::json;
use sqlx::Connection as _;

use crate::support::bounded;
use crate::support::engine::{World, document, effect_node, saga_node, world};
use crate::support::step_logs::{
    Attempt, accept_at_1_7, batch, ingest, leased, lines_of, member_session, records, send,
    set_namespace_policy, stream_of,
};

const LIMIT: Duration = Duration::from_secs(120);

fn stopped(reason: StopReason) -> BatchAnswer {
    BatchAnswer {
        verdict: if reason.stops_capture() {
            StepLogVerdict::StopCapture(reason)
        } else {
            StepLogVerdict::StopAttempt(reason)
        },
        stored_through_sequence: 0,
    }
}

async fn owner(w: &World, statement: &str, a: &Attempt) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
        .bind(w.admin.tenant_id().as_uuid())
        .bind(a.session.principal.subject().as_str())
        .execute(&w.db.pool)
        .await
        .expect("the owner's change");
}

async fn nothing_stored(w: &World, a: &Attempt) {
    assert!(lines_of(w, a).await.is_empty(), "no line is stored");
    assert_eq!(stream_of(w, a).await, None, "no stream is written");
}

/// T3: an expired session, a session of another namespace, of another subject, below 1.7, and a token that is not
/// the session's (replaced by a registration): each `STOP_CAPTURE` (`SESSION`) and nothing stored.
#[tokio::test]
async fn a_session_that_is_not_this_callers_live_1_7_session_stops_capture() {
    bounded::within("sessions", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        let session_id = |a: &Attempt| a.session.worker_id;
        // expired
        let a = leased(&w, "a").await;
        sqlx::query(
            "UPDATE worker_sessions SET expires_at = now() - interval '1 second' WHERE id = $1",
        )
        .bind(session_id(&a))
        .execute(&w.db.pool)
        .await
        .expect("expire");
        assert_eq!(
            send(&ingest, &a, records([1])).await,
            stopped(StopReason::Session)
        );
        nothing_stored(&w, &a).await;
        // another namespace of the tenant
        let a = leased(&w, "b").await;
        let elsewhere = w.add_namespace("elsewhere").await;
        sqlx::query("UPDATE worker_sessions SET namespace_id = $2 WHERE id = $1")
            .bind(session_id(&a))
            .bind(elsewhere.id().as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("move");
        assert_eq!(
            send(&ingest, &a, records([1])).await,
            stopped(StopReason::Session)
        );
        nothing_stored(&w, &a).await;
        // another subject's session, presented with its own token
        let a = leased(&w, "c").await;
        let other = member_session(&w, "worker-two", Role::Worker, "act.c").await;
        let mut theirs = batch(&a, records([1]));
        theirs.worker_id = other.worker_id;
        theirs.session_token = other.token.clone();
        assert_eq!(
            ingest
                .append(&a.session.principal, &theirs)
                .await
                .expect("answered"),
            stopped(StopReason::Session)
        );
        nothing_stored(&w, &a).await;
        // below 1.7
        let a = leased(&w, "d").await;
        sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 6 WHERE id = $1")
            .bind(session_id(&a))
            .execute(&w.db.pool)
            .await
            .expect("1.6");
        assert_eq!(
            send(&ingest, &a, records([1])).await,
            stopped(StopReason::Session)
        );
        nothing_stored(&w, &a).await;
        // a token that is not the session's (a re-registration replaced it)
        let a = leased(&w, "e").await;
        let mut replaced = batch(&a, records([1]));
        replaced.session_token = "the-token-before-the-registration".into();
        assert_eq!(
            ingest
                .append(&a.session.principal, &replaced)
                .await
                .expect("answered"),
            stopped(StopReason::Session)
        );
        nothing_stored(&w, &a).await;
    })
    .await;
}

/// T7: the binding removed, the membership revoked (a role without `worker:execute`), and a membership whose role is
/// runner or database-runner: each `STOP_ATTEMPT` (`AUTHORITY`), nothing stored.
#[tokio::test]
async fn a_caller_whose_binding_or_membership_no_longer_covers_the_step_gets_authority() {
    bounded::within("authority", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        for (node, change) in [
            ("a", "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = $2"),
            ("b", "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2"),
            ("c", "UPDATE memberships SET role = 'runner' WHERE tenant_id = $1 AND subject = $2"),
            ("d", "UPDATE memberships SET role = 'database-runner' WHERE tenant_id = $1 AND subject = $2"),
        ] {
            let name = format!("worker-{node}");
            let session = member_session(&w, &name, Role::Worker, &format!("act.{node}")).await;
            let workflow = crate::support::step_logs::one_activity(&w, node).await;
            let run = w.start(workflow, &json!({})).await;
            let task = w.one_task(&session).await;
            let a = Attempt { session, task, run };
            owner(&w, change, &a).await;
            assert_eq!(
                send(&ingest, &a, records([1])).await,
                stopped(StopReason::Authority),
                "{change}"
            );
            nothing_stored(&w, &a).await;
        }
    })
    .await;
}

/// D041-050: a caller whose binding is gone learns only `AUTHORITY`, even where `SESSION`, `CAPTURE_OFF`, `REMOVED`
/// or `FINAL_RECORDED` was due; and `stored_through_sequence` is 0 with `AUTHORITY` and `SESSION`.
#[tokio::test]
async fn authority_is_answered_whatever_else_was_due() {
    bounded::within("authority first", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        let unbind = "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = $2";
        // SESSION due
        let session = member_session(&w, "w-session", Role::Worker, "act.a").await;
        let run = w
            .start(
                crate::support::step_logs::one_activity(&w, "a").await,
                &json!({}),
            )
            .await;
        let task = w.one_task(&session).await;
        let a = Attempt { session, task, run };
        sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 6 WHERE id = $1")
            .bind(a.session.worker_id)
            .execute(&w.db.pool)
            .await
            .expect("1.6");
        owner(&w, unbind, &a).await;
        assert_eq!(
            send(&ingest, &a, records([1])).await,
            stopped(StopReason::Authority)
        );
        // REMOVED due: a stream stored, then removed, then the binding removed
        let session = member_session(&w, "w-removed", Role::Worker, "act.b").await;
        let run = w
            .start(
                crate::support::step_logs::one_activity(&w, "b").await,
                &json!({}),
            )
            .await;
        let task = w.one_task(&session).await;
        let b = Attempt { session, task, run };
        send(&ingest, &b, records([1])).await;
        sqlx::query("UPDATE step_log_streams SET removed_at = now() WHERE run_id = $1")
            .bind(b.run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("removed");
        owner(&w, unbind, &b).await;
        assert_eq!(
            send(&ingest, &b, records([2])).await,
            stopped(StopReason::Authority)
        );
        // CAPTURE_OFF due
        let session = member_session(&w, "w-off", Role::Worker, "act.c").await;
        let run = w
            .start(
                crate::support::step_logs::one_activity(&w, "c").await,
                &json!({}),
            )
            .await;
        let task = w.one_task(&session).await;
        let c = Attempt { session, task, run };
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 0).await;
        owner(&w, unbind, &c).await;
        assert_eq!(
            send(&ingest, &c, records([1])).await,
            stopped(StopReason::Authority)
        );
        nothing_stored(&w, &c).await;
    })
    .await;
}

/// T2: a valid task lease sent with another run's id is `LEASE`, and a run of another tenant is audited as
/// `access.denied.cross_tenant`; nothing is stored.
#[tokio::test]
async fn a_lease_sent_with_another_runs_id_is_lease_and_a_foreign_run_is_audited() {
    bounded::within("lease", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        let a = leased(&w, "a").await;
        let mine = w.start(crate::support::step_logs::one_activity(&w, "x").await, &json!({})).await;
        let mut wrong = batch(&a, records([1]));
        wrong.run_id = mine;
        assert_eq!(
            ingest.append(&a.session.principal, &wrong).await.expect("answered"),
            stopped(StopReason::Lease)
        );
        let foreign_workflow = w.publish_as_other_tenant(document(
            "foreign",
            "f",
            vec![crate::support::engine::activity("f")],
        ))
        .await;
        let foreign: RunId = w
            .start_as_other_tenant(foreign_workflow, &json!({}))
            .await
            .expect("a run of the other tenant");
        let mut across = batch(&a, records([1]));
        across.run_id = foreign;
        assert_eq!(
            ingest.append(&a.session.principal, &across).await.expect("answered"),
            stopped(StopReason::Lease)
        );
        ingest.store().flush_denial_writes().await;
        let audited: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant' AND resource_id = $1",
        )
        .bind(foreign.to_string())
        .fetch_one(&w.db.pool)
        .await
        .expect("the audit");
        assert_eq!(audited, 1);
        let unaudited: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant' AND resource_id = $1",
        )
        .bind(mine.to_string())
        .fetch_one(&w.db.pool)
        .await
        .expect("the audit");
        assert_eq!(unaudited, 0, "a run of the caller's own tenant is no cross-tenant access");
        nothing_stored(&w, &a).await;
    })
    .await;
}

/// OD-2026-11-16-11: a forward step of a stopping run is refused `LEASE` (as its report would be), and a compensation
/// step of it is stored. The forward step's `cancel_requested` is cleared by the fixture, so that only the run's
/// stopping refuses it.
#[tokio::test]
async fn a_forward_step_of_a_stopping_run_is_lease_and_its_compensation_is_stored() {
    bounded::within("stopping", LIMIT, async {
        let w = world().await;
        let (workflow, _) = w
            .publish(document(
                "settle",
                "settle",
                vec![
                    saga_node("settle", "reserve", None),
                    effect_node("reserve", Some("submit"), false),
                    effect_node("submit", None, false),
                ],
            ))
            .await;
        let session = w
            .session(&["act.reserve", "act.submit", "undo.reserve"])
            .await;
        accept_at_1_7(&w, &session).await;
        let run = w.start(workflow, &json!({})).await;
        let reserve = w.one_task(&session).await;
        w.complete(&session, &reserve, &json!({}))
            .await
            .expect("reserve");
        let submit = w.one_task(&session).await;
        w.operate_run(run, RunOperation::Cancel).await;
        sqlx::query(
            "UPDATE steps SET cancel_requested = false WHERE run_id = $1 AND node = 'submit'",
        )
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("only the run is stopping");
        let ingest = ingest(&w);
        let forward = Attempt {
            session: crate::support::engine::Session {
                principal: session.principal.clone(),
                worker_id: session.worker_id,
                token: session.token.clone(),
            },
            task: submit.clone(),
            run,
        };
        assert_eq!(
            send(&ingest, &forward, records([1])).await,
            stopped(StopReason::Lease)
        );
        w.fail_with(&session, &submit, "CANCELLED", true)
            .await
            .expect("the worker stops");
        let undo = w.one_task(&session).await;
        assert_eq!(undo.kind, TaskKind::Compensation);
        let compensation = Attempt {
            session,
            task: undo,
            run,
        };
        assert_eq!(
            send(&ingest, &compensation, records([1, 2])).await,
            BatchAnswer {
                verdict: StepLogVerdict::Continue,
                stored_through_sequence: 2
            }
        );
        assert_eq!(lines_of(&w, &compensation).await.len(), 2);
    })
    .await;
}

/// The security review of task T041-6: a caller whose binding is gone learns only `AUTHORITY`, even when T3 or T4
/// meets a lock it cannot take within its lock timeout (a change of a step-log resource holding the namespace fence
/// exclusively; another batch of the attempt holding its stream lock), where `RETRY_LATER` was due. A caller still
/// bound gets `RETRY_LATER` there.
#[tokio::test]
async fn authority_is_answered_when_t3_meets_a_held_lock() {
    bounded::within("authority under a held lock", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        let unbind = "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = $2";
        for (node, key) in [
            (
                "a",
                format!(
                    "quota-policy:step_log_attempt_records:{}:{}",
                    w.admin.tenant_id(),
                    w.namespace.id()
                ),
            ),
            ("b", String::new()),
        ] {
            let session = member_session(
                &w,
                &format!("w-held-{node}"),
                Role::Worker,
                &format!("act.{node}"),
            )
            .await;
            let workflow = crate::support::step_logs::one_activity(&w, node).await;
            let run = w.start(workflow, &json!({})).await;
            let task = w.one_task(&session).await;
            let a = Attempt { session, task, run };
            let key = if key.is_empty() {
                sagat_infra_postgres::stream_lock_key(
                    w.admin.tenant_id(),
                    a.task.run_id,
                    a.task.step_id,
                    a.task.fence.attempt,
                    a.task.fence.generation,
                )
            } else {
                key
            };
            let mut holder = sqlx::PgConnection::connect(&w.db.url)
                .await
                .expect("a holder");
            let mut tx = holder.begin().await.expect("a transaction");
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(&key)
                .execute(&mut *tx)
                .await
                .expect("the held lock");
            assert_eq!(
                send(&ingest, &a, records([1])).await,
                BatchAnswer {
                    verdict: StepLogVerdict::RetryLater { after_ms: 200 },
                    stored_through_sequence: 0
                },
                "{node}: still bound"
            );
            owner(&w, unbind, &a).await;
            assert_eq!(
                send(&ingest, &a, records([1])).await,
                stopped(StopReason::Authority),
                "{node}: unbound"
            );
            tx.rollback().await.expect("release");
            nothing_stored(&w, &a).await;
        }
    })
    .await;
}

/// T4: a stream whose final mark is recorded (by the attempt's carry, task T041-36; set here by the owner) answers
/// `STOP_ATTEMPT` (`FINAL_RECORDED`) with the stream's mark and stores nothing; a caller whose binding is gone gets
/// `AUTHORITY` there instead (D041-050).
#[tokio::test]
async fn a_recorded_final_mark_answers_final_recorded_with_the_mark_unless_authority_is_due() {
    bounded::within("final recorded", LIMIT, async {
        let w = world().await;
        let ingest = ingest(&w);
        let session = member_session(&w, "w-final", Role::Worker, "act.a").await;
        let run = w
            .start(
                crate::support::step_logs::one_activity(&w, "a").await,
                &json!({}),
            )
            .await;
        let task = w.one_task(&session).await;
        let a = Attempt { session, task, run };
        send(&ingest, &a, records([1, 2])).await;
        sqlx::query("UPDATE step_log_streams SET final_at = now() WHERE run_id = $1")
            .bind(a.run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("the final mark");
        assert_eq!(
            send(&ingest, &a, records([3])).await,
            BatchAnswer {
                verdict: StepLogVerdict::StopAttempt(StopReason::FinalRecorded),
                stored_through_sequence: 2
            }
        );
        assert_eq!(lines_of(&w, &a).await.len(), 2);
        assert_eq!(stream_of(&w, &a).await.expect("a stream").last_sequence, 2);
        owner(
            &w,
            "DELETE FROM worker_bindings WHERE tenant_id = $1 AND subject = $2",
            &a,
        )
        .await;
        assert_eq!(
            send(&ingest, &a, records([3])).await,
            stopped(StopReason::Authority)
        );
    })
    .await;
}
