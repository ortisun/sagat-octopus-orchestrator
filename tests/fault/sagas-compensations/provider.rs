//! A provider that really is a server (T003-030): it accepts a payment and answers too late, and
//! it is asked to undo the same thing twice. The platform's contract is at-least-once with keys;
//! what has to hold is that the provider ends up with exactly one effect.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use sagat_application::error::AppError;
use sagat_application::runs::ReviewAction;
use sagat_application::worker::TaskKind;
use sagat_domain::execution::{FailureCode, ReviewOutcome, RunStatus};
use serde_json::{Value, json};

use crate::common::{quick, settlement, snapshot, worker};
use crate::support::engine::{Session, World, world};

#[derive(Default)]
struct State_ {
    payments: BTreeMap<String, String>,
    payment_calls: BTreeMap<String, u32>,
    releases: BTreeMap<String, u32>,
    /// The next payment is recorded and the answer is held back this long.
    slow_next_payment: Option<Duration>,
    /// The next payment is not recorded at all: the provider fails before it accepts.
    reject_next_payment: bool,
}

#[derive(Clone, Default)]
struct Provider {
    state: Arc<Mutex<State_>>,
}

fn key_of(headers: &HeaderMap) -> String {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn pay(
    State(p): State<Provider>,
    headers: HeaderMap,
) -> (axum::http::StatusCode, Json<Value>) {
    let key = key_of(&headers);
    let (delay, reject) = {
        let mut s = p.state.lock().expect("lock");
        *s.payment_calls.entry(key.clone()).or_insert(0) += 1;
        let reject = std::mem::take(&mut s.reject_next_payment);
        let delay = if reject {
            None
        } else {
            s.slow_next_payment.take()
        };
        if !reject {
            let n = s.payments.len();
            s.payments
                .entry(key.clone())
                .or_insert_with(|| format!("pay-{n}"));
        }
        (delay, reject)
    };
    if reject {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "unavailable"})),
        );
    }
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    let id = p
        .state
        .lock()
        .expect("lock")
        .payments
        .get(&key)
        .cloned()
        .unwrap_or_default();
    (axum::http::StatusCode::OK, Json(json!({"id": id})))
}

async fn lookup(State(p): State<Provider>, Path(key): Path<String>) -> Json<Value> {
    let s = p.state.lock().expect("lock");
    Json(match s.payments.get(&key) {
        Some(id) => json!({"found": true, "id": id}),
        None => json!({"found": false}),
    })
}

async fn release(State(p): State<Provider>, headers: HeaderMap) -> Json<Value> {
    let key = key_of(&headers);
    let mut s = p.state.lock().expect("lock");
    let calls = s.releases.entry(key).or_insert(0);
    *calls += 1;
    Json(json!({"released": true, "duplicate": *calls > 1}))
}

struct Running {
    url: String,
    provider: Provider,
}

async fn serve() -> Running {
    let provider = Provider::default();
    let app = Router::new()
        .route("/payments", post(pay))
        .route("/payments/{key}", get(lookup))
        .route("/releases", post(release))
        .with_state(provider.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Running { url, provider }
}

fn client() -> reqwest::Client {
    sagat_api_http::auth::install_crypto_provider();
    reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .expect("client")
}

/// What a worker of `settlement` does with the provider, as an SDK would.
struct Sdk<'a> {
    w: &'a World,
    session: &'a Session,
    url: &'a str,
    http: reqwest::Client,
}

