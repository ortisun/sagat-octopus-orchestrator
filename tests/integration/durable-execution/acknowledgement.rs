//! Feature 006, Task 006-049 (`stream-completion.md`, "The server's acknowledgement"; Architecture decision 0099 decisions 3,
//! 4, 6 and 7): the delivery acknowledgement of protocol 1.8 on the real store, root and scoped, verdict by verdict and reason
//! by reason.
//!
//! This build negotiates at most its own minor, so a session is raised to 1.8 by SQL and the service under test claims at the
//! ceiling the `test-support` feature raises, as the tests of the marks do (`lease_marks.rs`). Nothing waits on time: a row
//! held by another transaction is held by the test, and a deadline is an instant the test chooses.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::{AccessDenial, AppError};
use sagat_application::execution::Engine;
use sagat_application::instances::{Requirement, Requirements, ServingGate};
use sagat_application::worker::acknowledgement::{
    Acknowledged, AcknowledgementFailure, AcknowledgementVerdict, CallDeadline, Cancelled, NotHeld,
};
use sagat_application::worker::{
    Capabilities, ClaimedTask, TaskRef, WorkerService, WorkerSettings,
};
use sagat_domain::execution::{Fence, RunId};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::blob_collections::{Sched, scheduler};
use crate::support::worker::{Platform, platform_with};

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

const SUBJECT: &str = "ada";

fn gate() -> Arc<ServingGate> {
    let gate = ServingGate::opened();
    let both = || vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()];
    gate.keep(&Requirements {
        epoch: 1,
        required: vec![
            Requirement {
                capability: "lease-withdrawal-v1".to_owned(),
                kinds: both(),
            },
            Requirement {
                capability: "lease-withdrawal-v2".to_owned(),
                kinds: both(),
            },
            Requirement {
                capability: "delivery-acknowledgement-v1".to_owned(),
                kinds: vec!["EXECUTION_ENGINE".to_owned()],
            },
        ],
    });
    Arc::new(gate)
}

struct Lab {
    p: Platform,
    principal: Principal,
    scheduler: Sched,
    workers: Workers,
}

async fn lab() -> Lab {
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    let p = platform_with(options).await;
    drop(p.worker(SUBJECT).await);
    let (status, body) = p
        .rest(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 64})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let scheduler = scheduler(&p, None).await;
    let principal = Principal::new(
        p.tenant,
        Subject::parse(SUBJECT).expect("subject"),
        Role::Worker,
    );
    let workers = WorkerService::new(
        p.api.executions.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_lease_withdrawal(gate())
    .with_new_withdrawals(true)
    .with_hand_back_ceiling(8);
    Lab {
        p,
        principal,
        scheduler,
        workers,
    }
}

