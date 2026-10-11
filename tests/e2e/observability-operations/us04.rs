//! US04 — execute an audited operational action: an operator with the permission retries a step with
//! a justification and the new attempts appear without erasing the previous ones (US04-AC01); holds a
//! step for review; starts a run again; drains a worker (FR-008-004, FR-008-006).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::worker::{Platform, platform, single_ir};

const REASON: &str = "the provider confirmed it is back up";

fn slow_retry() -> Value {
    // A retry that would wait an hour: nobody but an operator is going to start the next attempt.
    single_ir(
        "payment",
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 3_600_000, "maxDelayMs": 3_600_000, "jitter": false}),
        ),
    )
}

async fn operate(p: &Platform, sub: &str, run: &str, body: &Value) -> (u16, Value) {
    p.rest_as(
        sub,
        Method::POST,
        &format!("/runs/{run}/operations"),
        Some(body),
    )
    .await
}

async fn journal_types(p: &Platform, run: &str) -> Vec<String> {
    let (_, page) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|e| e["eventType"].as_str().expect("type").to_owned())
        .collect()
}

async fn audit(p: &Platform, action: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT resource_id, reason FROM audit_records WHERE action = $1 ORDER BY created_at",
    )
    .bind(action)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("audit")
}

#[tokio::test]
async fn us04_an_operator_retries_a_step_waiting_for_its_retry_and_the_attempts_before_it_stay() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let workflow = p.publish("payment", &slow_retry()).await;
    let run = p.start(&workflow, &json!({"n": 1})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let first = worker.one_task().await;
    worker
        .fail(&first, true)
        .await
        .expect("the provider was down");
    assert!(worker.poll(1).await.is_empty(), "the retry is an hour away");

    // A person who may only look is refused; the one who may operate, without a reason, is refused too.
    let body = json!({"action": "RETRY_STEP", "stepId": "a", "reason": REASON});
    assert_eq!(operate(&p, "vera", &run, &body).await.0, 403);
    assert_eq!(
        operate(
            &p,
            "olivia",
            &run,
            &json!({"action": "RETRY_STEP", "stepId": "a", "reason": "ok"})
        )
        .await
        .0,
        422
    );

    let (status, accepted) = operate(&p, "olivia", &run, &body).await;
    assert_eq!(status, 202, "{accepted}");
    let second = worker.one_task().await;
    assert_eq!(second.attempt, 2, "new attempts appear");

    // The first attempt is still in the journal, with its failure; the retry says who asked and why it was possible.
    let types = journal_types(&p, &run).await;
    assert_eq!(
        types.iter().filter(|t| t.as_str() == "step.leased").count(),
        2
    );
    assert!(
        types.contains(&"step.retry_scheduled".to_owned())
            && types.contains(&"step.retry_requested".to_owned())
    );
    let attempts = p
        .rest(Method::GET, &format!("/runs/{run}/attempts"), None)
        .await
        .1;
    assert_eq!(attempts["items"].as_array().expect("items").len(), 2);
    assert_eq!(
        attempts["items"][0]["outcome"], "retry",
        "the first attempt keeps its outcome"
    );
    let (_, page) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    let requested = page["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|e| e["eventType"] == "step.retry_requested")
        .expect("the request")
        .clone();
    assert_eq!(requested["actor"], "user:olivia");
    assert_eq!(
        requested["cause"],
        json!({"kind": "decision", "code": "RETRY"})
    );

    // Audited with the justification, who and which step.
    let rows = audit(&p, "saga.step_retried").await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.ends_with("/a"), "{rows:?}");
    assert_eq!(rows[0].1.as_deref(), Some(REASON));

    // Asking again for the same thing is refused: the first one won.
    let (status, again) = operate(&p, "olivia", &run, &body).await;
    assert_eq!(status, 409, "{again}");
    worker
        .complete(&second, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn us04_a_step_that_has_not_started_can_be_held_for_a_person_and_then_released() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;

    let hold = json!({"action": "MARK_MANUAL_REVIEW", "stepId": "a", "reason": "the provider asked us to wait"});
    let (status, accepted) = operate(&p, "olivia", &run, &hold).await;
    assert_eq!(status, 202, "{accepted}");
    assert_eq!(accepted["runStatus"], "MANUAL_REVIEW");
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW");
    assert!(
        worker.poll(1).await.is_empty(),
        "a held step is given to nobody"
    );
    let (_, journal) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    let held = journal["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|e| e["eventType"] == "step.needs_review")
        .expect("held")
        .clone();
    assert_eq!(
        held["cause"],
        json!({"kind": "review", "code": "MARKED_FOR_REVIEW"})
    );
    assert_eq!(audit(&p, "run.marked_for_review").await.len(), 1);

    // Holding it twice is refused; so is holding what a worker has, and so is holding a lookup.
    assert_eq!(operate(&p, "olivia", &run, &hold).await.0, 409);
    let (status, released) = operate(
        &p,
        "olivia",
        &run,
        &json!({"action": "RETRY_STEP", "stepId": "a", "reason": "the provider is ready now"}),
    )
    .await;
    assert_eq!(status, 202, "{released}");
    let lease = worker.one_task().await;
    assert_eq!(lease.attempt, 1, "it never ran before");
    assert_eq!(operate(&p, "olivia", &run, &hold).await.0, 409, "in flight");
    assert_eq!(operate(&p, "olivia", &run, &json!({"action": "MARK_MANUAL_REVIEW", "stepId": "a~r", "reason": "hold the lookup please"})).await.0, 404, "there is no lookup here");
    worker.complete(&lease, &json!({})).await.expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    // A step is needed.
    assert_eq!(
        operate(
            &p,
            "olivia",
            &run,
            &json!({"action": "MARK_MANUAL_REVIEW", "reason": "hold the whole run please"})
        )
        .await
        .0,
        422
    );
}

#[tokio::test]
async fn us04_a_run_that_ended_is_started_again_with_its_input_and_says_where_it_came_from() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    let input = json!({"orderId": "o-77", "amount": 125.5});
    let run = p.start(&workflow, &input).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let (status, reason) = operate(
        &p,
        "olivia",
        &run,
        &json!({"action": "RERUN_AS_NEW", "reason": "start it again please"}),
    )
    .await;
    assert_eq!(
        status, 409,
        "a run that is still going is not started again: {reason}"
    );
    let lease = worker.one_task().await;
    worker.fail(&lease, false).await.expect("fails for good");
    assert_eq!(p.status(&run).await, "FAILED");
    let original = p.run(&run).await;

    assert_eq!(
        operate(
            &p,
            "vera",
            &run,
            &json!({"action": "RERUN_AS_NEW", "reason": "start it again please"})
        )
        .await
        .0,
        403
    );
    let key = "rerun-key-0001";
    let ask = |reason: &'static str| {
        let (p, run) = (&p, run.clone());
        async move {
            p.api
                .as_user(
                    Method::POST,
                    &format!("/runs/{run}/operations"),
                    "olivia",
                    "acme",
                )
                .header("x-octopus-namespace", "payments")
                .header("idempotency-key", key)
                .json(&json!({"action": "RERUN_AS_NEW", "reason": reason}))
                .send()
                .await
                .expect("send")
        }
    };
    let response = ask("the provider fixed the bug").await;
    assert_eq!(response.status().as_u16(), 202);
    let accepted: Value = response.json().await.expect("json");
    let new_run = accepted["newRunId"]
        .as_str()
        .expect("the new run")
        .to_owned();
    assert_ne!(new_run, run);
    assert_eq!(
        accepted["runStatus"], "FAILED",
        "the answer is about the run it started from"
    );

    // The same request again is the same run, not another.
    let again: Value = ask("the provider fixed the bug")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(again["newRunId"], new_run.as_str());
    let (_, listed) = p
        .rest(
            Method::GET,
            &format!("/runs?workflowId={workflow}&limit=10"),
            None,
        )
        .await;
    assert_eq!(listed["items"].as_array().expect("items").len(), 2);

    let fresh = p.run(&new_run).await;
    assert_eq!(fresh["rerunOf"], run.as_str());
    assert_eq!(
        fresh["versionId"], original["versionId"],
        "the same version"
    );
    assert_eq!(fresh["status"], "RUNNING");
    assert_ne!(fresh["traceId"], original["traceId"], "a trace of its own");
    // The worker is handed the same input.
    let lease = worker.one_task().await;
    assert_eq!(lease.run_id, new_run);
    let given: Value = serde_json::from_str(&lease.input_json).expect("json input");
    assert_eq!(given["input"], input);
    worker
        .complete(&lease, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&new_run).await, "SUCCEEDED");
    assert_eq!(
        p.status(&run).await,
        "FAILED",
        "and the old run is as it was"
    );

    let rows = audit(&p, "run.rerun").await;
    assert_eq!(rows.len(), 1, "once, however many times it was asked");
    assert!(
        rows[0]
            .1
            .as_deref()
            .is_some_and(|r| r.contains(&run) && r.contains("the provider fixed the bug")),
        "{rows:?}"
    );
    // The same key for another run is not the same request.
    let other = p.start(&workflow, &json!({})).await;
    let mut worker2 = p.connect("worker-1", "acme").await;
    worker2.register(&["act.a"]).await;
    let task = worker2.one_task().await;
    worker2.fail(&task, false).await.expect("fails");
    let clash = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{other}/operations"),
            "olivia",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", key)
        .json(&json!({"action": "RERUN_AS_NEW", "reason": "start it again please"}))
        .send()
        .await
        .expect("send");
    assert_eq!(
        clash.status().as_u16(),
        409,
        "a key used for another run is refused"
    );
}