impl Sdk<'_> {
    /// Submits the payment. A timeout is not an error the worker can classify: it reports that the
    /// outcome is unknown.
    async fn submit(&self, task: &sagat_application::worker::ClaimedTask) -> bool {
        let sent = self
            .http
            .post(format!("{}/payments", self.url))
            .header("idempotency-key", &task.idempotency_key)
            .send()
            .await;
        match sent {
            Ok(response) if response.status().is_success() => {
                self.w
                    .complete(self.session, task, &json!({"paid": true}))
                    .await
                    .expect("report");
                true
            }
            Ok(_) | Err(_) => {
                self.w
                    .fail_with(self.session, task, "OUTCOME_UNKNOWN", false)
                    .await
                    .expect("report");
                false
            }
        }
    }

    /// Asks the provider whether the payment the lookup is about exists.
    async fn look_up(&self, task: &sagat_application::worker::ClaimedTask) {
        let target = task.target.as_ref().expect("a lookup names its target");
        let found: Value = self
            .http
            .get(format!("{}/payments/{}", self.url, target.idempotency_key))
            .send()
            .await
            .expect("lookup")
            .json()
            .await
            .expect("json");
        let answer = if found["found"] == true {
            json!({"effect": "applied", "output": {"id": found["id"]}})
        } else {
            json!({"effect": "not_applied"})
        };
        self.w
            .complete(self.session, task, &answer)
            .await
            .expect("answer");
    }
}

