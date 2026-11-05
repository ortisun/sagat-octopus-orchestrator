//! The schedule routes over HTTP (`T018-018`, `T018-023`): the contract as a client sees it, with a
//! real API, a real PostgreSQL and real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::json_of;
use crate::support::worker::{HUMAN, Platform, TENANT, chain_ir, platform};

struct Rig {
    p: Platform,
    namespace: String,
    workflow: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let namespace = p.namespace_id().await;
    let workflow = p.publish("reports", &chain_ir("reports")).await;
    Rig {
        p,
        namespace,
        workflow,
    }
}

fn body(workflow: &str, name: &str) -> Value {
    json!({
        "name": name,
        "spec": {"intervals": [{"everyMs": 3_600_000}]},
        "action": {"workflowId": workflow, "input": {"token": "hunter2", "region": "eu"}}
    })
}

impl Rig {
    fn schedules(&self) -> String {
        format!("/namespaces/{}/schedules", self.namespace)
    }

    fn schedule(&self, id: &str) -> String {
        format!("{}/{id}", self.schedules())
    }

    async fn create(&self, name: &str) -> Value {
        let (status, created) = self
            .p
            .rest(
                Method::POST,
                &self.schedules(),
                Some(&body(&self.workflow, name)),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        created
    }

    fn id(created: &Value) -> String {
        created["scheduleId"].as_str().expect("id").to_owned()
    }
}

fn reason(error: &Value) -> &str {
    error["details"]["reason"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn a_schedule_goes_through_its_whole_life_over_http() {
    let r = rig().await;
    let created = r.create("hourly").await;
    let id = Rig::id(&created);
    assert_eq!(created["name"], "hourly");
    assert_eq!(created["origin"], "API");
    assert_eq!(created["state"], "ACTIVE");
    assert_eq!(created["revision"], 1);
    assert_eq!(created["authorizedBy"], HUMAN);
    assert_eq!(created["spec"]["intervals"][0]["everyMs"], 3_600_000);
    assert_eq!(created["action"]["workflowId"], r.workflow);
    assert_eq!(
        created["action"]["inputDigest"]
            .as_str()
            .expect("digest")
            .len(),
        64
    );
    assert!(created["nextActionAt"].is_string());

    // describing it adds the next times and what it has done
    let (status, described) =
        r.p.rest(
            Method::GET,
            &format!("{}?nextCount=3&recentCount=5", r.schedule(&id)),
            None,
        )
        .await;
    assert_eq!(status, 200, "{described}");
    let next = described["nextActions"].as_array().expect("next");
    assert_eq!(next.len(), 3);
    assert_eq!(described["recentActions"], json!([]));
    assert_eq!(described["openRuns"], json!([]));
    assert_eq!(described["bufferedCount"], 0);
    assert_eq!(described["counters"], json!({}));
    assert_eq!(described["scheduleId"], id.as_str());

    // it is in the list
    let (status, listed) = r.p.rest(Method::GET, &r.schedules(), None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"].as_array().expect("items").len(), 1);
    assert_eq!(listed["nextCursor"], Value::Null);

    // replacing it makes a second revision; a stale revision is refused with the current one
    let mut replacement = body(&r.workflow, "hourly");
    replacement["spec"] = json!({"intervals": [{"everyMs": 7_200_000}]});
    replacement["expectedRevision"] = json!(1);
    let (status, updated) =
        r.p.rest(Method::PUT, &r.schedule(&id), Some(&replacement))
            .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["spec"]["intervals"][0]["everyMs"], 7_200_000);
    let (status, stale) =
        r.p.rest(Method::PUT, &r.schedule(&id), Some(&replacement))
            .await;
    assert_eq!(status, 409, "{stale}");
    assert_eq!(reason(&stale), "REVISION_CONFLICT");
    assert_eq!(stale["details"]["currentRevision"], 2);

    // deleting needs the head revision
    let (status, stale) =
        r.p.rest(
            Method::DELETE,
            &format!("{}?expectedRevision=1", r.schedule(&id)),
            None,
        )
        .await;
    assert_eq!(status, 409, "{stale}");
    let (status, _) =
        r.p.rest(
            Method::DELETE,
            &format!("{}?expectedRevision=2", r.schedule(&id)),
            None,
        )
        .await;
    assert_eq!(status, 204);
    let (status, gone) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(status, 404, "{gone}");
    let (_, listed) = r.p.rest(Method::GET, &r.schedules(), None).await;
    assert_eq!(listed["items"], json!([]));
}

#[tokio::test]
async fn the_input_of_the_runs_is_never_returned() {
    let r = rig().await;
    let created = r.create("private").await;
    let id = Rig::id(&created);
    let mut seen = vec![created.to_string()];
    seen.push(
        r.p.rest(Method::GET, &r.schedule(&id), None)
            .await
            .1
            .to_string(),
    );
    seen.push(
        r.p.rest(Method::GET, &r.schedules(), None)
            .await
            .1
            .to_string(),
    );
    let mut replacement = body(&r.workflow, "private");
    replacement["expectedRevision"] = json!(1);
    seen.push(
        r.p.rest(Method::PUT, &r.schedule(&id), Some(&replacement))
            .await
            .1
            .to_string(),
    );
    for text in seen {
        assert!(
            !text.contains("hunter2") && !text.contains("\"eu\""),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_schedule_that_cannot_be_accepted_says_why_and_where() {
    let r = rig().await;
    for (spec, expected_reason, field) in [
        (
            json!({"intervals": [{"everyMs": 3_600_000}], "timezone": "Mars/Olympus"}),
            "SCHEDULE_ZONE_UNKNOWN",
            Some("/spec/timezone"),
        ),
        (
            json!({"intervals": [{"everyMs": 10_000}]}),
            "SCHEDULE_TOO_FREQUENT",
            None,
        ),
        (
            json!({"intervals": [{"everyMs": 3_600_000}], "nonsense": 1}),
            "SCHEDULE_SPEC_INVALID",
            Some("/spec/nonsense"),
        ),
    ] {
        let mut doc = body(&r.workflow, "refused");
        doc["spec"] = spec;
        let (status, refused) = r.p.rest(Method::POST, &r.schedules(), Some(&doc)).await;
        assert_eq!(status, 422, "{refused}");
        assert_eq!(reason(&refused), expected_reason, "{refused}");
        if let Some(field) = field {
            assert_eq!(refused["details"]["field"], field, "{refused}");
        }
    }
    // a workflow that is not in the namespace, a name that is not a name, an input that is not an object
    for doc in [
        body("0194f5e2-7b1c-7000-8000-000000000001", "elsewhere"),
        body(&r.workflow, "Not A Name"),
        json!({"name": "no-object", "spec": {"intervals": [{"everyMs": 3_600_000}]},
               "action": {"workflowId": r.workflow, "input": "text"}}),
        json!({"name": "unknown-key", "spec": {"intervals": [{"everyMs": 3_600_000}]},
               "action": {"workflowId": r.workflow, "input": {}}, "surprise": true}),
    ] {
        let (status, refused) = r.p.rest(Method::POST, &r.schedules(), Some(&doc)).await;
        assert_eq!(status, 422, "{doc} -> {refused}");
    }
}

#[tokio::test]
async fn a_name_in_use_is_a_conflict_and_a_deleted_name_is_free() {
    let r = rig().await;
    let created = r.create("taken").await;
    let (status, again) =
        r.p.rest(
            Method::POST,
            &r.schedules(),
            Some(&body(&r.workflow, "taken")),
        )
        .await;
    assert_eq!(status, 409, "{again}");
    let (status, _) =
        r.p.rest(
            Method::DELETE,
            &format!("{}?expectedRevision=1", r.schedule(&Rig::id(&created))),
            None,
        )
        .await;
    assert_eq!(status, 204);
    r.create("taken").await;
}

#[tokio::test]
async fn the_list_is_newest_first_and_pages_by_cursor() {
    let r = rig().await;
    let mut ids = Vec::new();
    for n in 0..3 {
        ids.push(Rig::id(&r.create(&format!("page-{n}")).await));
    }
    let (status, first) =
        r.p.rest(Method::GET, &format!("{}?limit=2", r.schedules()), None)
            .await;
    assert_eq!(status, 200, "{first}");
    let names: Vec<_> = first["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|s| s["name"].as_str().expect("name").to_owned())
        .collect();
    assert_eq!(names, ["page-2", "page-1"]);
    let cursor = first["nextCursor"].as_str().expect("a next page");
    let (_, second) =
        r.p.rest(
            Method::GET,
            &format!("{}?limit=2&cursor={cursor}", r.schedules()),
            None,
        )
        .await;
    assert_eq!(second["items"][0]["name"], "page-0");
    assert_eq!(second["items"].as_array().expect("items").len(), 1);
    assert_eq!(second["nextCursor"], Value::Null);
    let (status, bad) =
        r.p.rest(
            Method::GET,
            &format!("{}?cursor=nonsense", r.schedules()),
            None,
        )
        .await;
    assert_eq!(status, 400, "{bad}");
}

/// The names of the schedules a listing returned, in order.
fn names(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|s| s["name"].as_str().expect("name").to_owned())
        .collect()
}

#[tokio::test]
async fn a_listing_can_be_narrowed_by_state_workflow_and_name_and_pages_keep_to_the_filter() {
    let r = rig().await;
    let other = r.p.publish("billing", &chain_ir("billing")).await;
    let (status, made) =
        r.p.rest(
            Method::POST,
            &r.schedules(),
            Some(&body(&other, "billing-nightly")),
        )
        .await;
    assert_eq!(status, 201, "{made}");
    let resting = Rig::id(&r.create("reports-resting").await);
    for n in 0..3 {
        r.create(&format!("reports-{n}")).await;
    }
    let (status, paused) =
        r.p.rest(
            Method::POST,
            &format!("{}/pause", r.schedule(&resting)),
            Some(&json!({"note": "release freeze"})),
        )
        .await;
    assert_eq!(status, 200, "{paused}");

    let listed = |query: String| {
        let url = format!("{}?{query}", r.schedules());
        let r = &r;
        async move {
            let (status, page) = r.p.rest(Method::GET, &url, None).await;
            assert_eq!(status, 200, "{url}: {page}");
            page
        }
    };
    assert_eq!(
        names(&listed("state=PAUSED".to_owned()).await),
        ["reports-resting"]
    );
    assert_eq!(
        names(&listed("state=ACTIVE".to_owned()).await),
        ["reports-2", "reports-1", "reports-0", "billing-nightly"]
    );
    assert_eq!(
        names(&listed(format!("workflowId={other}")).await),
        ["billing-nightly"]
    );
    assert_eq!(
        names(&listed("name=reports-r".to_owned()).await),
        ["reports-resting"],
        "a name is matched by its start"
    );
    assert_eq!(
        names(&listed("name=reports-&state=ACTIVE&origin=API".to_owned()).await),
        ["reports-2", "reports-1", "reports-0"],
        "every part given has to hold"
    );
    assert!(
        names(&listed("origin=TRIGGER".to_owned()).await).is_empty(),
        "none of these is owned by a trigger"
    );
    assert!(
        names(&listed(format!("workflowId={}&state=PAUSED", other)).await).is_empty(),
        "the filters are ANDed"
    );

    // a page keeps to the filter and ends where the filtered listing ends
    let first = listed("name=reports-&limit=2".to_owned()).await;
    assert_eq!(names(&first), ["reports-2", "reports-1"]);
    let cursor = first["nextCursor"].as_str().expect("a next page");
    let second = listed(format!("name=reports-&limit=2&cursor={cursor}")).await;
    assert_eq!(names(&second), ["reports-0", "reports-resting"]);
    assert_eq!(second["nextCursor"], Value::Null);
}

#[tokio::test]
async fn a_filter_that_is_not_one_is_refused_in_words() {
    let r = rig().await;
    for query in [
        "state=DELETED",
        "state=active",
        "origin=SOMEONE",
        "workflowId=reports",
        &format!("name={}", "n".repeat(129)),
    ] {
        let (status, refused) =
            r.p.rest(Method::GET, &format!("{}?{query}", r.schedules()), None)
                .await;
        assert_eq!(status, 400, "{query}: {refused}");
    }
}

#[tokio::test]
async fn every_change_needs_an_idempotency_key() {
    let r = rig().await;
    let created = r.create("keyed").await;
    let id = Rig::id(&created);
    let attempts = [
        (
            Method::POST,
            r.schedules(),
            Some(body(&r.workflow, "no-key")),
        ),
        (
            Method::PUT,
            r.schedule(&id),
            Some({
                let mut doc = body(&r.workflow, "keyed");
                doc["expectedRevision"] = json!(1);
                doc
            }),
        ),
        (
            Method::DELETE,
            format!("{}?expectedRevision=1", r.schedule(&id)),
            None,
        ),
        (
            Method::POST,
            format!("{}/pause", r.schedule(&id)),
            Some(json!({"note": "maintenance"})),
        ),
        (
            Method::POST,
            format!("{}/resume", r.schedule(&id)),
            Some(json!({"note": "done"})),
        ),
    ];
    for (method, path, doc) in attempts {
        let mut request = r.p.api.as_user(method.clone(), &path, HUMAN, TENANT);
        if let Some(doc) = &doc {
            request = request.json(doc);
        }
        let response = request.send().await.expect("send");
        assert_eq!(response.status(), 400, "{method} {path}");
    }
    // nothing changed
    let (_, read) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(
        (read["revision"].clone(), read["state"].clone()),
        (json!(1), json!("ACTIVE"))
    );
}

#[tokio::test]
async fn pausing_and_resuming_keep_a_note_and_the_times() {
    let r = rig().await;
    let id = Rig::id(&r.create("resting").await);
    let (status, paused) =
        r.p.rest(
            Method::POST,
            &format!("{}/pause", r.schedule(&id)),
            Some(&json!({"note": "release freeze"})),
        )
        .await;
    assert_eq!(status, 200, "{paused}");
    assert_eq!(paused["state"], "PAUSED");
    assert_eq!(paused["note"], "release freeze");
    assert_eq!(paused["noteBy"], HUMAN);
    assert_eq!(paused["nextActionAt"], Value::Null);
    let (_, described) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(
        described["nextActions"],
        json!([]),
        "nothing is acted on while paused"
    );

    let (status, resumed) =
        r.p.rest(
            Method::POST,
            &format!("{}/resume", r.schedule(&id)),
            Some(&json!({"note": "freeze over"})),
        )
        .await;
    assert_eq!(status, 200, "{resumed}");
    assert_eq!(resumed["state"], "ACTIVE");
    assert!(resumed["nextActionAt"].is_string());

    for bad in [
        json!({"note": ""}),
        json!({}),
        json!({"note": "x", "extra": 1}),
    ] {
        let (status, refused) =
            r.p.rest(
                Method::POST,
                &format!("{}/pause", r.schedule(&id)),
                Some(&bad),
            )
            .await;
        assert_eq!(status, 422, "{bad} -> {refused}");
    }
}

#[tokio::test]
async fn roles_decide_who_may_read_write_and_operate() {
    let r = rig().await;
    let id = Rig::id(&r.create("guarded").await);
    r.p.grant(r.p.tenant, "vera", Role::Viewer).await;
    r.p.grant(r.p.tenant, "olga", Role::Operator).await;
    r.p.grant(r.p.tenant, "dev", Role::Developer).await;

    // a viewer reads and does nothing else
    let (status, _) =
        r.p.rest_as("vera", Method::GET, &r.schedule(&id), None)
            .await;
    assert_eq!(status, 200);
    let (status, _) =
        r.p.rest_as(
            "vera",
            Method::POST,
            &r.schedules(),
            Some(&body(&r.workflow, "by-vera")),
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) =
        r.p.rest_as(
            "vera",
            Method::POST,
            &format!("{}/pause", r.schedule(&id)),
            Some(&json!({"note": "no"})),
        )
        .await;
    assert_eq!(status, 403);

    // an operator pauses but does not write
    let (status, _) =
        r.p.rest_as(
            "olga",
            Method::POST,
            &format!("{}/pause", r.schedule(&id)),
            Some(&json!({"note": "on call"})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, _) =
        r.p.rest_as(
            "olga",
            Method::POST,
            &r.schedules(),
            Some(&body(&r.workflow, "by-olga")),
        )
        .await;
    assert_eq!(status, 403);

    // a developer writes, and becomes the authority of what they replace
    let mut replacement = body(&r.workflow, "guarded");
    replacement["expectedRevision"] = json!(1);
    let (status, updated) =
        r.p.rest_as("dev", Method::PUT, &r.schedule(&id), Some(&replacement))
            .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["authorizedBy"], "dev");
}

#[tokio::test]
async fn a_schedule_is_invisible_outside_its_namespace_and_its_tenant() {
    let r = rig().await;
    let id = Rig::id(&r.create("hidden").await);

    // another namespace of the same tenant
    let workspace = r.p.api.create_workspace(HUMAN, TENANT, "second").await;
    let other =
        r.p.api
            .create_namespace(HUMAN, TENANT, &workspace, "elsewhere", "dev")
            .await;
    let through_other = format!("/namespaces/{other}/schedules/{id}");
    for (method, path, doc) in [
        (Method::GET, through_other.clone(), None),
        (
            Method::PUT,
            through_other.clone(),
            Some({
                let mut doc = body(&r.workflow, "hidden");
                doc["expectedRevision"] = json!(1);
                doc
            }),
        ),
        (
            Method::DELETE,
            format!("{through_other}?expectedRevision=1"),
            None,
        ),
        (
            Method::POST,
            format!("{through_other}/pause"),
            Some(json!({"note": "x"})),
        ),
    ] {
        let (status, found) = r.p.rest(method.clone(), &path, doc.as_ref()).await;
        assert_eq!(status, 404, "{method} {path}: {found}");
    }
    let (_, listed) =
        r.p.rest(Method::GET, &format!("/namespaces/{other}/schedules"), None)
            .await;
    assert_eq!(listed["items"], json!([]));

    // another tenant, with its own admin
    let response =
        r.p.api
            .as_user(Method::GET, &r.schedule(&id), "bob", "globex")
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 404);
    let body_of = json_of(response).await;
    assert!(!body_of.to_string().contains(&id), "{body_of}");
    let response =
        r.p.api
            .as_user(Method::GET, &r.schedules(), "bob", "globex")
            .send()
            .await
            .expect("send");
    assert_eq!(
        response.status(),
        404,
        "a namespace of another tenant does not exist"
    );

    // the schedule is untouched
    let (_, read) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(
        (read["revision"].clone(), read["state"].clone()),
        (json!(1), json!("ACTIVE"))
    );
}

#[tokio::test]
async fn an_identifier_that_is_not_one_is_not_found() {
    let r = rig().await;
    let (status, _) = r.p.rest(Method::GET, &r.schedule("not-an-id"), None).await;
    assert_eq!(status, 404);
    let (status, _) =
        r.p.rest(Method::GET, "/namespaces/not-an-id/schedules", None)
            .await;
    assert_eq!(status, 404);
    let (status, _) =
        r.p.rest(
            Method::GET,
            &r.schedule("0194f5e2-7b1c-7000-8000-000000000009"),
            None,
        )
        .await;
    assert_eq!(status, 404);
}

impl Rig {
    /// A request with an Idempotency-Key of the test's choosing.
    async fn keyed(
        &self,
        sub: &str,
        method: Method,
        path: &str,
        key: &str,
        body: Option<&Value>,
    ) -> (u16, Value) {
        let mut request = self
            .p
            .api
            .as_user(method, path, sub, TENANT)
            .header("idempotency-key", key);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("send");
        let status = response.status().as_u16();
        let text = response.text().await.expect("body");
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn trigger_now_acts_at_once_and_a_repeat_with_the_key_is_told_the_same() {
    let r = rig().await;
    let id = Rig::id(&r.create("manual").await);
    let trigger = format!("{}/trigger", r.schedule(&id));

    // the body is optional
    let (status, first) = r
        .keyed(HUMAN, Method::POST, &trigger, "trigger-key-0001", None)
        .await;
    assert_eq!(status, 202, "{first}");
    assert_eq!(first["kind"], "TRIGGERED");
    assert_eq!(first["origin"], "MANUAL");
    assert_eq!(first["outcome"], "STARTED");
    let run = first["runId"].as_str().expect("a run").to_owned();

    let (status, again) = r
        .keyed(HUMAN, Method::POST, &trigger, "trigger-key-0001", None)
        .await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again, first, "the answer of the first request");
    let (status, reused) = r
        .keyed(
            HUMAN,
            Method::POST,
            &trigger,
            "trigger-key-0001",
            Some(&json!({"overlap": "ALLOW_ALL"})),
        )
        .await;
    assert_eq!(status, 409, "{reused}");
    assert_eq!(reason(&reused), "IDEMPOTENCY_KEY_REUSED");

    // the schedule shows it
    let (_, described) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(described["openRuns"], json!([run]));
    assert_eq!(described["recentActions"][0]["outcome"], "STARTED");
    assert_eq!(described["recentActions"][0]["origin"], "MANUAL");
    assert_eq!(described["counters"]["STARTED"], 1);

    // while that run is open the schedule's own rule (skip) applies, unless the request overrides it
    let (status, skipped) = r
        .keyed(
            HUMAN,
            Method::POST,
            &trigger,
            "trigger-key-0002",
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 202, "{skipped}");
    assert_eq!(skipped["outcome"], "SKIPPED_OVERLAP");
    let (status, allowed) = r
        .keyed(
            HUMAN,
            Method::POST,
            &trigger,
            "trigger-key-0003",
            Some(&json!({"overlap": "ALLOW_ALL"})),
        )
        .await;
    assert_eq!(status, 202, "{allowed}");
    assert_eq!(allowed["outcome"], "STARTED");
}

#[tokio::test]
async fn trigger_now_works_while_paused_and_refuses_a_bad_request() {
    let r = rig().await;
    let id = Rig::id(&r.create("paused-manual").await);
    let (status, _) =
        r.p.rest(
            Method::POST,
            &format!("{}/pause", r.schedule(&id)),
            Some(&json!({"note": "freeze"})),
        )
        .await;
    assert_eq!(status, 200);
    let trigger = format!("{}/trigger", r.schedule(&id));
    let (status, started) = r
        .keyed(HUMAN, Method::POST, &trigger, "trigger-key-0001", None)
        .await;
    assert_eq!(status, 202, "{started}");
    assert_eq!(started["outcome"], "STARTED");
    let (_, read) = r.p.rest(Method::GET, &r.schedule(&id), None).await;
    assert_eq!(read["state"], "PAUSED", "it stays paused");

    for (n, bad) in [
        json!({"overlap": "SOMETIMES"}),
        json!({"surprise": true}),
        json!("text"),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, refused) = r
            .keyed(
                HUMAN,
                Method::POST,
                &trigger,
                &format!("trigger-bad-000{n}"),
                Some(&bad),
            )
            .await;
        assert_eq!(status, 422, "{bad} -> {refused}");
    }
    // the key is required
    let response =
        r.p.api
            .as_user(Method::POST, &trigger, HUMAN, TENANT)
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn only_those_who_may_start_runs_can_trigger_and_nobody_outside_the_namespace() {
    let r = rig().await;
    let id = Rig::id(&r.create("guarded-manual").await);
    r.p.grant(r.p.tenant, "vera", Role::Viewer).await;
    r.p.grant(r.p.tenant, "dev", Role::Developer).await;
    let trigger = format!("{}/trigger", r.schedule(&id));

    let (status, _) = r
        .keyed("vera", Method::POST, &trigger, "trigger-key-0001", None)
        .await;
    assert_eq!(status, 403, "a viewer reads");
    let (status, started) = r
        .keyed("dev", Method::POST, &trigger, "trigger-key-0002", None)
        .await;
    assert_eq!(status, 202, "{started}");

    let workspace = r.p.api.create_workspace(HUMAN, TENANT, "second").await;
    let other =
        r.p.api
            .create_namespace(HUMAN, TENANT, &workspace, "elsewhere", "dev")
            .await;
    let (status, _) = r
        .keyed(
            HUMAN,
            Method::POST,
            &format!("/namespaces/{other}/schedules/{id}/trigger"),
            "trigger-key-0003",
            None,
        )
        .await;
    assert_eq!(status, 404, "another namespace");
    let response =
        r.p.api
            .as_user(Method::POST, &trigger, "bob", "globex")
            .header("idempotency-key", "trigger-key-0004")
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 404, "another tenant");
}

/// A range of the past: the three hours before the last whole hour.
fn past_range() -> Value {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let end = now - now % 3_600;
    let rfc = |s: i64| {
        time::OffsetDateTime::from_unix_timestamp(s)
            .expect("instant")
            .format(&time::format_description::well_known::Rfc3339)
            .expect("format")
    };
    json!({"start": rfc(end - 3 * 3_600), "end": rfc(end)})
}

#[tokio::test]
async fn a_backfill_is_asked_for_listed_and_stopped_over_http() {
    let r = rig().await;
    let id = Rig::id(&r.create("history").await);
    let backfills = format!("{}/backfills", r.schedule(&id));

    let (status, accepted) = r
        .keyed(
            HUMAN,
            Method::POST,
            &backfills,
            "backfill-key-0001",
            Some(&past_range()),
        )
        .await;
    assert_eq!(status, 202, "{accepted}");
    assert_eq!(accepted["state"], "PENDING");
    assert_eq!(
        accepted["overlap"], "BUFFER_ALL",
        "the default of a backfill"
    );
    assert_eq!(accepted["counts"], json!({}));
    assert_eq!(accepted["cursor"], Value::Null);
    let backfill = accepted["backfillId"].as_str().expect("id").to_owned();

    let (status, listed) = r.p.rest(Method::GET, &backfills, None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"].as_array().expect("items").len(), 1);
    assert_eq!(listed["items"][0]["backfillId"], backfill.as_str());

    // one at a time on a schedule
    let (status, busy) = r
        .keyed(
            HUMAN,
            Method::POST,
            &backfills,
            "backfill-key-0002",
            Some(&past_range()),
        )
        .await;
    assert_eq!(status, 429, "{busy}");

    let cancel = format!("{backfills}/{backfill}/cancel");
    let (status, stopped) = r
        .keyed(HUMAN, Method::POST, &cancel, "backfill-key-0003", None)
        .await;
    assert_eq!(status, 200, "{stopped}");
    assert_eq!(stopped["state"], "CANCELLED");
    let (status, again) = r
        .keyed(HUMAN, Method::POST, &cancel, "backfill-key-0004", None)
        .await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again, stopped, "stopping it twice changes nothing");
    let (status, _) = r
        .keyed(
            HUMAN,
            Method::POST,
            &format!("{backfills}/{}/cancel", Uuid::now_v7()),
            "backfill-key-0005",
            None,
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_range_that_makes_no_sense_is_refused_and_the_roles_are_kept() {
    let r = rig().await;
    let id = Rig::id(&r.create("bounds").await);
    let backfills = format!("{}/backfills", r.schedule(&id));
    let range = past_range();
    let mut backwards = range.clone();
    backwards["start"] = range["end"].clone();
    backwards["end"] = range["start"].clone();
    let (status, refused) = r
        .keyed(
            HUMAN,
            Method::POST,
            &backfills,
            "backfill-key-0001",
            Some(&backwards),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "BACKFILL_RANGE_INVALID");
    let (status, refused) = r
        .keyed(
            HUMAN,
            Method::POST,
            &backfills,
            "backfill-key-0002",
            Some(&json!({"start": range["start"], "end": range["end"], "overlap": "SOMETIMES"})),
        )
        .await;
    assert_eq!(status, 422, "{refused}");

    r.p.grant(r.p.tenant, "vera", Role::Viewer).await;
    let (status, _) = r
        .keyed("vera", Method::GET, &backfills, "backfill-key-0003", None)
        .await;
    assert_eq!(status, 200, "a viewer reads them");
    let (status, _) = r
        .keyed(
            "vera",
            Method::POST,
            &backfills,
            "backfill-key-0004",
            Some(&range),
        )
        .await;
    assert_eq!(status, 403, "and asks for none");
    let response =
        r.p.api
            .as_user(Method::POST, &backfills, "bob", "globex")
            .header("idempotency-key", "backfill-key-0005")
            .json(&range)
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 404, "another tenant");
}

#[tokio::test]
async fn the_runs_a_schedule_started_are_listed_by_it() {
    let r = rig().await;
    let first = Rig::id(&r.create("first").await);
    let second = Rig::id(&r.create("second").await);
    let run_of = |answer: &Value| answer["runId"].as_str().expect("a run").to_owned();
    let (status, a) = r
        .keyed(
            HUMAN,
            Method::POST,
            &format!("{}/trigger", r.schedule(&first)),
            "filter-key-0001",
            None,
        )
        .await;
    assert_eq!(status, 202, "{a}");
    let (status, b) = r
        .keyed(
            HUMAN,
            Method::POST,
            &format!("{}/trigger", r.schedule(&second)),
            "filter-key-0002",
            None,
        )
        .await;
    assert_eq!(status, 202, "{b}");
    // and a run nobody scheduled
    let (status, plain) =
        r.p.rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": r.workflow, "input": {}})),
        )
        .await;
    assert_eq!(status, 202, "{plain}");

    let listed = |query: String| {
        let r = &r;
        async move {
            let (status, page) = r.p.rest(Method::GET, &format!("/runs?{query}"), None).await;
            (status, page)
        }
    };
    let ids = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|run| run["runId"].as_str().expect("id").to_owned())
            .collect()
    };
    let (status, page) = listed(format!("scheduleId={first}")).await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(ids(&page), [run_of(&a)]);
    let (status, page) = listed(format!("scheduleId={second}")).await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(ids(&page), [run_of(&b)]);
    let (_, everything) = listed(String::new()).await;
    assert_eq!(
        ids(&everything).len(),
        3,
        "the filter narrows, it does not replace"
    );
    // a schedule nobody has: no runs. one that is not an id: refused in words.
    let (status, none) = listed(format!("scheduleId={}", uuid::Uuid::now_v7())).await;
    assert_eq!(status, 200, "{none}");
    assert!(ids(&none).is_empty());
    let (status, refused) = listed("scheduleId=not-an-id".to_owned()).await;
    assert_eq!(status, 422, "{refused}");
}
