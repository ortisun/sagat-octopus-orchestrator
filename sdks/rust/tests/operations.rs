//! The client surface of feature 018 against a recording server: what each call sends, what it keeps
//! across a retry, what it refuses before sending, and how it reads an answer that is newer than the
//! SDK. These are the Rust ports of `sdks/typescript/test/operations.test.ts`.
#![allow(clippy::expect_used)]
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use futures_util::StreamExt as _;
use sagat_octopus_orchestrator_sdk::{
    BatchFilter, BatchListFilter, BatchRequest, BatchSelector, BusinessIdConflictPolicy,
    BusinessIdDisposition, BusinessIdReusePolicy, CallOptions, Client, ClientOptions, Moment,
    OverlapPolicy, PageOptions, ReapplyPolicy, ResetRequest, RunStatus, ScheduleAction,
    ScheduleActionFilter, ScheduleCalendar, ScheduleDescribeOptions, ScheduleListFilter,
    SchedulePolicies, ScheduleRange, ScheduleSpec, ScheduleWrite, StartOptions, TokenSource,
    UpdateWithStartOptions, WithStartOptions,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const NS: &str = "0190aaaa-0000-7000-8000-00000000aaaa";
const WF: &str = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
const SCHEDULE: &str = "0190aaaa-0000-7000-8000-0000000000a1";
const RUN: &str = "0190aaaa-0000-7000-8000-000000000001";
const BATCH: &str = "0190aaaa-0000-7000-8000-0000000000b1";

fn run(overrides: Value) -> Value {
    let mut base = json!({
        "runId": RUN, "workflowId": WF, "versionId": "0190aaaa-0000-7000-8000-000000000002",
        "status": "RUNNING", "version": 3, "paused": false, "startedBy": "alice",
        "createdAt": "2026-10-10T10:00:00Z", "updatedAt": "2026-10-10T10:00:01Z", "finishedAt": null,
        "deadline": null, "traceId": "abcdefabcdefabcdefabcdefabcdefab", "correlationId": null,
        "depth": 0, "parent": null,
    });
    merge(&mut base, overrides);
    base
}
fn schedule(overrides: Value) -> Value {
    let mut base = json!({
        "scheduleId": SCHEDULE, "name": "nightly", "origin": "API", "state": "ACTIVE", "revision": 2,
        "createdAt": "2026-10-10T10:00:00Z",
        "action": {"workflowId": WF, "versionPolicy": "ACTIVE", "inputDigest": "a".repeat(64)},
        "note": null, "remainingActions": null, "nextActionAt": "2099-01-01T09:00:00Z",
        "updatedAt": "2026-10-10T10:00:00Z",
    });
    merge(&mut base, overrides);
    base
}
fn action(overrides: Value) -> Value {
    let mut base = json!({
        "nominalTime": "2026-10-10T10:00:00Z", "actualTime": "2026-10-10T10:00:01Z",
        "kind": "TRIGGERED", "origin": "MANUAL", "revision": 2, "outcome": "STARTED", "reason": null,
        "runId": RUN, "backfillId": null, "decidedAt": "2026-10-10T10:00:01Z",
    });
    merge(&mut base, overrides);
    base
}
fn merge(base: &mut Value, overrides: Value) {
    if let (Some(base), Value::Object(overrides)) = (base.as_object_mut(), overrides) {
        base.extend(overrides);
    }
}
fn unavailable() -> Reply {
    Reply::Json(
        503,
        json!({"code":"UNAVAILABLE","message":"busy","retryable":true,"traceId":"e".repeat(32)}),
    )
}

#[derive(Clone)]
struct Seen {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Option<Value>,
}
impl Seen {
    fn path(&self) -> &str {
        self.uri.path()
    }
    fn query(&self) -> Vec<(String, String)> {
        url::form_urlencoded::parse(self.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect()
    }
    fn key(&self) -> Option<&str> {
        self.headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
    }
    fn body(&self) -> &Value {
        self.body.as_ref().expect("a body")
    }
}
enum Reply {
    Json(u16, Value),
    Empty(u16),
}
type Handler = Arc<dyn Fn(usize, &Seen) -> Reply + Send + Sync>;
#[derive(Clone)]
struct Shared {
    seen: Arc<Mutex<Vec<Seen>>>,
    handler: Arc<Mutex<Handler>>,
}
async fn answer(
    State(shared): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let seen = Seen {
        method,
        uri,
        headers,
        body: (!body.is_empty()).then(|| serde_json::from_slice(&body).expect("a JSON body")),
    };
    let count = {
        let mut calls = shared.seen.lock().expect("calls");
        calls.push(seen.clone());
        calls.len()
    };
    let handler = shared.handler.lock().expect("handler").clone();
    match handler(count, &seen) {
        Reply::Json(status, body) => (
            StatusCode::from_u16(status).expect("status"),
            [("x-trace-id", "f".repeat(32))],
            axum::Json(body),
        )
            .into_response(),
        Reply::Empty(status) => StatusCode::from_u16(status)
            .expect("status")
            .into_response(),
    }
}

struct Recorder {
    base: String,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Recorder {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("owned listener");
        let base = format!("http://{}/v1", listener.local_addr().expect("address"));
        let nothing: Handler = Arc::new(|_, _| Reply::Empty(500));
        let shared = Shared {
            seen: Arc::default(),
            handler: Arc::new(Mutex::new(nothing)),
        };
        let app = Router::new()
            .fallback(any(answer))
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("owned server");
        });
        Self { base, shared, task }
    }
    /// Forgets what was seen and answers the next requests with `handler`.
    fn prepare(&self, handler: impl Fn(usize, &Seen) -> Reply + Send + Sync + 'static) {
        self.shared.seen.lock().expect("calls").clear();
        *self.shared.handler.lock().expect("handler") = Arc::new(handler);
    }
    fn answer_with(&self, status: u16, body: Value) {
        self.prepare(move |_, _| Reply::Json(status, body.clone()));
    }
    fn seen(&self) -> Vec<Seen> {
        self.shared.seen.lock().expect("calls").clone()
    }
    fn only(&self) -> Seen {
        let seen = self.seen();
        assert_eq!(seen.len(), 1, "exactly one request");
        seen[0].clone()
    }
    fn client(&self) -> Client {
        let mut options = ClientOptions::new(&self.base, "payments", TokenSource::bearer("tok-1"));
        options.retry.initial_delay = Duration::from_millis(1);
        options.retry.max_delay = Duration::from_millis(2);
        options.retry.max_attempts = 3;
        Client::new(options).expect("client")
    }
}
fn keyed(key: &str) -> CallOptions {
    CallOptions {
        idempotency_key: Some(key.to_owned()),
        ..CallOptions::default()
    }
}
fn is_validation(error: &sagat_octopus_orchestrator_sdk::SagatError) -> bool {
    error.code == "VALIDATION_FAILED" && error.status.is_none()
}
fn write(name: &str, spec: ScheduleSpec) -> ScheduleWrite {
    ScheduleWrite::new(name, spec, ScheduleAction::new(WF, json!({"n": 1})))
}

