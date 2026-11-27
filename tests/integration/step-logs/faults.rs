//! Design 041, task T041-8: the server's fault and security cases of the ingest, called directly (`StepLogIngest`)
//! against a real PostgreSQL, with barriers and held locks, never with sleeps. The cases through the gRPC boundary (a
//! lost answer, the cross-tenant audit, the per-tenant tickets, a restart) are in `rpc_faults`. A crash before the
//! commit and between the commit and the answer, at the ingest, are `ingest::a_lost_answer_is_resent_and_a_failed_commit_is_resent`
//! (task T041-6).
//!
//! The server canary: the server does not redact by value (FR-041-007: value redaction is the SDK's, the server's is by
//! shape and by sensitive field name), so the server's own canary is a secret the attempt resolved whose value has a
//! shape the server scrubs (a connection string's password), logged whole by a client that skips its own redaction (the
//! risk row "a hand-written 1.7 client skips redaction"); the canary is the password, which must reach no stored row,
//! no journal event, no audit record and no line of the captured `tracing` output. The unshaped canary is the six SDKs'
//! (`step-logs-redaction`, task T041-14).

use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::step_logs::{BatchAnswer, StepLogRecordIn, StepLogVerdict};
use sagat_application::worker::{
    UndeliveredLease, WithdrawalOutcome, WorkerService, WorkerSettings,
};
use sagat_domain::execution::WithdrawalCause;
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::{FieldValue, StopReason};
use sagat_infra_ir::JsonEvaluator;
use serde_json::json;

use crate::support::bounded;
use crate::support::engine::{
    Session, World, activity, document, flaky, parallel_node, task_ref, with, world,
};
use crate::support::step_logs::{
    Attempt, Hook, accept_at_1_7, batch, hooked, hooked_on, ingest, leased, lines_of, pool_of,
    record, records, send, set_namespace_policy, set_tenant_limit, spawn_send, step_log_usage,
    stream_of,
};

const LIMIT: Duration = Duration::from_secs(120);
/// The per-row charge of the tenant quota (OD-2026-11-15-6).
const ROW: i64 = 128;
/// The first sequence of the server's markers, 2^62.
const MARKER: i64 = 1 << 62;

const RETRY: BatchAnswer = BatchAnswer {
    verdict: StepLogVerdict::RetryLater { after_ms: 200 },
    stored_through_sequence: 0,
};

fn continued(through: u64) -> BatchAnswer {
    BatchAnswer {
        verdict: StepLogVerdict::Continue,
        stored_through_sequence: through,
    }
}

fn final_only(through: u64) -> BatchAnswer {
    BatchAnswer {
        verdict: StepLogVerdict::FinalOnly,
        stored_through_sequence: through,
    }
}

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

/// The same session again (a session is not `Clone`: one worker's handle).
fn same(session: &Session) -> Session {
    Session {
        principal: session.principal.clone(),
        worker_id: session.worker_id,
        token: session.token.clone(),
    }
}

/// Every stored step-log row of the database, lines and streams, as text: what a later read of the tables would see.
async fn every_step_log_row(w: &World) -> String {
    sqlx::query_scalar(
        "SELECT coalesce(string_agg(t, E'\\n' ORDER BY t), '') FROM ( \
            SELECT l::text AS t FROM step_log_lines l UNION ALL SELECT s::text FROM step_log_streams s) rows",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the step-log rows")
}

/// Two identical batches at once: the second meets the first's stream lock (held after T3) and is told to retry
/// (`RETRY_LATER`, the 200 ms lock timeout), storing nothing; once the first has committed, the resend of the second
/// stores nothing more and is answered as the first was.
#[tokio::test]
async fn the_same_batch_twice_at_once_is_retried_and_its_resend_stores_nothing_more() {
    bounded::within("twice at once", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let (held, gate) = hooked(&w, Hook::HoldAfterPolicies);
        let first = spawn_send(&held, &a, records([1, 2]));
        gate.reached().await;
        let plain = ingest(&w);
        assert_eq!(send(&plain, &a, records([1, 2])).await, RETRY);
        assert_eq!(stream_of(&w, &a).await, None, "the second stored nothing");
        gate.release();
        assert_eq!(
            first.await.expect("the first").expect("answered"),
            continued(2)
        );
        assert_eq!(send(&plain, &a, records([1, 2])).await, continued(2));
        assert_eq!(lines_of(&w, &a).await.len(), 2);
        let stream = stream_of(&w, &a).await.expect("the stream");
        assert_eq!((stream.records, stream.last_sequence), (2, 2));
    })
    .await;
}

