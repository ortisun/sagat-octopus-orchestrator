//! The business-id prefix of a schedule's action in the runner: the write of a scenario reaches
//! the typed action as given, and an observation prints the three fields only when the server
//! sent them, so the observation of every other schedule is what it always was.
use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use std::sync::{Arc, Mutex};

const SCHEDULE: &str = "0190aaaa-0000-7000-8000-0000000000a1";
const NAMESPACE: &str = "0190aaaa-0000-7000-8000-00000000aaaa";
const WORKFLOW: &str = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
const PREFIX: &str = "Order/Nightly-A";

/// A schedule as the server answers it, with the action object given.
fn server_schedule(action: Value) -> Value {
    json!({
        "scheduleId": SCHEDULE, "name": "nightly", "origin": "API", "state": "ACTIVE",
        "revision": 1, "createdAt": "2026-10-10T10:00:00Z", "action": action,
    })
}
fn with_keys(mut base: Value, extra: &Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        base.extend(extra.clone());
    }
    base
}

/// What the runner prints for the schedule a `scheduleCreate` and a `scheduleGet` read from an
/// answer whose action is `action`: the two observations, which must agree on the action.
async fn printed(action: Value) -> Value {
    let config = json!({"namespaceId": NAMESPACE});
    let results = BTreeMap::from([("create".to_owned(), json!({"scheduleId": SCHEDULE}))]);
    let (base, _) = double(server_schedule(action)).await;
    let client = client_of(&base);
    let create = json!({"id": "create", "op": "scheduleCreate", "idempotencyKey": "sched-create-9999",
                        "write": {"name": "n", "spec": {}, "action": {"workflowId": WORKFLOW, "input": {}}}});
    let created = play("scheduleCreate", &client, &create, &config, &results, &[])
        .await
        .expect("play")
        .expect("an observation");
    let get = json!({"id": "get", "op": "scheduleGet", "schedule": "create"});
    let described = play("scheduleGet", &client, &get, &config, &results, &[])
        .await
        .expect("play")
        .expect("an observation");
    assert_eq!(created["action"], described["action"]);
    created["action"].clone()
}

#[tokio::test]
async fn a_schedule_without_a_business_id_prints_the_action_it_always_printed() {
    let stored = json!({"workflowId": WORKFLOW, "versionPolicy": "ACTIVE",
                        "inputDigest": "a".repeat(64)});
    assert_eq!(
        printed(stored.clone()).await,
        json!({
            "workflowId": WORKFLOW, "versionPolicy": "ACTIVE", "versionId": null,
            "inputDigest": "a".repeat(64), "nominalTimeField": null, "runTimeoutMs": null,
        }),
        "absent => the keys are not printed, so every existing golden stays as it is"
    );
    // A null from the server is absent too.
    let nulls = with_keys(
        stored,
        &json!({"businessIdPrefix": null, "businessIdConflictPolicy": null,
                "businessIdReusePolicy": null}),
    );
    let action = printed(nulls).await;
    assert_eq!(action.as_object().expect("an object").len(), 6);
}

#[tokio::test]
async fn a_schedule_with_a_business_id_prints_its_prefix_and_policies_as_the_strings_the_server_sent()
 {
    let stored = json!({"workflowId": WORKFLOW, "versionPolicy": "ACTIVE",
                        "inputDigest": "a".repeat(64)});
    let action = printed(with_keys(
        stored.clone(),
        &json!({"businessIdPrefix": " Order/Nightly-A ", "businessIdConflictPolicy": "USE_EXISTING",
                "businessIdReusePolicy": "A_NEWER_ONE"}),
    ))
    .await;
    assert_eq!(action["businessIdPrefix"], " Order/Nightly-A ");
    assert_eq!(action["businessIdConflictPolicy"], "USE_EXISTING");
    assert_eq!(action["businessIdReusePolicy"], "A_NEWER_ONE");
    // Only what is present: a prefix alone prints no policy key, not a null.
    let alone = printed(with_keys(stored, &json!({"businessIdPrefix": "p"}))).await;
    assert_eq!(alone["businessIdPrefix"], "p");
    let keys = alone.as_object().expect("an object");
    assert!(!keys.contains_key("businessIdConflictPolicy"));
    assert!(!keys.contains_key("businessIdReusePolicy"));
}

#[test]
fn the_write_of_a_scenario_carries_the_three_fields_into_the_typed_action_as_given() {
    let scenario = json!({
        "name": "n", "spec": {"cron": ["0 9 * * *"]},
        "action": {
            "workflowId": WORKFLOW, "input": {"n": 1}, "businessIdPrefix": " Mixed/Case ",
            "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "REJECT_DUPLICATE",
        },
    });
    let write: ScheduleWrite = typed(&scenario, "schedule").expect("a valid write");
    assert_eq!(
        write.action.business_id_prefix.as_deref(),
        Some(" Mixed/Case ")
    );
    assert_eq!(
        write.action.business_id_conflict_policy,
        Some(BusinessIdConflictPolicy::UseExisting)
    );
    assert_eq!(
        write.action.business_id_reuse_policy,
        Some(BusinessIdReusePolicy::RejectDuplicate)
    );
    let empty = json!({"name": "n", "spec": {},
                       "action": {"workflowId": WORKFLOW, "input": {}, "businessIdPrefix": ""}});
    let write: ScheduleWrite = typed(&empty, "schedule").expect("a valid write");
    assert_eq!(write.action.business_id_prefix.as_deref(), Some(""));
    let plain = json!({"name": "n", "spec": {}, "action": {"workflowId": WORKFLOW, "input": {}}});
    let write: ScheduleWrite = typed(&plain, "schedule").expect("a valid write");
    assert!(write.action.business_id_prefix.is_none());
    assert!(write.action.business_id_conflict_policy.is_none());
    assert!(write.action.business_id_reuse_policy.is_none());
}