// ------------------------------------------------------------------------------- start options

#[tokio::test]
async fn a_start_sends_the_business_identifier_its_policies_and_the_delay_once_and_says_whether_it_created_the_run()
 {
    let server = Recorder::start().await;
    server.answer_with(
        202,
        run(json!({"businessId":"order-1","startDelayedUntil":"2026-10-10T10:00:05Z"})),
    );
    let started = server
        .client()
        .start(
            WF,
            &json!({"a": 1}),
            StartOptions {
                call: keyed("key-12345678"),
                business_id: Some("Order/1 ".into()),
                business_id_conflict_policy: Some(BusinessIdConflictPolicy::UseExisting),
                business_id_reuse_policy: Some(BusinessIdReusePolicy::RejectDuplicate),
                // 5000.9 ms: truncated toward zero, once.
                start_delay: Some(Duration::from_nanos(5_000_900_000)),
                ..StartOptions::default()
            },
        )
        .await
        .expect("start");
    assert_eq!(
        server.only().body(),
        &json!({
            "workflowId": WF, "input": {"a": 1}, "businessId": "Order/1 ",
            "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "REJECT_DUPLICATE",
            "startDelayMs": 5000,
        })
    );
    assert!(started.started);
    assert_eq!(started.business_id.as_deref(), Some("order-1"));
    assert_eq!(
        started.start_delayed_until.map(|at| at.unix_timestamp()),
        Some(1_791_626_405)
    );
    server.answer_with(202, run(json!({})));
    let plain = server
        .client()
        .start(WF, &json!({}), keyed("key-12345678"))
        .await
        .expect("start");
    assert!(plain.business_id.is_none() && plain.start_delayed_until.is_none());
}

#[tokio::test]
async fn the_200_of_use_existing_is_a_run_that_was_not_started_here_and_a_start_without_the_new_options_sends_none_of_them()
 {
    let server = Recorder::start().await;
    server.answer_with(200, run(json!({})));
    let existing = server
        .client()
        .start(
            WF,
            &json!({}),
            StartOptions {
                call: keyed("key-12345678"),
                business_id: Some("o-1".into()),
                business_id_conflict_policy: Some(BusinessIdConflictPolicy::UseExisting),
                ..StartOptions::default()
            },
        )
        .await
        .expect("start");
    assert!(!existing.started);
    assert_eq!(existing.id.to_string(), RUN, "it reads as the run it holds");
    server.answer_with(202, run(json!({})));
    server
        .client()
        .start(WF, &json!({}), keyed("key-12345678"))
        .await
        .expect("start");
    assert_eq!(
        server.only().body(),
        &json!({"workflowId": WF, "input": {}}),
        "an old call is byte for byte what it was"
    );
    // A 200 that nobody asked for is not what the contract promises.
    server.answer_with(200, run(json!({})));
    let error = server
        .client()
        .start(WF, &json!({}), keyed("key-12345678"))
        .await
        .expect_err("a 200 without USE_EXISTING");
    assert_eq!(error.code, "PROTOCOL");
}

#[tokio::test]
async fn a_retry_keeps_the_one_delay_the_one_body_and_the_one_key_and_a_delay_is_truncated_once() {
    let server = Recorder::start().await;
    server.prepare(|n, _| {
        if n < 2 {
            unavailable()
        } else {
            Reply::Json(202, run(json!({})))
        }
    });
    server
        .client()
        .start(
            WF,
            &json!({}),
            StartOptions {
                business_id: Some("o-2".into()),
                // 1.999999 ms is 1 ms, not 2.
                start_delay: Some(Duration::from_nanos(1_999_999)),
                ..StartOptions::default()
            },
        )
        .await
        .expect("start");
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body(), seen[1].body(), "the retry is the same body");
    assert_eq!(seen[0].body()["startDelayMs"], 1);
    assert!(seen[0].key().is_some());
    assert_eq!(seen[0].key(), seen[1].key());
    // Nothing is held back when the delay is zero: it is sent for the server to judge.
    server.answer_with(202, run(json!({})));
    server
        .client()
        .start(
            WF,
            &json!({}),
            StartOptions {
                start_delay: Some(Duration::ZERO),
                ..StartOptions::default()
            },
        )
        .await
        .expect("start");
    assert_eq!(server.only().body()["startDelayMs"], 0);
}