/// T2: the lease of an earlier attempt (the step failed and was leased again) is `LEASE`, and nothing of it is
/// stored; the current attempt's batch is stored in its own stream.
#[tokio::test]
async fn a_stale_task_lease_is_lease_and_stores_nothing() {
    bounded::within("stale lease", LIMIT, async {
        let w = world().await;
        let workflow = w
            .publish(document("logs-stale", "a", vec![flaky("a", 3)]))
            .await
            .0;
        let session = w.session(&["act.a"]).await;
        accept_at_1_7(&w, &session).await;
        let run = w.start(workflow, &json!({})).await;
        let first = w.one_task(&session).await;
        w.fail(&session, &first, true)
            .await
            .expect("a retryable failure");
        // Aged by an hour, not by the helper's second: the VM's clock can step back up to 2 s.
        sqlx::query("UPDATE steps SET next_attempt_at = clock_timestamp() - interval '1 hour' WHERE run_id = $1 AND node = 'a'")
            .bind(run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("the retry is due");
        w.scheduler.tick().await;
        let second = w.one_task(&session).await;
        assert_eq!((first.fence.attempt, second.fence.attempt), (1, 2));
        let stale = Attempt {
            session: same(&session),
            task: first,
            run,
        };
        let current = Attempt {
            session,
            task: second,
            run,
        };
        let ingest = ingest(&w);
        assert_eq!(
            send(&ingest, &stale, records([1])).await,
            stopped(StopReason::Lease)
        );
        assert_eq!(stream_of(&w, &stale).await, None, "attempt 1 has no stream");
        assert_eq!(send(&ingest, &current, records([1])).await, continued(1));
        assert_eq!(
            stream_of(&w, &current)
                .await
                .expect("attempt 2's stream")
                .records,
            1
        );
    })
    .await;
}

/// T2: a task lease past its expiry that the scheduler has not reclaimed yet (the step still `LEASED`) is `LEASE`, as a
/// report of it would be refused.
#[tokio::test]
async fn an_expired_task_lease_not_yet_reclaimed_is_lease() {
    bounded::within("expired lease", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records([1])).await, continued(1));
        // Aged by an hour, not by the helper's second: the VM's clock can step back up to 2 s.
        sqlx::query("UPDATE steps SET lease_expires_at = clock_timestamp() - interval '1 hour' WHERE run_id = $1 AND node = 'a'")
            .bind(a.run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("the lease expires");
        let status: String =
            sqlx::query_scalar("SELECT status FROM steps WHERE run_id = $1 AND node = 'a'")
                .bind(a.run.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("the step");
        assert_eq!(status, "LEASED", "not reclaimed");
        assert_eq!(
            send(&ingest, &a, records([2])).await,
            stopped(StopReason::Lease)
        );
        assert_eq!(lines_of(&w, &a).await.len(), 1, "nothing more is stored");
    })
    .await;
}

/// T2: once the attempt's report is recorded, a batch of it is `LEASE` and stores nothing more, while its run goes on
/// (a second step follows, so the run's end is not what refuses it): the step's lease is no longer live.
#[tokio::test]
async fn a_batch_after_its_attempts_report_was_recorded_is_lease() {
    bounded::within("after the report", LIMIT, async {
        let w = world().await;
        let workflow = w
            .publish(document(
                "logs-after",
                "a",
                vec![with(activity("a"), "next", json!("b")), activity("b")],
            ))
            .await
            .0;
        let session = w.session(&["act.a"]).await;
        accept_at_1_7(&w, &session).await;
        let run = w.start(workflow, &json!({})).await;
        let a = Attempt {
            task: w.one_task(&session).await,
            session,
            run,
        };
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records([1])).await, continued(1));
        w.complete(&a.session, &a.task, &json!({}))
            .await
            .expect("the report");
        assert!(w.step_status(run, "b").await.is_some(), "the run goes on");
        assert_eq!(
            send(&ingest, &a, records([2])).await,
            stopped(StopReason::Lease)
        );
        assert_eq!(lines_of(&w, &a).await.len(), 1);
        assert_eq!(
            stream_of(&w, &a).await.expect("the stream").last_sequence,
            1
        );
    })
    .await;
}