#[tokio::test]
async fn us04_a_drained_worker_is_given_nothing_new_keeps_what_it_holds_and_can_be_let_back() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let ns = p.namespace_id().await;
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    let first = p.start(&workflow, &json!({})).await;
    let second = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let held = worker.poll(1).await;
    assert_eq!(held.len(), 1, "it holds one attempt");

    let drain = format!("/namespaces/{ns}/workers/worker-1/drain");
    assert_eq!(
        p.rest_as(
            "vera",
            Method::POST,
            &drain,
            Some(&json!({"reason": "deploying a new version"}))
        )
        .await
        .0,
        403,
        "a viewer may look and not drain"
    );
    assert_eq!(
        p.rest_as(
            "olivia",
            Method::POST,
            &drain,
            Some(&json!({"reason": "no"}))
        )
        .await
        .0,
        422
    );
    let (status, drained) = p
        .rest_as(
            "olivia",
            Method::POST,
            &drain,
            Some(&json!({"reason": "deploying a new version"})),
        )
        .await;
    assert_eq!(status, 200, "{drained}");
    assert_eq!(drained["subject"], "worker-1");
    assert_eq!(drained["drainedBy"], "olivia");

    // Nothing new, though a step is ready; what it holds is still its own.
    assert!(worker.poll(1).await.is_empty());
    let (status, listed) = p
        .rest_as(
            "vera",
            Method::GET,
            &format!("/namespaces/{ns}/workers"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{listed}");
    let sessions = listed["items"].as_array().expect("items");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        (
            sessions[0]["draining"].as_bool(),
            sessions[0]["inFlight"].as_u64()
        ),
        (Some(true), Some(1))
    );
    assert_eq!(listed["drains"][0]["reason"], "deploying a new version");
    worker
        .complete(&held[0], &json!({}))
        .await
        .expect("what it held can be finished");
    let (_, listed) = p
        .rest(Method::GET, &format!("/namespaces/{ns}/workers"), None)
        .await;
    assert_eq!(listed["items"][0]["inFlight"], 0, "it can be stopped now");

    // A restart does not undo it.
    let mut restarted = p.connect("worker-1", "acme").await;
    restarted.register(&["act.a"]).await;
    assert!(
        restarted.poll(1).await.is_empty(),
        "the identity is drained, not the session"
    );

    let undrain = format!("/namespaces/{ns}/workers/worker-1/undrain");
    let (status, back) = p
        .rest_as(
            "olivia",
            Method::POST,
            &undrain,
            Some(&json!({"reason": "the new version is out"})),
        )
        .await;
    assert_eq!(
        (status, back["wasDraining"].as_bool()),
        (200, Some(true)),
        "{back}"
    );
    let task = restarted.one_task().await;
    restarted
        .complete(&task, &json!({}))
        .await
        .expect("complete");
    let (_, again) = p
        .rest_as(
            "olivia",
            Method::POST,
            &undrain,
            Some(&json!({"reason": "the new version is out"})),
        )
        .await;
    assert_eq!(again["wasDraining"], false, "nothing to undo");

    assert_eq!(audit(&p, "worker.drained").await.len(), 1);
    assert_eq!(
        audit(&p, "worker.undrained").await.len(),
        1,
        "an undo that undid nothing leaves nothing"
    );
    let _ = (first, second);
}

