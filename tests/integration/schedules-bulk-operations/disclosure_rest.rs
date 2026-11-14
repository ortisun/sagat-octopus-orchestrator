//! `POST …/schedules/{scheduleId}/input-disclosures` over HTTP (`T018-033`, `FR-018-042`): the values of a
//! schedule's input come out of this route and of no other, under `private, no-store`, in the shape of
//! the contract, and a reveal of personal data is refused without its grant. A real API, a real
//! PostgreSQL, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::engine::{document, end_node};
use crate::support::worker::{HUMAN, Platform, TENANT, platform};

struct Rig {
    p: Platform,
    namespace: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let namespace = p.namespace_id().await;
    Rig { p, namespace }
}

impl Rig {
    fn schedules(&self) -> String {
        format!("/namespaces/{}/schedules", self.namespace)
    }

    /// A schedule whose target classifies each field of its input at another level.
    async fn classified(&self) -> String {
        let mut definition = document("classified-rest", "done", vec![end_node("done")]);
        definition["inputSchema"] = json!({
            "type": "object", "additionalProperties": false,
            "x-sagat-classification": "public",
            "properties": {
                "open": {"type": "string"},
                "inside": {"type": "string", "x-sagat-classification": "internal"},
                "personal": {"type": "string", "x-sagat-classification": "pii"},
                "sealed": {"type": "string", "x-sagat-classification": "secret"}
            }
        });
        let workflow = self.p.publish("classified-rest", &definition).await;
        let (status, created) = self
            .p
            .rest(
                Method::POST,
                &self.schedules(),
                Some(&json!({
                    "name": "classified",
                    "spec": {"intervals": [{"everyMs": 3_600_000}]},
                    "action": {"workflowId": workflow, "input": {
                        "open": "shown", "inside": "staff",
                        "personal": "pii-canary", "sealed": "secret-canary"
                    }}
                })),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        created["scheduleId"].as_str().expect("id").to_owned()
    }

    fn disclosures(&self, id: &str) -> String {
        format!("{}/{id}/input-disclosures", self.schedules())
    }
}

fn names(tree: &Value) -> Vec<&str> {
    tree["children"]
        .as_array()
        .map(|children| {
            children
                .iter()
                .filter(|child| child["node"]["kind"] == "value")
                .filter_map(|child| child["name"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn the_values_of_an_input_come_out_of_this_route_only_and_never_a_secret() {
    let r = rig().await;
    let id = r.classified().await;

    // an ordinary read of the schedule has the digest and no value
    let (status, read) =
        r.p.rest(Method::GET, &format!("{}/{id}", r.schedules()), None)
            .await;
    assert_eq!(status, 200, "{read}");
    let text = read.to_string();
    for canary in ["shown", "staff", "pii-canary", "secret-canary"] {
        assert!(!text.contains(canary), "{canary} is in an ordinary read");
    }

    let response =
        r.p.api
            .as_user(Method::POST, &r.disclosures(&id), HUMAN, TENANT)
            .header("x-octopus-namespace", "payments")
            .json(&json!({}))
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["scheduleId"], id.as_str());
    assert_eq!(body["revision"], 1);
    assert_eq!(body["policy"], "classified-v1");
    assert_eq!(body["kind"], "disclosed");
    assert_eq!(body["caller"]["revealed"], false);
    let levels = body["caller"]["disclosedLevels"]
        .as_array()
        .expect("levels");
    assert!(levels.iter().any(|level| level == "public"));
    assert!(levels.iter().any(|level| level == "internal"));
    assert!(!levels.iter().any(|level| level == "pii"));
    let mut shown = names(&body["tree"]);
    shown.sort_unstable();
    assert_eq!(shown, ["inside", "open"], "{}", body["tree"]);
    // what is withheld says why, never what it was
    let withheld = |name: &str| {
        body["tree"]["children"]
            .as_array()
            .and_then(|children| children.iter().find(|child| child["name"] == name))
            .map(|child| child["node"]["reason"].clone())
    };
    assert_eq!(withheld("personal"), Some(json!("LEVEL_NOT_PERMITTED")));
    assert_eq!(withheld("sealed"), Some(json!("SECRET")));
    let text = body.to_string();
    for canary in ["pii-canary", "secret-canary"] {
        assert!(!text.contains(canary), "{canary} came out");
    }
}

#[tokio::test]
async fn personal_data_is_refused_without_a_grant_and_the_request_is_checked_in_words() {
    let r = rig().await;
    let id = r.classified().await;
    let send = |body: Value| {
        r.p.api
            .as_user(Method::POST, &r.disclosures(&id), HUMAN, TENANT)
            .header("x-octopus-namespace", "payments")
            .json(&body)
            .send()
    };

    // asked for, not granted
    let refused = send(json!({"reveal": ["pii"]})).await.expect("send");
    assert_eq!(refused.status(), 403);
    let refused: Value = refused.json().await.expect("json");
    assert_eq!(refused["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
    assert!(!refused.to_string().contains("pii-canary"));

    // and the request takes nothing else
    for body in [
        json!({"reveal": ["secret"]}),
        json!({"reveal": ["pii", "pii"]}),
    ] {
        assert_eq!(send(body).await.expect("send").status(), 422);
    }
    assert_eq!(
        send(json!({"include": "everything"}))
            .await
            .expect("send")
            .status(),
        400
    );
    // a request is at most 1 KiB, and the refusal of a larger one is private as well
    let large = send(json!({"reveal": [], "padding": "x".repeat(2048)}))
        .await
        .expect("send");
    assert_eq!(large.status(), 413);
    assert_eq!(
        large
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );
}

#[tokio::test]
async fn a_schedule_of_another_tenant_or_namespace_is_not_found() {
    let r = rig().await;
    let id = r.classified().await;

    // another tenant's administrator, whatever the path says
    let response =
        r.p.api
            .as_user(Method::POST, &r.disclosures(&id), "bob", "globex")
            .header("x-octopus-namespace", "payments")
            .json(&json!({}))
            .send()
            .await
            .expect("send");
    assert_eq!(response.status(), 404);
    let answer = response.text().await.expect("body");
    assert!(!answer.contains("pii-canary"));

    // a schedule id that is not one
    let (status, _) =
        r.p.rest(
            Method::POST,
            &r.disclosures("00000000-0000-7000-8000-000000000000"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 404);
}