/// Two attempts of one run (two parallel steps), each held after T3 with its own stream lock and released together:
/// each meets its own record ceiling of 3 (3 stored + 2 dropped by the server = 5, one ceiling marker each); there is
/// no ceiling per run (OD-2026-11-16-10), so neither takes from the other.
#[tokio::test]
async fn two_attempts_of_one_run_meet_their_ceilings_at_once() {
    bounded::within("two attempts at their ceilings", LIMIT, async {
        let w = world().await;
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 3).await;
        let workflow = w
            .publish(document(
                "logs-pair",
                "fan",
                vec![
                    with(
                        with(parallel_node("fan", &["l", "r"], "all"), "next", json!("j")),
                        "join",
                        json!("all"),
                    ),
                    activity("l"),
                    activity("r"),
                    activity("j"),
                ],
            ))
            .await
            .0;
        let session = w.session(&["act.l", "act.r"]).await;
        accept_at_1_7(&w, &session).await;
        let run = w.start(workflow, &json!({})).await;
        let mut tasks = w.poll(&session, 8).await;
        assert_eq!(tasks.len(), 2, "both branches are leased");
        tasks.sort_by(|x, y| x.activity.as_str().cmp(y.activity.as_str()));
        let right_task = tasks.pop().expect("act.r");
        let left_task = tasks.pop().expect("act.l");
        let left = Attempt {
            session: same(&session),
            task: left_task,
            run,
        };
        let right = Attempt {
            session: same(&session),
            task: right_task,
            run,
        };
        let (held_left, left_gate) = hooked(&w, Hook::HoldAfterPolicies);
        let (held_right, right_gate) = hooked(&w, Hook::HoldAfterPolicies);
        let l = spawn_send(&held_left, &left, records(1..=5));
        let r = spawn_send(&held_right, &right, records(1..=5));
        left_gate.reached().await;
        right_gate.reached().await;
        left_gate.release();
        right_gate.release();
        assert_eq!(l.await.expect("left").expect("answered"), final_only(5));
        assert_eq!(r.await.expect("right").expect("answered"), final_only(5));
        for attempt in [&left, &right] {
            let stream = stream_of(&w, attempt).await.expect("a stream");
            assert_eq!(
                (
                    stream.records,
                    stream.dropped_by_server,
                    stream.last_sequence,
                    stream.ceiling_reason.as_deref()
                ),
                (3, 2, 5, Some("attempt_records"))
            );
            let kinds: Vec<(i64, String)> = lines_of(&w, attempt)
                .await
                .into_iter()
                .map(|(sequence, kind, _)| (sequence, kind))
                .collect();
            assert_eq!(kinds.last(), Some(&(MARKER, "ceiling_marker".to_owned())));
        }
        // 2 × (three lines of 6 bytes + 4 rows' charge): "line 1" is 6 bytes.
        assert_eq!(step_log_usage(&w).await, 2 * (18 + 4 * ROW));
    })
    .await;
}