// ---------------------------------------------------------------------------------- schedules

#[tokio::test]
async fn create_sends_the_schedule_with_its_instants_in_utc_and_one_key_and_reads_the_schedule_it_gets_back()
 {
    let server = Recorder::start().await;
    server.answer_with(201, schedule(json!({"futureField": 1})));
    let mut spec = ScheduleSpec {
        calendars: vec![ScheduleCalendar {
            hour: vec![ScheduleRange::at(9)],
            year: vec![ScheduleRange::at(2099)],
            ..ScheduleCalendar::default()
        }],
        start_at: Some(Moment::from("2099-01-01T01:00:00+01:00")),
        timezone: Some("UTC".into()),
        ..ScheduleSpec::default()
    };
    let mut body = write("nightly", spec.clone());
    body.policies = Some(SchedulePolicies {
        overlap: Some(OverlapPolicy::Skip),
        ..SchedulePolicies::default()
    });
    let created = server
        .client()
        .schedules()
        .create(NS, &body, keyed("sched-key-1"))
        .await
        .expect("create");
    let request = server.only();
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.path(), format!("/v1/namespaces/{NS}/schedules"));
    assert_eq!(request.key(), Some("sched-key-1"));
    assert_eq!(
        request.body(),
        &json!({
            "name": "nightly",
            "spec": {"calendars": [{"hour": [{"start": 9}], "year": [{"start": 2099}]}],
                     "startAt": "2099-01-01T00:00:00Z", "timezone": "UTC"},
            "policies": {"overlap": "SKIP"},
            "action": {"workflowId": WF, "input": {"n": 1}},
        })
    );
    assert_eq!(created.revision, 2);
    assert_eq!(
        created.next_action_at.map(|at| at.unix_timestamp()),
        Some(4_070_941_200)
    );
    assert_eq!(
        created.action.input_digest.as_deref(),
        Some("a".repeat(64).as_str()),
        "the input is never returned, only its digest"
    );
    assert_eq!(created.raw["futureField"], 1);
    // Text that is not an instant is refused before anything is sent.
    server.prepare(|_, _| Reply::Empty(500));
    spec.end_at = Some(Moment::from("not a time"));
    let error = server
        .client()
        .schedules()
        .create(NS, &write("n", spec), CallOptions::default())
        .await
        .expect_err("not an instant");
    assert!(is_validation(&error));
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn describe_asks_for_the_counts_it_was_given_and_keeps_an_outcome_it_does_not_know_as_the_string_it_came_as()
 {
    let server = Recorder::start().await;
    let mut described = schedule(json!({}));
    merge(
        &mut described,
        json!({
            "nextActions": [{"nominalTime": "2099-01-01T09:00:00Z", "actualTime": "2099-01-01T09:00:00Z"}],
            "recentActions": [action(json!({})), action(json!({"outcome": "SOMETHING_NEWER"}))],
            "openRuns": [RUN], "bufferedCount": 1, "counters": {"STARTED": 1, "SOMETHING_NEWER": 1},
        }),
    );
    server.answer_with(200, described);
    let described = server
        .client()
        .schedules()
        .get(
            NS,
            SCHEDULE,
            ScheduleDescribeOptions {
                next_count: Some(3),
                recent_count: Some(5),
            },
            CallOptions::default(),
        )
        .await
        .expect("get");
    let request = server.only();
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}")
    );
    assert_eq!(
        request.query(),
        vec![
            ("nextCount".to_owned(), "3".to_owned()),
            ("recentCount".to_owned(), "5".to_owned())
        ]
    );
    assert_eq!(
        described.next_actions[0].nominal_time.unix_timestamp(),
        4_070_941_200
    );
    assert_eq!(described.recent_actions[1].outcome, "SOMETHING_NEWER");
    assert_eq!(described.open_runs[0].to_string(), RUN);
    assert_eq!(described.counters["STARTED"], 1);
    assert_eq!(described.counters["SOMETHING_NEWER"], 1);
    assert_eq!(described.buffered_count, 1);
    assert_eq!(
        described.name, "nightly",
        "it reads as the schedule it describes"
    );
}