/// One activity `a`, at the root or in the body of a `forEach` of one item.
fn workflow(name: &str, activity: &str, scoped: bool) -> Value {
    let node = json!({"id": "a", "type": "activity", "activity": activity,
        "retry": {"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}});
    if !scoped {
        return json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "a", "nodes": [node]
        });
    }
    let mut node = node;
    node["next"] = json!("returned");
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "a", "nodes": [node, {"id": "returned", "type": "return", "result": "1"}]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 1, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// A lease the test holds: what the host was handed.
struct Lease {
    run: RunId,
    task: ClaimedTask,
}

impl Lease {
    fn task_ref(&self) -> TaskRef<'_> {
        TaskRef {
            run_id: self.task.run_id,
            instance: &self.task.instance,
            step_id: Some(self.task.step_id),
            fence: self.task.fence,
            lease_token: self.task.lease_token.expose(),
        }
    }

    fn node(&self) -> &str {
        self.task.instance.node().as_str()
    }
}

impl Lab {
    /// A session for `activity` raised to `minor`, as `as_protocol_1_8` does.
    async fn session(&self, activity: &str, minor: u32) -> (Uuid, String) {
        let registered = self
            .workers
            .register(
                &self.principal,
                "payments",
                &format!("ack-{}", Uuid::now_v7()),
                &Capabilities {
                    activities: vec![activity.to_owned()],
                    languages: vec![],
                    pools: vec![],
                    max_inflight: 8,
                    protocol_version: "1.6".to_owned(),
                    declines_acknowledgement: false,
                },
            )
            .await
            .expect("register");
        sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = $2 WHERE id = $1")
            .bind(registered.worker_id)
            .bind(i32::try_from(minor).expect("minor"))
            .execute(&self.p.api.db.pool)
            .await
            .expect("raise the minor");
        (
            registered.worker_id,
            registered.session_token.expose().to_owned(),
        )
    }

    /// A run of `activity` claimed by a session at `minor`: root, or its one scoped item.
    async fn leased(&self, activity: &str, scoped: bool, minor: u32) -> Lease {
        let session = self.session(activity, minor).await;
        let name = format!("ack-{}", Uuid::now_v7());
        let id = self
            .p
            .publish(&name, &workflow(&name, activity, scoped))
            .await;
        let input = if scoped {
            json!({"lines": [1]})
        } else {
            json!({})
        };
        let run: RunId = self.p.start(&id, &input).await.parse().expect("run id");
        for _ in 0..20 {
            if scoped {
                self.scheduler.tick().await;
            }
            let mut tasks = self
                .workers
                .poll(&self.principal, session.0, &session.1, 1, None)
                .await
                .expect("poll");
            if let Some(task) = tasks.pop() {
                assert_eq!(task.instance.path().is_root(), !scoped);
                return Lease { run, task };
            }
        }
        panic!("scoped={scoped}: step a was never claimed");
    }

    /// A provisional lease (a session at 1.8 that does not decline, every requirement recorded).
    async fn provisional(&self, activity: &str, scoped: bool) -> Lease {
        let lease = self.leased(activity, scoped, 8).await;
        assert!(
            lease.task.acknowledge_before_start(),
            "flagged at its claim"
        );
        lease
    }

    async fn acknowledge(
        &self,
        leases: &[&Lease],
    ) -> Result<
        sagat_application::worker::acknowledgement::AcknowledgementAnswer,
        AcknowledgementFailure,
    > {
        self.acknowledge_by(leases, CallDeadline::of_arrival(Instant::now()))
            .await
    }

    async fn acknowledge_by(
        &self,
        leases: &[&Lease],
        deadline: CallDeadline,
    ) -> Result<
        sagat_application::worker::acknowledgement::AcknowledgementAnswer,
        AcknowledgementFailure,
    > {
        let tasks: Vec<TaskRef<'_>> = leases.iter().map(|lease| lease.task_ref()).collect();
        self.workers
            .acknowledge_tasks(&self.principal, &tasks, deadline)
            .await
            .map_err(|failed| failed.failure)
    }

    async fn verdicts(&self, leases: &[&Lease]) -> Vec<AcknowledgementVerdict> {
        self.acknowledge(leases).await.expect("answered").verdicts
    }

    /// Runs `sql` on the step row of `lease` (`$1` tenant, `$2` run, `$3` node).
    async fn on_row(&self, lease: &Lease, sql: &str) {
        // the assignments are the test's own literals
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE steps SET {sql} WHERE tenant_id = $1 AND run_id = $2 AND node = $3"
        )))
        .bind(self.p.tenant.as_uuid())
        .bind(lease.run.as_uuid())
        .bind(lease.node())
        .execute(&self.p.api.db.pool)
        .await
        .expect("update the step row");
    }

    async fn acknowledged_at(&self, lease: &Lease) -> Option<time::OffsetDateTime> {
        sqlx::query_scalar(
            "SELECT lease_acknowledged_at FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
        )
        .bind(self.p.tenant.as_uuid())
        .bind(lease.run.as_uuid())
        .bind(lease.node())
        .fetch_one(&self.p.api.db.pool)
        .await
        .expect("the step row")
    }
}

fn acknowledged(how: Acknowledged) -> impl Fn(&AcknowledgementVerdict) -> bool {
    move |verdict| matches!(verdict, AcknowledgementVerdict::Acknowledged { how: h, .. } if *h == how)
}

