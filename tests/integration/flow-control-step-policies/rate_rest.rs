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

/// A one-step workflow whose start takes a slot of `input.accountId`, `limit` per `per`, buffered for at most two hours
/// (longer than every window, so an estimate one window away comes before the deadline; decision D022-068 item 1).
fn rated_start(name: &str, limit: u32, per: &str) -> Value {
    rated_start_waiting(name, limit, per, 7_200_000)
}

/// [`rated_start`] buffered for at most `queue_timeout_ms`.
fn rated_start_waiting(name: &str, limit: u32, per: &str, queue_timeout_ms: u64) -> Value {
    let mut doc =
        crate::support::engine::document(name, "g", vec![crate::support::engine::activity("g")]);
    doc["rateLimit"] = json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": per,
                              "queueTimeoutMs": queue_timeout_ms});
    doc
}

/// Holds `body` to the canonical contract's schema of `GET template`'s `200` (review of task T022-078, architecture
/// minor 8: the rate bodies are `additionalProperties: false`).
fn conforms(template: &str, body: &Value) {
    crate::support::contract::OpenApi::load()
        .validate_body("GET", template, 200, body)
        .expect("a documented JSON body")
        .unwrap_or_else(|errors| panic!("GET {template} against the contract: {errors}\n{body}"));
}

/// The run's operation `action` (`PAUSE`, `RESUME`, `CANCEL`) as `alice`.
async fn operate(api: &TestApi, run: &str, action: &str) {
    let done = api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/operations"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"action": action, "reason": "a test of the rate gate's reads"}))
        .send()
        .await
        .expect("operation");
    assert!(done.status().is_success(), "{action}: {}", done.status());
}

/// The run's waiter `(enqueued_at, deadline_at)` in microseconds.
async fn waiter_us(api: &TestApi, run: &str) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (extract(epoch FROM enqueued_at) * 1000000)::bigint, (extract(epoch FROM deadline_at) * 1000000)::bigint \
           FROM rate_limit_waiters WHERE run_id = $1::uuid AND limit_scope = 'RUN'",
    )
    .bind(run)
    .fetch_one(&api.db.pool)
    .await
    .expect("the run's waiter")
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
    conforms("/runs/{runId}", &held);
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
    conforms("/runs/{runId}", &waiting);
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

/// The estimate's room edge (plan "Visibility of the rate gate"): with `limit: 1, per: second`, once the counted slot has
/// aged out and before a pass grants the waiter, there is room (`k = 0`) and the estimate is the read's own instant. A
/// run's slot that no longer counts is shown no more. (The instant one microsecond either side of an age-out is pinned
/// by the unit tests of `estimated_grant`.)
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
    let (api, namespace) = api().await;
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
    conforms("/runs/{runId}/graph", &graph);
    // the step key's window: its slot is granted and not leased (no worker polled), so it frees by a lease or a return,
    // not by time, and has no next slot (review of task T022-078, correctness minor 4), even beside a leased grant that
    // counts and would otherwise name one (its re-review, correctness Major 2)
    sqlx::query(
        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
                                       key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
         SELECT tenant_id, namespace_id, run_id, 'leased', 1, pool, limit_scope, group_name, key_digest, declared_limit, \
                period_ms, clock_timestamp() - interval '1 minute', clock_timestamp() - interval '1 minute', shard \
           FROM rate_limit_grants WHERE limit_scope = 'STEP' AND leased_at IS NULL",
    )
    .execute(&api.db.pool)
    .await
    .expect("a leased grant that counts");
    let (status, page) = read(&api, &format!("/namespaces/{namespace}/rate-limit-keys")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    conforms("/namespaces/{namespaceId}/rate-limit-keys", &page);
    let item = &page["items"][0];
    assert_eq!(
        (
            item["scope"].as_str(),
            item["unleasedGrants"].as_u64(),
            item["grantsInWindow"].as_u64(),
            item["waiters"].as_u64()
        ),
        (Some("STEP"), Some(1), Some(2), Some(1)),
        "{page}"
    );
    assert!(item["nextSlotAt"].is_null(), "an unleased grant: {item}");
}