#[tokio::test]
async fn list_sends_the_filters_it_was_given_and_list_all_follows_the_opaque_cursors_to_the_end() {
    let server = Recorder::start().await;
    server.prepare(|n, _| {
        Reply::Json(
            200,
            if n == 1 {
                json!({"items": [schedule(json!({"name": "a-1"}))], "nextCursor": "c1"})
            } else {
                json!({"items": [schedule(json!({"name": "a-2"}))], "nextCursor": null})
            },
        )
    });
    let client = server.client();
    let schedules = client.schedules();
    let stream = schedules.list_all(
        NS,
        ScheduleListFilter {
            state: Some("PAUSED".into()),
            origin: Some("API".into()),
            name: Some("a-".into()),
            limit: Some(1),
            ..ScheduleListFilter::default()
        },
        CallOptions::default(),
    );
    futures_util::pin_mut!(stream);
    let mut names = Vec::new();
    while let Some(item) = stream.next().await {
        names.push(item.expect("a schedule").name);
    }
    assert_eq!(names, ["a-1", "a-2"]);
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    let filters = [
        ("limit", "1"),
        ("state", "PAUSED"),
        ("origin", "API"),
        ("name", "a-"),
    ];
    let expected = |cursor: Option<&str>| {
        cursor
            .map(|c| ("cursor".to_owned(), c.to_owned()))
            .into_iter()
            .chain(
                filters
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            )
            .collect::<Vec<_>>()
    };
    let sorted = |mut pairs: Vec<(String, String)>| {
        pairs.sort();
        pairs
    };
    assert_eq!(sorted(seen[0].query()), sorted(expected(None)));
    assert_eq!(sorted(seen[1].query()), sorted(expected(Some("c1"))));
}

#[tokio::test]
async fn update_carries_the_revision_it_expects_in_the_body_delete_in_the_query_and_both_carry_a_key()
 {
    let server = Recorder::start().await;
    server.answer_with(200, schedule(json!({"revision": 3})));
    let body = ScheduleWrite::new(
        "nightly",
        ScheduleSpec {
            cron: vec!["0 9 * * *".into()],
            ..ScheduleSpec::default()
        },
        ScheduleAction::new(WF, json!({})),
    );
    let client = server.client();
    let updated = client
        .schedules()
        .update(NS, SCHEDULE, &body, 2, keyed("upd-key-0001"))
        .await
        .expect("update");
    assert_eq!(updated.revision, 3);
    let request = server.only();
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.key(), Some("upd-key-0001"));
    assert_eq!(
        request.body(),
        &json!({
            "name": "nightly", "spec": {"cron": ["0 9 * * *"]},
            "action": {"workflowId": WF, "input": {}}, "expectedRevision": 2,
        })
    );
    server.prepare(|_, _| Reply::Empty(204));
    client
        .schedules()
        .delete(NS, SCHEDULE, 3, keyed("del-key-0001"))
        .await
        .expect("delete");
    let request = server.only();
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}")
    );
    assert_eq!(
        request.query(),
        vec![("expectedRevision".to_owned(), "3".to_owned())]
    );
    assert_eq!(request.key(), Some("del-key-0001"));
}

#[tokio::test]
async fn a_stale_revision_is_a_conflict_with_its_reason_and_a_revision_that_cannot_be_one_is_refused_before_it_is_sent()
 {
    let server = Recorder::start().await;
    server.answer_with(
        409,
        json!({"code":"CONFLICT","message":"stale","retryable":false,"traceId":"e".repeat(32),
               "details":{"reason":"REVISION_CONFLICT","currentRevision":4}}),
    );
    let client = server.client();
    let schedules = client.schedules();
    let body = write("n", ScheduleSpec::default());
    let error = schedules
        .update(NS, SCHEDULE, &body, 2, CallOptions::default())
        .await
        .expect_err("stale");
    assert_eq!(error.code, "CONFLICT");
    assert_eq!(error.reason(), Some("REVISION_CONFLICT"));
    assert_eq!(server.seen().len(), 1, "a conflict is never retried");
    server.prepare(|_, _| Reply::Empty(500));
    for error in [
        schedules
            .delete(NS, SCHEDULE, 0, CallOptions::default())
            .await
            .expect_err("revision 0"),
        schedules
            .update(NS, SCHEDULE, &body, 0, CallOptions::default())
            .await
            .expect_err("revision 0"),
        schedules
            .get(
                NS,
                "../x",
                ScheduleDescribeOptions::default(),
                CallOptions::default(),
            )
            .await
            .expect_err("not an id"),
        schedules
            .get(
                "payments",
                SCHEDULE,
                ScheduleDescribeOptions::default(),
                CallOptions::default(),
            )
            .await
            .expect_err("a namespace name is not a namespace id"),
    ] {
        assert!(is_validation(&error), "{error:?}");
    }
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn pause_and_resume_send_the_note_trigger_sends_the_overlap_for_this_action_only_and_a_trigger_retry_keeps_its_key()
 {
    let server = Recorder::start().await;
    server.answer_with(
        200,
        schedule(json!({"state": "PAUSED", "note": "maintenance"})),
    );
    let client = server.client();
    let schedules = client.schedules();
    let paused = schedules
        .pause(NS, SCHEDULE, "maintenance", keyed("pause-key-01"))
        .await
        .expect("pause");
    assert_eq!(paused.state, "PAUSED");
    let request = server.only();
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}/pause")
    );
    assert_eq!(request.body(), &json!({"note": "maintenance"}));
    server.answer_with(200, schedule(json!({})));
    schedules
        .resume(NS, SCHEDULE, "done", keyed("resume-key-1"))
        .await
        .expect("resume");
    assert_eq!(
        server.only().path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}/resume")
    );
    server.prepare(|_, _| Reply::Empty(500));
    let error = schedules
        .pause(NS, SCHEDULE, "", CallOptions::default())
        .await
        .expect_err("an empty note");
    assert!(is_validation(&error));
    assert!(server.seen().is_empty());

    server.prepare(|n, _| {
        if n < 2 {
            unavailable()
        } else {
            Reply::Json(202, action(json!({"outcome": "SKIPPED_OVERLAP"})))
        }
    });
    let triggered = schedules
        .trigger(
            NS,
            SCHEDULE,
            Some(OverlapPolicy::Skip),
            CallOptions::default(),
        )
        .await
        .expect("trigger");
    assert_eq!(triggered.outcome, "SKIPPED_OVERLAP");
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].key().is_some());
    assert_eq!(seen[0].key(), seen[1].key());
    assert_eq!(seen[1].body(), &json!({"overlap": "SKIP"}));
    // Without an overlap the body is empty, never a null field.
    server.answer_with(202, action(json!({})));
    schedules
        .trigger(NS, SCHEDULE, None, CallOptions::default())
        .await
        .expect("trigger");
    assert_eq!(server.only().body(), &json!({}));
}

