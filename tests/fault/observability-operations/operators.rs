//! What goes wrong around an operator's action: the manual retry races the automatic one, a worker
//! is drained while it holds work, two operators ask for the same thing (FR-008-004, the edge cases
//! "manual retry racing with auto retry").

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::{Platform, platform, single_ir};

fn retrying() -> Value {
    single_ir(
        "payment",
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1000, "maxDelayMs": 1000, "jitter": false}),
        ),
    )
}

async fn leases_of(p: &Platform, run: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1::uuid AND event_type = 'step.leased'",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count")
}

async fn count_of(p: &Platform, run: &str, event_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1::uuid AND event_type = $2",
    )
    .bind(run)
    .bind(event_type)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count")
}

#[tokio::test]
async fn a_manual_retry_and_the_automatic_one_start_exactly_one_attempt_whoever_is_first() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let workflow = p.publish("payment", &retrying()).await;
    let runs: Vec<String> = {
        let mut out = Vec::new();
        for _ in 0..12 {
            out.push(p.start(&workflow, &json!({})).await);
        }
        out
    };
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    for lease in worker.poll(12).await {
        worker.fail(&lease, true).await.expect("fails");
    }
    // Every retry is due at this moment: the scheduler and the operators arrive together.
    for run in &runs {
        p.elapse_retry_delay(run, "a").await;
    }
    let operators = runs.iter().map(|run| {
        let p = &p;
        async move {
            p.rest_as(
                "olivia",
                Method::POST,
                &format!("/runs/{run}/operations"),
                Some(&json!({"action": "RETRY_STEP", "stepId": "a", "reason": "do not wait for the delay"})),
            )
            .await
            .0
        }
    });
    let (statuses, _) = tokio::join!(futures_util::future::join_all(operators), p.tick());
    for status in &statuses {
        assert!(
            matches!(status, 202 | 409),
            "the operator is either first or told the scheduler was: {status}"
        );
    }
    for (run, status) in runs.iter().zip(&statuses) {
        let automatic = count_of(&p, run, "step.retry_due").await;
        let manual = count_of(&p, run, "step.retry_requested").await;
        assert_eq!(
            automatic + manual,
            1,
            "one of them started the attempt: {automatic} + {manual}"
        );
        assert_eq!(
            manual == 1,
            *status == 202,
            "and the operator was told which"
        );
    }
    // One attempt each, not two: the next poll hands out exactly 12 tasks and no more.
    let tasks = worker.poll(50).await;
    assert_eq!(tasks.len(), 12, "twelve runs, one new attempt each");
    for run in &runs {
        assert_eq!(
            leases_of(&p, run).await,
            2,
            "the first attempt and one more, for {run}"
        );
    }
}

#[tokio::test]
async fn two_operators_asking_for_the_same_retry_get_one_attempt_and_one_refusal() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "omar", Role::Operator).await;
    let workflow = p.publish("payment", &retrying()).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    worker.fail(&lease, true).await.expect("fails");
    let ask = |sub: &'static str| {
        let (p, run) = (&p, run.clone());
        async move {
            p.rest_as(
                sub,
                Method::POST,
                &format!("/runs/{run}/operations"),
                Some(&json!({"action": "RETRY_STEP", "stepId": "a", "reason": "the provider is back"})),
            )
            .await
            .0
        }
    };
    let (a, b) = tokio::join!(ask("olivia"), ask("omar"));
    let mut both = [a, b];
    both.sort_unstable();
    assert_eq!(both, [202, 409]);
    assert_eq!(count_of(&p, &run, "step.retry_requested").await, 1);
    assert_eq!(worker.poll(5).await.len(), 1);
}

#[tokio::test]
async fn a_drain_and_a_poll_at_the_same_moment_never_hand_out_work_after_the_drain_returned() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let ns = p.namespace_id().await;
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    for _ in 0..20 {
        p.start(&workflow, &json!({})).await;
    }
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let drain = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/namespaces/{ns}/workers/worker-1/drain"),
            Some(&json!({"reason": "deploying a new version"})),
        )
        .await;
    assert_eq!(drain.0, 200);
    // From the moment the call returned, however hard the worker asks, it is given nothing.
    for _ in 0..10 {
        assert!(worker.poll(10).await.is_empty());
    }
    let leased: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE status IN ('LEASED','RUNNING')")
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
    assert_eq!(leased, 0);
}

#[tokio::test]
async fn a_drain_of_one_namespace_does_not_stop_the_same_identity_in_another() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    // A second namespace of the same tenant.
    let (_, workspaces) = p.rest(Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"]
        .as_str()
        .expect("workspace")
        .to_owned();
    let (status, other) = p
        .rest(
            Method::POST,
            &format!("/workspaces/{workspace}/namespaces"),
            Some(&json!({"slug": "billing", "environment": "dev"})),
        )
        .await;
    assert_eq!(status, 201, "{other}");
    let billing = other["id"].as_str().expect("id").to_owned();
    let payments = p.namespace_id().await;
    let (status, drained) = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/namespaces/{billing}/workers/worker-1/drain"),
            Some(&json!({"reason": "billing is being migrated"})),
        )
        .await;
    assert_eq!(status, 200, "{drained}");
    // The worker serves `payments`: not drained there.
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    assert_eq!(
        worker.poll(5).await.len(),
        1,
        "drained in billing, not in payments"
    );
    let (_, listed) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{payments}/workers"),
            None,
        )
        .await;
    assert!(listed["drains"].as_array().expect("drains").is_empty());
}