/// The lease's time left is reported from the server's clock, never below zero and never above the time-to-live (30 s).
fn assert_time_left(verdict: &AcknowledgementVerdict) {
    let AcknowledgementVerdict::Acknowledged {
        lease_time_left,
        attempt_time_left,
        ..
    } = verdict
    else {
        panic!("not acknowledged: {verdict:?}");
    };
    assert!(
        *lease_time_left > Duration::from_secs(6),
        "{lease_time_left:?}"
    );
    // the time-to-live, and the 2 s the virtual machine of the CI can step the database clock back between the two readings
    assert!(
        *lease_time_left <= Duration::from_secs(32),
        "{lease_time_left:?}"
    );
    assert_eq!(*attempt_time_left, None, "no timeoutMs");
}

#[tokio::test]
async fn a_provisional_lease_is_acknowledged_once_and_a_repeat_is_a_duplicate_root_and_scoped() {
    let lab = lab().await;
    for scoped in [false, true] {
        let lease = lab.provisional(&format!("ack.first{scoped}"), scoped).await;
        assert_eq!(lab.acknowledged_at(&lease).await, None);
        let first = lab.verdicts(&[&lease]).await;
        assert!(
            acknowledged(Acknowledged::Written)(&first[0]),
            "{scoped}: {first:?}"
        );
        assert_time_left(&first[0]);
        let written = lab.acknowledged_at(&lease).await.expect("written");
        let again = lab.verdicts(&[&lease]).await;
        assert!(
            acknowledged(Acknowledged::Duplicate)(&again[0]),
            "{scoped}: {again:?}"
        );
        assert_eq!(
            lab.acknowledged_at(&lease).await,
            Some(written),
            "{scoped}: a repeat writes nothing"
        );
        // the acknowledgement renews nothing and changes no status
        let (status, generation): (String, i64) = sqlx::query_as(
            "SELECT status, generation FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
        )
        .bind(lab.p.tenant.as_uuid())
        .bind(lease.run.as_uuid())
        .fetch_one(&lab.p.api.db.pool)
        .await
        .expect("the row");
        assert_eq!(status, "LEASED");
        assert_eq!(
            u64::try_from(generation).expect("generation"),
            lease.task.fence.generation
        );
    }
}

#[tokio::test]
async fn a_lease_never_offered_or_presented_with_another_fence_is_not_held() {
    let lab = lab().await;
    // a session of 1.6: unreserved, unmarked
    let unmarked = lab.leased("ack.unmarked", false, 6).await;
    assert!(!unmarked.task.acknowledge_before_start());
    assert_eq!(
        lab.verdicts(&[&unmarked]).await,
        [AcknowledgementVerdict::NotHeld(NotHeld::NotProvisional)]
    );
    assert_eq!(
        lab.acknowledged_at(&unmarked).await,
        None,
        "nothing written"
    );

    let lease = lab.provisional("ack.fence", false).await;
    let wrong = |fence: Fence, step: Option<Uuid>| TaskRef {
        run_id: lease.task.run_id,
        instance: &lease.task.instance,
        step_id: step,
        fence,
        lease_token: lease.task.lease_token.expose(),
    };
    let fence = lease.task.fence;
    for (case, task) in [
        (
            "an attempt after",
            wrong(
                Fence {
                    attempt: fence.attempt + 1,
                    ..fence
                },
                None,
            ),
        ),
        (
            "a generation after",
            wrong(
                Fence {
                    generation: fence.generation + 1,
                    ..fence
                },
                None,
            ),
        ),
        ("another step", wrong(fence, Some(Uuid::now_v7()))),
    ] {
        let answer = lab
            .workers
            .acknowledge_tasks(
                &lab.principal,
                &[task],
                CallDeadline::of_arrival(Instant::now()),
            )
            .await
            .expect("answered");
        assert_eq!(
            answer.verdicts,
            [AcknowledgementVerdict::NotHeld(NotHeld::NoMatch)],
            "{case}"
        );
    }
    assert_eq!(lab.acknowledged_at(&lease).await, None, "nothing written");
}