/// A server that records the body of every request and answers `answer` to each.
async fn double(answer: Value) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an owned listener");
    let base = format!("http://{}/v1", listener.local_addr().expect("an address"));
    let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
    let recorded = bodies.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (answer, recorded) = (answer.clone(), recorded.clone());
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(stream),
                        service_fn(move |request: Request<Incoming>| {
                            let (answer, recorded) = (answer.clone(), recorded.clone());
                            async move {
                                // A create is the only POST of these tests: it answers 201.
                                let status = if request.method() == hyper::Method::POST {
                                    201
                                } else {
                                    200
                                };
                                let body = request.into_body().collect().await?.to_bytes();
                                if let Ok(body) = serde_json::from_slice::<Value>(&body) {
                                    recorded.lock().expect("bodies").push(body);
                                }
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(status)
                                        .header("content-type", "application/json")
                                        .body(Full::new(Bytes::from(answer.to_string())))
                                        .expect("a response"),
                                )
                            }
                        }),
                    )
                    .await;
            });
        }
    });
    (base, bodies)
}
fn client_of(base: &str) -> Client {
    Client::new(ClientOptions::new(
        base,
        "payments",
        TokenSource::bearer("tok-1"),
    ))
    .expect("a client")
}

#[tokio::test]
async fn create_update_and_get_play_the_prefix_through_the_typed_write_and_print_it_back_only_when_present()
 {
    let config = json!({"namespaceId": NAMESPACE});
    let results = BTreeMap::from([("create".to_owned(), json!({"scheduleId": SCHEDULE}))]);
    let order = vec!["create".to_owned()];
    let bare = json!({"workflowId": WORKFLOW, "input": {}});
    let write = |extra: &Value| json!({"name": "n", "spec": {"cron": ["0 9 * * *"]}, "action": with_keys(bare.clone(), extra)});
    let with = json!({"businessIdPrefix": PREFIX, "businessIdConflictPolicy": "USE_EXISTING",
                      "businessIdReusePolicy": "REJECT_DUPLICATE"});
    let stored = json!({"workflowId": WORKFLOW, "versionPolicy": "ACTIVE",
                        "inputDigest": "a".repeat(64)});
    let (base, bodies) = double(server_schedule(with_keys(stored.clone(), &with))).await;
    let client = client_of(&base);
    let create = json!({"id": "create", "op": "scheduleCreate",
                        "idempotencyKey": "sched-create-0001", "write": write(&with)});
    let observed = play(
        "scheduleCreate",
        &client,
        &create,
        &config,
        &results,
        &order,
    )
    .await
    .expect("play")
    .expect("an observation");
    assert_eq!(
        bodies.lock().expect("bodies")[0]["action"],
        with_keys(bare.clone(), &with)
    );
    assert_eq!(observed["action"]["businessIdPrefix"], PREFIX);
    assert_eq!(
        observed["action"]["businessIdConflictPolicy"],
        "USE_EXISTING"
    );
    assert_eq!(
        observed["action"]["businessIdReusePolicy"],
        "REJECT_DUPLICATE"
    );
    let update = json!({"id": "update", "op": "scheduleUpdate", "schedule": "create",
                        "expectedRevision": 1, "idempotencyKey": "sched-update-0001",
                        "write": write(&with)});
    let observed = play(
        "scheduleUpdate",
        &client,
        &update,
        &config,
        &results,
        &order,
    )
    .await
    .expect("play")
    .expect("an observation");
    assert_eq!(
        bodies.lock().expect("bodies")[1]["action"]["businessIdPrefix"],
        PREFIX
    );
    assert_eq!(observed["action"]["businessIdPrefix"], PREFIX);
    let get = json!({"id": "get", "op": "scheduleGet", "schedule": "create"});
    let observed = play("scheduleGet", &client, &get, &config, &results, &order)
        .await
        .expect("play")
        .expect("an observation");
    assert_eq!(observed["action"]["businessIdPrefix"], PREFIX);

    // Nothing set, nothing sent, and nothing printed: the keys are absent, not null.
    let (base, bodies) = double(server_schedule(stored)).await;
    let client = client_of(&base);
    let plain = json!({"id": "create", "op": "scheduleCreate",
                       "idempotencyKey": "sched-create-0002", "write": write(&json!({}))});
    let observed = play("scheduleCreate", &client, &plain, &config, &results, &order)
        .await
        .expect("play")
        .expect("an observation");
    assert_eq!(bodies.lock().expect("bodies")[0]["action"], bare);
    let action = observed["action"].as_object().expect("an object");
    for key in [
        "businessIdPrefix",
        "businessIdConflictPolicy",
        "businessIdReusePolicy",
    ] {
        assert!(
            !action.contains_key(key),
            "{key} is not printed: {action:?}"
        );
    }
}