/// Two runs of one tenant at its `tenant_step_log_bytes` limit at once: both batches read room at T6 and are held after
/// T7, each on a backend of its own (so each charges its own quota lease), then released together. The limit is
/// 4,096 bytes and each batch charges 2 × (1,000 + 128) = 2,256 bytes: the first charge refills its quota lease with
/// max(2,256, ⌊4,096 / 2⌋) = 2,256 bytes, which leaves 1,840 free, so the second charge raises
/// `tenant_step_log_quota_exceeded` (ADR-0097 decision 4) whichever comes first: one batch is stored, the other told to
/// retry, and the usage never passes the limit. The resend reads the limit at T6 and counts its records
/// `tenant_quota`.
#[test]
fn two_runs_at_the_tenant_limit_at_once_never_store_past_it() {
    let page = crate::counted::recorded(|_| async {
        bounded::within("two runs at the tenant limit", LIMIT, async {
            let w = world().await;
            set_tenant_limit(&w, 4_096).await;
            let a = leased(&w, "a").await;
            let b = leased(&w, "b").await;
            let big = || vec![record(1, &"x".repeat(1_000)), record(2, &"y".repeat(1_000))];
            let (held_a, gate_a) = hooked_on(pool_of(&w, 1).await, Hook::HoldAfterAuthority);
            let (held_b, gate_b) = hooked_on(pool_of(&w, 1).await, Hook::HoldAfterAuthority);
            let first = spawn_send(&held_a, &a, big());
            let second = spawn_send(&held_b, &b, big());
            gate_a.reached().await;
            gate_b.reached().await;
            gate_a.release();
            gate_b.release();
            let mut answers = vec![
                first.await.expect("a").expect("answered"),
                second.await.expect("b").expect("answered"),
            ];
            answers.sort_by_key(|answer| answer.stored_through_sequence);
            assert_eq!(answers, [RETRY, continued(2)]);
            assert_eq!(step_log_usage(&w).await, 2_256);
            let refused = if stream_of(&w, &a).await.is_none() {
                &a
            } else {
                &b
            };
            assert_eq!(
                send(&ingest(&w), refused, big()).await.verdict,
                StepLogVerdict::FinalOnly
            );
            let stream = stream_of(&w, refused)
                .await
                .expect("the refused run's stream");
            assert_eq!(
                (
                    stream.records,
                    stream.dropped_by_server,
                    stream.ceiling_reason.as_deref()
                ),
                (0, 2, Some("tenant_quota"))
            );
            assert!(step_log_usage(&w).await <= 4_096);
        })
        .await;
    });
    // The second charge was refused for the limit, not told to retry by a lock timeout behind the first's refill.
    assert_eq!(
        crate::counted::series(
            &page,
            "step_log_quota_refusals_total",
            &[("cause", "limit")]
        ),
        Some(1.0)
    );
    assert_eq!(
        crate::counted::series(&page, "step_log_lock_timeouts_total", &[]),
        None
    );
}

/// A value that redaction lengthens, at the tenant limit: a field `password` whose value is the integer 1 is stored as
/// the string `[REDACTED]`, so the line "m" measures 1 + |{"password":"[REDACTED]"}| = 1 + 25 = 26 bytes (154
/// charged), against 1 + |{"password":1}| = 15 (143 charged) as sent. With a limit of 150 bytes the line does not fit as
/// stored, though it would as sent: the plan measures after redaction (FR-041-006), so the record is counted
/// `tenant_quota` and nothing is stored (at the tenant's limit no marker is planned, `plan_batch`), never a charge past
/// the limit. Measured as sent, the plan would keep it and its charge of 154 bytes would be refused at T8.
#[tokio::test]
async fn a_value_that_redaction_lengthens_is_measured_as_stored_at_the_tenant_limit() {
    bounded::within("redaction lengthens", LIMIT, async {
        let w = world().await;
        set_tenant_limit(&w, 150).await;
        let a = leased(&w, "a").await;
        let line = StepLogRecordIn {
            fields: vec![("password".to_owned(), FieldValue::Int(1))],
            ..record(1, "m")
        };
        assert_eq!(send(&ingest(&w), &a, vec![line]).await, final_only(1));
        let stream = stream_of(&w, &a).await.expect("a stream");
        assert_eq!(
            (
                stream.records,
                stream.dropped_by_server,
                stream.ceiling_reason.as_deref()
            ),
            (0, 1, Some("tenant_quota"))
        );
        assert!(lines_of(&w, &a).await.is_empty(), "nothing is stored");
        assert_eq!(step_log_usage(&w).await, 0, "nothing is charged");
    })
    .await;
}

/// A client that ignores every verdict: after `FINAL_ONLY` it keeps sending, and after the attempt's report it keeps
/// sending; the stored records and bytes stay where the ceilings put them, only the drop counts grow.
#[tokio::test]
async fn a_client_that_ignores_every_verdict_stores_nothing_past_the_ceilings() {
    bounded::within("ignored verdicts", LIMIT, async {
        let w = world().await;
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 3).await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records(1..=5)).await, final_only(5));
        let at_ceiling = stream_of(&w, &a).await.expect("a stream");
        let usage = step_log_usage(&w).await;
        for start in (6..=46).step_by(10) {
            assert_eq!(
                send(&ingest, &a, records(start..start + 10)).await,
                final_only(start + 9)
            );
        }
        let after = stream_of(&w, &a).await.expect("the stream");
        assert_eq!(
            (after.records, after.bytes),
            (at_ceiling.records, at_ceiling.bytes)
        );
        assert_eq!(after.dropped_by_server, 2 + 50);
        assert_eq!(step_log_usage(&w).await, usage, "no byte more is charged");
        w.complete(&a.session, &a.task, &json!({}))
            .await
            .expect("the report");
        for start in [56, 66] {
            assert_eq!(
                send(&ingest, &a, records(start..start + 10)).await,
                stopped(StopReason::Lease)
            );
        }
        assert_eq!(stream_of(&w, &a).await.expect("the stream"), after);
        assert_eq!(
            lines_of(&w, &a).await.len(),
            4,
            "three lines and the marker"
        );
    })
    .await;
}