#[tokio::test]
async fn a_forged_token_takes_no_lock_and_a_withdrawn_token_is_answered_withdrawn() {
    let lab = lab().await;
    let lease = lab.provisional("ack.forged", false).await;
    // the row is held by another transaction: a forged token is answered without asking for its lock (`NOT_HELD`, never
    // `RETRY`)
    let mut holder = lab.p.api.db.pool.begin().await.expect("begin");
    sqlx::query(
        "SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a' FOR UPDATE",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(lease.run.as_uuid())
    .execute(&mut *holder)
    .await
    .expect("hold the row");
    let forged = TaskRef {
        lease_token: "0000000000000000000000000000000000000000000000000000000000000000",
        ..lease.task_ref()
    };
    let answer = lab
        .workers
        .acknowledge_tasks(
            &lab.principal,
            &[forged],
            CallDeadline::of_arrival(Instant::now()),
        )
        .await
        .expect("answered");
    assert_eq!(
        answer.verdicts,
        [AcknowledgementVerdict::NotHeld(NotHeld::NoMatch)]
    );
    assert_eq!(
        answer.unmatched_runs,
        [lease.run],
        "left for the cross-tenant check"
    );
    holder.rollback().await.expect("release");

    // a withdrawal of a marked lease keeps its token's hash: a probe of the withdrawn lease is answered `withdrawn`
    lab.on_row(
        &lease,
        "withdrawn_lease_token_hash = lease_token_hash, withdrawn_on_hand_back = false, status = 'READY', \
         lease_token_hash = NULL, lease_expires_at = NULL, leased_by = NULL",
    )
    .await;
    assert_eq!(
        lab.verdicts(&[&lease]).await,
        [AcknowledgementVerdict::NotHeld(NotHeld::Withdrawn)]
    );
}

#[tokio::test]
async fn a_mark_of_another_server_is_recovered_and_writes_nothing() {
    let lab = lab().await;
    let lease = lab.provisional("ack.recovered", false).await;
    lab.on_row(
        &lease,
        "lease_claim_incarnation = lease_claim_incarnation - interval '1 day'",
    )
    .await;
    let verdicts = lab.verdicts(&[&lease]).await;
    assert!(
        acknowledged(Acknowledged::Recovered)(&verdicts[0]),
        "{verdicts:?}"
    );
    assert_eq!(lab.acknowledged_at(&lease).await, None, "nothing written");
}

/// The deadlines are set 1 s ahead, five short of the margin, so that a database clock stepped back by up to 2 s (as the
/// virtual machine of the CI steps it) still finds them short.
#[tokio::test]
async fn too_little_time_left_before_either_deadline_is_too_late() {
    let lab = lab().await;
    for scoped in [false, true] {
        let expiring = lab.provisional(&format!("ack.late{scoped}"), scoped).await;
        lab.on_row(
            &expiring,
            "lease_expires_at = clock_timestamp() + interval '1 second'",
        )
        .await;
        let timing_out = lab
            .provisional(&format!("ack.timeout{scoped}"), scoped)
            .await;
        lab.on_row(
            &timing_out,
            "attempt_deadline_at = clock_timestamp() + interval '1 second'",
        )
        .await;
        assert_eq!(
            lab.verdicts(&[&expiring, &timing_out]).await,
            [
                AcknowledgementVerdict::NotHeld(NotHeld::TooLate),
                AcknowledgementVerdict::NotHeld(NotHeld::TooLate)
            ],
            "{scoped}"
        );
        // acknowledged with the margin, then short of it: run nothing, hand back
        let acknowledged_then_late = lab.provisional(&format!("ack.after{scoped}"), scoped).await;
        lab.verdicts(&[&acknowledged_then_late]).await;
        lab.on_row(
            &acknowledged_then_late,
            "lease_expires_at = clock_timestamp() + interval '1 second'",
        )
        .await;
        assert_eq!(
            lab.verdicts(&[&acknowledged_then_late]).await,
            [AcknowledgementVerdict::Cancelled(Cancelled::TooLate)],
            "{scoped}"
        );
    }
}

#[tokio::test]
async fn a_cancellation_or_a_stopping_run_is_cancelled_and_writes_nothing() {
    let lab = lab().await;
    for scoped in [false, true] {
        let cancelled = lab
            .provisional(&format!("ack.cancel{scoped}"), scoped)
            .await;
        lab.on_row(&cancelled, "cancel_requested = true").await;
        let stopping = lab.provisional(&format!("ack.stop{scoped}"), scoped).await;
        sqlx::query("UPDATE runs SET stop_reason = 'CANCELLED' WHERE tenant_id = $1 AND id = $2")
            .bind(lab.p.tenant.as_uuid())
            .bind(stopping.run.as_uuid())
            .execute(&lab.p.api.db.pool)
            .await
            .expect("stop the run");
        assert_eq!(
            lab.verdicts(&[&cancelled, &stopping]).await,
            [
                AcknowledgementVerdict::Cancelled(Cancelled::StopAsked),
                AcknowledgementVerdict::Cancelled(Cancelled::StopAsked)
            ],
            "{scoped}"
        );
        assert_eq!(lab.acknowledged_at(&cancelled).await, None);
        assert_eq!(lab.acknowledged_at(&stopping).await, None);
    }
}

#[tokio::test]
async fn a_row_held_by_another_transaction_is_retried_while_the_others_are_acknowledged() {
    let lab = lab().await;
    let held = lab.provisional("ack.held", false).await;
    let free = lab.provisional("ack.free", true).await;
    let mut holder = lab.p.api.db.pool.begin().await.expect("begin");
    sqlx::query(
        "SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a' FOR UPDATE",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(held.run.as_uuid())
    .execute(&mut *holder)
    .await
    .expect("hold the row");
    let verdicts = lab.verdicts(&[&held, &free]).await;
    assert_eq!(verdicts[0], AcknowledgementVerdict::Retry, "{verdicts:?}");
    assert!(
        acknowledged(Acknowledged::Written)(&verdicts[1]),
        "{verdicts:?}"
    );
    assert_eq!(lab.acknowledged_at(&held).await, None, "nothing written");
    holder.rollback().await.expect("release");
    let again = lab.verdicts(&[&held, &free]).await;
    assert!(acknowledged(Acknowledged::Written)(&again[0]), "{again:?}");
    assert!(
        acknowledged(Acknowledged::Duplicate)(&again[1]),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_row_share_locked_by_a_foreign_key_check_is_not_skipped() {
    let lab = lab().await;
    let lease = lab.provisional("ack.keyshare", false).await;
    // what the foreign-key check of a referencing insert takes on the step row
    let mut holder = lab.p.api.db.pool.begin().await.expect("begin");
    sqlx::query(
        "SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a' FOR KEY SHARE",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(lease.run.as_uuid())
    .execute(&mut *holder)
    .await
    .expect("share the row's key");
    let verdicts = lab.verdicts(&[&lease]).await;
    assert!(
        acknowledged(Acknowledged::Written)(&verdicts[0]),
        "{verdicts:?}"
    );
    holder.rollback().await.expect("release");
}

#[tokio::test]
async fn a_narrowed_binding_is_unbound_for_the_acknowledgement_and_the_run_transactions_alike() {
    let lab = lab().await;
    let lease = lab.provisional("ack.narrowed", false).await;
    let scoped = lab.provisional("ack.narrowed.scoped", true).await;
    let kept = lab.provisional("ack.kept", false).await;
    sqlx::query(
        "UPDATE worker_bindings SET pattern = 'ack.kept' WHERE tenant_id = $1 AND subject = $2",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(SUBJECT)
    .execute(&lab.p.api.db.pool)
    .await
    .expect("narrow the binding");
    let answer = lab.acknowledge(&[&lease, &kept]).await.expect("answered");
    assert_eq!(
        answer.verdicts[0],
        AcknowledgementVerdict::NotHeld(NotHeld::Unbound)
    );
    assert!(
        acknowledged(Acknowledged::Written)(&answer.verdicts[1]),
        "{answer:?}"
    );
    assert_eq!(
        answer.denial,
        Some(AccessDenial::Refused("NOT_BOUND")),
        "one denial for the call"
    );
    // the same narrowed binding refuses the path transaction's heartbeat and the acknowledgement of the scoped lease
    assert_eq!(
        lab.verdicts(&[&scoped]).await,
        [AcknowledgementVerdict::NotHeld(NotHeld::Unbound)]
    );
    assert!(matches!(
        lab.workers
            .heartbeat(&lab.principal, &scoped.task_ref())
            .await,
        Err(AppError::NotBound)
    ));
    // and the whole-run transaction's heartbeat
    assert!(matches!(
        lab.workers
            .heartbeat(&lab.principal, &lease.task_ref())
            .await,
        Err(AppError::NotBound)
    ));
}

#[tokio::test]
async fn a_revoked_membership_refuses_the_call() {
    let lab = lab().await;
    let lease = lab.provisional("ack.revoked", false).await;
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = $2")
        .bind(lab.p.tenant.as_uuid())
        .bind(SUBJECT)
        .execute(&lab.p.api.db.pool)
        .await
        .expect("revoke the membership");
    assert!(matches!(
        lab.acknowledge(&[&lease]).await,
        Err(AcknowledgementFailure::Refused(AppError::Forbidden { .. }))
    ));
    assert_eq!(lab.acknowledged_at(&lease).await, None);
}

#[tokio::test]
async fn a_call_past_its_deadline_writes_nothing_and_a_malformed_call_is_refused() {
    let lab = lab().await;
    let lease = lab.provisional("ack.deadline", false).await;
    let passed = CallDeadline::at(Instant::now());
    assert!(matches!(
        lab.acknowledge_by(&[&lease], passed).await,
        Err(AcknowledgementFailure::Deadline)
    ));
    // The binding's lock held exclusively, as a binding replacement holds it, until the call's deadline has passed, and the
    // namespace's secrets lock held by another transaction for the whole call: the statements after the binding's lock are
    // not run (had they run, the membership's would have waited for the secrets lock and ended at its lock timeout, a
    // retryable failure, not the deadline), and the transaction is rolled back.
    let mut secrets = lab.p.api.db.pool.begin().await.expect("begin");
    let mut replacement = lab.p.api.db.pool.begin().await.expect("begin");
    let namespace: Uuid = sqlx::query_scalar(
        "SELECT namespace_id FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(lease.run.as_uuid())
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("namespace");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "binding:{}:{}:{}",
            lab.p.tenant, namespace, SUBJECT
        ))
        .execute(&mut *replacement)
        .await
        .expect("hold the binding");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota:secrets:{}:{}", lab.p.tenant, namespace))
        .execute(&mut *secrets)
        .await
        .expect("hold the secrets lock");
    let deadline = CallDeadline::at(Instant::now() + Duration::from_millis(300));
    let leases = [&lease];
    let call = lab.acknowledge_by(&leases, deadline);
    let release = async {
        // released once the acknowledgement waits for the lock and its deadline has passed; a call that never waits fails
        // the test instead of hanging it
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' \
                                 AND query LIKE '%pg_advisory_xact_lock_shared%')",
                )
                .fetch_one(&lab.p.api.db.pool)
                .await
                .expect("activity");
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the acknowledgement waits for the binding's lock");
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline.instant())).await;
        replacement.rollback().await.expect("release");
    };
    let (answer, ()) = tokio::join!(call, release);
    assert!(
        matches!(answer, Err(AcknowledgementFailure::Deadline)),
        "{answer:?}"
    );
    assert_eq!(lab.acknowledged_at(&lease).await, None, "nothing committed");
    secrets.rollback().await.expect("release");

    for (case, tasks) in [
        ("none", Vec::new()),
        (
            "the same lease twice",
            vec![lease.task_ref(), lease.task_ref()],
        ),
    ] {
        assert!(
            matches!(
                lab.workers
                    .acknowledge_tasks(
                        &lab.principal,
                        &tasks,
                        CallDeadline::of_arrival(Instant::now())
                    )
                    .await
                    .map_err(|failed| failed.failure),
                Err(AcknowledgementFailure::Malformed(_))
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn the_acknowledgement_commits_with_synchronous_commit_at_least_local() {
    use sagat_application::worker::acknowledgement::AcknowledgementStore;
    let lab = lab().await;
    let options = lab.p.api.db.pool.connect_options().as_ref().clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET synchronous_commit = off")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("a pool whose sessions run with synchronous_commit off");
    let store = PgRunStore::new(
        pool,
        sagat_application::execution::DefinitionCache::new(
            Arc::new(sagat_infra_ir::JsonIrCodec),
            8,
        ),
    );
    let mut tx = store
        .begin_acknowledgement(lab.p.tenant, CallDeadline::of_arrival(Instant::now()))
        .await
        .expect("begin");
    assert_eq!(
        tx.synchronous_commit_for_tests().await.expect("read back"),
        "local",
        "raised for the transaction"
    );
}

#[tokio::test]
async fn the_tasks_of_two_namespaces_are_refused_before_any_lock() {
    let lab = lab().await;
    let first = lab.provisional("ack.namespace.first", false).await;
    let second = lab.provisional("ack.namespace.second", false).await;
    let workspace = lab.p.api.create_workspace("alice", "acme", "second").await;
    let other: String = lab
        .p
        .api
        .create_namespace("alice", "acme", &workspace, "refunds", "dev")
        .await;
    // a fixture: the second step row moved to the other namespace of the tenant
    lab.on_row(
        &second,
        &format!(
            "namespace_id = '{}'::uuid",
            Uuid::parse_str(&other).expect("namespace id")
        ),
    )
    .await;
    assert!(matches!(
        lab.acknowledge(&[&first, &second]).await,
        Err(AcknowledgementFailure::Malformed(_))
    ));
    assert_eq!(lab.acknowledged_at(&first).await, None, "nothing written");
}

#[tokio::test]
async fn the_answer_reports_the_time_left_before_the_attempt_deadline() {
    let lab = lab().await;
    for scoped in [false, true] {
        let lease = lab
            .provisional(&format!("ack.attempt{scoped}"), scoped)
            .await;
        lab.on_row(
            &lease,
            "attempt_deadline_at = clock_timestamp() + interval '20 seconds'",
        )
        .await;
        let verdicts = lab.verdicts(&[&lease]).await;
        let AcknowledgementVerdict::Acknowledged {
            how: Acknowledged::Written,
            attempt_time_left: Some(left),
            ..
        } = verdicts[0]
        else {
            panic!("{scoped}: {verdicts:?}");
        };
        // 20 s from the fixture's clock reading, read again by the acknowledgement: never more, give or take the 2 s the
        // virtual machine of the CI can step the clock back, and well above the margin
        assert!(left > Duration::from_secs(6), "{scoped}: {left:?}");
        assert!(left <= Duration::from_secs(22), "{scoped}: {left:?}");
    }
}

#[tokio::test]
async fn a_run_of_another_tenant_is_recorded_once_and_a_run_of_its_own_is_not() {
    let lab = lab().await;
    let lease = lab.provisional("ack.tenant", false).await;
    let unknown = RunId::new(Uuid::now_v7());
    // the run is acme's: for acme it is its own, or unknown, and nothing is recorded
    assert!(
        !lab.workers
            .audit_other_tenants(&lab.principal, &[unknown, lease.run], None)
            .await
            .expect("checked")
    );
    // for a worker of globex it is a run of another tenant: one record, naming the run, in globex's audit trail
    let outsider = Principal::new(
        lab.p.other_tenant,
        Subject::parse("mallory").expect("subject"),
        Role::Worker,
    );
    assert!(
        lab.workers
            .audit_other_tenants(
                &outsider,
                &[unknown, lease.run],
                Some("0123456789abcdef0123456789abcdef".to_owned())
            )
            .await
            .expect("checked")
    );
    // the denial is written off the response path (`IdentityStore::record_denial`), so the trail is read until it holds it
    let recorded: Vec<(String, String)> =
        crate::support::eventually("the cross-tenant record", || async {
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT action, resource_id FROM audit_records WHERE tenant_id = $1 AND actor = 'mallory'",
            )
            .bind(lab.p.other_tenant.as_uuid())
            .fetch_all(&lab.p.api.db.pool)
            .await
            .expect("the audit trail");
            (!rows.is_empty()).then_some(rows)
        })
        .await;
    assert_eq!(
        recorded,
        [(
            "access.denied.cross_tenant".to_owned(),
            lease.run.to_string()
        )]
    );
}
