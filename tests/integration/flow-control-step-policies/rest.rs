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

/// FR-022-057, D022-060 (keys review, security Major 1): a key over an unclassified field reads `pii` (decision D022-085), so
/// its label is shown only to a caller who may reveal personal data in the namespace (a role that may use a reveal
/// grant, and the grant); every other reader gets `keyLabel: null` in both run reads, and the key's group instead of the
/// key in the listing (`view: GROUPS`, decision D022-086), its counts and whole instants those of the one key.
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
    let waiter = start(&api, &workflow, CANARY).await;
    let label = label_of(&api, &holder).await;
    let keys = format!("/namespaces/{namespace}/concurrency-keys");
    let labels_seen_by = |subject: &'static str| {
        let api = &api;
        let holder = holder.clone();
        let waiter = waiter.clone();
        let keys = keys.clone();
        async move {
            let instant = |text: &Value| {
                time::OffsetDateTime::parse(
                    text.as_str().expect("an instant"),
                    &time::format_description::well_known::Rfc3339,
                )
                .expect("rfc3339")
            };
            let (status, waiting) = read_as(api, &format!("/runs/{waiter}"), subject).await;
            assert_eq!(status, StatusCode::OK, "{waiting}");
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
            // a caller who may see labels reads the key; any other reads its group, never the key (decision D022-086)
            let revealed = page["view"] == "KEYS";
            let row = if revealed {
                assert_eq!(page["groups"], json!([]), "{page}");
                page["items"][0].clone()
            } else {
                assert_eq!(page["view"], "GROUPS", "{page}");
                assert_eq!(page["items"], json!([]), "{page}");
                assert!(page["groups"][0].get("keys").is_none(), "{page}");
                page["groups"][0].clone()
            };
            assert_eq!(row["holders"], 1, "{page}");
            assert_eq!(row["waiters"], 1, "{page}");
            // whole in both views: a key's to a caller who may see its label, a group's to any other (D022-061 (b))
            for (field, since) in [
                ("oldestHolderSince", &run["concurrency"]["since"]),
                ("oldestWaitSince", &waiting["concurrency"]["since"]),
            ] {
                assert_eq!(instant(&row[field]), instant(since), "{field}: {page}");
            }
            assert!(
                run["concurrency"].get("keyLabel").is_some(),
                "the field is there: {run}"
            );
            [
                run["concurrency"]["keyLabel"].clone(),
                listed["keyLabel"].clone(),
                row.get("keyLabel").cloned().unwrap_or(Value::Null),
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

/// FR-022-057 (task T022-104, decision D022-085): a key's label in `Run.concurrency` and `Run.rateLimit` is shown to a
/// caller who may disclose, at its level, every location the key reads: a key over `public` fields to every reader,
/// over `internal` ones to a role that reads internal payloads (a developer, not a viewer), over `pii` ones only to a
/// caller with the reveal grant; a key over `public` and `pii` fields is `pii`. The key listings keep their labels for
/// the reveal grant (task T022-111 lists groups to every other caller).
#[tokio::test]
async fn a_key_label_is_shown_to_whoever_may_disclose_every_location_the_key_reads() {
    use sagat_domain::identity::Role;
    let (api, namespace, acme) = api().await;
    member(&api, acme, "vera-viewer", Role::Viewer).await;
    member(&api, acme, "dan-developer", Role::Developer).await;
    let schema = json!({
        "type": "object",
        "x-sagat-classification": "public",
        "properties": {
            "accountId": {"type": "string"},
            "tier": {"type": "string"},
            "team": {"type": "string", "x-sagat-classification": "internal"},
            "email": {"type": "string", "x-sagat-classification": "pii"}
        }
    });
    let mut runs = Vec::new();
    for (level, key) in [
        ("public", "input.tier"),
        ("internal", "input.team"),
        ("pii", "input.tier + input.email"),
    ] {
        let name = format!("rest-level-{level}");
        let mut document = serial_start(&name, 1);
        document["inputSchema"] = schema.clone();
        document["concurrency"] =
            json!({"group": format!("level-{level}"), "key": key, "limit": 1});
        document["rateLimit"] =
            json!({"group": format!("level-{level}"), "key": key, "limit": 1, "per": "hour"});
        let workflow = publish(&api, &name, document).await.expect("published");
        let started = api
            .as_user(Method::POST, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId": workflow, "input": {
                "accountId": CANARY, "tier": "gold", "team": "billing", "email": "ana@example.com"}}))
            .send()
            .await
            .expect("start run");
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let run = json_of(started).await["runId"]
            .as_str()
            .expect("run id")
            .to_owned();
        let rate_label: String = sqlx::query_scalar(
            "SELECT encode(substr(key_digest, 1, 6), 'hex') FROM rate_limit_grants \
              WHERE run_id = $1::uuid AND node = ''",
        )
        .bind(&run)
        .fetch_one(&api.db.pool)
        .await
        .expect("the run's grant row");
        runs.push((level, run.clone(), (label_of(&api, &run).await, rate_label)));
    }
    let keys = format!("/namespaces/{namespace}/concurrency-keys");
    let rate_keys = format!("/namespaces/{namespace}/rate-limit-keys");
    let seen_by = |subject: &'static str| {
        let api = &api;
        let runs = runs.clone();
        let (keys, rate_keys) = (keys.clone(), rate_keys.clone());
        async move {
            let (status, listing) = read_as(api, "/runs", subject).await;
            assert_eq!(status, StatusCode::OK, "{listing}");
            let mut seen = Vec::new();
            for (level, run, label) in &runs {
                let (status, body) = read_as(api, &format!("/runs/{run}"), subject).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                assert_eq!(body["concurrency"]["state"], "HOLDING", "{body}");
                assert_eq!(body["rateLimit"]["state"], "GRANTED", "{body}");
                let listed = listing["items"]
                    .as_array()
                    .expect("items")
                    .iter()
                    .find(|item| item["runId"] == run.as_str())
                    .expect("listed")
                    .clone();
                let shown = |value: &Value, label: &String| {
                    if value.is_null() {
                        false
                    } else {
                        assert_eq!(value, &json!(label), "{level}");
                        true
                    }
                };
                let reads = [
                    shown(&body["concurrency"]["keyLabel"], &label.0),
                    shown(&body["rateLimit"]["keyLabel"], &label.1),
                    shown(&listed["concurrency"]["keyLabel"], &label.0),
                    shown(&listed["rateLimit"]["keyLabel"], &label.1),
                ];
                assert!(
                    reads.iter().all(|shown| *shown == reads[0]),
                    "{level}: one rule for the four reads {reads:?}"
                );
                seen.push((*level, reads[0]));
            }
            let mut listed = Vec::new();
            let mut views = Vec::new();
            for path in [&keys, &rate_keys] {
                let (status, page) = read_as(api, path, subject).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                views.push((
                    page["view"].as_str().unwrap_or_default().to_owned(),
                    page["items"].as_array().map_or(usize::MAX, Vec::len),
                ));
                listed.extend(
                    page["items"]
                        .as_array()
                        .expect("items")
                        .iter()
                        .map(|item| item["keyLabel"].is_string()),
                );
            }
            (seen, listed, views)
        }
    };
    // a caller who may not see a key's labels gets the group view, whose key list is empty: the two checks of the key
    // rows below would pass on that empty list alone (batch 4 correctness minor 6)
    let groups_only = vec![("GROUPS".to_owned(), 0), ("GROUPS".to_owned(), 0)];
    let (viewer, listed, views) = seen_by("vera-viewer").await;
    assert_eq!(views, groups_only, "a viewer lists groups");
    assert_eq!(
        viewer,
        [("public", true), ("internal", false), ("pii", false)],
        "a viewer reads public payloads only"
    );
    assert!(listed.iter().all(|shown| !shown), "{listed:?}");
    let (developer, listed, views) = seen_by("dan-developer").await;
    assert_eq!(
        views, groups_only,
        "a developer without a grant lists groups"
    );
    assert_eq!(
        developer,
        [("public", true), ("internal", true), ("pii", false)],
        "a developer reads internal payloads, not personal data without a grant"
    );
    assert!(listed.iter().all(|shown| !shown), "{listed:?}");
    let (admin, listed, views) = seen_by("alice").await;
    assert!(views.iter().all(|(view, _)| view == "KEYS"), "{views:?}");
    assert_eq!(
        admin,
        [("public", true), ("internal", true), ("pii", true)],
        "an admin with the reveal grant"
    );
    assert!(
        !listed.is_empty() && listed.iter().all(|shown| *shown),
        "{listed:?}"
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
            // keys to a caller with the reveal grant, the group of the keys to any other (decision D022-086)
            let rows = if page["view"] == "GROUPS" {
                "groups"
            } else {
                "items"
            };
            let shape: Vec<(i64, i64)> = page[rows]
                .as_array()
                .expect("rows")
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

/// Task T022-111 (decisions D022-061 (b), D022-086): a caller without the reveal right reads one row per pool, scope and
/// group, its keys summed with whole instants, in both listings, paged by a sealed cursor and narrowed by `group`; a
/// caller with it reads the keys by label. No body a viewer reads carries a key's label, digest or value (the canary);
/// a cursor of one view is a `400` in the other.
#[tokio::test]
async fn a_caller_without_the_reveal_right_reads_one_row_per_group_and_pages_them() {
    use sagat_domain::identity::Role;
    let (api, namespace, acme) = api().await;
    member(&api, acme, "vera-viewer", Role::Viewer).await;
    let accounts = publish(&api, "rest-groups-a", serial_start("rest-groups-a", 1))
        .await
        .expect("published");
    let mut regions_document = serial_start("rest-groups-r", 1);
    regions_document["concurrency"]["group"] = json!("regions");
    let regions = publish(&api, "rest-groups-r", regions_document)
        .await
        .expect("published");
    let mut rated = serial_start("rest-groups-rate", 1);
    rated
        .as_object_mut()
        .expect("a document")
        .remove("concurrency");
    rated["rateLimit"] =
        json!({"group": "providers", "key": "input.accountId", "limit": 1, "per": "hour"});
    let rated = publish(&api, "rest-groups-rate", rated)
        .await
        .expect("published");
    let a_holder = start(&api, &accounts, CANARY).await;
    let a_waiter = start(&api, &accounts, CANARY).await;
    let b_holder = start(&api, &accounts, "acct-other-2291").await;
    let r_holder = start(&api, &regions, CANARY).await;
    let rate_holder = start(&api, &rated, CANARY).await;
    let rate_waiter = start(&api, &rated, CANARY).await;
    let mut labels = Vec::new();
    for run in [&a_holder, &b_holder, &r_holder] {
        labels.push(label_of(&api, run).await);
    }
    let digests: Vec<String> = sqlx::query_scalar(
        "SELECT encode(key_digest, 'hex') FROM concurrency_semaphores \
         UNION ALL SELECT encode(key_digest, 'hex') FROM rate_limit_windows",
    )
    .fetch_all(&api.db.pool)
    .await
    .expect("the digests");
    labels.extend(digests.iter().map(|digest| digest[..12].to_owned()));
    let since = |body: &Value, gate: &str| body[gate]["since"].as_str().expect("since").to_owned();
    let run = |id: &String| {
        let api = &api;
        let id = id.clone();
        async move { read(api, &format!("/runs/{id}")).await.1 }
    };
    let instant = |text: &Value| {
        time::OffsetDateTime::parse(
            text.as_str().expect("an instant"),
            &time::format_description::well_known::Rfc3339,
        )
        .expect("rfc3339")
    };
    // what the viewer reads, checked for every label and digest of the namespace's keys
    let as_viewer = |path: String| {
        let api = &api;
        let labels = labels.clone();
        async move {
            let response = api
                .as_user(Method::GET, &path, "vera-viewer", "acme")
                .header("x-octopus-namespace", "payments")
                .send()
                .await
                .expect("read");
            let status = response.status();
            let text = response.text().await.expect("body");
            assert!(!text.contains(CANARY), "a raw key value in {path}: {text}");
            for label in &labels {
                assert!(
                    !text.contains(label.as_str()),
                    "a key's label or digest in {path}: {text}"
                );
            }
            assert!(
                !text.contains("keyLabel"),
                "no key row reaches a viewer: {text}"
            );
            (status, serde_json::from_str::<Value>(&text).expect("json"))
        }
    };

    let keys = format!("/namespaces/{namespace}/concurrency-keys");
    let (status, first) = as_viewer(format!("{keys}?limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["view"], "GROUPS", "{first}");
    assert_eq!(first["items"], json!([]), "{first}");
    assert_eq!(
        first["queuedRuns"], 2,
        "the accounts waiter and the rate waiter: {first}"
    );
    let group = &first["groups"][0];
    assert_eq!(
        (&group["pool"], &group["scope"], &group["group"]),
        (&json!("ORDINARY"), &json!("RUN"), &json!("accounts")),
        "{first}"
    );
    assert_eq!(
        (&group["holders"], &group["waiters"]),
        (&json!(2), &json!(1))
    );
    assert!(
        group.get("keys").is_none(),
        "no count of keys reaches a viewer: {first}"
    );
    let (holder_a, holder_b, waiter) = (
        run(&a_holder).await,
        run(&b_holder).await,
        run(&a_waiter).await,
    );
    let oldest_holder = instant(&json!(since(&holder_a, "concurrency")))
        .min(instant(&json!(since(&holder_b, "concurrency"))));
    assert_eq!(
        instant(&group["oldestHolderSince"]),
        oldest_holder,
        "whole: {first}"
    );
    assert_eq!(
        instant(&group["oldestWaitSince"]),
        instant(&json!(since(&waiter, "concurrency"))),
        "whole: {first}"
    );
    let cursor = first["nextCursor"]
        .as_str()
        .expect("a next page")
        .to_owned();
    let (status, second) = as_viewer(format!("{keys}?limit=1&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["groups"][0]["group"], "regions", "{second}");
    assert_eq!(second["groups"][0]["holders"], json!(1), "{second}");
    assert_eq!(
        second["groups"][0]["oldestWaitSince"],
        Value::Null,
        "{second}"
    );
    assert_eq!(second["nextCursor"], Value::Null, "the last page: {second}");
    let (status, regions_only) = as_viewer(format!("{keys}?group=regions")).await;
    assert_eq!(status, StatusCode::OK, "{regions_only}");
    assert_eq!(
        regions_only["groups"].as_array().map(Vec::len),
        Some(1),
        "{regions_only}"
    );
    assert_eq!(
        regions_only["groups"][0]["group"], "regions",
        "{regions_only}"
    );
    // the listing holds to its contract in the group view too
    crate::support::contract::OpenApi::load()
        .validate_body(
            "GET",
            "/namespaces/{namespaceId}/concurrency-keys",
            200,
            &first,
        )
        .expect("a documented JSON body")
        .unwrap_or_else(|errors| panic!("against the contract: {errors}\n{first}"));

    // the rate-limit listing, the same way
    let rate_keys = format!("/namespaces/{namespace}/rate-limit-keys");
    let (status, rate) = as_viewer(rate_keys.clone()).await;
    assert_eq!(status, StatusCode::OK, "{rate}");
    assert_eq!(rate["view"], "GROUPS", "{rate}");
    assert_eq!(rate["items"], json!([]), "{rate}");
    let group = &rate["groups"][0];
    assert_eq!(group["group"], "providers", "{rate}");
    assert_eq!(
        (&group["grantsInWindow"], &group["waiters"]),
        (&json!(1), &json!(1)),
        "{rate}"
    );
    assert!(
        group.get("keys").is_none() && group.get("nextSlotAt").is_none(),
        "neither a count of windows nor a next slot reaches a viewer: {rate}"
    );
    assert_eq!(
        instant(&group["oldestWaitSince"]),
        instant(&json!(since(&run(&rate_waiter).await, "rateLimit"))),
        "whole: {rate}"
    );
    let _ = rate_holder;
    crate::support::contract::OpenApi::load()
        .validate_body(
            "GET",
            "/namespaces/{namespaceId}/rate-limit-keys",
            200,
            &rate,
        )
        .expect("a documented JSON body")
        .unwrap_or_else(|errors| panic!("against the contract: {errors}\n{rate}"));

    // a caller with the reveal right reads the keys, by label; its cursor is refused in the group view and back
    let (status, revealed) = read(&api, &format!("{keys}?limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{revealed}");
    assert_eq!(revealed["view"], "KEYS", "{revealed}");
    assert_eq!(revealed["groups"], json!([]), "{revealed}");
    assert!(revealed["items"][0]["keyLabel"].is_string(), "{revealed}");
    let key_cursor = revealed["nextCursor"]
        .as_str()
        .expect("a next page")
        .to_owned();
    let (status, refused) = as_viewer(format!("{keys}?limit=1&cursor={key_cursor}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused) = read(&api, &format!("{keys}?limit=1&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

/// Whether the plan scans every row of `relation`.
fn scans_all_of(plan: &Value, relation: &str) -> bool {
    (plan["Node Type"] == "Seq Scan" && plan["Relation Name"] == relation)
        || plan["Plans"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|child| scans_all_of(child, relation))
}

/// Every row the plan's scans of `relation` returned, over all their loops.
fn rows_of(plan: &Value, relation: &str) -> f64 {
    let mut rows = 0.0;
    if plan["Relation Name"] == relation {
        rows += plan["Actual Rows"].as_f64().unwrap_or(0.0)
            * plan["Actual Loops"].as_f64().unwrap_or(1.0);
    }
    for child in plan["Plans"].as_array().into_iter().flatten() {
        rows += rows_of(child, relation);
    }
    rows
}

/// Seeds `count` keys of `group` in both gates' key tables of `namespace` (the counters only: one holder and one waiter
/// a concurrency key, one waiter a window, its oldest wait an hour ago), digests `sha256(group/i)`.
async fn seed_group_keys(
    api: &TestApi,
    tenant: sagat_domain::identity::TenantId,
    namespace: &str,
    group: &str,
    count: i32,
) {
    for table in [
        "concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, holders, waiters",
        "rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, live_grants, waiters",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table}, oldest_wait_at, key_version, key_shard, updated_at) \
             SELECT $1, $2::uuid, 'ORDINARY', 'RUN', $3, sha256(convert_to($3 || '/' || i, 'UTF8')), \
                    CASE WHEN '{table}' LIKE 'concurrency%' THEN 1 ELSE 0 END, 1, \
                    clock_timestamp() - interval '1 hour', \
                    (SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'), i % 64, clock_timestamp() \
               FROM generate_series(1, $4) i"
        )))
        .bind(tenant.as_uuid())
        .bind(namespace)
        .bind(group)
        .bind(count)
        .execute(&api.db.pool)
        .await
        .expect("keys seeded");
    }
}

/// Batch 4 reviews of 2026-11-25, correctness Major 1 and security minor 2 (decision D022-086 item 5): a page of groups
/// reads at most its budget of key rows, whatever its groups' sizes: each statement of the walk, asked of the planner over
/// a group of 20,000 keys beside 100 groups of 50, reads at most the budget and one more of the key table (and one probe
/// of the holders a key), and the probe for the next group, named or not, at most four rows; never a scan.
#[tokio::test]
async fn a_page_of_groups_reads_at_most_its_budget_of_key_rows() {
    let (api, namespace, tenant) = api().await;
    let pool = &api.db.pool;
    seed_group_keys(&api, tenant, &namespace, "big", 20_000).await;
    for group in 0..100 {
        seed_group_keys(&api, tenant, &namespace, &format!("group-{group:03}"), 50).await;
    }
    for table in [
        "concurrency_semaphores",
        "rate_limit_windows",
        "concurrency_holders",
        "rate_limit_grants",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .expect("analyze");
    }
    let budget = i64::from(sagat_application::flow_control_keys::GROUP_PAGE_KEY_ROWS);
    let namespace_id = Uuid::parse_str(&namespace).expect("uuid");
    let explain = |sql: &'static str| format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}");
    for (keys, relation, others) in [
        (
            sagat_infra_postgres::CONCURRENCY_GROUP_KEYS_SQL,
            "concurrency_semaphores",
            "concurrency_holders",
        ),
        (
            sagat_infra_postgres::RATE_GROUP_KEYS_SQL,
            "rate_limit_windows",
            "rate_limit_grants",
        ),
    ] {
        // from the group's first key, and from the middle of its range
        for after in [Vec::new(), vec![0x80_u8; 32]] {
            let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain(keys)))
                .bind(tenant.as_uuid())
                .bind(namespace_id)
                .bind("ORDINARY")
                .bind("RUN")
                .bind("big")
                .bind(&after)
                .bind(budget + 1)
                .fetch_one(pool)
                .await
                .expect("a plan");
            let text = plan.to_string();
            assert!(
                !scans_all_of(&plan[0]["Plan"], relation),
                "no scan of every key: {text}"
            );
            let read = rows_of(&plan[0]["Plan"], relation);
            assert!(
                read > 0.0 && read <= (budget + 1) as f64,
                "{relation} rows read {read} for a budget of {budget}: {text}"
            );
            let probed = rows_of(&plan[0]["Plan"], others);
            assert!(
                probed <= 4.0 * (budget + 1) as f64,
                "{others} rows read {probed}: {text}"
            );
        }
    }
    for (sql, relation, named) in [
        (
            sagat_infra_postgres::CONCURRENCY_GROUP_AFTER_SQL,
            "concurrency_semaphores",
            false,
        ),
        (
            sagat_infra_postgres::CONCURRENCY_GROUP_NAMED_AFTER_SQL,
            "concurrency_semaphores",
            true,
        ),
        (
            sagat_infra_postgres::RATE_GROUP_AFTER_SQL,
            "rate_limit_windows",
            false,
        ),
        (
            sagat_infra_postgres::RATE_GROUP_NAMED_AFTER_SQL,
            "rate_limit_windows",
            true,
        ),
    ] {
        for after in ["", "big", "group-050"] {
            let mut query = sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(explain(sql)))
                .bind(tenant.as_uuid())
                .bind(namespace_id)
                .bind(if after.is_empty() { "" } else { "ORDINARY" })
                .bind(if after.is_empty() { "" } else { "RUN" })
                .bind(after);
            if named {
                query = query.bind("group-077");
            }
            let plan = query.fetch_one(pool).await.expect("a plan");
            let text = plan.to_string();
            assert!(
                !scans_all_of(&plan[0]["Plan"], relation),
                "no scan of every key: {text}"
            );
            let read = rows_of(&plan[0]["Plan"], relation);
            assert!(
                read <= 4.0,
                "{relation} rows read {read} (after {after:?}): {text}"
            );
        }
    }
}

/// Batch 4 reviews of 2026-11-25, correctness Major 1 (decision D022-086 item 5): one group of 100,000 keys and a group
/// `zz` after it. A viewer pages both listings one group at a time: every page answers `200`, the first holds no group
/// (the big group is summed across pages, its partial sums in the sealed cursor), the big group's row comes whole, its
/// sums those of every key, and `zz` is reached.
#[tokio::test]
async fn a_group_larger_than_a_page_is_summed_across_pages_and_the_next_group_is_reached() {
    use sagat_domain::identity::Role;
    let (api, namespace, tenant) = api().await;
    member(&api, tenant, "vera-viewer", Role::Viewer).await;
    seed_group_keys(&api, tenant, &namespace, "big", 100_000).await;
    seed_group_keys(&api, tenant, &namespace, "zz", 1).await;
    for (listing, sums) in [
        ("concurrency-keys", ["holders", "waiters"]),
        ("rate-limit-keys", ["waiters", "unleasedGrants"]),
    ] {
        let mut cursor: Option<String> = None;
        let mut groups: Vec<Value> = Vec::new();
        let mut pages = 0_u32;
        loop {
            let path = match &cursor {
                Some(cursor) => {
                    format!("/namespaces/{namespace}/{listing}?limit=1&cursor={cursor}")
                }
                None => format!("/namespaces/{namespace}/{listing}?limit=1"),
            };
            let (status, page) = read_as(&api, &path, "vera-viewer").await;
            assert_eq!(status, StatusCode::OK, "page {pages} of {listing}: {page}");
            assert_eq!(page["view"], "GROUPS", "{page}");
            if pages == 0 {
                assert_eq!(
                    page["groups"],
                    json!([]),
                    "the big group does not fit one page: {page}"
                );
            }
            pages += 1;
            groups.extend(page["groups"].as_array().into_iter().flatten().cloned());
            match page["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
            assert!(pages <= 1_000, "{listing} never ends");
        }
        let names: Vec<&str> = groups.iter().filter_map(|g| g["group"].as_str()).collect();
        assert_eq!(names, ["big", "zz"], "{listing}: {groups:?}");
        assert!(
            pages >= 500,
            "{listing}: at most 200 keys a page, {pages} pages"
        );
        let expected = if listing == "concurrency-keys" {
            [100_000, 100_000]
        } else {
            [100_000, 0]
        };
        for (field, value) in sums.iter().zip(expected) {
            assert_eq!(
                groups[0][*field],
                json!(value),
                "{listing} {field}: {groups:?}"
            );
        }
        assert!(groups[0]["oldestWaitSince"].is_string(), "{groups:?}");
    }
}

/// Batch 4 reviews of 2026-11-25, security Major 1: what a caller without the reveal right reads of a group does not
/// change with how many keys its runs share. Two groups with two holders each, one on a single key (limit 2) and one on
/// two keys: their rows, but for the group's name and instants, are equal.
#[tokio::test]
async fn a_group_view_tells_nothing_that_changes_with_the_number_of_keys() {
    use sagat_domain::identity::Role;
    let (api, namespace, tenant) = api().await;
    member(&api, tenant, "vera-viewer", Role::Viewer).await;
    let mut shared = serial_start("rest-shared-key", 2);
    shared["concurrency"]["group"] = json!("shared");
    let shared = publish(&api, "rest-shared-key", shared)
        .await
        .expect("published");
    let mut apart = serial_start("rest-apart-keys", 2);
    apart["concurrency"]["group"] = json!("apart");
    let apart = publish(&api, "rest-apart-keys", apart)
        .await
        .expect("published");
    start(&api, &shared, "acct-1").await;
    start(&api, &shared, "acct-1").await;
    start(&api, &apart, "acct-1").await;
    start(&api, &apart, "acct-2").await;
    let (status, page) = read_as(
        &api,
        &format!("/namespaces/{namespace}/concurrency-keys"),
        "vera-viewer",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let mut rows: Vec<Value> = page["groups"].as_array().expect("groups").clone();
    assert_eq!(rows.len(), 2, "{page}");
    for row in &mut rows {
        let row = row.as_object_mut().expect("a group row");
        for field in ["group", "oldestWaitSince", "oldestHolderSince"] {
            row.remove(field);
        }
    }
    assert_eq!(rows[0], rows[1], "{page}");
}

/// The version a sealed cursor names in clear: `c1.` and its first two bytes.
fn cursor_version(cursor: &str) -> u16 {
    u16::from_str_radix(&cursor["c1.".len().."c1.".len() + 4], 16).expect("a version")
}

/// D022-059 end to end, and the keys reviews' minor on a leaked key (2026-11-20): a page's cursor is sealed under the
/// version the database has active in its snapshot; during a rotation a cursor of the retiring version still opens;
/// once the operator forces that version out (ADR-0093 decision 5, a leaked key) its cursors are refused `400` although
/// the process still holds it, while a cursor of the active version pages on.
#[tokio::test]
async fn a_cursor_is_sealed_under_the_active_version_and_one_of_a_forced_version_is_refused() {
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
    grant_reveal(&api, &namespace, "alice").await;
    let workflow = publish(&api, "rest-rotation", serial_start("rest-rotation", 1))
        .await
        .expect("published");
    for account in ["acct-1", "acct-2", "acct-3"] {
        start(&api, &workflow, account).await;
    }
    let path = format!("/namespaces/{namespace}/concurrency-keys?limit=1");
    let next = |page: &Value| page["nextCursor"].as_str().expect("a cursor").to_owned();
    let (status, page) = read(&api, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let under_one = next(&page);
    assert_eq!(cursor_version(&under_one), 1, "{under_one}");

    // version 2 activated while keys live under 1: 1 retires, 2 seals
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(2::smallint, 'operator')",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("activated");
    assert_eq!(outcome, "ACTIVATED");
    let (status, page) = read(&api, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let under_two = next(&page);
    assert_eq!(
        cursor_version(&under_two),
        2,
        "sealed under the active version"
    );
    let (status, body) = read(&api, &format!("{path}&cursor={under_one}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retiring version's cursor opens: {body}"
    );

    // the operator forces version 1 out: its cursors no longer open, version 2's do
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

/// Batch 4 correctness nit 1 (decision D022-092): a cursor of the key view, read again by its caller once the caller's
/// reveal grant is gone (the caller now gets the group view), is refused `400` with a message of its own, so the caller
/// learns to read the listing again from its start rather than that its cursor was altered.
#[tokio::test]
async fn a_cursor_of_the_other_view_is_refused_with_its_own_message() {
    let (api, namespace, acme) = api().await;
    let workflow = publish(&api, "rest-other-view", serial_start("rest-other-view", 1))
        .await
        .expect("published");
    let _first = start(&api, &workflow, CANARY).await;
    let _second = start(&api, &workflow, "acct-other-view").await;
    let keys = format!("/namespaces/{namespace}/concurrency-keys");
    let (status, page) = read_as(&api, &format!("{keys}?limit=1"), "alice").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page["view"], "KEYS",
        "the admin with the grant reads keys: {page}"
    );
    let cursor = page["nextCursor"].as_str().expect("a next page").to_owned();
    sqlx::query("DELETE FROM payload_reveal_grants WHERE tenant_id = $1 AND subject = 'alice'")
        .bind(acme.as_uuid())
        .execute(&api.db.pool)
        .await
        .expect("the grant revoked");
    let (status, refused) =
        read_as(&api, &format!("{keys}?limit=1&cursor={cursor}"), "alice").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused.to_string().contains("other view of this listing"),
        "{refused}"
    );
}

/// Batch 5 security suspicion 3 (decision D022-100): a key listing's cursor expires fifteen minutes after its page. A
/// real page's cursor of both listings and both views, opened and sealed again with the deployment's ring sixteen
/// minutes earlier, is refused `400` with a message of its own; sealed fourteen minutes earlier, it is read.
#[tokio::test]
async fn an_expired_cursor_is_refused_with_its_own_message() {
    use sagat_application::flow_control_keys::{KeyListing, KeyPosition};
    use sagat_application::keyed_digest::KeyRing;
    use sagat_domain::identity::Role;
    let (api, namespace, acme) = api().await;
    member(&api, acme, "vera-viewer", Role::Viewer).await;
    let accounts = publish(&api, "rest-expired-a", serial_start("rest-expired-a", 1))
        .await
        .expect("published");
    let mut regions = serial_start("rest-expired-r", 1);
    regions["concurrency"]["group"] = json!("regions");
    let regions = publish(&api, "rest-expired-r", regions)
        .await
        .expect("published");
    let mut rated = serial_start("rest-expired-rate", 1);
    rated
        .as_object_mut()
        .expect("a document")
        .remove("concurrency");
    rated["rateLimit"] =
        json!({"group": "providers", "key": "input.accountId", "limit": 1, "per": "hour"});
    let rated = publish(&api, "rest-expired-rate", rated)
        .await
        .expect("published");
    for (workflow, account) in [
        (&accounts, CANARY),
        (&accounts, "acct-expired"),
        (&regions, CANARY),
        (&rated, CANARY),
        (&rated, "acct-expired"),
    ] {
        start(&api, workflow, account).await;
    }
    let ring = KeyRing::parse(crate::support::flow_control::RING).expect("the ring");
    let namespace_id: sagat_domain::identity::NamespaceId = namespace.parse().expect("an id");
    for (gate, path, subject) in [
        (KeyListing::Concurrency, "concurrency-keys", "alice"),
        (KeyListing::Concurrency, "concurrency-keys", "vera-viewer"),
        (KeyListing::RateLimit, "rate-limit-keys", "alice"),
    ] {
        let listing = format!("/namespaces/{namespace}/{path}");
        let (status, page) = read_as(&api, &format!("{listing}?limit=1"), subject).await;
        assert_eq!(status, StatusCode::OK, "{path} as {subject}: {page}");
        let cursor = page["nextCursor"].as_str().expect("a next page");
        let opened = KeyPosition::open_either(cursor, gate, &ring, acme, namespace_id)
            .expect("a cursor of the deployment's ring");
        let aged = |minutes: i64| {
            opened
                .position
                .seal(
                    gate.of(opened.view),
                    &ring,
                    1,
                    acme,
                    namespace_id,
                    opened.sealed_at - time::Duration::minutes(minutes),
                )
                .expect("sealed again")
        };
        let (status, read) = read_as(
            &api,
            &format!("{listing}?limit=1&cursor={}", aged(14)),
            subject,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{path} as {subject}: {read}");
        let (status, refused) = read_as(
            &api,
            &format!("{listing}?limit=1&cursor={}", aged(16)),
            subject,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} as {subject}: {refused}"
        );
        assert!(
            refused.to_string().contains("the cursor has expired"),
            "{path} as {subject}: {refused}"
        );
    }
}
