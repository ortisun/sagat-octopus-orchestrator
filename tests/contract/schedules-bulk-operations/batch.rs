//! Live conformance of the batch operations marked `x-implemented: '018'` (`T018-045`): every
//! response the API really produces uses a documented status, conforms to the documented schema and
//! carries the documented headers, on the normal path and on every refusal.

use std::collections::BTreeSet;

use sagat_domain::identity::{Membership, Role};
use serde_json::json;

use crate::support::api::start;
use crate::support::contract::{As, OpenApi, Scenario, check_body, scenario};
use crate::support::{now, subject};

const PREVIEW: &str = "/namespaces/{namespaceId}/batch-operations/preview";
const JOBS: &str = "/namespaces/{namespaceId}/batch-operations";
const JOB: &str = "/namespaces/{namespaceId}/batch-operations/{batchId}";
const CANCEL_JOB: &str = "/namespaces/{namespaceId}/batch-operations/{batchId}/cancel";
const ITEMS: &str = "/namespaces/{namespaceId}/batch-operations/{batchId}/items";

fn keyed(s: Scenario, n: u32) -> Scenario {
    s.with_header("idempotency-key", format!("contract-key-{n:04}"))
}

#[tokio::test]
async fn every_batch_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    api.owner_store
        .insert_membership(&Membership::restore(
            acme,
            subject("vera"),
            Role::Viewer,
            now(),
        ))
        .await
        .expect("grant");
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let bob = As::User("bob", "globex");
    let jobs = format!("/namespaces/{ns}/batch-operations");
    let nobody = "0194f5e2-7b1c-7000-8000-000000000009";
    let cancel_all = |reason: &str| {
        json!({
            "selector": {"runIds": [nobody]},
            "action": "CANCEL",
            "reason": reason,
            "itemsPerSecond": 50
        })
    };
    let mut seen = BTreeSet::new();
    let mut record = |(method, template, _status): (String, String, u16)| {
        seen.insert((method, template));
    };

    // ---- preview
    for (who, body, expect) in [
        (alice.clone(), json!({"selector": {"filter": {}}}), 200),
        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "maxItems": 5}),
            200,
        ),
        (vera.clone(), json!({"selector": {"filter": {}}}), 403),
        (bob.clone(), json!({"selector": {"filter": {}}}), 404),
        (As::Anonymous, json!({"selector": {"filter": {}}}), 401),
        (
            alice.clone(),
            json!({"selector": {"runIds": ["not-an-id"]}}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"filter": {"status": ["NOT_A_STATUS"]}}}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"query": "status = 'RUNNING'"}}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"filter": {}}, "nonsense": 1}),
            422,
        ),
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &scenario(
                "POST",
                PREVIEW,
                &format!("{jobs}/preview"),
                who,
                Some(body),
                expect,
            ),
        )
        .await;
        record((m, t, s));
    }

    // ---- create
    let (m, t, s, body) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                JOBS,
                &jobs,
                alice.clone(),
                Some(cancel_all("clean up the stuck runs")),
                202,
            ),
            1,
        ),
    )
    .await;
    record((m, t, s));
    assert_eq!(body["state"], "SELECTING");
    assert_eq!(body["action"], "CANCEL");
    assert_eq!(body["hasPayload"], false);
    let id = body["batchId"].as_str().expect("id").to_owned();
    // a job with a payload says that it has one; neither the payload nor its digest comes back, for a
    // digest would let a reader confirm a guess of what they may not read
    let (m, t, s, signal) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                JOBS,
                &jobs,
                alice.clone(),
                Some(json!({
                    "selector": {"runIds": [nobody]}, "action": "SIGNAL",
                    "signal": {"name": "order.approved", "payload": {"approved": true}},
                    "reason": "tell them it was approved", "itemsPerSecond": 50
                })),
                202,
            ),
            90,
        ),
    )
    .await;
    record((m, t, s));
    assert_eq!(signal["hasPayload"], true);
    assert!(signal.get("payloadDigest").is_none(), "{signal}");
    assert!(
        !signal.to_string().contains("approved\":true"),
        "the payload is not returned"
    );
    let one = format!("{jobs}/{id}");
    for (n, (who, doc, expect)) in [
        (vera.clone(), cancel_all("clean up the stuck runs"), 403),
        (As::Anonymous, cancel_all("clean up the stuck runs"), 401),
        (bob.clone(), cancel_all("clean up the stuck runs"), 404),
        (alice.clone(), cancel_all("short"), 422),
        // a reason is plain text: a new line in it would make the single-run use cases refuse it
        (alice.clone(), cancel_all("clean up\nthe stuck runs"), 422),
        (
            alice.clone(),
            // an Update names the operation and carries its arguments
            json!({"selector": {"runIds": [nobody]}, "action": "UPDATE", "reason": "clean up the stuck runs"}),
            422,
        ),

        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "action": "RETRY_STEP", "reason": "clean up the stuck runs"}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "action": "SIGNAL", "signal": {"name": "go", "payload": {"a": 1}}, "reason": "clean up the stuck runs", "itemsPerSecond": 500}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "action": "WIPE", "reason": "clean up the stuck runs"}),
            422,
        ),
        // the business id is a rerun's: DROP on a cancel is a parameter the action refuses
        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "action": "CANCEL", "businessId": "DROP", "reason": "clean up the stuck runs"}),
            422,
        ),
        (
            alice.clone(),
            json!({"selector": {"runIds": [nobody]}, "action": "RERUN", "businessId": "WIPE", "reason": "clean up the stuck runs"}),
            422,
        ),
        (alice.clone(), json!({"nonsense": true}), 422),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 2;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(scenario("POST", JOBS, &jobs, who, Some(doc), expect), n),
        )
        .await;
        record((m, t, s));
    }
    // a missing Idempotency-Key is a documented 400
    let (m, t, s, _) = check_body(
        &api,
        &spec,
        &scenario(
            "POST",
            JOBS,
            &jobs,
            alice.clone(),
            Some(cancel_all("clean up the stuck runs")),
            400,
        ),
    )
    .await;
    record((m, t, s));
    // and so is a key of the platform's own, which a caller could use to decide what an item does
    for reserved in [
        "batch:0194f5e2-reserved-by-the-platform",
        "schedule:0194f5e2-reserved",
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &scenario(
                "POST",
                JOBS,
                &jobs,
                alice.clone(),
                Some(cancel_all("clean up the stuck runs")),
                400,
            )
            .with_header("idempotency-key", reserved),
        )
        .await;
        record((m, t, s));
    }

    // ---- read and list
    for (path, who, expect) in [
        (jobs.clone(), alice.clone(), 200),
        (jobs.clone(), vera.clone(), 200),
        (jobs.clone(), bob.clone(), 404),
        (jobs.clone(), As::Anonymous, 401),
        (format!("{jobs}?state=SELECTING"), alice.clone(), 200),
        (format!("{jobs}?state=NOT_A_STATE"), alice.clone(), 400),
        (
            format!("{jobs}?limit=1&cursor=nonsense"),
            alice.clone(),
            400,
        ),
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &scenario("GET", JOBS, &path, who, None, expect),
        )
        .await;
        record((m, t, s));
    }
    for (path, who, expect) in [
        (one.clone(), alice.clone(), 200),
        (one.clone(), vera.clone(), 200),
        (one.clone(), bob.clone(), 404),
        (one.clone(), As::Anonymous, 401),
        (format!("{jobs}/{nobody}"), alice.clone(), 404),
        (format!("{jobs}/not-an-id"), alice.clone(), 404),
    ] {
        let (m, t, s, _) =
            check_body(&api, &spec, &scenario("GET", JOB, &path, who, None, expect)).await;
        record((m, t, s));
    }

    // a job is only in the namespace it was created in, whoever asks through another one
    let billing = api
        .create_namespace("alice", "acme", &ws, "billing", "dev")
        .await;
    for (template, method, suffix, body) in [
        (JOB, "GET", "", None),
        (ITEMS, "GET", "/items", None),
        (
            CANCEL_JOB,
            "POST",
            "/cancel",
            Some(json!({"reason": "stop this batch"})),
        ),
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(
                scenario(
                    method,
                    template,
                    &format!("/namespaces/{billing}/batch-operations/{id}{suffix}"),
                    alice.clone(),
                    body,
                    404,
                ),
                30,
            ),
        )
        .await;
        record((m, t, s));
    }

    // ---- items
    for (path, who, expect) in [
        (format!("{one}/items"), alice.clone(), 200),
        (format!("{one}/items"), vera.clone(), 200),
        (format!("{one}/items"), bob.clone(), 404),
        (format!("{one}/items"), As::Anonymous, 401),
        (
            format!("{one}/items?state=PENDING&limit=5"),
            alice.clone(),
            200,
        ),
        (format!("{one}/items?state=NOT_A_STATE"), alice.clone(), 400),
        (format!("{one}/items?cursor=nonsense"), alice.clone(), 400),
        (format!("{jobs}/{nobody}/items"), alice.clone(), 404),
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &scenario("GET", ITEMS, &path, who, None, expect),
        )
        .await;
        record((m, t, s));
    }

    // ---- cancel
    for (n, (who, doc, expect)) in [
        (vera.clone(), json!({"reason": "stop this batch"}), 403),
        (bob.clone(), json!({"reason": "stop this batch"}), 404),
        (As::Anonymous, json!({"reason": "stop this batch"}), 401),
        (alice.clone(), json!({"reason": "short"}), 422),
        (alice.clone(), json!({"nonsense": true}), 422),
        (alice.clone(), json!({"reason": "stop this batch"}), 202),
        // asking twice is asking once
        (alice.clone(), json!({"reason": "stop this batch"}), 202),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 40;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(
                scenario(
                    "POST",
                    CANCEL_JOB,
                    &format!("{one}/cancel"),
                    who,
                    Some(doc),
                    expect,
                ),
                n,
            ),
        )
        .await;
        record((m, t, s));
    }
    // a missing Idempotency-Key is a documented 400
    let (m, t, s, _) = check_body(
        &api,
        &spec,
        &scenario(
            "POST",
            CANCEL_JOB,
            &format!("{one}/cancel"),
            alice.clone(),
            Some(json!({"reason": "stop this batch"})),
            400,
        ),
    )
    .await;
    record((m, t, s));
    // a job that has ended cannot be cancelled
    sqlx::query("UPDATE batch_jobs SET state = 'CANCELLED', finished_at = now() WHERE id = $1")
        .bind(id.parse::<uuid::Uuid>().expect("uuid"))
        .execute(&api.db.pool)
        .await
        .expect("end the job");
    let (m, t, s, body) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                CANCEL_JOB,
                &format!("{one}/cancel"),
                alice.clone(),
                Some(json!({"reason": "stop this batch"})),
                409,
            ),
            50,
        ),
    )
    .await;
    record((m, t, s));
    assert_eq!(body["details"]["reason"], "BATCH_NOT_CANCELLABLE");

    // every operation the contract marks as built by this feature was exercised
    let implemented: BTreeSet<(String, String)> = spec
        .all_operations()
        .into_iter()
        .filter(|(_, template, _, implemented)| {
            *implemented && template.contains("/batch-operations")
        })
        .map(|(method, template, _, _)| (method, template))
        .collect();
    assert_eq!(
        implemented.len(),
        6,
        "the contract names six batch operations"
    );
    for operation in &implemented {
        assert!(
            seen.contains(operation),
            "{operation:?} is implemented and was not exercised"
        );
    }
}