#[tokio::test]
async fn a_backfill_sends_its_range_as_utc_instants_a_cancel_names_the_backfill_and_the_history_can_be_filtered_by_outcome()
 {
    let server = Recorder::start().await;
    server.answer_with(
        202,
        json!({"backfillId": BATCH, "start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z",
               "cursor": null, "state": "PENDING", "overlap": "ALLOW_ALL", "counts": {"STARTED": 2}}),
    );
    let client = server.client();
    let schedules = client.schedules();
    let start = time::OffsetDateTime::from_unix_timestamp(1_791_619_200).expect("instant");
    let backfill = schedules
        .backfill(
            NS,
            SCHEDULE,
            start.to_offset(time::UtcOffset::from_hms(5, 30, 0).expect("offset")),
            "2026-10-10T10:00:00+00:00",
            Some(OverlapPolicy::AllowAll),
            keyed("back-key-001"),
        )
        .await
        .expect("backfill");
    assert_eq!(
        server.only().body(),
        &json!({"start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z", "overlap": "ALLOW_ALL"}),
        "an instant with a local offset is sent in UTC"
    );
    assert!(backfill.cursor.is_none());
    assert_eq!(backfill.counts["STARTED"], 2);
    server.prepare(|_, _| Reply::Empty(500));
    let error = schedules
        .backfill(
            NS,
            SCHEDULE,
            "not a time",
            start,
            None,
            CallOptions::default(),
        )
        .await
        .expect_err("not an instant");
    assert!(is_validation(&error));
    assert!(server.seen().is_empty());

    server.answer_with(
        200,
        json!({"backfillId": BATCH, "start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z",
               "state": "CANCELLED", "overlap": "ALLOW_ALL", "counts": {}}),
    );
    let cancelled = schedules
        .cancel_backfill(NS, SCHEDULE, BATCH, CallOptions::default())
        .await
        .expect("cancel");
    assert_eq!(cancelled.state, "CANCELLED");
    assert_eq!(
        server.only().path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}/backfills/{BATCH}/cancel")
    );
    server.answer_with(
        200,
        json!({"items": [action(json!({"outcome": "ALREADY_ACTED"}))], "nextCursor": null}),
    );
    let actions = schedules
        .list_actions(
            NS,
            SCHEDULE,
            ScheduleActionFilter {
                outcome: Some("ALREADY_ACTED".into()),
                limit: Some(10),
                cursor: None,
            },
            CallOptions::default(),
        )
        .await
        .expect("actions");
    let request = server.only();
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/schedules/{SCHEDULE}/actions")
    );
    assert_eq!(
        request.query(),
        vec![
            ("limit".to_owned(), "10".to_owned()),
            ("outcome".to_owned(), "ALREADY_ACTED".to_owned())
        ]
    );
    assert_eq!(actions.items[0].outcome, "ALREADY_ACTED");
}

// ------------------------------------------------------------------------------- business ids

#[tokio::test]
async fn a_business_identifier_is_percent_encoded_as_given_and_one_that_cannot_be_a_path_segment_is_refused_locally()
 {
    let server = Recorder::start().await;
    server.answer_with(
        200,
        json!({"businessId": "Ord/1#x?y", "executionId": RUN, "currentRunId": RUN, "open": false,
               "lastStatus": "SUCCEEDED", "workflowId": WF, "boundAt": "2026-10-10T10:00:00Z",
               "reusableAfter": "2026-11-09T10:00:00Z"}),
    );
    let client = server.client();
    let ids = client.business_ids();
    let binding = ids
        .get(NS, "Ord/1#x?y", CallOptions::default())
        .await
        .expect("binding");
    assert_eq!(
        server.only().uri.path(),
        format!("/v1/namespaces/{NS}/business-ids/Ord%2F1%23x%3Fy"),
        "no trimming and no case folding"
    );
    assert!(!binding.open);
    assert_eq!(
        binding.reusable_after.map(|at| at.unix_timestamp()),
        Some(1_794_218_400)
    );
    // A percent sign and a backslash are data, not syntax.
    server.prepare(|_, _| Reply::Empty(404));
    let _ = ids.get(NS, "a%2Fb\\c", CallOptions::default()).await;
    assert_eq!(
        server.only().uri.path(),
        format!("/v1/namespaces/{NS}/business-ids/a%252Fb%5Cc")
    );
    server.prepare(|_, _| Reply::Empty(404));
    let long = "x".repeat(201);
    for bad in ["", " padded", "with space", long.as_str(), "ünï", ".", ".."] {
        let error = ids
            .get(NS, bad, CallOptions::default())
            .await
            .expect_err("refused");
        assert!(is_validation(&error), "{bad:?}: {error:?}");
    }
    assert!(server.seen().is_empty(), "nothing is sent for them");
    // A value that merely contains dots is an identifier like any other.
    let _ = ids.get(NS, "...", CallOptions::default()).await;
    let _ = ids.get(NS, "a..b", CallOptions::default()).await;
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0].uri.path(),
        format!("/v1/namespaces/{NS}/business-ids/...")
    );
    assert_eq!(
        seen[1].uri.path(),
        format!("/v1/namespaces/{NS}/business-ids/a..b")
    );
}

