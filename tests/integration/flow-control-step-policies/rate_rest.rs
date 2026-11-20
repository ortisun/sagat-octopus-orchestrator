//! The rate gate's reads over HTTP (task T022-078; FR-022-048; plan "Visibility of the rate gate", decision D022-067):
//! `Run.rateLimit` on the single-run read (with the waiting run's position and estimate) and on the listing (without
//! them), a step's wait in the runtime graph, and `GET /namespaces/{namespaceId}/rate-limit-keys` with the concurrency
//! listing's protections (labels to the reveal grant only, instants to the minute otherwise, a sealed cursor of its own
//! kind, `400` and `422`). Every instant is the database clock's; a window that must pass is waited for on that clock
//! with a bound (`until_past`), never by a sleep.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::support::api::{Options, TestApi, json_of, start_with};
use crate::support::flow_control::until_past;
use crate::support::scope_flow::publish;

/// A raw key value no body may carry.
const CANARY: &str = "acct-canary-7731";
const SECOND_US: i64 = 1_000_000;
const HOUR_US: i64 = 3_600 * SECOND_US;

/// An API with flow control on, tenant `acme` (admin `alice`, who may reveal personal data in it) and its namespace
/// `payments`, whose id is the second value.
async fn api() -> (TestApi, String) {
    let api = start_with(Options {
        flow_control: true,
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "flow").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let granted = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/payload-reveal-grants/alice"),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("grant");
    assert_eq!(granted.status(), StatusCode::OK);
    (api, namespace)
}

/// A one-step workflow whose start takes a slot of `input.accountId`, `limit` per `per`.
fn rated_start(name: &str, limit: u32, per: &str) -> Value {
    let mut doc =
        crate::support::engine::document(name, "g", vec![crate::support::engine::activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": per});
    doc
}

async fn start(api: &TestApi, workflow: &str, account: &str) -> String {
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {"accountId": account}}))
        .send()
        .await
        .expect("start run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .to_owned()
}

/// The body of `path` as `subject` of `acme`, checked for the canary.
async fn read_as(api: &TestApi, path: &str, subject: &str) -> (StatusCode, Value) {
    let response = api
        .as_user(Method::GET, path, subject, "acme")
        .header("x-octopus-namespace", "payments")
        .send()
        .await
        .expect("read");
    let status = response.status();
    let text = response.text().await.expect("body");
    assert!(!text.contains(CANARY), "a raw key value in {path}: {text}");
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn read(api: &TestApi, path: &str) -> (StatusCode, Value) {
    read_as(api, path, "alice").await
}

/// `subject` joins the tenant of `namespace` as an operator, without the reveal grant.
async fn member(api: &TestApi, namespace: &str, subject: &str) {
    let tenant: Uuid = sqlx::query_scalar("SELECT tenant_id FROM namespaces WHERE id = $1::uuid")
        .bind(namespace)
        .fetch_one(&api.db.pool)
        .await
        .expect("tenant");
    api.owner_store
        .insert_membership(&sagat_domain::identity::Membership::restore(
            sagat_domain::identity::TenantId::new(tenant),
            sagat_domain::identity::Subject::parse(subject).expect("subject"),
            sagat_domain::identity::Role::Operator,
            crate::support::now(),
        ))
        .await
        .expect("membership");
}

fn us(text: &Value) -> i64 {
    let at = OffsetDateTime::parse(text.as_str().expect("an instant"), &Rfc3339).expect("rfc3339");
    i64::try_from(at.unix_timestamp_nanos() / 1_000).expect("microseconds")
}

async fn database_now_us(api: &TestApi) -> i64 {
    sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint")
        .fetch_one(&api.db.pool)
        .await
        .expect("clock")
}

/// The grant instant of a run's slot, in microseconds.
async fn granted_us(api: &TestApi, run: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM granted_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1::uuid AND limit_scope = 'RUN'",
    )
    .bind(run)
    .fetch_one(&api.db.pool)
    .await
    .expect("the run's grant")
}

/// FR-022-048 and decision D022-067: a run shows the slot it holds (`GRANTED`, its limit and window, no deadline) or waits
/// for (`WAITING`, its queue deadline); the single-run read adds a waiting run's position and estimate, the listing
/// never does. With `limit: 1, per: hour`, one slot counted and two buffered: the first is at position 1, granted when
/// the counted slot ages out (its grant plus an hour, to the microsecond: `k = 1 + 0 - 1 + 1 = 1`); the second at
/// position 2 has no estimate (`k = 1 + 1 - 1 + 1 = 2` exceeds the one counted grant: beyond one window).
#[tokio::test]
async fn a_run_shows_its_rate_slot_and_a_waiting_run_its_place() {
    let (api, namespace) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-run",
        rated_start("rest-rate-run", 1, "hour"),
    )
    .await
    .expect("published");
    let holder = start(&api, &workflow, CANARY).await;
    let first = start(&api, &workflow, CANARY).await;
    let second = start(&api, &workflow, CANARY).await;

    let (status, held) = read(&api, &format!("/runs/{holder}")).await;
    assert_eq!(status, StatusCode::OK, "{held}");
    let slot = &held["rateLimit"];
    assert_eq!(slot["state"], "GRANTED", "{held}");
    assert_eq!(slot["group"], "accounts");
    assert_eq!(
        (slot["limit"].as_u64(), slot["per"].as_str()),
        (Some(1), Some("hour"))
    );
    assert_eq!(us(&slot["since"]), granted_us(&api, &holder).await);
    assert!(
        slot["queueDeadline"].is_null() && slot["position"].is_null(),
        "{slot}"
    );
    assert!(
        slot["keyLabel"].as_str().is_some_and(|l| l.len() == 12),
        "{slot}"
    );

    let (_, waiting) = read(&api, &format!("/runs/{first}")).await;
    let slot = &waiting["rateLimit"];
    assert_eq!(slot["state"], "WAITING", "{waiting}");
    assert!(slot["queueDeadline"].is_string());
    assert_eq!(slot["position"], 1, "{slot}");
    assert_eq!(
        us(&slot["estimatedGrantAt"]),
        granted_us(&api, &holder).await + HOUR_US,
        "the counted slot's grant plus the window, exactly"
    );
    // a member without the reveal grant: no label and no estimate (another run's grant instant plus the window,
    // decision D022-067 item 8); the position stays
    member(&api, &namespace, "olivia").await;
    let (status, hidden) = read_as(&api, &format!("/runs/{first}"), "olivia").await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    let slot = &hidden["rateLimit"];
    assert_eq!(slot["position"], 1, "{slot}");
    assert!(
        slot["keyLabel"].is_null() && slot["estimatedGrantAt"].is_null(),
        "{slot}"
    );
    let (_, behind) = read(&api, &format!("/runs/{second}")).await;
    assert_eq!(behind["rateLimit"]["position"], 2, "{behind}");
    assert!(
        behind["rateLimit"]["estimatedGrantAt"].is_null(),
        "beyond one window: {behind}"
    );

    let (_, listed) = read(&api, "/runs").await;
    for item in listed["items"].as_array().expect("items") {
        let slot = &item["rateLimit"];
        assert!(slot.is_object(), "every run here holds or waits: {item}");
        assert!(
            slot["position"].is_null() && slot["estimatedGrantAt"].is_null(),
            "a listing computes neither: {slot}"
        );
    }
}