/// A task delivered again under a new generation (a lease withdrawn before it reached a host, decision D006-003: the
/// same attempt, the next generation) writes a new stream: the stream key holds the generation, so the earlier
/// generation's lines stay in their own stream and the new one starts at sequence 1.
#[tokio::test]
async fn a_task_delivered_again_under_a_new_generation_writes_a_new_stream() {
    use sagat_application::instances::{Requirement, Requirements, ServingGate};
    bounded::within("new generation", LIMIT, async {
        let w = world().await;
        let recorded: String = sqlx::query_scalar(
            "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
             ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE'], 'test')",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("activate the withdrawal");
        assert_eq!(recorded, "RECORDED");
        let gate = ServingGate::opened();
        gate.keep(&Requirements {
            epoch: 1,
            required: vec![Requirement {
                capability: "lease-withdrawal-v1".to_owned(),
                kinds: vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()],
            }],
        });
        let withdrawing = WorkerService::new(
            w.store.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
            SystemEntropy,
            WorkerSettings::default(),
        )
        .with_lease_withdrawal(std::sync::Arc::new(gate));
        let first = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &first, records([1, 2])).await, continued(2));
        assert_eq!(
            withdrawing
                .withdraw_undelivered(
                    vec![UndeliveredLease::of(w.admin.tenant_id(), &first.task)],
                    WithdrawalCause::NotSent,
                )
                .await,
            [WithdrawalOutcome::Withdrawn]
        );
        let again = Attempt {
            task: w.one_task(&first.session).await,
            session: same(&first.session),
            run: first.run,
        };
        assert_eq!(
            (again.task.fence.attempt, again.task.fence.generation),
            (first.task.fence.attempt, first.task.fence.generation + 1)
        );
        assert_eq!(send(&ingest, &again, records([1])).await, continued(1));
        let streams: Vec<(i64, i32, i64)> = sqlx::query_as(
            "SELECT generation, records, last_sequence FROM step_log_streams WHERE run_id = $1 ORDER BY generation",
        )
        .bind(first.run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the streams");
        let generation = i64::try_from(first.task.fence.generation).expect("a generation");
        assert_eq!(streams, [(generation, 2, 2), (generation + 1, 1, 1)]);
        // The earlier generation is `LEASE` now.
        assert_eq!(send(&ingest, &first, records([3])).await, stopped(StopReason::Lease));
    })
    .await;
}

/// Step logs are no part of a run's journal or projections ("rebuilding the projections" of task T041-8 is the journal
/// replay `verify_run`, the platform having no projection writer, D041-152 (3)): with lines stored, while the step runs
/// and after its report, the replay of the journal equals the stored projections, and the journal holds no step-log
/// event. The replay only reads, so the log rows compared around it guard the helper, not the ingest.
#[tokio::test]
async fn a_runs_journal_replays_to_its_projections_with_its_step_logs_stored() {
    bounded::within("replay", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records(1..=3)).await, continued(3));
        // While the step still runs, and again once its report is recorded.
        w.assert_consistent(a.run).await;
        w.complete(&a.session, &a.task, &json!({}))
            .await
            .expect("the report");
        let before = every_step_log_row(&w).await;
        w.assert_consistent(a.run).await;
        assert!(
            w.journal(a.run)
                .await
                .iter()
                .all(|event| !event.contains("log")),
            "no step-log event in the journal"
        );
        assert_eq!(every_step_log_row(&w).await, before);
    })
    .await;
}

