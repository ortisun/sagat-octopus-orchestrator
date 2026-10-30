//! Live conformance of the operations marked `x-implemented: '018'` (`T018-018`, `T018-023`): every
//! response the API really produces uses a documented status, conforms to the documented schema and
//! carries the documented headers.

use std::collections::BTreeSet;

use reqwest::Method;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};

use crate::support::api::start;
use crate::support::contract::{As, OpenApi, Scenario, check_body, scenario};
use crate::support::{now, subject};

const LIST: &str = "/namespaces/{namespaceId}/schedules";
const ONE: &str = "/namespaces/{namespaceId}/schedules/{scheduleId}";
const PAUSE: &str = "/namespaces/{namespaceId}/schedules/{scheduleId}/pause";
const RESUME: &str = "/namespaces/{namespaceId}/schedules/{scheduleId}/resume";

fn keyed(s: Scenario, n: u32) -> Scenario {
    s.with_header("idempotency-key", format!("contract-key-{n:04}"))
}

#[tokio::test]
async fn every_schedule_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
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
    let flow = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "reports"}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
    });
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "contract-setup-0001")
        .json(&json!({"name": "reports", "ir": flow}))
        .send()
        .await
        .expect("send");
    assert_eq!(created.status(), 201);
    let workflow = created.json::<Value>().await.expect("json")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "contract-setup-0002")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let bob = As::User("bob", "globex");
    let list = format!("/namespaces/{ns}/schedules");
    let document = |name: &str| {
        json!({
            "name": name,
            "spec": {"intervals": [{"everyMs": 3_600_000}], "timezone": "UTC", "jitterMs": 1000},
            "policies": {"overlap": "BUFFER_ONE", "pauseOnFailure": true},
            "action": {"workflowId": workflow, "input": {"region": "eu"}, "nominalTimeField": "at"},
            "note": "created by the contract test",
            "remainingActions": 100
        })
    };
    let mut seen = BTreeSet::new();
    let mut record = |(method, template, _status): (String, String, u16)| {
        seen.insert((method, template));
    };

    // ---- create
    let (m, t, s, body) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                LIST,
                &list,
                alice.clone(),
                Some(document("daily")),
                201,
            ),
            1,
        ),
    )
    .await;
    record((m, t, s));
    let id = body["scheduleId"].as_str().expect("id").to_owned();
    let one = format!("{list}/{id}");
    for (n, (who, doc, expect)) in [
        (alice.clone(), Some(document("daily")), 409),
        (vera.clone(), Some(document("by-vera")), 403),
        (As::Anonymous, Some(document("anonymous")), 401),
        (bob.clone(), Some(document("by-bob")), 404),
        (
            alice.clone(),
            Some(json!({"name": "bad", "spec": {"cron": ["not a cron"]}, "action": {"workflowId": workflow, "input": {}}})),
            422,
        ),
        (alice.clone(), Some(json!({"nonsense": true})), 422),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 2;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(scenario("POST", LIST, &list, who, doc, expect), n),
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
            LIST,
            &list,
            alice.clone(),
            Some(document("keyless")),
            400,
        ),
    )
    .await;
    record((m, t, s));

    // ---- read, list, describe
    for (who, expect) in [
        (alice.clone(), 200),
        (vera.clone(), 200),
        (bob.clone(), 404),
    ] {
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &scenario("GET", LIST, &list, who, None, expect),
        )
        .await;
        record((m, t, s));
    }
    let (m, t, s, _) = check_body(
        &api,
        &spec,
        &scenario(
            "GET",
            LIST,
            &format!("{list}?limit=1&cursor=nonsense"),
            alice.clone(),
            None,
            400,
        ),
    )
    .await;
    record((m, t, s));
    for (path, who, expect) in [
        (
            format!("{one}?nextCount=3&recentCount=2"),
            alice.clone(),
            200,
        ),
        (one.clone(), vera.clone(), 200),
        (one.clone(), bob.clone(), 404),
        (one.clone(), As::Anonymous, 401),
        (
            format!("{list}/0194f5e2-7b1c-7000-8000-000000000009"),
            alice.clone(),
            404,
        ),
    ] {
        let (m, t, s, _) =
            check_body(&api, &spec, &scenario("GET", ONE, &path, who, None, expect)).await;
        record((m, t, s));
    }

    // ---- replace
    let mut replacement = document("daily");
    replacement["expectedRevision"] = json!(1);
    let (m, t, s, body) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "PUT",
                ONE,
                &one,
                alice.clone(),
                Some(replacement.clone()),
                200,
            ),
            20,
        ),
    )
    .await;
    record((m, t, s));
    assert_eq!(body["revision"], 2);
    for (n, (who, expect)) in [
        (alice.clone(), 409),
        (vera.clone(), 403),
        (bob.clone(), 404),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 21;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(
                scenario("PUT", ONE, &one, who, Some(replacement.clone()), expect),
                n,
            ),
        )
        .await;
        record((m, t, s));
    }
    let mut invalid = document("daily");
    invalid["expectedRevision"] = json!(2);
    invalid["spec"] = json!({"intervals": [{"everyMs": 3_600_000}], "timezone": "Mars/Olympus"});
    let (m, t, s, _) = check_body(
        &api,
        &spec,
        &keyed(
            scenario("PUT", ONE, &one, alice.clone(), Some(invalid), 422),
            30,
        ),
    )
    .await;
    record((m, t, s));

    // ---- pause, resume
    let note = json!({"note": "release freeze"});
    for (n, (template, path, who, expect)) in [
        (PAUSE, format!("{one}/pause"), alice.clone(), 200),
        (PAUSE, format!("{one}/pause"), vera.clone(), 403),
        (PAUSE, format!("{one}/pause"), bob.clone(), 404),
        (RESUME, format!("{one}/resume"), alice.clone(), 200),
        (RESUME, format!("{one}/resume"), vera.clone(), 403),
        (RESUME, format!("{one}/resume"), bob.clone(), 404),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 40;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(
                scenario("POST", template, &path, who, Some(note.clone()), expect),
                n,
            ),
        )
        .await;
        record((m, t, s));
    }
    let (m, t, s, _) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                PAUSE,
                &format!("{one}/pause"),
                alice.clone(),
                Some(json!({"note": ""})),
                422,
            ),
            50,
        ),
    )
    .await;
    record((m, t, s));

    // ---- delete
    for (n, (path, who, expect)) in [
        (format!("{one}?expectedRevision=1"), alice.clone(), 409),
        (format!("{one}?expectedRevision=2"), vera.clone(), 403),
        (format!("{one}?expectedRevision=2"), bob.clone(), 404),
        (one.clone(), alice.clone(), 400),
        (format!("{one}?expectedRevision=2"), alice.clone(), 204),
        (format!("{one}?expectedRevision=2"), alice.clone(), 404),
    ]
    .into_iter()
    .enumerate()
    {
        let n = u32::try_from(n).expect("small") + 60;
        let (m, t, s, _) = check_body(
            &api,
            &spec,
            &keyed(scenario("DELETE", ONE, &path, who, None, expect), n),
        )
        .await;
        record((m, t, s));
    }

    // every operation the contract marks as built by this feature was exercised
    let implemented: BTreeSet<(String, String)> = spec
        .all_operations()
        .into_iter()
        .filter(|(_, template, _, implemented)| *implemented && template.contains("/schedules"))
        .map(|(method, template, _, _)| (method, template))
        .collect();
    assert_eq!(
        implemented.len(),
        7,
        "the contract names seven schedule operations"
    );
    for operation in &implemented {
        assert!(
            seen.contains(operation),
            "{operation:?} is implemented and was not exercised"
        );
    }
}