/// The estimate's two other edges (plan "Visibility of the rate gate"): with `limit: 1, per: second`, one microsecond
/// before the counted slot ages out the waiter's estimate is that slot's grant plus a second; once it has aged out and
/// before a pass grants the waiter, there is room (`k = 0`) and the estimate is the read's own instant. A run's slot that
/// no longer counts is shown no more.
#[tokio::test]
async fn the_estimate_is_the_read_instant_once_there_is_room() {
    let (api, _) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-room",
        rated_start("rest-rate-room", 1, "second"),
    )
    .await
    .expect("published");
    let holder = start(&api, &workflow, CANARY).await;
    let waiter = start(&api, &workflow, CANARY).await;
    let granted = granted_us(&api, &holder).await;
    until_past(&api.db.pool, granted + SECOND_US).await;
    let before = database_now_us(&api).await;
    let (_, read_back) = read(&api, &format!("/runs/{waiter}")).await;
    let after = database_now_us(&api).await;
    let slot = &read_back["rateLimit"];
    assert_eq!(
        (slot["state"].as_str(), slot["position"].as_u64()),
        (Some("WAITING"), Some(1)),
        "{slot}"
    );
    let estimate = us(&slot["estimatedGrantAt"]);
    assert!(
        (before..=after).contains(&estimate) && estimate >= granted + SECOND_US,
        "the read's instant: {before} <= {estimate} <= {after}"
    );
    let (_, aged) = read(&api, &format!("/runs/{holder}")).await;
    assert!(
        aged.get("rateLimit").is_none(),
        "a slot that no longer counts is not shown: {aged}"
    );
}

/// FR-022-048: a step waiting for its rate-limit slot is shown as such on its node of the runtime graph.
#[tokio::test]
async fn a_step_waiting_for_a_slot_is_shown_waiting_on_its_rate_limit() {
    let (api, _) = api().await;
    let doc = crate::support::engine::document(
        "rest-rate-step",
        "g",
        vec![crate::support::engine::with(
            crate::support::engine::activity("g"),
            "rateLimit",
            json!({"group": "provider", "key": "input.accountId", "limit": 1, "per": "hour"}),
        )],
    );
    let workflow = publish(&api, "rest-rate-step", doc)
        .await
        .expect("published");
    let _granted = start(&api, &workflow, CANARY).await;
    let waiting = start(&api, &workflow, CANARY).await;
    let (status, graph) = read(&api, &format!("/runs/{waiting}/graph")).await;
    assert_eq!(status, StatusCode::OK, "{graph}");
    let node = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == "g")
        .unwrap_or_else(|| panic!("node g: {graph}"));
    assert_eq!(
        (node["status"].as_str(), node["waitingOn"].as_str()),
        (Some("WAITING"), Some("rate_limit")),
        "{node}"
    );
}

