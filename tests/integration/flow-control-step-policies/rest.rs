//! The flow-control reads of the API over HTTP (T022-026; FR-022-022, D1168, D022-055): `Run.concurrency` on the
//! single-run read and on the listing, and `GET /namespaces/{namespaceId}/concurrency-keys` (holders, waiters, oldest
//! ages, queued runs, oldest wait first, exact paging). A key appears only as its 12-hex label: the raw value a run was
//! started with is in no body.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::run_admission::serial_start;
use crate::support::api::{Options, TestApi, json_of, start_with};
use crate::support::scope_flow::publish;

/// A raw key value no body may carry.
const CANARY: &str = "acct-canary-5517";

/// An API with flow control on, tenant `acme` (admin `alice`) and its namespace `payments`.
async fn api() -> (TestApi, String, sagat_domain::identity::TenantId) {
    let api = start_with(Options {
        flow_control: true,
        ..Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "flow").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    (api, namespace, tenant)
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

/// The body of `path` as `alice`, and its text, which is checked for the canary.
async fn read(api: &TestApi, path: &str) -> (StatusCode, Value) {
    let response = api
        .as_user(Method::GET, path, "alice", "acme")
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

/// The label of the key a run's holder or waiter row is filed under, from the row's digest (the first six bytes).
async fn label_of(api: &TestApi, run: &str) -> String {
    sqlx::query_scalar(
        "SELECT encode(substr(key_digest, 1, 6), 'hex') FROM ( \
             SELECT key_digest FROM concurrency_holders WHERE run_id = $1::uuid AND holder_scope = 'RUN' \
             UNION ALL SELECT key_digest FROM concurrency_waiters WHERE run_id = $1::uuid AND permit_scope = 'RUN') k",
    )
    .bind(run)
    .fetch_one(&api.db.pool)
    .await
    .expect("the run's key row")
}

/// FR-022-022: a run shows the permit it holds (`HOLDING`, no deadline) or waits for (`WAITING`, with its queue
/// deadline), in the single-run read and in the listing, by the label of its key only; a run of a definition without
/// `concurrency` shows none.
#[tokio::test]
async fn a_run_shows_the_permit_it_holds_or_waits_for_by_its_label_only() {
    let (api, _, _) = api().await;
    let gated = publish(&api, "rest-run-permit", serial_start("rest-run-permit", 1))
        .await
        .expect("published");
    let plain = publish(
        &api,
        "rest-run-plain",
        crate::support::engine::document(
            "rest-run-plain",
            "g",
            vec![crate::support::engine::activity("g")],
        ),
    )
    .await
    .expect("published");
    let holder = start(&api, &gated, CANARY).await;
    let waiter = start(&api, &gated, CANARY).await;
    let other = start(&api, &plain, CANARY).await;
    let label = label_of(&api, &holder).await;
    assert_eq!(label_of(&api, &waiter).await, label, "one key");

    let (status, held) = read(&api, &format!("/runs/{holder}")).await;
    assert_eq!(status, StatusCode::OK, "{held}");
    let permit = &held["concurrency"];
    assert_eq!(permit["state"], "HOLDING", "{held}");
    assert_eq!(permit["pool"], "ORDINARY");
    assert_eq!(permit["group"], "accounts");
    assert_eq!(permit["keyLabel"], label.as_str());
    assert!(permit["since"].is_string());
    assert!(permit["queueDeadline"].is_null(), "{permit}");

    let (_, waiting) = read(&api, &format!("/runs/{waiter}")).await;
    let permit = &waiting["concurrency"];
    assert_eq!(permit["state"], "WAITING", "{waiting}");
    assert_eq!(permit["keyLabel"], label.as_str());
    let stored: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM deadline_at) * 1000000)::bigint FROM concurrency_waiters \
          WHERE run_id = $1::uuid",
    )
    .bind(&waiter)
    .fetch_one(&api.db.pool)
    .await
    .expect("the waiter's deadline");
    let shown = time::OffsetDateTime::parse(
        permit["queueDeadline"].as_str().expect("a queue deadline"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("rfc3339");
    assert_eq!(
        i64::try_from(shown.unix_timestamp_nanos() / 1_000).expect("microseconds"),
        stored,
        "the waiter row's deadline, to the microsecond"
    );

    let (_, plain_run) = read(&api, &format!("/runs/{other}")).await;
    assert!(
        plain_run.get("concurrency").is_none(),
        "no permit: {plain_run}"
    );

    let (status, listed) = read(&api, "/runs").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let items = listed["items"].as_array().expect("items");
    let of = |run: &str| {
        items
            .iter()
            .find(|item| item["runId"] == run)
            .unwrap_or_else(|| panic!("{run} listed"))
            .get("concurrency")
            .cloned()
    };
    assert_eq!(of(&holder).expect("a permit")["state"], "HOLDING");
    assert_eq!(of(&waiter).expect("a permit")["state"], "WAITING");
    assert_eq!(of(&other), None);
}

/// FR-022-022: the keys of a namespace, oldest wait first and keys nobody waits for last, each with its holders,
/// waiters and oldest ages, the namespace's queued runs in the page header; pages of one key resume exactly after the
/// last key shown (D022-055), the group filter narrows the keys and not the count of queued runs.
#[tokio::test]
async fn the_namespace_lists_its_keys_oldest_wait_first_and_pages_them_exactly() {
    let (api, namespace, _) = api().await;
    let workflow = publish(&api, "rest-keys", serial_start("rest-keys", 1))
        .await
        .expect("published");
    let first_holder = start(&api, &workflow, "acct-a").await;
    let first_waiter = start(&api, &workflow, "acct-a").await;
    start(&api, &workflow, "acct-b").await;
    start(&api, &workflow, "acct-b").await;
    let alone = start(&api, &workflow, CANARY).await;
    let labels = [
        label_of(&api, &first_holder).await,
        label_of(&api, &first_waiter).await,
    ];
    assert_eq!(labels[0], labels[1]);
    let path = format!("/namespaces/{namespace}/concurrency-keys");

    let (status, page) = read(&api, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["queuedRuns"], 2, "{page}");
    assert!(page["nextCursor"].is_null());
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 3, "{page}");
    assert_eq!(
        items[0]["keyLabel"],
        labels[0].as_str(),
        "the oldest wait first"
    );
    assert_eq!(
        items[2]["keyLabel"],
        label_of(&api, &alone).await.as_str(),
        "nobody waits: last"
    );
    for (item, waiters) in items.iter().zip([1, 1, 0]) {
        assert_eq!(item["pool"], "ORDINARY");
        assert_eq!(item["scope"], "RUN");
        assert_eq!(item["group"], "accounts");
        assert_eq!(item["holders"], 1, "{item}");
        assert_eq!(item["waiters"], waiters, "{item}");
        assert!(item["oldestHolderSince"].is_string(), "{item}");
        assert_eq!(item["oldestWaitSince"].is_string(), waiters == 1, "{item}");
    }
    assert!(
        items[0]["oldestWaitSince"].as_str() < items[1]["oldestWaitSince"].as_str(),
        "{page}"
    );

    // one key a page: every key once, in the same order, and no cursor after the last
    let mut cursor: Option<String> = None;
    let mut paged = Vec::new();
    for _ in 0..4 {
        let next = match &cursor {
            Some(cursor) => format!("{path}?limit=1&cursor={cursor}"),
            None => format!("{path}?limit=1"),
        };
        let (status, page) = read(&api, &next).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let shown = page["items"].as_array().expect("items");
        assert!(shown.len() <= 1);
        paged.extend(shown.iter().map(|item| item["keyLabel"].clone()));
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none(), "the last page has no cursor");
    let all: Vec<Value> = items.iter().map(|item| item["keyLabel"].clone()).collect();
    assert_eq!(paged, all);

    let (_, filtered) = read(&api, &format!("{path}?group=accounts")).await;
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(3));
    let (_, other) = read(&api, &format!("{path}?group=another")).await;
    assert_eq!(other["items"], json!([]));
    assert_eq!(other["queuedRuns"], 2, "the count is the namespace's");
    let (_, derived) = read(&api, &format!("{path}?group=%3Drest-keys")).await;
    assert_eq!(derived["items"], json!([]), "a derived group of no key");

    let (status, _) = read(&api, &format!("{path}?group=Not%20A%20Group")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = read(&api, &format!("{path}?cursor=k1.n.ORDINARY.RUN.00.00")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The listing needs `ops:read` in the caller's own tenant: another tenant's namespace is a `404` like one that does not
/// exist, and a worker identity (which holds no `ops:read`) is refused.
#[tokio::test]
async fn the_keys_of_a_namespace_are_its_tenants_and_need_ops_read() {
    use sagat_domain::identity::{Membership, Role, Subject};
    let (api, namespace, acme) = api().await;
    let path = format!("/namespaces/{namespace}/concurrency-keys");
    let tenant = api.tenant("globex", "bob").await;
    let foreign = api.get_as(&path, "bob", "globex").await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let unknown = api
        .get_as(
            &format!("/namespaces/{}/concurrency-keys", Uuid::now_v7()),
            "alice",
            "acme",
        )
        .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_ne!(acme, tenant);
    api.owner_store
        .insert_membership(&Membership::restore(
            acme,
            Subject::parse("rest-worker").expect("subject"),
            Role::Worker,
            crate::support::now(),
        ))
        .await
        .expect("worker membership");
    let worker = api.get_as(&path, "rest-worker", "acme").await;
    assert_eq!(worker.status(), StatusCode::FORBIDDEN);
}