// ------------------------------------------------------------------------------------- batch

#[tokio::test]
async fn a_batch_is_previewed_created_with_one_key_read_cancelled_and_its_items_filtered_by_state()
{
    let server = Recorder::start().await;
    let job = json!({"batchId": BATCH, "action": "CANCEL", "state": "RUNNING", "createdBy": "alice",
        "createdAt": "2026-10-10T10:00:00Z",
        "counts": {"selected": 2, "succeeded": 1, "skipped": 0, "failed": 0, "cancelled": 0, "deferred": 1}});
    server.answer_with(200, json!({"matched": 2, "capped": false, "sample": [RUN]}));
    let client = server.client();
    let batch = client.batch();
    let preview = batch
        .preview(
            NS,
            &BatchSelector::RunIds(vec![RUN.into()]),
            Some(50),
            CallOptions::default(),
        )
        .await
        .expect("preview");
    let request = server.only();
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/batch-operations/preview")
    );
    assert_eq!(
        request.body(),
        &json!({"selector": {"runIds": [RUN]}, "maxItems": 50})
    );
    assert_eq!(
        (
            preview.matched,
            preview.capped,
            preview.sample[0].to_string()
        ),
        (2, false, RUN.to_owned())
    );

    server.answer_with(202, job.clone());
    let mut asked = BatchRequest::new(
        BatchSelector::Filter(BatchFilter {
            status: vec![RunStatus::Running],
            created_after: Some(Moment::from("2026-10-10T02:00:00+02:00")),
            ..BatchFilter::default()
        }),
        "CANCEL",
        "clean up the day",
    );
    asked.max_items = Some(10);
    let created = batch
        .create(NS, &asked, keyed("batch-key-01"))
        .await
        .expect("create");
    let request = server.only();
    assert_eq!(
        request.body(),
        &json!({
            "selector": {"filter": {"status": ["RUNNING"], "createdAfter": "2026-10-10T00:00:00Z"}},
            "action": "CANCEL", "reason": "clean up the day", "maxItems": 10,
        })
    );
    assert_eq!(request.key(), Some("batch-key-01"));
    assert_eq!(
        (
            created.counts.selected,
            created.counts.succeeded,
            created.counts.deferred
        ),
        (2, 1, 1)
    );
    assert!(!created.has_payload);

    server.answer_with(202, json!({"state": "CANCELLING", "batchId": BATCH, "action": "CANCEL", "createdBy": "alice", "createdAt": "2026-10-10T10:00:00Z"}));
    let cancelling = batch
        .cancel(NS, BATCH, "stop the clean up", keyed("batch-cancel-1"))
        .await
        .expect("cancel");
    let request = server.only();
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/batch-operations/{BATCH}/cancel")
    );
    assert_eq!(request.body(), &json!({"reason": "stop the clean up"}));
    assert_eq!(request.key(), Some("batch-cancel-1"));
    assert_eq!(cancelling.state, "CANCELLING");

    server.answer_with(
        200,
        json!({"items": [
            {"ordinal": 0, "runId": RUN, "state": "SKIPPED", "reason": "ALREADY_TERMINAL"},
            {"ordinal": 1, "runId": RUN, "state": "NEWER_STATE"}], "nextCursor": "n"}),
    );
    let items = batch
        .list_items(
            NS,
            BATCH,
            BatchListFilter {
                state: Some("SKIPPED".into()),
                limit: Some(2),
                cursor: None,
            },
            CallOptions::default(),
        )
        .await
        .expect("items");
    let request = server.only();
    assert_eq!(
        request.path(),
        format!("/v1/namespaces/{NS}/batch-operations/{BATCH}/items")
    );
    assert_eq!(
        request.query(),
        vec![
            ("limit".to_owned(), "2".to_owned()),
            ("state".to_owned(), "SKIPPED".to_owned())
        ]
    );
    assert_eq!(items.items[0].reason.as_deref(), Some("ALREADY_TERMINAL"));
    assert_eq!(
        items.items[1].state, "NEWER_STATE",
        "a state of a newer server is kept as a string"
    );
    assert_eq!(items.next_cursor.as_deref(), Some("n"));
}

// ----------------------------------------------------------------------------------- with start

