//! The flow-control reads of the API over HTTP (T022-026; FR-022-022, D1168, D022-055): `Run.concurrency` on the
//! single-run read and on the listing, and `GET /namespaces/{namespaceId}/concurrency-keys` (holders, waiters, oldest
//! ages, queued runs, oldest wait first, exact paging). A key appears only as its 12-hex label, and only to a caller who
//! may reveal personal data in the namespace (D022-060); the raw value a run was started with is in no body, and a
//! cursor is sealed for its namespace (D022-059). A page's cost does not grow with the namespace's keys (D022-058).

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
    grant_reveal(&api, &namespace, "alice").await;
    (api, namespace, tenant)
}

/// `subject` may reveal personal data in `namespace` of `acme` (feature 009's grant, given by the admin `alice`).
async fn grant_reveal(api: &TestApi, namespace: &str, subject: &str) {
    let granted = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/payload-reveal-grants/{subject}"),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("grant");
    assert_eq!(
        granted.status(),
        StatusCode::OK,
        "{}",
        granted.text().await.unwrap_or_default()
    );
}

/// `subject` joins `tenant` with `role`.
async fn member(
    api: &TestApi,
    tenant: sagat_domain::identity::TenantId,
    subject: &str,
    role: sagat_domain::identity::Role,
) {
    api.owner_store
        .insert_membership(&sagat_domain::identity::Membership::restore(
            tenant,
            sagat_domain::identity::Subject::parse(subject).expect("subject"),
            role,
            crate::support::now(),
        ))
        .await
        .expect("membership");
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
    read_as(api, path, "alice").await
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
    // a second key nobody waits for: two keys in the last bucket, ordered by their tie-breakers (correctness m2)
    let alone_too = start(&api, &workflow, "acct-c").await;
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
    assert_eq!(items.len(), 4, "{page}");
    assert_eq!(
        items[0]["keyLabel"],
        labels[0].as_str(),
        "the oldest wait first"
    );
    let mut last_two = [
        label_of(&api, &alone).await,
        label_of(&api, &alone_too).await,
    ];
    last_two.sort();
    assert_eq!(
        [&items[2]["keyLabel"], &items[3]["keyLabel"]],
        [last_two[0].as_str(), last_two[1].as_str()],
        "nobody waits: last, by digest (whose first bytes are the label)"
    );
    for (item, waiters) in items.iter().zip([1, 1, 0, 0]) {
        assert_eq!(item["pool"], "ORDINARY");
        assert_eq!(item["scope"], "RUN");
        assert_eq!(item["group"], "accounts");
        assert_eq!(item["holders"], 1, "{item}");
        assert_eq!(item["waiters"], waiters, "{item}");
        assert!(item["oldestHolderSince"].is_string(), "{item}");
        assert_eq!(item["oldestWaitSince"].is_string(), waiters == 1, "{item}");
    }
    let instant = |item: &Value| {
        time::OffsetDateTime::parse(
            item["oldestWaitSince"].as_str().expect("an instant"),
            &time::format_description::well_known::Rfc3339,
        )
        .expect("rfc3339")
    };
    assert!(instant(&items[0]) < instant(&items[1]), "{page}");

    // one key a page: every key once, in the same order, and no cursor after the last
    let mut cursor: Option<String> = None;
    let mut paged = Vec::new();
    let mut cursors = Vec::new();
    for _ in 0..5 {
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
        cursors.extend(cursor.clone());
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none(), "the last page has no cursor");
    let all: Vec<Value> = items.iter().map(|item| item["keyLabel"].clone()).collect();
    assert_eq!(paged, all);

    assert_eq!(cursors.len(), 3);

    // D022-059: a cursor names no key: neither a label nor the digest it is cut from is in it
    for (cursor, item) in cursors.iter().zip(items) {
        let label = item["keyLabel"].as_str().expect("a label");
        assert!(!cursor.contains(label), "{cursor} carries {label}");
    }
    // a cursor altered in any one place, a forged plain position, one before PostgreSQL's first instant and one of
    // another namespace are each a 400, never a 500 (security M2 and minor 2)
    let sealed = &cursors[0];
    for at in [3, sealed.len() / 2, sealed.len() - 1] {
        let mut altered = sealed.clone().into_bytes();
        altered[at] = if altered[at] == b'0' { b'1' } else { b'0' };
        let altered = String::from_utf8(altered).expect("text");
        let (status, body) = read(&api, &format!("{path}?cursor={altered}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let digest: String = sqlx::query_scalar(
        "SELECT encode(key_digest, 'hex') FROM concurrency_holders WHERE run_id = $1::uuid",
    )
    .bind(&first_holder)
    .fetch_one(&api.db.pool)
    .await
    .expect("a digest");
    let group_hex: String = "accounts".bytes().map(|b| format!("{b:02x}")).collect();
    for forged in [
        format!("k1.n.ORDINARY.RUN.{digest}.{group_hex}"),
        format!("k1.-300000000000000000.ORDINARY.RUN.{digest}.{group_hex}"),
    ] {
        let (status, body) = read(&api, &format!("{path}?cursor={forged}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let billing = api
        .create_namespace(
            "alice",
            "acme",
            &api.create_workspace("alice", "acme", "flow-billing").await,
            "billing",
            "dev",
        )
        .await;
    let (status, body) = read(
        &api,
        &format!("/namespaces/{billing}/concurrency-keys?cursor={sealed}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "another namespace's cursor: {body}"
    );
    // a cursor resumes one group's listing only after a key of that group
    let (status, body) = read(&api, &format!("{path}?group=another&cursor={sealed}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = read(&api, &format!("{path}?group=accounts&cursor={sealed}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(3), "{body}");

    let (_, filtered) = read(&api, &format!("{path}?group=accounts")).await;
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(4));
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
    // the attempt is audited in the attacker's tenant, off the request path (keys review, architecture m5)
    crate::support::eventually("the cross-tenant denial is audited", || async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
              WHERE t.slug = 'globex' AND a.action = 'access.denied.cross_tenant' AND a.actor = 'bob'",
        )
        .fetch_one(&api.db.pool)
        .await
        .expect("audit");
        (n >= 1).then_some(())
    })
    .await;
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

/// FR-022-057, D022-060 (keys review, security M1): until T022-104 classifies what a key reads, a key's label is shown
/// only to a caller who may reveal personal data in the namespace (a role that may use a reveal grant, and the grant);
/// every other reader gets `keyLabel: null` in both reads, the rest of the permit and the key unchanged.
#[tokio::test]
async fn a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data() {
    use sagat_domain::identity::Role;
    let (api, namespace, acme) = api().await;
    member(&api, acme, "vera-viewer", Role::Viewer).await;
    member(&api, acme, "dan-developer", Role::Developer).await;
    let workflow = publish(&api, "rest-label", serial_start("rest-label", 1))
        .await
        .expect("published");
    let holder = start(&api, &workflow, CANARY).await;
    start(&api, &workflow, CANARY).await;
    let label = label_of(&api, &holder).await;
    let keys = format!("/namespaces/{namespace}/concurrency-keys");
    let labels_seen_by = |subject: &'static str| {
        let api = &api;
        let holder = holder.clone();
        let keys = keys.clone();
        async move {
            let (status, run) = read_as(api, &format!("/runs/{holder}"), subject).await;
            assert_eq!(status, StatusCode::OK, "{run}");
            assert_eq!(run["concurrency"]["state"], "HOLDING", "{run}");
            let (status, runs) = read_as(api, "/runs", subject).await;
            assert_eq!(status, StatusCode::OK, "{runs}");
            let listed = runs["items"]
                .as_array()
                .expect("items")
                .iter()
                .find(|item| item["runId"] == holder.as_str())
                .expect("listed")["concurrency"]
                .clone();
            let (status, page) = read_as(api, &keys, subject).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["items"][0]["holders"], 1, "{page}");
            // the key's instants go with its label: without it they would link the runs of one key (review of the
            // fixes, security Major 1)
            let revealed = page["items"][0]["keyLabel"].is_string();
            assert_eq!(
                page["items"][0]["oldestHolderSince"].is_string(),
                revealed,
                "{page}"
            );
            assert_eq!(
                page["items"][0]["oldestWaitSince"].is_string(),
                revealed,
                "{page}"
            );
            assert!(
                run["concurrency"].get("keyLabel").is_some(),
                "the field is there: {run}"
            );
            [
                run["concurrency"]["keyLabel"].clone(),
                listed["keyLabel"].clone(),
                page["items"][0]["keyLabel"].clone(),
            ]
        }
    };
    let hidden = [Value::Null, Value::Null, Value::Null];
    let shown = [json!(label), json!(label), json!(label)];
    assert_eq!(labels_seen_by("vera-viewer").await, hidden, "a viewer");
    assert_eq!(
        labels_seen_by("dan-developer").await,
        hidden,
        "a developer without a grant"
    );
    assert_eq!(
        labels_seen_by("alice").await,
        shown,
        "an admin with the grant"
    );
    grant_reveal(&api, &namespace, "dan-developer").await;
    assert_eq!(
        labels_seen_by("dan-developer").await,
        shown,
        "the developer once granted"
    );
    // the role half of the rule (review of the fixes, correctness m3): a grant row beside a role that may not use it
    // (demoted to viewer, the row put back as a defect would leave it) reveals nothing
    let pool = &api.db.pool;
    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'dan-developer'",
    )
    .bind(acme.as_uuid())
    .execute(pool)
    .await
    .expect("demoted");
    sqlx::query(
        "INSERT INTO payload_reveal_grants (tenant_id, namespace_id, subject, granted_by, granted_at) \
         VALUES ($1, $2::uuid, 'dan-developer', 'alice', clock_timestamp()) ON CONFLICT DO NOTHING",
    )
    .bind(acme.as_uuid())
    .bind(&namespace)
    .execute(pool)
    .await
    .expect("a grant row beside the viewer");
    assert_eq!(
        labels_seen_by("dan-developer").await,
        hidden,
        "a viewer with a grant row"
    );
}

/// Publishes `document` as `name` in namespace `namespace` of `tenant`, as its admin `admin`.
async fn publish_in(
    api: &TestApi,
    admin: &str,
    tenant: &str,
    namespace: &str,
    name: &str,
    document: Value,
) -> String {
    let created = api
        .as_user(Method::POST, "/workflows", admin, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            admin,
            tenant,
        )
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish workflow");
    assert_eq!(published.status(), StatusCode::CREATED);
    workflow
}

/// Starts `workflow` in namespace `namespace` of `tenant` as `admin` for `account`.
async fn start_in(
    api: &TestApi,
    admin: &str,
    tenant: &str,
    namespace: &str,
    workflow: &str,
    account: &str,
) {
    let started = api
        .as_user(Method::POST, "/runs", admin, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {"accountId": account}}))
        .send()
        .await
        .expect("start run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
}

/// Keys review (2026-11-20), security minor 1: the listing and its count of queued runs are of one namespace of one
/// tenant. Two namespaces of one tenant and a namespace of the same slug in another tenant each hold keys of the same
/// group and value; each listing shows its own keys and its own queued runs.
#[tokio::test]
async fn a_listing_shows_only_its_own_namespaces_keys_and_queued_runs() {
    let (api, payments, _) = api().await;
    let billing = api
        .create_namespace(
            "alice",
            "acme",
            &api.create_workspace("alice", "acme", "flow-billing").await,
            "billing",
            "dev",
        )
        .await;
    grant_reveal(&api, &billing, "alice").await;
    api.tenant("globex", "bob").await;
    let elsewhere = api
        .create_namespace(
            "bob",
            "globex",
            &api.create_workspace("bob", "globex", "flow").await,
            "payments",
            "dev",
        )
        .await;
    // payments: one key, a holder and a waiter; billing: two keys, one with two waiters; globex: three waiters
    for (admin, tenant, namespace, runs) in [
        ("alice", "acme", "payments", vec!["acct-a", "acct-a"]),
        (
            "alice",
            "acme",
            "billing",
            vec!["acct-a", "acct-a", "acct-a", "acct-b"],
        ),
        (
            "bob",
            "globex",
            "payments",
            vec!["acct-a", "acct-a", "acct-a", "acct-a"],
        ),
    ] {
        let workflow = publish_in(
            &api,
            admin,
            tenant,
            namespace,
            "rest-scope",
            serial_start("rest-scope", 1),
        )
        .await;
        for account in runs {
            start_in(&api, admin, tenant, namespace, &workflow, account).await;
        }
    }
    let listing = |namespace: &str, admin: &'static str, tenant: &'static str| {
        let path = format!("/namespaces/{namespace}/concurrency-keys");
        let api = &api;
        async move {
            let response = api.get_as(&path, admin, tenant).await;
            assert_eq!(response.status(), StatusCode::OK);
            let page = json_of(response).await;
            let shape: Vec<(i64, i64)> = page["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(|item| {
                    (
                        item["holders"].as_i64().expect("holders"),
                        item["waiters"].as_i64().expect("waiters"),
                    )
                })
                .collect();
            (page["queuedRuns"].as_i64().expect("queued runs"), shape)
        }
    };
    assert_eq!(listing(&payments, "alice", "acme").await, (1, vec![(1, 1)]));
    assert_eq!(
        listing(&billing, "alice", "acme").await,
        (2, vec![(1, 2), (1, 0)])
    );
    assert_eq!(
        listing(&elsewhere, "bob", "globex").await,
        (3, vec![(1, 3)])
    );
}

