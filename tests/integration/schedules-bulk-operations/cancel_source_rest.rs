//! `CANCEL_SOURCE` over HTTP (`T018-057`, `FR-018-038`, `FR-018-041`): the request of a reset of a run that has
//! not ended answers `202` with an entry that waits, a retry is the same entry, a deployment that did not enable it
//! says so with a reason, the namespace's limit is a `429` with its reason, and the register shows the entry to
//! who may read it. A real API, a real PostgreSQL, a real worker over gRPC, real tokens. (The engine pass that
//! creates the run is proved in `cancel_source.rs` and, with a real engine process, by the SDK conformance
//! scenario `reset`.)
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::worker::{HUMAN, Platform, TENANT, chain_ir, platform, platform_with};

const REASON: &str = "the second step used a wrong price list";

fn reason(error: &Value) -> &str {
    error["details"]["reason"].as_str().unwrap_or_default()
}

/// `POST /runs/{run}/resets` with the key given.
async fn reset_with(p: &Platform, run: &str, key: &str, body: &Value) -> (u16, Value) {
    let response = p
        .api
        .as_user(Method::POST, &format!("/runs/{run}/resets"), HUMAN, TENANT)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn cancel_source(sequence: u64) -> Value {
    json!({
        "sequence": sequence, "reason": REASON,
        "sourcePolicy": "CANCEL_SOURCE", "acknowledgeReexecution": true
    })
}

/// A run whose first activity is leased to a worker: it is open, and cancelling it leaves it `CANCELLING` until the
/// attempt in flight reports.
async fn in_flight(p: &Platform, workflow: &str) -> String {
    let run = p.start(workflow, &json!({"item": "book"})).await;
    let mut worker = p.worker(&format!("cancel-source-worker-{run}")).await;
    worker.register(&["act.a", "act.b"]).await;
    p.tick().await;
    let _task = worker.one_task().await;
    assert_eq!(p.status(&run).await, "RUNNING");
    run
}

#[tokio::test]
async fn the_request_over_http_cancels_the_open_run_and_answers_202_with_an_entry_that_waits() {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = in_flight(&p, &workflow).await;

    let (status, made) = reset_with(&p, &run, "cancel-source-key-0001", &cancel_source(1)).await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(made["state"], "PENDING_SOURCE");
    assert_eq!(made["newRunId"], Value::Null);
    assert_eq!(made["failureReason"], Value::Null);
    assert_eq!(made["sourceRunId"], run);
    assert_eq!(made["reapplyReport"], json!([]));
    assert_eq!(
        p.status(&run).await,
        "CANCELLING",
        "cancelled in the ordinary way"
    );
    let reset_id = made["resetId"].as_str().expect("id").to_owned();

    // a retry is the same entry, and cancels nothing a second time
    let (status, again) = reset_with(&p, &run, "cancel-source-key-0001", &cancel_source(1)).await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["resetId"], reset_id);
    assert_eq!(again["state"], "PENDING_SOURCE");
    // the same key for another request is not a retry
    let (status, other) = reset_with(&p, &run, "cancel-source-key-0001", &cancel_source(2)).await;
    assert_eq!(status, 409, "{other}");
    assert_eq!(reason(&other), "IDEMPOTENCY_KEY_REUSED");
    let (status, plain) = reset_with(
        &p,
        &run,
        "cancel-source-key-0001",
        &json!({"sequence": 1, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 409, "{plain}");
    assert_eq!(reason(&plain), "IDEMPOTENCY_KEY_REUSED");

    // the register shows it, to who may read runs
    let (status, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"].as_array().expect("items").len(), 1);
    assert_eq!(listed["items"][0]["resetId"], reset_id);
    assert_eq!(listed["items"][0]["state"], "PENDING_SOURCE");
    let (status, one) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{reset_id}"), None)
        .await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one["state"], "PENDING_SOURCE");
}

#[tokio::test]
async fn a_deployment_that_did_not_enable_it_refuses_an_open_source_with_a_reason_and_cancels_nothing()
 {
    let p = platform_with(Options {
        reset_cancel_source: false,
        ..Options::default()
    })
    .await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = in_flight(&p, &workflow).await;

    let (status, refused) = reset_with(&p, &run, "cancel-source-key-0010", &cancel_source(1)).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(refused["code"], "VALIDATION_FAILED");
    assert_eq!(reason(&refused), "RESET_CANCEL_SOURCE_NOT_AVAILABLE");
    assert!(
        refused["remediation"]
            .as_str()
            .unwrap_or_default()
            .contains("operator of the platform"),
        "{refused}"
    );
    assert_eq!(p.status(&run).await, "RUNNING", "nothing was cancelled");
    let (_, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(listed["items"], json!([]));

    // the policy that does not cancel is refused as it always was
    let (status, plain) = reset_with(
        &p,
        &run,
        "cancel-source-key-0011",
        &json!({"sequence": 1, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 409, "{plain}");
    assert_eq!(reason(&plain), "RESET_SOURCE_NOT_TERMINAL");
}

#[tokio::test]
async fn the_namespace_holds_at_most_the_pending_resets_its_limit_allows_and_says_so() {
    let p = platform().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2::uuid, 'pending_resets', 1, 'admin')",
    )
    .bind(p.tenant.as_uuid())
    .bind(p.namespace_id().await)
    .execute(&p.api.db.pool)
    .await
    .expect("a limit of one");
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let first = in_flight(&p, &workflow).await;
    let second = in_flight(&p, &workflow).await;

    let (status, made) = reset_with(&p, &first, "cancel-source-key-0020", &cancel_source(1)).await;
    assert_eq!(status, 202, "{made}");
    let (status, refused) =
        reset_with(&p, &second, "cancel-source-key-0021", &cancel_source(1)).await;
    assert_eq!(status, 429, "{refused}");
    assert_eq!(refused["code"], "QUOTA_EXCEEDED");
    assert_eq!(reason(&refused), "PENDING_RESET_LIMIT");
    assert!(refused["details"]["limit"].is_string(), "{refused}");
    assert_eq!(
        p.status(&second).await,
        "RUNNING",
        "the refused request cancelled nothing"
    );

    // a retry of the one that was accepted is answered, the limit notwithstanding
    let (status, again) = reset_with(&p, &first, "cancel-source-key-0020", &cancel_source(1)).await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["resetId"], made["resetId"]);
}

#[tokio::test]
async fn who_may_not_reset_may_not_cancel_by_resetting_and_a_stranger_sees_nothing() {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = in_flight(&p, &workflow).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;

    let (status, _) = p
        .rest_as(
            "vera",
            Method::POST,
            &format!("/runs/{run}/resets"),
            Some(&cancel_source(1)),
        )
        .await;
    assert_eq!(status, 403);
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/resets"),
            "bob",
            "globex",
        )
        .header("idempotency-key", "cancel-source-key-0029")
        .json(&cancel_source(1))
        .send()
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        404,
        "a run of another tenant is not there"
    );
    assert_eq!(p.status(&run).await, "RUNNING", "nobody cancelled it");

    // a policy the API does not know is a validation error, not a guess
    let (status, bad) = reset_with(
        &p,
        &run,
        "cancel-source-key-0030",
        &json!({"sequence": 1, "reason": REASON, "sourcePolicy": "SOMETIMES"}),
    )
    .await;
    assert_eq!(status, 422, "{bad}");
    assert_eq!(p.status(&run).await, "RUNNING");
}

