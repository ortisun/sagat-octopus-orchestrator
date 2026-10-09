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
use sagat_application::worker::TaskKind;
use sagat_domain::execution::RunStatus;
use serde_json::{Value, json};

use crate::common::{quick, settlement, worker};
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
async fn a_provider_that_failed_before_accepting_is_asked_and_then_paid_with_the_same_key() {
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
    sdk.look_up(&lookup).await; // "not found": it did not happen
    w.age_retry(run, "submit").await;
    w.scheduler.tick().await;

    let second = w.one_task(&s).await;
    assert_eq!(second.activity.as_str(), "act.submit");
    assert_eq!(
        second.idempotency_key, first.idempotency_key,
        "the provider can tie the two together"
    );
    assert!(sdk.submit(&second).await);
    {
        let state = server.provider.state.lock().expect("lock");
        assert_eq!(state.payments.len(), 1);
        assert_eq!(state.payment_calls[&first.idempotency_key], 2);
    }
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_release_delivered_twice_is_performed_once_and_the_duplicate_is_said_so() {
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

    // Worker 1 asks the provider to release and dies before reporting the answer.
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
    w.age_lease(run, "reserve~c").await;
    w.scheduler.tick().await;
    w.age_retry(run, "reserve~c").await;
    w.scheduler.tick().await;

    // Worker 2 asks again, with the key the platform keeps stable, and is told it is a repeat.
    let session_two = worker(&w).await;
    let second = w.one_task(&session_two).await;
    assert_eq!(second.idempotency_key, first.idempotency_key);
    let answer: Value = http
        .post(format!("{}/releases", server.url))
        .header("idempotency-key", &second.idempotency_key)
        .send()
        .await
        .expect("release")
        .json()
        .await
        .expect("json");
    assert_eq!(answer["duplicate"], true, "the provider recognised the key");
    w.complete(&session_two, &second, &json!({}))
        .await
        .expect("report");

    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    {
        let state = server.provider.state.lock().expect("lock");
        assert_eq!(state.releases.len(), 1, "one release in the world");
        assert_eq!(state.releases[&first.idempotency_key], 2);
    }
    w.assert_consistent(run).await;
}