#[tokio::test]
async fn signal_with_start_names_the_workflow_by_id_carries_the_signal_and_one_key_and_says_what_the_signal_woke()
 {
    let server = Recorder::start().await;
    server.answer_with(
        202,
        json!({"runId": RUN, "started": true, "deliveredTo": ["wait"]}),
    );
    let result = server
        .client()
        .signal_with_start(
            WF,
            "order-9",
            &json!({"a": 1}),
            "go",
            &json!({"b": 2}),
            WithStartOptions {
                call: keyed("sws-key-0001"),
                business_id_reuse_policy: Some(BusinessIdReusePolicy::AllowDuplicateFailedOnly),
                ..WithStartOptions::default()
            },
        )
        .await
        .expect("signal with start");
    let request = server.only();
    assert_eq!(request.path(), "/v1/runs/signal-with-start");
    assert_eq!(
        request.body(),
        &json!({
            "workflowId": WF, "businessId": "order-9", "input": {"a": 1},
            "signal": {"name": "go", "payload": {"b": 2}},
            "businessIdReusePolicy": "ALLOW_DUPLICATE_FAILED_ONLY",
        })
    );
    assert_eq!(request.key(), Some("sws-key-0001"));
    assert_eq!(
        (
            result.run_id.to_string(),
            result.started,
            result.delivered_to
        ),
        (RUN.to_owned(), true, vec!["wait".to_owned()])
    );
}

#[tokio::test]
async fn update_with_start_requires_the_callers_key_keeps_it_across_a_retry_and_reads_a_rejection_as_an_answer()
 {
    let server = Recorder::start().await;
    let receipt = json!({"runId": RUN, "operation": "set", "updateKey": "u-1", "status": "APPLIED",
        "result": 7, "beforeStateRevision": 0, "afterStateRevision": 1, "runVersion": 4, "replayed": true});
    server.answer_with(
        200,
        json!({"runId": RUN, "started": true, "update": {"outcome": "APPLIED", "receipt": receipt}}),
    );
    let client = server.client();
    let applied = client
        .update_with_start(
            WF,
            "order-9",
            &json!({}),
            "set",
            "u-1",
            &json!({"value": 7}),
            UpdateWithStartOptions {
                with_start: WithStartOptions {
                    call: keyed("uws-key-0001"),
                    ..WithStartOptions::default()
                },
                business_id_conflict_policy: Some(BusinessIdConflictPolicy::Fail),
                start_delay: Some(Duration::from_secs(2)),
                expected_state_revision: Some(0),
            },
        )
        .await
        .expect("update with start");
    let request = server.only();
    assert_eq!(request.path(), "/v1/runs/update-with-start");
    assert_eq!(
        request.body(),
        &json!({
            "workflowId": WF, "businessId": "order-9", "input": {},
            "update": {"name": "set", "updateKey": "u-1", "args": {"value": 7}, "expectedStateRevision": 0},
            "businessIdConflictPolicy": "FAIL", "startDelayMs": 2000,
        })
    );
    assert_eq!(applied.update.outcome, "APPLIED");
    let got = applied.update.receipt.expect("a receipt");
    assert_eq!(got.after_state_revision, 1);
    assert_eq!(got.result, json!(7));
    assert!(got.replayed, "a replay says so");

    server.answer_with(
        200,
        json!({"runId": RUN, "started": false, "update": {"outcome": "REJECTED", "reason": "REVISION_CONFLICT"}}),
    );
    let rejected = client
        .update_with_start(
            WF,
            "order-9",
            &json!({}),
            "set",
            "u-2",
            &json!({}),
            keyed("uws-key-0002"),
        )
        .await
        .expect("a rejection is an answer");
    assert_eq!(
        (
            rejected.started,
            rejected.update.outcome.as_str(),
            rejected.update.reason.as_deref(),
            rejected.update.receipt.is_none()
        ),
        (false, "REJECTED", Some("REVISION_CONFLICT"), true)
    );

    server.prepare(|_, _| Reply::Empty(500));
    for error in [
        client
            .update_with_start(
                WF,
                "order-9",
                &json!({}),
                "set",
                "",
                &json!({}),
                CallOptions::default(),
            )
            .await
            .expect_err("no update key"),
        client
            .update_with_start(
                WF,
                "order-9",
                &json!({}),
                "set",
                "has space",
                &json!({}),
                CallOptions::default(),
            )
            .await
            .expect_err("an update key is printable ASCII"),
        client
            .update_with_start(
                WF,
                "order-9",
                &json!({}),
                "set",
                "u-3",
                &json!({}),
                UpdateWithStartOptions {
                    expected_state_revision: Some(u64::MAX),
                    ..UpdateWithStartOptions::default()
                },
            )
            .await
            .expect_err("a revision the wire cannot carry"),
    ] {
        assert!(is_validation(&error), "{error:?}");
    }
    assert!(server.seen().is_empty());

    server.prepare(move |n, _| {
        if n < 2 {
            unavailable()
        } else {
            Reply::Json(
                200,
                json!({"runId": RUN, "started": true, "update": {"outcome": "APPLIED", "receipt": receipt}}),
            )
        }
    });
    client
        .update_with_start(
            WF,
            "order-9",
            &json!({}),
            "set",
            "u-4",
            &json!({}),
            CallOptions::default(),
        )
        .await
        .expect("update with start");
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body(), seen[1].body());
    assert!(seen[0].key().is_some());
    assert_eq!(seen[0].key(), seen[1].key());
}

// ------------------------------------------------------------------------------------- reset