/// FR-022-048 (review of task T022-078, architecture Major 5): a step waiting for its concurrency permit is shown as
/// such on its node of the runtime graph.
#[tokio::test]
async fn a_step_waiting_for_a_permit_is_shown_waiting_on_its_concurrency() {
    let (api, _) = api().await;
    let doc = crate::support::engine::document(
        "rest-permit-step",
        "g",
        vec![crate::support::engine::with(
            crate::support::engine::activity("g"),
            "concurrency",
            json!({"key": "input.accountId", "limit": 1}),
        )],
    );
    let workflow = publish(&api, "rest-permit-step", doc)
        .await
        .expect("published");
    let _holding = start(&api, &workflow, CANARY).await;
    let waiting = start(&api, &workflow, CANARY).await;
    let (status, graph) = read(&api, &format!("/runs/{waiting}/graph")).await;
    assert_eq!(status, StatusCode::OK, "{graph}");
    conforms("/runs/{runId}/graph", &graph);
    let node = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == "g")
        .unwrap_or_else(|| panic!("node g: {graph}"));
    assert_eq!(
        (node["status"].as_str(), node["waitingOn"].as_str()),
        (Some("WAITING"), Some("concurrency")),
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
    conforms("/namespaces/{namespaceId}/rate-limit-keys", &page);
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

    // a member without the reveal grant: the group, summed over its windows, no key (decision D022-086)
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
    assert_eq!(hidden["view"], "GROUPS", "{hidden}");
    assert_eq!(hidden["items"], json!([]), "{hidden}");
    let group = &hidden["groups"][0];
    assert!(group.get("keyLabel").is_none(), "{group}");
    // whole, the earliest of its windows' (decision D022-061 (b))
    assert_eq!(
        group["oldestWaitSince"], waited["oldestWaitSince"],
        "{hidden}"
    );
    // neither when its next slot frees nor how many windows it has (batch 4 reviews, security Major 1 and minor 1)
    assert!(
        group.get("nextSlotAt").is_none() && group.get("keys").is_none(),
        "{hidden}"
    );
    conforms("/namespaces/{namespaceId}/rate-limit-keys", &hidden);
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

/// The `k`-th counted lease for `k` of 2 and more (review of task T022-078, correctness Major 1): with `limit: 2, per:
/// hour`, holders granted at `g1 < g2` and three waiters, the first is granted when the oldest slot ages out (`k = 2 + 0
/// - 2 + 1 = 1`: `g1` plus an hour), the second when the younger one does (`k = 2`: `g2` plus an hour, the offset 1),
/// and the third has no estimate (`k = 3` exceeds the two counted grants: beyond one window).
#[tokio::test]
async fn each_waiter_is_estimated_from_its_own_counted_lease() {
    let (api, _) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-kth",
        rated_start("rest-rate-kth", 2, "hour"),
    )
    .await
    .expect("published");
    let older = start(&api, &workflow, CANARY).await;
    let younger = start(&api, &workflow, CANARY).await;
    let waiters = [
        start(&api, &workflow, CANARY).await,
        start(&api, &workflow, CANARY).await,
        start(&api, &workflow, CANARY).await,
    ];
    let (g1, g2) = (
        granted_us(&api, &older).await,
        granted_us(&api, &younger).await,
    );
    assert!(g1 < g2, "granted in order: {g1} {g2}");
    let expected = [Some(g1 + HOUR_US), Some(g2 + HOUR_US), None];
    for (index, (run, estimate)) in waiters.iter().zip(expected).enumerate() {
        let (status, body) = read(&api, &format!("/runs/{run}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let slot = &body["rateLimit"];
        assert_eq!(slot["position"].as_u64(), Some(index as u64 + 1), "{slot}");
        assert_eq!(
            slot["estimatedGrantAt"]
                .as_str()
                .map(|_| us(&slot["estimatedGrantAt"])),
            estimate,
            "waiter {}: {slot}",
            index + 1
        );
    }
}

/// The waiters ahead are the ones the pass visits (decision D022-067 item 2; review of task T022-078, correctness Major
/// 2), and a waiter the pass will not grant has no estimate (decision D022-068 item 1, correctness minor 2). With
/// `limit: 1, per: hour` and a holder, four runs buffered in order: A's run is paused, B's run has ended with its waiter
/// row left behind (an orphan the pass skips), C is past its own one-second deadline and not yet timed out (no pass runs
/// here). D, the last, is next (position 1, each of the three filters alone would make it 2) with the holder's grant
/// plus an hour. A is at position 1 with no estimate (paused); C at position 1 with none (its deadline has passed).
#[tokio::test]
async fn the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate() {
    let (api, _) = api().await;
    let long = publish(
        &api,
        "rest-rate-skip",
        rated_start("rest-rate-skip", 1, "hour"),
    )
    .await
    .expect("published");
    let short = publish(
        &api,
        "rest-rate-skip-short",
        rated_start_waiting("rest-rate-skip-short", 1, "hour", 1_000),
    )
    .await
    .expect("published");
    let holder = start(&api, &long, CANARY).await;
    let a = start(&api, &long, CANARY).await;
    let b = start(&api, &long, CANARY).await;
    let c = start(&api, &short, CANARY).await;
    let d = start(&api, &long, CANARY).await;
    operate(&api, &a, "PAUSE").await;
    sqlx::query(
        "UPDATE runs SET status = 'CANCELLED', finished_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&b)
    .execute(&api.db.pool)
    .await
    .expect("an ended run whose waiter row stays");
    let (_, c_deadline) = waiter_us(&api, &c).await;
    until_past(&api.db.pool, c_deadline).await;
    assert_eq!(
        waiter_us(&api, &c).await.1,
        c_deadline,
        "still buffered: no timeout routine runs here"
    );

    let (_, body) = read(&api, &format!("/runs/{d}")).await;
    let slot = &body["rateLimit"];
    assert_eq!(
        slot["position"], 1,
        "paused, ended and timed-out waiters are not ahead: {slot}"
    );
    assert_eq!(
        us(&slot["estimatedGrantAt"]),
        granted_us(&api, &holder).await + HOUR_US,
        "{slot}"
    );
    let (_, body) = read(&api, &format!("/runs/{a}")).await;
    let slot = &body["rateLimit"];
    assert_eq!(
        (slot["state"].as_str(), slot["position"].as_u64()),
        (Some("WAITING"), Some(1)),
        "{slot}"
    );
    assert!(slot["estimatedGrantAt"].is_null(), "a paused run: {slot}");
    let (_, body) = read(&api, &format!("/runs/{c}")).await;
    let slot = &body["rateLimit"];
    assert_eq!(slot["position"], 1, "{slot}");
    assert!(
        slot["estimatedGrantAt"].is_null(),
        "past its deadline: {slot}"
    );
}

/// Decision D022-068 item 1: an estimate at or after the run's own queue deadline is withheld, since the run times out
/// first. With `limit: 1, per: hour` and a ten-minute queue, the waiter would be granted an hour after the holder.
#[tokio::test]
async fn an_estimate_after_the_queue_deadline_is_withheld() {
    let (api, _) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-deadline",
        rated_start_waiting("rest-rate-deadline", 1, "hour", 600_000),
    )
    .await
    .expect("published");
    let holder = start(&api, &workflow, CANARY).await;
    let waiter = start(&api, &workflow, CANARY).await;
    let (_, deadline) = waiter_us(&api, &waiter).await;
    assert!(
        granted_us(&api, &holder).await + HOUR_US > deadline,
        "the slot frees after the deadline"
    );
    let (_, body) = read(&api, &format!("/runs/{waiter}")).await;
    let slot = &body["rateLimit"];
    assert_eq!(us(&slot["queueDeadline"]), deadline, "{slot}");
    assert_eq!(slot["position"], 1, "{slot}");
    assert!(slot["estimatedGrantAt"].is_null(), "{slot}");
}

/// Decision D022-014 (review of task T022-078, architecture Major 5 and correctness minor 1): a run that waits for both
/// its concurrency permit and its rate-limit slot is granted both or neither, so its place is shown and no estimate.
#[tokio::test]
async fn a_run_waiting_for_both_gates_has_a_place_and_no_estimate() {
    let (api, _) = api().await;
    let mut doc = rated_start("rest-rate-dual", 1, "hour");
    // the same two hours for the permit: a run's gates share the earlier deadline, and the default ten minutes would
    // withhold the estimate by the deadline rule instead of the rule under test (hand mutation of the re-review)
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1,
                                "queueTimeoutMs": 7_200_000});
    let workflow = publish(&api, "rest-rate-dual", doc)
        .await
        .expect("published");
    let _holder = start(&api, &workflow, CANARY).await;
    let waiter = start(&api, &workflow, CANARY).await;
    let (status, body) = read(&api, &format!("/runs/{waiter}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    conforms("/runs/{runId}", &body);
    assert_eq!(body["concurrency"]["state"], "WAITING", "{body}");
    let slot = &body["rateLimit"];
    assert_eq!(
        (slot["state"].as_str(), slot["position"].as_u64()),
        (Some("WAITING"), Some(1)),
        "{slot}"
    );
    assert!(
        slot["estimatedGrantAt"].is_null(),
        "granted both or neither: {slot}"
    );
}

/// Decision D022-068 item 2 (review of task T022-078, architecture minor 5, security minor 3): the place is read in its
/// own transaction with a statement timeout, and a read that does not finish in time leaves the run shown without a
/// place instead of failing it. The test holds the reveal grants' table so the read stops between the run's row and its
/// place (seen waiting in `pg_stat_activity`), then holds the grants' table and lets it go on: the place's statement
/// waits for that lock until its own timeout ends it. Once released, the same read shows the place again.
#[tokio::test]
async fn a_place_that_cannot_be_read_in_time_is_left_out_and_the_run_is_shown() {
    let (api, _) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-slow",
        rated_start("rest-rate-slow", 1, "hour"),
    )
    .await
    .expect("published");
    let _holder = start(&api, &workflow, CANARY).await;
    let waiter = start(&api, &workflow, CANARY).await;
    let path = format!("/runs/{waiter}");
    let pool = &api.db.pool;
    let mut reveal = pool.begin().await.expect("a transaction");
    sqlx::query("LOCK TABLE payload_reveal_grants IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *reveal)
        .await
        .expect("the reveal grants held");
    let waiting_on = |table: &'static str| async move {
        crate::support::bounded::within(
            format!("the read waiting on {table}"),
            std::time::Duration::from_secs(30),
            async {
                loop {
                    let waiting: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                           AND wait_event_type = 'Lock' AND state = 'active' AND query LIKE '%' || $1 || '%' \
                           AND pid <> pg_backend_pid()",
                    )
                    .bind(table)
                    .fetch_one(pool)
                    .await
                    .expect("activity");
                    if waiting > 0 {
                        return;
                    }
                    tokio::task::yield_now().await;
                }
            },
        )
        .await;
    };
    let control = async {
        waiting_on("payload_reveal_grants").await;
        let mut grants = pool.begin().await.expect("a transaction");
        sqlx::query("LOCK TABLE rate_limit_grants IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *grants)
            .await
            .expect("the grants held");
        reveal.commit().await.expect("the reveal grants released");
        (grants, std::time::Instant::now())
    };
    let ((status, body), (grants, released)) = crate::support::bounded::within(
        "the read of a run whose place is held",
        std::time::Duration::from_secs(60),
        async { tokio::join!(read(&api, &path), control) },
    )
    .await;
    // the place's own two seconds, not the test pool's ten-second statement timeout (re-review, architecture minor 3)
    let waited = released.elapsed();
    grants.rollback().await.expect("the grants released");
    assert!(
        waited < std::time::Duration::from_secs(7),
        "the place's own timeout ended its wait: {waited:?}"
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    let slot = &body["rateLimit"];
    assert_eq!(slot["state"], "WAITING", "{slot}");
    assert!(
        slot["position"].is_null() && slot["estimatedGrantAt"].is_null(),
        "no place: {slot}"
    );
    let (_, body) = read(&api, &path).await;
    assert_eq!(body["rateLimit"]["position"], 1, "{body}");
}

/// Plan "Visibility of the rate gate", decision D022-068 item 3 (review of task T022-078, architecture Majors 1 and 2):
/// a page of the rate-limit listing reads its own keys through the index of its order and stops, whatever the
/// namespace's keys, the purge's backlog or a key's queue. The planner, asked about the very statements the store runs
/// on a namespace of 5,000 windows, one step key with 600 waiters and 600 aged grants (more than the 256 waiters the bound
/// reads, and grants an hour older than their window) and another with 5,000 waiters, walks `rate_limit_windows_listing_idx` (or the group's)
/// without a sort and reads exactly the page's rows; no grant probe reads more than one row and no waiter scan more than
/// 256. The busy key's next slot is its one counted grant's lease plus the hour.
#[tokio::test]
async fn a_page_of_rate_keys_reads_only_its_own_keys() {
    let (api, namespace) = api().await;
    let pool = &api.db.pool;
    let plain = publish(
        &api,
        "rest-rate-cost",
        crate::support::engine::document(
            "rest-rate-cost",
            "g",
            vec![crate::support::engine::activity("g")],
        ),
    )
    .await
    .expect("published");
    let run = start(&api, &plain, "acct-1").await;
    let tenant: Uuid = sqlx::query_scalar("SELECT tenant_id FROM namespaces WHERE id = $1::uuid")
        .bind(&namespace)
        .fetch_one(pool)
        .await
        .expect("tenant");
    let namespace_id = Uuid::parse_str(&namespace).expect("uuid");
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
                                        key_version, key_shard, updated_at) \
         SELECT $1, $2, 'ORDINARY', 'RUN', CASE WHEN i % 2 = 0 THEN 'accounts' ELSE 'regions' END, \
                sha256(int4send(i)), (SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'), i % 64, \
                clock_timestamp() \
           FROM generate_series(1, 5000) i \
         UNION ALL \
         SELECT $1, $2, 'ORDINARY', 'STEP', 'provider', decode(repeat(d, 32), 'hex'), \
                (SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'), 7, clock_timestamp() \
           FROM (VALUES ('77'), ('78'), ('79')) k(d)",
    )
    .bind(tenant)
    .bind(namespace_id)
    .execute(pool)
    .await
    .expect("5,001 windows");
    let counted: OffsetDateTime = sqlx::query_scalar(
        "WITH aged AS ( \
           INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
                                         key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
           SELECT $1, $2, $3::uuid, 'n' || i, 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('77', 32), 'hex'), 1, \
                  3600000, now() - interval '2 hours', now() - interval '2 hours', 7 \
             FROM generate_series(1, 600) i RETURNING 1), \
         queued AS ( \
           INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                                          group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, \
                                          shard) \
           SELECT $1, $2, $3::uuid, 'n' || i, 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('77', 32), 'hex'), 1, \
                  3600000, now() - make_interval(secs => i), now() + interval '1 hour', 7 \
             FROM generate_series(1, 600) i RETURNING 1), \
         elsewhere AS ( \
           INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                                          group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, \
                                          shard) \
           SELECT $1, $2, $3::uuid, 'o' || i, 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('78', 32), 'hex'), 1, \
                  3600000, now() - make_interval(secs => i / 1000.0), now() + interval '1 hour', 7 \
             FROM generate_series(1, 5000) i RETURNING 1), \
         periods AS ( \
           INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                                          group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, \
                                          shard) \
           SELECT $1, $2, $3::uuid, 'p' || p, 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('79', 32), 'hex'), 1, \
                  p, now() - make_interval(secs => CASE p WHEN 3600000 THEN 10 ELSE 9 END), now() + interval '1 hour', 7 \
             FROM (VALUES (3600000), (60000)) v(p) RETURNING 1), \
         recent AS ( \
           INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                                         group_name, key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
           SELECT $1, $2, $3::uuid, 'm79', 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('79', 32), 'hex'), 1, \
                  60000, now() - interval '30 seconds', now() - interval '30 seconds', 7 \
           UNION ALL \
           SELECT $1, $2, $3::uuid, 'm79-older', 1, 'ORDINARY', 'STEP', 'provider', \
                  decode(repeat('79', 32), 'hex'), 1, 3600000, now() - interval '30 minutes', \
                  now() - interval '30 minutes', 7 RETURNING 1) \
         INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
                                       key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
         SELECT $1, $2, $3::uuid, 'counted', 1, 'ORDINARY', 'STEP', 'provider', decode(repeat('77', 32), 'hex'), 1, \
                3600000, now() - interval '1 minute', now() - interval '1 minute', 7 \
          WHERE (SELECT count(*) FROM aged) = 600 AND (SELECT count(*) FROM queued) = 600 \
            AND (SELECT count(*) FROM elsewhere) = 5000 \
            AND (SELECT count(*) FROM periods) = 2 AND (SELECT count(*) FROM recent) = 2 \
         RETURNING leased_at",
    )
    .bind(tenant)
    .bind(namespace_id)
    .bind(&run)
    .fetch_one(pool)
    .await
    .expect("a busy step key");
    for table in [
        "rate_limit_windows",
        "rate_limit_grants",
        "rate_limit_waiters",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .expect("analyze");
    }
    let (_, page) = read(
        &api,
        &format!("/namespaces/{namespace}/rate-limit-keys?limit=1"),
    )
    .await;
    let busy = &page["items"][0];
    assert_eq!(
        (
            busy["scope"].as_str(),
            busy["waiters"].as_u64(),
            busy["grantsInWindow"].as_u64()
        ),
        (Some("STEP"), Some(600), Some(1)),
        "{busy}"
    );
    assert_eq!(
        us(&busy["nextSlotAt"]),
        i64::try_from(counted.unix_timestamp_nanos() / 1_000).expect("us") + HOUR_US,
        "{busy}"
    );

    // a key whose waiters have two windows (an hour, a minute), a grant leased thirty seconds ago and one thirty minutes
    // ago: for the minute waiter only the recent grant counts, so the next slot is its lease plus the minute (the older
    // grant counts for the hour waiter until its lease plus the hour, later); a window read as the longest would take
    // the older grant plus the minute, an instant already past (re-review, correctness minor 2)
    let (_, three) = read(
        &api,
        &format!("/namespaces/{namespace}/rate-limit-keys?group=provider"),
    )
    .await;
    let two_windows = three["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["waiters"] == 2)
        .unwrap_or_else(|| panic!("the key of two windows: {three}"));
    let recent: OffsetDateTime =
        sqlx::query_scalar("SELECT leased_at FROM rate_limit_grants WHERE node = 'm79'")
            .fetch_one(pool)
            .await
            .expect("the recent grant");
    assert_eq!(
        us(&two_windows["nextSlotAt"]),
        i64::try_from(recent.unix_timestamp_nanos() / 1_000).expect("us") + 60 * SECOND_US,
        "{two_windows}"
    );
    let digest = [0x80_u8; 32];
    for (sql, group, after) in [
        (sagat_infra_postgres::RATE_KEY_LISTING_SQL, None, false),
        (sagat_infra_postgres::RATE_KEY_LISTING_SQL, None, true),
        (
            sagat_infra_postgres::RATE_KEY_LISTING_OF_GROUP_SQL,
            Some("accounts"),
            false,
        ),
        (
            sagat_infra_postgres::RATE_KEY_LISTING_OF_GROUP_SQL,
            Some("accounts"),
            true,
        ),
        (
            sagat_infra_postgres::RATE_KEY_LISTING_OF_GROUP_SQL,
            Some("provider"),
            false,
        ),
    ] {
        let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, FORMAT JSON) {sql}"
        )))
        .bind(tenant)
        .bind(namespace_id)
        .bind(group)
        .bind(if after { "infinity" } else { "-infinity" })
        .bind(None::<OffsetDateTime>)
        .bind(if after { "ORDINARY" } else { "" })
        .bind(if after { "RUN" } else { "" })
        .bind(if after {
            "accounts"
        } else {
            group.unwrap_or("")
        })
        .bind(if after { digest.as_slice() } else { &[][..] })
        .bind(51_i64)
        .fetch_one(pool)
        .await
        .expect("a plan");
        let text = plan.to_string();
        assert!(
            !text.contains("\"Seq Scan\""),
            "no scan of every key: {text}"
        );
        let scan = &plan[0]["Plan"]["Plans"][0];
        let index = if group.is_some() {
            "rate_limit_windows_group_listing_idx"
        } else {
            "rate_limit_windows_listing_idx"
        };
        assert_eq!(scan["Index Name"], index, "{text}");
        let rows = if group == Some("provider") { 3 } else { 51 };
        assert_eq!(
            scan["Actual Rows"], rows,
            "the page's rows, not the namespace's: {text}"
        );
        let mut nodes = Vec::new();
        collect_nodes(&plan[0]["Plan"], &mut nodes);
        for node in &nodes {
            // rows a node returns, per loop, and rows it read and dropped: a predicate the index does not serve reads the
            // key's every row and drops them (the re-review's Major); `as_f64`, since PostgreSQL 18 reports fractions
            let read = node["Actual Rows"].as_f64().unwrap_or(0.0);
            let dropped = node["Rows Removed by Filter"].as_f64().unwrap_or(0.0)
                + node["Rows Removed by Index Recheck"]
                    .as_f64()
                    .unwrap_or(0.0);
            match node["Relation Name"].as_str() {
                Some("rate_limit_grants") => {
                    assert!(
                        read <= 1.0 && dropped == 0.0,
                        "a grant probe reads one row: {node}"
                    );
                }
                Some("rate_limit_waiters") => {
                    assert!(
                        read <= 256.0 && dropped == 0.0,
                        "at most the 256 waiters a pass grants: {node}"
                    );
                }
                _ => {}
            }
        }
    }
}