/// FR-022-048 and decision D022-067: the rate-limit keys of a namespace, oldest wait first and keys nobody waits for
/// last, with the grants that count, the unleased grants, the waiters, the oldest wait and the next slot's instant;
/// pages resume after the last key shown with a cursor of this listing's own kind (a concurrency listing's cursor and an
/// altered one are a `400`, a group that is no group name a `422`); a caller without the reveal grant sees no label and
/// the instants to the minute.
#[tokio::test]
async fn the_namespace_lists_its_rate_keys_with_the_concurrency_listings_protections() {
    let (api, namespace) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-keys",
        rated_start("rest-rate-keys", 1, "hour"),
    )
    .await
    .expect("published");
    // `acct-1`: a slot and a waiter; `acct-2`: a slot, nobody waits
    let holder = start(&api, &workflow, "acct-1").await;
    let _waiter = start(&api, &workflow, "acct-1").await;
    let other = start(&api, &workflow, "acct-2").await;
    let path = format!("/namespaces/{namespace}/rate-limit-keys");
    let (status, page) = read(&api, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["queuedRuns"], 1, "{page}");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "{page}");
    let waited = &items[0];
    assert_eq!(
        (
            waited["waiters"].as_u64(),
            waited["grantsInWindow"].as_u64(),
            waited["unleasedGrants"].as_u64()
        ),
        (Some(1), Some(1), Some(0)),
        "oldest wait first: {page}"
    );
    assert_eq!(
        us(&waited["nextSlotAt"]),
        granted_us(&api, &holder).await + HOUR_US,
        "{waited}"
    );
    assert!(
        waited["oldestWaitSince"].is_string() && waited["keyLabel"].is_string(),
        "{waited}"
    );
    let idle = &items[1];
    assert_eq!(
        (idle["waiters"].as_u64(), idle["grantsInWindow"].as_u64()),
        (Some(0), Some(1)),
        "{idle}"
    );
    assert!(
        idle["nextSlotAt"].is_null() && idle["oldestWaitSince"].is_null(),
        "nobody waits: {idle}"
    );
    let _ = other;

    // pages of one key resume exactly after the last one shown
    let (_, first) = read(&api, &format!("{path}?limit=1")).await;
    let cursor = first["nextCursor"]
        .as_str()
        .expect("a next page")
        .to_owned();
    let (status, second) = read(&api, &format!("{path}?limit=1&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["items"][0]["waiters"], 0, "{second}");
    assert!(second["nextCursor"].is_null(), "{second}");

    // a cursor of the concurrency listing, an altered one, and a group that is no group name
    let (_, concurrency) = read(
        &api,
        &format!("/namespaces/{namespace}/concurrency-keys?limit=1"),
    )
    .await;
    assert_eq!(
        concurrency["items"].as_array().map(Vec::len),
        Some(0),
        "no concurrency key here: {concurrency}"
    );
    let mut altered = cursor.clone();
    let last = altered.pop().expect("a character");
    altered.push(if last == 'A' { 'B' } else { 'A' });
    let (status, _) = read(&api, &format!("{path}?cursor={altered}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = read(&api, &format!("{path}?group=NOT%20A%20GROUP")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // a member without the reveal grant: no label, the instants to the minute
    api.owner_store
        .insert_membership(&sagat_domain::identity::Membership::restore(
            sagat_domain::identity::TenantId::new(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT tenant_id FROM namespaces WHERE id = $1::uuid",
                )
                .bind(&namespace)
                .fetch_one(&api.db.pool)
                .await
                .expect("tenant"),
            ),
            sagat_domain::identity::Subject::parse("olivia").expect("subject"),
            sagat_domain::identity::Role::Operator,
            crate::support::now(),
        ))
        .await
        .expect("membership");
    let (status, hidden) = read_as(&api, &path, "olivia").await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    let item = &hidden["items"][0];
    assert!(item["keyLabel"].is_null(), "{item}");
    for field in ["oldestWaitSince", "nextSlotAt"] {
        assert_eq!(
            us(&item[field]) % (60 * SECOND_US),
            0,
            "{field} to the minute: {item}"
        );
    }
}

/// Decision D022-067 item 5: a cursor of the concurrency listing is refused by the rate-limit listing.
#[tokio::test]
async fn a_concurrency_listings_cursor_is_refused_by_the_rate_listing() {
    let (api, namespace) = api().await;
    let gated = publish(
        &api,
        "rest-rate-cross",
        crate::run_admission::serial_start("rest-rate-cross", 1),
    )
    .await
    .expect("published");
    start(&api, &gated, "acct-1").await;
    start(&api, &gated, "acct-2").await;
    let (_, page) = read(
        &api,
        &format!("/namespaces/{namespace}/concurrency-keys?limit=1"),
    )
    .await;
    let cursor = page["nextCursor"].as_str().expect("a next page");
    let (status, refused) = read(
        &api,
        &format!("/namespaces/{namespace}/rate-limit-keys?cursor={cursor}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}