#[tokio::test]
async fn us04_the_actions_never_cross_a_tenant() {
    let p = platform().await;
    let ns = p.namespace_id().await;
    let workflow = p.publish("payment", &single_ir("payment", None)).await;
    let run = p.start(&workflow, &json!({})).await;
    for (method, path, body) in [
        (Method::GET, format!("/namespaces/{ns}/workers"), None),
        (
            Method::POST,
            format!("/namespaces/{ns}/workers/worker-1/drain"),
            Some(json!({"reason": "an attempt from outside"})),
        ),
        (
            Method::POST,
            format!("/runs/{run}/operations"),
            Some(json!({"action": "RERUN_AS_NEW", "reason": "an attempt from outside"})),
        ),
        (
            Method::POST,
            format!("/runs/{run}/operations"),
            Some(
                json!({"action": "MARK_MANUAL_REVIEW", "stepId": "a", "reason": "an attempt from outside"}),
            ),
        ),
    ] {
        let mut request = p
            .api
            .as_user(method.clone(), &path, "bob", "globex")
            .header("idempotency-key", "outside-key-0001");
        if let Some(body) = &body {
            request = request.json(body);
        }
        let response = request.send().await.expect("send");
        assert_eq!(response.status().as_u16(), 404, "{method} {path}");
    }
    let (_, count): (u16, Value) = p
        .rest(
            Method::GET,
            &format!("/runs?workflowId={workflow}&limit=10"),
            None,
        )
        .await;
    assert_eq!(
        count["items"].as_array().expect("items").len(),
        1,
        "nothing was started"
    );
}