/// Every node of an `EXPLAIN (FORMAT JSON)` plan, its subplans included.
fn collect_nodes<'a>(node: &'a Value, into: &mut Vec<&'a Value>) {
    into.push(node);
    for child in node["Plans"].as_array().into_iter().flatten() {
        collect_nodes(child, into);
    }
}

/// The listing of one group and its pages (review of task T022-078, correctness minor 4): `?group=` lists that group's
/// windows only, in the full listing's order within it, and pages through keys nobody waits for (ordered by pool, scope
/// and digest) one at a time, each exactly once; a cursor of one group's listing is a `400` in another group's.
#[tokio::test]
async fn one_groups_rate_keys_page_through_the_keys_nobody_waits_for() {
    let (api, namespace) = api().await;
    let accounts = publish(
        &api,
        "rest-rate-groups",
        rated_start("rest-rate-groups", 1, "hour"),
    )
    .await
    .expect("published");
    let mut doc = rated_start("rest-rate-regions", 1, "hour");
    doc["rateLimit"]["group"] = json!("regions");
    let regions = publish(&api, "rest-rate-regions", doc)
        .await
        .expect("published");
    for account in ["acct-1", "acct-1", "acct-2", "acct-3"] {
        start(&api, &accounts, account).await;
    }
    start(&api, &regions, "acct-1").await;
    let path = format!("/namespaces/{namespace}/rate-limit-keys");
    let labels = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["keyLabel"].as_str().expect("a label").to_owned())
            .collect()
    };
    let (status, all) = read(&api, &format!("{path}?group=accounts")).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    conforms("/namespaces/{namespaceId}/rate-limit-keys", &all);
    let whole = labels(&all);
    assert_eq!(whole.len(), 3, "{all}");
    assert_eq!(all["items"][0]["waiters"], 1, "oldest wait first: {all}");
    assert!(
        all["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["group"] == "accounts"),
        "{all}"
    );
    let mut paged = Vec::new();
    let mut cursor: Option<String> = None;
    let mut first_cursor = None;
    for _ in 0..3 {
        let page_path = match &cursor {
            None => format!("{path}?group=accounts&limit=1"),
            Some(c) => format!("{path}?group=accounts&limit=1&cursor={c}"),
        };
        let (status, page) = read(&api, &page_path).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        paged.extend(labels(&page));
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        first_cursor = first_cursor.or_else(|| cursor.clone());
    }
    assert_eq!(paged, whole, "each key once, in the listing's order");
    assert!(cursor.is_none(), "no page after the last key");
    let (status, regions_page) = read(&api, &format!("{path}?group=regions")).await;
    assert_eq!(status, StatusCode::OK, "{regions_page}");
    assert_eq!(labels(&regions_page).len(), 1, "{regions_page}");
    let (status, refused) = read(
        &api,
        &format!(
            "{path}?group=regions&cursor={}",
            first_cursor.expect("a cursor")
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "another group's cursor: {refused}"
    );
}

/// ADR-0093 decision 5 on the rate-limit listing (re-review of task T022-078, architecture minor 4): a cursor sealed under
/// a digest key version that the operator forced out is refused `400` although the process still holds the version,
/// while a cursor of the active version pages on.
#[tokio::test]
async fn a_rate_cursor_of_a_forced_version_is_refused() {
    let api = start_with(Options {
        flow_control: true,
        flow_control_ring: Some(crate::support::flow_control::RING_TWO),
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "flow").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    // the keys' own cursors: a caller with the reveal grant (decision D022-086)
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
    let workflow = publish(
        &api,
        "rest-rate-rotation",
        rated_start("rest-rate-rotation", 1, "hour"),
    )
    .await
    .expect("published");
    for account in ["acct-1", "acct-2", "acct-3"] {
        start(&api, &workflow, account).await;
    }
    let path = format!("/namespaces/{namespace}/rate-limit-keys?limit=1");
    let next = |page: &Value| page["nextCursor"].as_str().expect("a cursor").to_owned();
    let (status, page) = read(&api, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let under_one = next(&page);
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(2::smallint, 'operator')",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("activated");
    assert_eq!(outcome, "ACTIVATED");
    let (_, page) = read(&api, &path).await;
    let under_two = next(&page);
    let (status, body) = read(&api, &format!("{path}&cursor={under_one}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retiring version's cursor opens: {body}"
    );
    sqlx::query(
        "SELECT out_outcome FROM flow_control_key_force_retire(1::smallint, 'operator:incident', 'a leaked key')",
    )
    .execute(&api.db.pool)
    .await
    .expect("forced");
    let (status, body) = read(&api, &format!("{path}&cursor={under_one}")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a forced version's cursor: {body}"
    );
    let (status, body) = read(&api, &format!("{path}&cursor={under_two}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Decision D022-075 item 3 (review of task T022-080, Major 3): a waiting run's place and limit follow the limit its grant
/// is decided with, the lower of its filed limit and the namespace's current ceiling. With `limit: 3, per: hour`, three
/// slots counted and one run buffered behind them, the run shows limit 3 and the first counted slot's age-out
/// (`k = 3 + 0 - 3 + 1 = 1`). Once the namespace lowers its hourly ceiling to 1, the grant routine admits the run only
/// when `n + a < 1`: it shows limit 1 and the third counted slot's age-out (`k = 3 + 0 - 1 + 1 = 3`).
#[tokio::test]
async fn a_lowered_ceiling_moves_the_estimate_and_the_limit_a_waiting_run_shows() {
    let (api, namespace) = api().await;
    let workflow = publish(
        &api,
        "rest-rate-lowered",
        rated_start("rest-rate-lowered", 3, "hour"),
    )
    .await
    .expect("published");
    let mut holders = Vec::new();
    for _ in 0..3 {
        holders.push(start(&api, &workflow, CANARY).await);
    }
    let waiter = start(&api, &workflow, CANARY).await;
    let (_, before) = read(&api, &format!("/runs/{waiter}")).await;
    let slot = &before["rateLimit"];
    assert_eq!(
        (slot["state"].as_str(), slot["limit"].as_u64()),
        (Some("WAITING"), Some(3)),
        "{slot}"
    );
    assert_eq!(
        us(&slot["estimatedGrantAt"]),
        granted_us(&api, &holders[0]).await + HOUR_US
    );
    let lowered = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/quotas/rate_limit_per_hour"),
            "alice",
            "acme",
        )
        .json(&json!({"limit": 1}))
        .send()
        .await
        .expect("lower the ceiling");
    assert_eq!(lowered.status(), StatusCode::OK);
    let (_, after) = read(&api, &format!("/runs/{waiter}")).await;
    conforms("/runs/{runId}", &after);
    let slot = &after["rateLimit"];
    assert_eq!(
        (slot["position"].as_u64(), slot["limit"].as_u64()),
        (Some(1), Some(1)),
        "the limit the grant is decided with: {slot}"
    );
    assert_eq!(
        us(&slot["estimatedGrantAt"]),
        granted_us(&api, &holders[2]).await + HOUR_US,
        "the third counted slot's age-out"
    );
    // the listing shows the limit the grant is decided with too (decision D022-076 item 5; re-review minor 2)
    let (status, listed) = read(&api, "/runs").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let listed_limit = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["runId"] == waiter.as_str())
        .map(|item| item["rateLimit"]["limit"].as_u64());
    assert_eq!(listed_limit, Some(Some(1)), "{listed}");
}

/// Decision D022-076 item 5 (re-review minor 2): the place of a waiting run counts ahead only the waiters the grant
/// routine counts ahead. On a key shared by an hour's `limit: 1` and a minute's `limit: 2`, the hour's holder counts for
/// both and the hour's waiter `wa` is refused by its own limit for the hour; the minute's waiter `wb` behind it is first
/// in its key's queue (`wa` is not eligible), while a count of every live waiter ahead would place it second.
#[tokio::test]
async fn the_place_of_a_waiting_run_counts_only_the_eligible_waiters_ahead() {
    let (api, _) = api().await;
    let hourly = publish(
        &api,
        "rest-rate-place-hour",
        rated_start("rest-rate-place-hour", 1, "hour"),
    )
    .await
    .expect("published");
    let minutely = publish(
        &api,
        "rest-rate-place-minute",
        rated_start("rest-rate-place-minute", 2, "minute"),
    )
    .await
    .expect("published");
    let _holder = start(&api, &hourly, CANARY).await;
    let wa = start(&api, &hourly, CANARY).await;
    let other = start(&api, &minutely, CANARY).await;
    let wb = start(&api, &minutely, CANARY).await;
    let (_, run) = read(&api, &format!("/runs/{wb}")).await;
    let slot = &run["rateLimit"];
    assert_eq!(slot["state"].as_str(), Some("WAITING"), "{slot}");
    assert_eq!(
        slot["position"].as_u64(),
        Some(1),
        "the hour's waiter is not ahead of it: {slot}"
    );
    let (_, run) = read(&api, &format!("/runs/{wa}")).await;
    assert_eq!(run["rateLimit"]["state"].as_str(), Some("WAITING"));
    let (_, run) = read(&api, &format!("/runs/{other}")).await;
    assert_eq!(run["rateLimit"]["state"].as_str(), Some("GRANTED"), "{run}");
}