/// D022-058 (keys review, architecture M3): a page of the listing reads its own keys through the index of its order and
/// stops, whatever the number of the namespace's live keys: the planner, asked about the very statements the store runs
/// on a namespace of 5,000 keys, walks `concurrency_semaphores_listing_idx` (or the group's) without a sort and reads
/// exactly the page's rows.
#[tokio::test]
async fn a_page_of_keys_reads_only_its_own_keys() {
    let (api, namespace, tenant) = api().await;
    let pool = &api.db.pool;
    sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
                                             key_version, key_shard, updated_at) \
         SELECT $1, $2::uuid, 'ORDINARY', 'RUN', CASE WHEN i % 2 = 0 THEN 'accounts' ELSE 'regions' END, \
                sha256(int4send(i)), (SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'), i % 64, \
                clock_timestamp() \
           FROM generate_series(1, 5000) i",
    )
    .bind(tenant.as_uuid())
    .bind(&namespace)
    .execute(pool)
    .await
    .expect("5,000 keys");
    sqlx::query("ANALYZE concurrency_semaphores")
        .execute(pool)
        .await
        .expect("analyze");
    let digest = [0x80_u8; 32];
    for (sql, group, after) in [
        (sagat_infra_postgres::KEY_LISTING_SQL, None, false),
        (sagat_infra_postgres::KEY_LISTING_SQL, None, true),
        (
            sagat_infra_postgres::KEY_LISTING_OF_GROUP_SQL,
            Some("accounts"),
            false,
        ),
        (
            sagat_infra_postgres::KEY_LISTING_OF_GROUP_SQL,
            Some("accounts"),
            true,
        ),
    ] {
        let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, FORMAT JSON) {sql}"
        )))
        .bind(tenant.as_uuid())
        .bind(uuid::Uuid::parse_str(&namespace).expect("uuid"))
        .bind(group)
        .bind(if after { "infinity" } else { "-infinity" })
        .bind(None::<time::OffsetDateTime>)
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
        assert!(!text.contains("\"Sort\""), "no sort: {text}");
        assert!(
            !text.contains("\"Seq Scan\""),
            "no scan of every key: {text}"
        );
        let scan = &plan[0]["Plan"]["Plans"][0];
        let index = if group.is_some() {
            "concurrency_semaphores_group_listing_idx"
        } else {
            "concurrency_semaphores_listing_idx"
        };
        assert_eq!(scan["Index Name"], index, "{text}");
        assert_eq!(
            scan["Actual Rows"], 51,
            "the page's rows, not the namespace's: {text}"
        );
    }
}