#[tokio::test]
async fn the_reset_calls_list_the_points_preview_the_plan_reset_with_a_key_and_read_the_register() {
    let server = Recorder::start().await;
    server.answer_with(
        200,
        json!({"items": [
            {"sequence": 1, "kind": "RUN_STARTED", "node": null, "occurredAt": "2026-10-10T10:00:00Z"},
            {"sequence": 4, "kind": "SOMETHING_NEWER", "node": "a", "occurredAt": "2026-10-10T10:00:01Z"}],
            "nextCursor": null}),
    );
    let client = server.client();
    let points = client
        .reset_points(
            RUN,
            PageOptions {
                limit: Some(5),
                cursor: None,
            },
            CallOptions::default(),
        )
        .await
        .expect("points");
    let request = server.only();
    assert_eq!(request.path(), format!("/v1/runs/{RUN}/reset-points"));
    assert_eq!(request.query(), vec![("limit".to_owned(), "5".to_owned())]);
    assert_eq!(
        points
            .items
            .iter()
            .map(|p| (p.sequence, p.kind.as_str(), p.node.as_deref()))
            .collect::<Vec<_>>(),
        vec![(1, "RUN_STARTED", None), (4, "SOMETHING_NEWER", Some("a"))]
    );

    server.answer_with(
        200,
        json!({"sequence": 4, "restored": [{"node": "a", "kind": "STEP"}],
               "reexecuted": [{"node": "b", "completedInSource": true, "markers": ["irreversible"]}],
               "reapply": [], "requiresAcknowledgement": true, "prefixEvents": 6}),
    );
    let plan = client
        .preview_reset(RUN, 4, Some(ReapplyPolicy::Nothing), CallOptions::default())
        .await
        .expect("plan");
    assert_eq!(
        server.only().body(),
        &json!({"sequence": 4, "reapply": "NONE"})
    );
    assert!(plan.requires_acknowledgement);
    assert_eq!(plan.reexecuted[0].node, "b");
    assert!(plan.reexecuted[0].completed_in_source);
    assert_eq!(plan.reexecuted[0].markers, ["irreversible"]);
    server.prepare(|_, _| Reply::Empty(500));
    let error = client
        .preview_reset(RUN, 0, None, CallOptions::default())
        .await
        .expect_err("sequence 0");
    assert!(is_validation(&error));
    assert!(server.seen().is_empty());

    server.answer_with(
        202,
        json!({"resetId": BATCH, "sourceRunId": RUN, "sequence": 4, "state": "CREATED",
               "newRunId": SCHEDULE, "reapplyReport": [], "createdAt": "2026-10-10T10:00:02Z"}),
    );
    let mut asked = ResetRequest::new(4, "start over from a");
    asked.acknowledge_reexecution = Some(true);
    asked.business_id = Some(BusinessIdDisposition::Drop);
    let made = client
        .reset(RUN, &asked, keyed("reset-key-001"))
        .await
        .expect("reset");
    let request = server.only();
    assert_eq!(request.path(), format!("/v1/runs/{RUN}/resets"));
    assert_eq!(request.key(), Some("reset-key-001"));
    assert_eq!(
        request.body(),
        &json!({"sequence": 4, "reason": "start over from a", "acknowledgeReexecution": true, "businessId": "DROP"})
    );
    assert_eq!(
        (
            made.state.as_str(),
            made.new_run_id.map(|id| id.to_string()),
            made.failure_reason
        ),
        ("CREATED", Some(SCHEDULE.to_owned()), None)
    );
    server.answer_with(
        202,
        json!({"resetId": BATCH, "sourceRunId": RUN, "sequence": 1, "state": "PENDING_SOURCE"}),
    );
    client
        .reset(
            RUN,
            &ResetRequest::new(1, "start over from the top"),
            CallOptions::default(),
        )
        .await
        .expect("reset");
    assert_eq!(
        server.only().body(),
        &json!({"sequence": 1, "reason": "start over from the top"}),
        "what was not asked is not sent"
    );

    server.answer_with(
        200,
        json!({"items": [{"resetId": BATCH, "sourceRunId": RUN, "sequence": 4, "state": "CREATED"}]}),
    );
    assert_eq!(
        client
            .list_resets(RUN, CallOptions::default())
            .await
            .expect("resets")
            .len(),
        1
    );
    server.answer_with(
        200,
        json!({"resetId": BATCH, "sourceRunId": RUN, "sequence": 4, "state": "CREATED"}),
    );
    let one = client
        .get_reset(RUN, BATCH, CallOptions::default())
        .await
        .expect("reset");
    assert_eq!(one.reset_id.to_string(), BATCH);
    assert_eq!(
        server.only().path(),
        format!("/v1/runs/{RUN}/resets/{BATCH}")
    );
}

#[tokio::test]
async fn a_start_with_an_input_blob_refuses_the_options_it_cannot_carry_and_the_token_is_in_nothing_a_group_exposes()
 {
    let server = Recorder::start().await;
    server.prepare(|_, _| Reply::Empty(500));
    let client = server.client();
    let descriptor: sagat_octopus_orchestrator_sdk::BlobDescriptor = serde_json::from_value(json!({
        "id": SCHEDULE, "size": 3, "sha256": "a".repeat(64), "contentType": "application/octet-stream"
    }))
    .expect("descriptor");
    let error = client
        .start_with_blob(
            WF,
            &descriptor,
            StartOptions {
                business_id: Some("order-1".into()),
                ..StartOptions::default()
            },
        )
        .await
        .expect_err("a business identifier cannot ride with an input blob");
    assert!(is_validation(&error));
    assert!(server.seen().is_empty());
    for shown in [
        format!("{:?}", client.schedules()),
        format!("{:?}", client.batch()),
        format!("{:?}", client.business_ids()),
        format!("{client:?}"),
    ] {
        assert!(!shown.contains("tok-1"), "{shown}");
    }
}