#[tokio::test]
async fn a_provider_that_accepts_before_it_times_out_is_asked_instead_of_paid_again() {
    let w = world().await;
    let server = serve().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let sdk = Sdk {
        w: &w,
        session: &s,
        url: &server.url,
        http: client(),
    };
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");

    // The provider records the payment and then takes longer than the client waits.
    server
        .provider
        .state
        .lock()
        .expect("lock")
        .slow_next_payment = Some(Duration::from_millis(900));
    let submit = w.one_task(&s).await;
    assert!(!sdk.submit(&submit).await, "the worker saw a timeout");

    // The platform asks before it does anything else.
    let lookup = w.one_task(&s).await;
    assert_eq!(lookup.kind, TaskKind::Reconciliation);
    sdk.look_up(&lookup).await;
    let notify = w.one_task(&s).await;
    assert_eq!(notify.activity.as_str(), "act.notify");
    w.complete(&s, &notify, &json!({})).await.expect("notify");

    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    {
        let state = server.provider.state.lock().expect("lock");
        assert_eq!(state.payments.len(), 1, "exactly one payment exists");
        assert_eq!(
            state.payment_calls.values().sum::<u32>(),
            1,
            "and it was requested once"
        );
    }
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn negative_provider_lookup_requires_human_finality_and_never_pays_again() {
    let w = world().await;
    let server = serve().await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let sdk = Sdk {
        w: &w,
        session: &s,
        url: &server.url,
        http: client(),
    };
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");

    server
        .provider
        .state
        .lock()
        .expect("lock")
        .reject_next_payment = true;
    let first = w.one_task(&s).await;
    assert!(!sdk.submit(&first).await);
    let lookup = w.one_task(&s).await;
    sdk.look_up(&lookup).await; // "not found now" is not final absence.
    w.scheduler.tick().await;

    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert!(w.poll(&s, 8).await.is_empty(), "no second payment attempt");
    {
        let state = server.provider.state.lock().expect("lock");
        assert!(
            state.payments.is_empty(),
            "the rejected request had no effect"
        );
        assert_eq!(state.payment_calls[&first.idempotency_key], 1);
    }

    w.review(
        run,
        "submit",
        ReviewAction::Resolve(ReviewOutcome::Failed(
            FailureCode::parse("NOT_APPLIED_CONFIRMED").expect("code"),
        )),
        "provider operator confirmed final absence",
    )
    .await
    .expect("human finality is recorded");
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({}))
        .await
        .expect("undo reserve");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn failed_required_review_audit_dispatches_nothing_and_retry_commits_once_after_restart() {
    let w = world().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail_with(&s, &submit, "OUTCOME_UNKNOWN", false)
        .await
        .expect("unknown original");
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("lookup remains uncertain");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);

    let before = snapshot(&w, run).await;
    let key = "provider-review-audit-retry-0001";
    let node = sagat_domain::workflow::NodeId::parse_stored("submit").expect("node");
    let action = ReviewAction::Resolve(ReviewOutcome::Succeeded);
    let reason = "provider operator confirmed the payment settled";
    sqlx::raw_sql(
        "CREATE FUNCTION reject_provider_review_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected review audit failure'; END $$; \
         CREATE TRIGGER reject_provider_review_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW WHEN (NEW.action='saga.step_resolved') \
         EXECUTE FUNCTION reject_provider_review_audit()",
    )
    .execute(&w.db.pool)
    .await
    .expect("install audit fault");
    let failed = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &action,
            reason,
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(matches!(failed, Err(AppError::Internal)), "{failed:?}");
    assert_eq!(
        snapshot(&w, run).await,
        before,
        "failed required audit rolls back state, journal, outbox, and projection"
    );
    assert!(
        w.poll(&s, 8).await.is_empty(),
        "no successor was dispatched"
    );
    let failed_effects: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM audit_records WHERE resource_id=$1 AND action='saga.step_resolved'), \
         (SELECT count(*) FROM command_dedupe WHERE scope=$2 AND idempotency_key=$3)",
    )
    .bind(format!("{run}/submit"))
    .bind(format!("run.review:{run}"))
    .bind(key)
    .fetch_one(&w.db.pool)
    .await
    .expect("failed review effects");
    assert_eq!(
        failed_effects,
        (0, 0),
        "no success audit or receipt committed"
    );

    sqlx::raw_sql(
        "DROP TRIGGER reject_provider_review_audit ON audit_records; \
         DROP FUNCTION reject_provider_review_audit()",
    )
    .execute(&w.db.pool)
    .await
    .expect("remove audit fault");
    let replica = w.replica().await;
    replica
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &action,
            reason,
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await
        .expect("same decision commits after restart");
    w.runs
        .review(
            &w.admin,
            run,
            &node,
            &action,
            reason,
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await
        .expect("identical retry replays the committed decision");

    let committed: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.resolved'), \
         (SELECT count(*) FROM audit_records WHERE resource_id=$2 AND action='saga.step_resolved'), \
         (SELECT count(*) FROM command_dedupe WHERE scope=$3 AND idempotency_key=$4)",
    )
    .bind(run.as_uuid())
    .bind(format!("{run}/submit"))
    .bind(format!("run.review:{run}"))
    .bind(key)
    .fetch_one(&w.db.pool)
    .await
    .expect("committed review effects");
    assert_eq!(committed, (1, 1, 1), "one durable decision after retry");
    let notify = w.one_task(&s).await;
    assert_eq!(notify.activity.as_str(), "act.notify");
    w.complete(&s, &notify, &json!({})).await.expect("notify");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_injected_transport_duplicate_uses_the_same_key_without_a_second_engine_attempt() {
    let w = world().await;
    let server = serve().await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let http = client();
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("declined");

    // The worker has one lease; transport fault injection delivers the request twice.
    let first = w.one_task(&s).await;
    let answer: Value = http
        .post(format!("{}/releases", server.url))
        .header("idempotency-key", &first.idempotency_key)
        .send()
        .await
        .expect("release")
        .json()
        .await
        .expect("json");
    assert_eq!(answer["duplicate"], false);
    // Inject a transport duplicate into this synthetic provider under the same live lease.
    // This is not an engine-authorized retry after an unknown outcome or a provider qualification.
    let answer: Value = http
        .post(format!("{}/releases", server.url))
        .header("idempotency-key", &first.idempotency_key)
        .send()
        .await
        .expect("release")
        .json()
        .await
        .expect("json");
    assert_eq!(answer["duplicate"], true, "the provider recognised the key");
    w.complete(&s, &first, &json!({})).await.expect("report");

    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    {
        let state = server.provider.state.lock().expect("lock");
        assert_eq!(state.releases.len(), 1, "one release in the world");
        assert_eq!(state.releases[&first.idempotency_key], 2);
    }
    w.assert_consistent(run).await;
}