/// The server canary (SC-041-004 on the server side; see the module's notes): a secret the attempt resolved, a
/// connection string whose password is the canary, logged whole by a client that skips its own redaction, in the
/// message, the exception, a field value under a neutral key, a field key, and alone under a sensitive field name.
/// The canary reaches no stored row, no event of the run's journal, no audit record and no line of the captured
/// `tracing` output of the batch and of the run's report.
#[tokio::test]
async fn the_server_canary_reaches_no_row_event_audit_record_or_trace() {
    bounded::within("server canary", LIMIT, async {
        let w = world().await;
        let canary = format!("cnry{}", uuid::Uuid::now_v7().simple());
        let secret = format!("postgres://svc:{canary}@db.internal/app");
        // Captured from before the secret is stored and resolved, so that the resolution is covered too.
        let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
        w.put_secret("db-url", &secret).await;
        let workflow = w
            .publish(document(
                "logs-canary",
                "a",
                vec![with(activity("a"), "secrets", json!(["db-url"]))],
            ))
            .await
            .0;
        let session = w.session(&["act.a"]).await;
        accept_at_1_7(&w, &session).await;
        let run = w.start(workflow, &json!({})).await;
        let a = Attempt {
            task: w.one_task(&session).await,
            session,
            run,
        };
        let resolved = w
            .workers
            .resolve_secrets(&a.session.principal, &task_ref(&a.task), None)
            .await
            .expect("the attempt resolves its secret");
        let value = String::from_utf8(resolved[0].value.expose().to_vec()).expect("text");
        assert_eq!(value, secret);
        let line = StepLogRecordIn {
            exception: format!("ConnectionError: {value} refused"),
            fields: vec![
                (
                    "target".to_owned(),
                    FieldValue::String(format!("dsn {value}")),
                ),
                (format!("dsn {value}"), FieldValue::Int(1)),
                ("password".to_owned(), FieldValue::String(canary.clone())),
            ],
            ..record(1, &format!("connecting with {value}"))
        };
        assert_eq!(send(&ingest(&w), &a, vec![line]).await, continued(1));
        w.complete(&a.session, &a.task, &json!({}))
            .await
            .expect("the report");
        drop(guard);
        let rows = every_step_log_row(&w).await;
        assert!(rows.contains("[REDACTED]"), "the line is stored, scrubbed");
        let journal: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(e::text, E'\\n'), '') FROM run_events e WHERE run_id = $1",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the journal");
        let audit: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(a::text, E'\\n'), '') FROM \
             (SELECT a::text FROM audit_records a UNION ALL SELECT s::text FROM security_audit_records s) a",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("the audit records");
        for (surface, text) in [
            ("the stored step-log rows", rows),
            ("the run's journal", journal),
            ("the audit records", audit),
            ("the captured tracing output", captured.text()),
        ] {
            assert!(!text.contains(&canary), "the canary reached {surface}");
        }
    })
    .await;
}

/// A database error that quotes the bound text (a `BEFORE INSERT` trigger the test installs on `step_log_lines`,
/// raising with `NEW.message` in its message): the batch fails, and the line's text is in neither the error answered
/// nor the captured `tracing` output; nothing is stored.
#[tokio::test]
async fn a_database_error_that_quotes_the_bound_text_is_answered_and_logged_without_it() {
    bounded::within("quoted text", LIMIT, async {
        let w = world().await;
        for statement in [
            "CREATE FUNCTION test_quote_the_line() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'refused line: %', NEW.message; END $$",
            "CREATE TRIGGER test_quote_the_line BEFORE INSERT ON step_log_lines \
             FOR EACH ROW EXECUTE FUNCTION test_quote_the_line()",
        ] {
            sqlx::query(statement)
                .execute(&w.db.pool)
                .await
                .expect("the quoting trigger");
        }
        let a = leased(&w, "a").await;
        let text = format!("bound-{}", uuid::Uuid::now_v7().simple());
        let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
        let error = ingest(&w)
            .append(&a.session.principal, &batch(&a, vec![record(1, &text)]))
            .await
            .expect_err("the insert is refused");
        drop(guard);
        for (surface, shown) in [
            ("the error's text", error.to_string()),
            ("the error's debug form", format!("{error:?}")),
            ("the captured tracing output", captured.text()),
        ] {
            assert!(!shown.contains(&text), "the bound text reached {surface}");
        }
        assert_eq!(stream_of(&w, &a).await, None);
    })
    .await;
}