#[tokio::test]
async fn us04_the_audit_trail_shows_what_was_done_scrubs_what_looks_like_a_credential_and_records_who_read_it()
 {
    let p = platform().await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let ns = p.namespace_id().await;
    let drain = format!("/namespaces/{ns}/workers/worker-1/drain");
    // An operator pastes more than they meant to into the reason.
    let reason = "deploying; the token was Bearer abcDEF1234567890xyzTOKEN in the ticket";
    assert_eq!(
        p.rest_as(
            "olivia",
            Method::POST,
            &drain,
            Some(&json!({"reason": reason}))
        )
        .await
        .0,
        200
    );

    // An operator may not read the trail; the refusal is not a reading.
    assert_eq!(
        p.rest_as("olivia", Method::GET, "/audit", None).await.0,
        403
    );
    let readings = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE action = 'audit.read'",
        )
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count")
    };
    assert_eq!(readings().await, 0);

    let (status, page) = p
        .rest(Method::GET, "/audit?action=worker.drained", None)
        .await;
    assert_eq!(status, 200, "{page}");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["actor"], "olivia");
    assert_eq!(items[0]["resourceType"], "worker");
    assert!(
        items[0]["resourceId"]
            .as_str()
            .expect("id")
            .ends_with("/worker-1")
    );
    let shown = items[0]["reason"].as_str().expect("reason");
    assert!(
        shown.contains("deploying") && shown.contains("[redacted]"),
        "{shown}"
    );
    assert!(!shown.contains("abcDEF1234567890xyzTOKEN"), "{shown}");
    // The trail itself keeps what was written.
    let kept: String =
        sqlx::query_scalar("SELECT reason FROM audit_records WHERE action = 'worker.drained'")
            .fetch_one(&p.api.db.pool)
            .await
            .expect("reason");
    assert_eq!(kept, reason);

    // Reading is on the trail, once for the same filters however often it is asked, and again for others.
    assert_eq!(readings().await, 1);
    p.rest(Method::GET, "/audit?action=worker.drained", None)
        .await;
    assert_eq!(readings().await, 1);
    let (_, everything) = p.rest(Method::GET, "/audit?limit=200", None).await;
    assert_eq!(readings().await, 2);
    let reading = everything["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|r| r["action"] == "audit.read")
        .expect("the first reading")
        .clone();
    assert_eq!(reading["actor"], "alice");
    assert!(
        reading["reason"]
            .as_str()
            .expect("reason")
            .contains("action=worker.drained")
    );

    // A page at a time, newest first.
    let (_, first) = p.rest(Method::GET, "/audit?limit=1", None).await;
    let cursor = first["nextCursor"]
        .as_str()
        .expect("more records")
        .to_owned();
    let (_, second) = p
        .rest(
            Method::GET,
            &format!("/audit?limit=1&cursor={cursor}"),
            None,
        )
        .await;
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);

    // Another tenant's administrator sees their own trail and none of this one's.
    let bob = p
        .api
        .as_user(Method::GET, "/audit?action=worker.drained", "bob", "globex")
        .send()
        .await
        .expect("send");
    assert_eq!(bob.status().as_u16(), 200);
    let body: Value = bob.json().await.expect("json");
    assert!(
        body["items"].as_array().expect("items").is_empty(),
        "{body}"
    );
}

