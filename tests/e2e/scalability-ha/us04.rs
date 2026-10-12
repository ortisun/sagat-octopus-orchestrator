//! US04 — Isolate noisy neighbors. Given tenant A fires a burst, when tenant B has ready tasks,
//! then fairness and quotas prevent starvation beyond an explicit budget (US04-AC01).
//!
//! The explicit budget: a tenant that has work due in a pass is served in that pass, up to its
//! share of the batch (`docs/runbooks/scaling.md`). Here B's work is less than its share, so all of
//! it is served in the first pass, while A's burst is still being worked through.

use std::time::{Duration, Instant};

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::worker::{Platform, RunningEngine, engine_env, platform};

const NOISY: usize = 300;
const QUIET: usize = 15;

fn timer_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 3_600_000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    })
}

/// A REST call as `bob`, the administrator of globex, in its namespace `payments`.
async fn globex(p: &Platform, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
    let mut request = p
        .api
        .as_user(method, path, "bob", "globex")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", format!("key-{}", uuid::Uuid::now_v7()));
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Tenant A starts `NOISY` timers, then globex starts `QUIET`; every timer falls due at the same
/// instant; the engine is started only then. Returns how many of A's timers had fired by the time
/// all of B's had.
async fn noisy_then_quiet(fair: bool) -> usize {
    let p = platform().await;
    let ws = p.api.create_workspace("bob", "globex", "platform").await;
    p.api
        .create_namespace("bob", "globex", &ws, "payments", "dev")
        .await;
    let a = p.publish("noisy", &timer_ir("noisy")).await;
    let (status, created) = globex(
        &p,
        Method::POST,
        "/workflows",
        Some(&json!({"name": "quiet", "ir": timer_ir("quiet")})),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let b = created["id"].as_str().expect("id").to_owned();
    let (status, published) = globex(
        &p,
        Method::POST,
        &format!("/workflows/{b}/publish"),
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{published}");
    for i in 0..NOISY {
        p.start(&a, &json!({ "i": i })).await;
    }
    for i in 0..QUIET {
        let (status, run) = globex(
            &p,
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": b, "input": {"i": i}})),
        )
        .await;
        assert_eq!(status, 202, "{run}");
    }
    sqlx::query("UPDATE steps SET due_at = clock_timestamp() - interval '1 second' WHERE waiting_kind = 'TIMER'")
        .execute(&p.api.db.pool)
        .await
        .expect("make every timer due");

    // One pass takes 30 candidates; the engine acts on four at a time.
    let fairness = if fair { "true" } else { "false" };
    let engine = RunningEngine::start(engine_env(
        &p.api.db,
        &p.api.idp,
        &[
            ("SAGAT_SCHEDULER_BATCH", "30"),
            ("SAGAT_SCHEDULER_CONCURRENCY", "4"),
            ("SAGAT_SCHEDULER_FAIRNESS", fairness),
        ],
    ))
    .await;
    let globex_id =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM tenants WHERE slug = 'globex'")
            .fetch_one(&p.api.db.pool)
            .await
            .expect("globex");
    let deadline = Instant::now() + Duration::from_secs(60);
    let noisy_when_quiet_done = loop {
        let (quiet, noisy): (i64, i64) = sqlx::query_as(
            "SELECT count(DISTINCT run_id) FILTER (WHERE tenant_id = $1), \
                    count(DISTINCT run_id) FILTER (WHERE tenant_id <> $1) \
             FROM run_events WHERE event_type = 'timer.fired'",
        )
        .bind(globex_id)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
        if usize::try_from(quiet).expect("fits") == QUIET {
            break usize::try_from(noisy).expect("fits");
        }
        assert!(
            Instant::now() < deadline,
            "globex's timers never all fired ({quiet}/{QUIET})"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    drop(engine);
    noisy_when_quiet_done
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_with_a_little_work_is_served_in_the_first_pass_behind_a_burst_of_another() {
    let noisy_done = noisy_then_quiet(true).await;
    assert!(
        noisy_done <= NOISY / 2,
        "{noisy_done} of the burst's {NOISY} timers fired before the quiet tenant's {QUIET}: it was made to wait"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_fairness_the_same_tenant_waits_for_the_whole_burst() {
    // The negative control: it is what the test above would have seen before feature 010, and it
    // shows that the test above is sensitive to what it claims to prove.
    let noisy_done = noisy_then_quiet(false).await;
    assert_eq!(
        noisy_done, NOISY,
        "with the oldest candidates taken first the quiet tenant is served after the burst"
    );
}