/// Looks at `GET /runs/{run}/resets/{id}` until the entry has left `PENDING_SOURCE`, for at most two minutes.
async fn settled(p: &Platform, run: &str, reset: &str) -> Value {
    crate::support::bounded::within(
        "the engine completes the reset",
        std::time::Duration::from_secs(120),
        async {
            loop {
                let (status, entry) = p
                    .rest(Method::GET, &format!("/runs/{run}/resets/{reset}"), None)
                    .await;
                assert_eq!(status, 200, "{entry}");
                if entry["state"] != "PENDING_SOURCE" {
                    return entry;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        },
    )
    .await
}

#[tokio::test]
async fn an_engine_started_with_the_flag_creates_the_run_of_the_reset_the_api_accepted() {
    use crate::support::worker::{RunningEngine, engine_env};
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = p.start(&workflow, &json!({"item": "book"})).await;
    let mut worker = p.worker("cancel-source-wiring-worker").await;
    worker.register(&["act.a", "act.b"]).await;
    p.tick().await;
    let task = worker.one_task().await;

    let (status, made) = reset_with(&p, &run, "cancel-source-key-0040", &cancel_source(1)).await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(made["state"], "PENDING_SOURCE");
    worker
        .complete(&task, &json!({"ok": true}))
        .await
        .expect("the attempt in flight completes after the cancellation");
    assert_eq!(p.status(&run).await, "CANCELLED");
    let reset_id = made["resetId"].as_str().expect("id").to_owned();

    // the flag the control plane read is the one the engine reads: with it, the engine makes the run
    let mut engine = RunningEngine::start(engine_env(
        &p.api.db,
        &p.api.idp,
        &[
            ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
            ("SAGAT_RESET_CANCEL_SOURCE_POLL_MS", "50"),
        ],
    ))
    .await;
    let entry = settled(&p, &run, &reset_id).await;
    assert_eq!(entry["state"], "CREATED", "{entry}");
    let new = entry["newRunId"].as_str().expect("the run it made");
    assert_ne!(new, run);
    assert_eq!(p.run(new).await["rerunOf"], run);
    engine.finish().await.expect("the engine stops");
}

// ------------------------------------------------------------------ the race for the last place (review 2026-11-06, C)

/// Waits until `n` backends wait for an advisory lock (`pg_stat_activity`, not a time), at most two minutes.
async fn wait_for_advisory_waiters(p: &Platform, n: i64, stage: &str) {
    crate::support::bounded::within(stage, std::time::Duration::from_secs(120), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid() \
                   AND wait_event_type = 'Lock' AND wait_event = 'advisory'",
            )
            .fetch_one(&p.api.db.pool)
            .await
            .expect("look at the backends");
            if waiting >= n {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

#[tokio::test]
async fn two_requests_for_the_last_place_are_judged_one_after_the_other() {
    let p = platform().await;
    let namespace = p.namespace_id().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2::uuid, 'pending_resets', 1, 'admin')",
    )
    .bind(p.tenant.as_uuid())
    .bind(&namespace)
    .execute(&p.api.db.pool)
    .await
    .expect("one place");
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let first = in_flight(&p, &workflow).await;
    let second = in_flight(&p, &workflow).await;

    // the lock the request takes before it counts is held from a side connection: both requests reach it, and neither
    // has counted when the other is let through
    let mut holder = p.api.db.pool.begin().await.expect("a side connection");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota:pending_resets:{}:{namespace}", p.tenant))
        .execute(&mut *holder)
        .await
        .expect("hold the key of the pending resets");

    let body = cancel_source(1);
    let (a, b, ()) = tokio::join!(
        reset_with(&p, &first, "cancel-source-key-0040", &body),
        reset_with(&p, &second, "cancel-source-key-0041", &body),
        async {
            wait_for_advisory_waiters(&p, 2, "both requests wait for the place").await;
            holder.commit().await.expect("release the key");
        }
    );

    let mut statuses = [a.0, b.0];
    statuses.sort_unstable();
    assert_eq!(statuses, [202, 429], "{a:?} {b:?}");
    let ((winner, _), (loser, refused)) = if a.0 == 202 {
        ((&first, a.1), (&second, b.1))
    } else {
        ((&second, b.1), (&first, a.1))
    };
    assert_eq!(refused["code"], "QUOTA_EXCEEDED", "{refused}");
    assert_eq!(reason(&refused), "PENDING_RESET_LIMIT");
    // the loser was refused before anything was done to its run; the winner's run is being cancelled
    assert_eq!(
        p.status(loser).await,
        "RUNNING",
        "the refused request cancelled nothing"
    );
    assert_eq!(p.status(winner).await, "CANCELLING");
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_resets WHERE tenant_id = $1 AND state = 'PENDING_SOURCE'",
    )
    .bind(p.tenant.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the register");
    assert_eq!(waiting, 1, "one place, one entry");
}

// ------------------------------------------------------ the acknowledgement is asked before anything is cancelled (D2008)

#[tokio::test]
async fn a_step_in_flight_needs_the_acknowledgement_and_a_request_without_it_cancels_nothing() {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = in_flight(&p, &workflow).await;

    // the preview says it for the run that is open: the attempt that is in flight can complete after the cancellation
    let (status, plan) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": 1})),
        )
        .await;
    assert_eq!(status, 200, "{plan}");
    assert_eq!(plan["requiresAcknowledgement"], true, "{plan}");
    assert!(
        plan["reexecuted"]
            .as_array()
            .expect("reexecuted")
            .iter()
            .any(|step| step["node"] == "a" && step["completedInSource"] == false),
        "{plan}"
    );

    // a request that does not give it is refused with its reason, before the run is touched
    let without = json!({"sequence": 1, "reason": REASON, "sourcePolicy": "CANCEL_SOURCE"});
    let (status, refused) = reset_with(&p, &run, "cancel-source-key-0050", &without).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "RESET_REEXECUTION_NOT_ACKNOWLEDGED");
    assert_eq!(p.status(&run).await, "RUNNING", "nothing was cancelled");
    let (_, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(listed["items"], json!([]), "and no entry was written");

    // with the acknowledgement it is accepted, and the source is being cancelled
    let (status, made) = reset_with(&p, &run, "cancel-source-key-0051", &cancel_source(1)).await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(p.status(&run).await, "CANCELLING");
}

// ------------------------------------------------------------------ the expiry of an entry that waits (D2059)

/// What a caller reads of an entry that waited a whole window for a source that did not end: over HTTP, as the register
/// shows it to who may read runs. The expiry is made by the engine's step on the API's own store (the runtime role in the
/// runtime-role sweep), under a clock a window past the request.
#[tokio::test]
async fn an_entry_that_expired_reads_failed_with_its_reason_and_no_run_and_the_source_is_as_it_was()
{
    use sagat_application::reset_completion::{
        Completed, DueReset, ResetCompleter, SOURCE_DID_NOT_END,
    };
    use sagat_domain::execution::ResetId;
    use sagat_infra_postgres::PgIdentityStore;
    use time::OffsetDateTime;

    use crate::cancel_source_expiry::NeverCompletes;
    use crate::support::schedules::TestClock;

    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let run = in_flight(&p, &workflow).await;
    let (status, made) = reset_with(&p, &run, "cancel-source-key-0060", &cancel_source(1)).await;
    assert_eq!(status, 202, "{made}");
    let reset_id = made["resetId"].as_str().expect("id").to_owned();
    let id = ResetId::new(reset_id.parse().expect("an id"));
    let asked: OffsetDateTime =
        sqlx::query_scalar("SELECT created_at FROM run_resets WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&p.api.db.pool)
            .await
            .expect("when it was asked");

    // inside the window it waits, and the read says so
    let completer = |at: OffsetDateTime| {
        ResetCompleter::new(
            p.api.executions.clone(),
            NeverCompletes,
            PgIdentityStore::new(p.api.api_pool.clone()),
            TestClock::at(at),
        )
    };
    let day = time::Duration::days(1);
    let due = DueReset {
        tenant_id: p.tenant,
        id,
    };
    assert_eq!(
        completer(asked + day - time::Duration::microseconds(1))
            .expire(&due, asked - time::Duration::microseconds(1), None)
            .await
            .expect("inside the window"),
        Completed::Idle
    );
    let (_, waiting) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{reset_id}"), None)
        .await;
    assert_eq!(waiting["state"], "PENDING_SOURCE", "{waiting}");

    // a window after the request, with the source still being cancelled, it ends
    assert_eq!(
        completer(asked + day)
            .expire(&due, asked, None)
            .await
            .expect("the expiry"),
        Completed::Expired
    );
    let (status, one) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{reset_id}"), None)
        .await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one["state"], "FAILED", "{one}");
    assert_eq!(one["failureReason"], SOURCE_DID_NOT_END, "{one}");
    assert_eq!(one["newRunId"], Value::Null, "{one}");
    assert_eq!(one["sourceRunId"], run);
    assert_eq!(one["reapplyReport"], json!([]));
    let (_, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(
        listed["items"].as_array().expect("items").len(),
        1,
        "{listed}"
    );
    assert_eq!(listed["items"][0]["state"], "FAILED");
    assert_eq!(listed["items"][0]["failureReason"], SOURCE_DID_NOT_END);
    // the run was cancelled by the request and stays so: the expiry did not touch it
    assert_eq!(p.status(&run).await, "CANCELLING");
    // and the place is free: nothing waits in the namespace
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_resets WHERE tenant_id = $1 AND state = 'PENDING_SOURCE'",
    )
    .bind(p.tenant.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the register");
    assert_eq!(waiting, 0);
    // a retry of the request that was accepted is still answered from the register, whatever became of the entry
    let (status, again) = reset_with(&p, &run, "cancel-source-key-0060", &cancel_source(1)).await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["resetId"], reset_id);
    assert_eq!(again["state"], "FAILED", "{again}");
}