/// A denial is recorded only when the resource exists in another tenant, so a trail that listed it
/// would answer "does this id exist elsewhere?" for anyone who could read it. The record stays for
/// whoever runs the platform; the tenant does not get it back through the API.
#[tokio::test]
async fn us04_the_trail_of_a_tenant_does_not_tell_whether_an_identifier_it_probed_exists_elsewhere()
{
    let p = platform().await;
    let ns = p.namespace_id().await;

    // Another tenant's administrator asks for a namespace that is not theirs: indistinguishable from
    // one that does not exist.
    let ghost = Uuid::now_v7().to_string();
    for id in [&ns, &ghost] {
        let bob = p
            .api
            .as_user(
                Method::GET,
                &format!("/namespaces/{id}/workers"),
                "bob",
                "globex",
            )
            .send()
            .await
            .expect("send");
        assert_eq!(bob.status().as_u16(), 404);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant' \
             AND resource_id = $1",
        )
        .bind(&ns)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count"),
        1,
        "the platform keeps the denial"
    );

    let read = p
        .api
        .as_user(Method::GET, "/audit?limit=200", "bob", "globex")
        .send()
        .await
        .expect("send");
    assert_eq!(read.status().as_u16(), 200);
    let body: Value = read.json().await.expect("json");
    let text = body.to_string();
    assert!(!text.contains(&ns), "{text}");
    assert!(!text.contains("access.denied.cross_tenant"), "{text}");

    // Asking for them by name gets nothing either.
    let by_name = p
        .api
        .as_user(
            Method::GET,
            "/audit?action=access.denied.cross_tenant",
            "bob",
            "globex",
        )
        .send()
        .await
        .expect("send");
    let body: Value = by_name.json().await.expect("json");
    assert!(
        body["items"].as_array().expect("items").is_empty(),
        "{body}"
    );
}

#[tokio::test]
async fn us04_a_page_of_the_biggest_size_says_when_there_is_more() {
    let p = platform().await;
    // More records than the largest page: each reading with a different filter is a record of its own.
    for i in 0..205 {
        let (status, _) = p
            .rest(
                Method::GET,
                &format!("/audit?actor=nobody-{i}&limit=1"),
                None,
            )
            .await;
        assert_eq!(status, 200);
    }
    let (status, page) = p.rest(Method::GET, "/audit?limit=200", None).await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["items"].as_array().expect("items").len(), 200);
    let cursor = page["nextCursor"].as_str().expect("there is a next page");
    let (_, rest) = p
        .rest(
            Method::GET,
            &format!("/audit?limit=200&cursor={cursor}"),
            None,
        )
        .await;
    assert!(
        !rest["items"].as_array().expect("items").is_empty(),
        "the next page has the rest: {rest}"
    );
}

#[tokio::test]
async fn us04_a_namespace_has_a_bounded_number_of_drained_identities() {
    let p = platform().await;
    let ns = p.namespace_id().await;
    let reason = json!({"reason": "a drain for every pod of the fleet"});
    for i in 0..500 {
        let (status, body) = p
            .rest(
                Method::POST,
                &format!("/namespaces/{ns}/workers/pod-{i}/drain"),
                Some(&reason),
            )
            .await;
        assert_eq!(status, 200, "{i}: {body}");
    }
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/workers/one-more/drain"),
            Some(&reason),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    // One that is drained already can still be drained again, with another reason.
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/workers/pod-7/drain"),
            Some(&json!({"reason": "the second reason is the one that stands"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // And lifting one makes room.
    let (status, _) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/workers/pod-3/undrain"),
            Some(&reason),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/workers/one-more/drain"),
            Some(&reason),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}
