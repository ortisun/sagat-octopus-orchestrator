use std::collections::BTreeSet;

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::contract::{As, OpenApi, Scenario, check_body, scenario};
use crate::support::worker::{Platform, platform, single_ir};

fn key() -> String {
    format!("key-{}", Uuid::now_v7())
}

fn in_namespace(s: Scenario, namespace: &str) -> Scenario {
    s.with_header("x-octopus-namespace", namespace)
        .with_header("idempotency-key", key())
}

fn flow(name: &str, marker: &str) -> Value {
    let mut ir = single_ir(name, None);
    ir["metadata"]["description"] = json!(marker);
    ir
}

async fn namespace(p: &Platform, workspace: &str, slug: &str, environment: &str) -> String {
    let (status, created) = p
        .rest(
            Method::POST,
            &format!("/workspaces/{workspace}/namespaces"),
            Some(&json!({"slug": slug, "environment": environment})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    created["id"].as_str().expect("id").to_owned()
}

struct Run<'a> {
    p: &'a Platform,
    spec: OpenApi,
    exercised: BTreeSet<(String, String, u16)>,
}

impl Run<'_> {
    /// Sends the scenario, checks it against the contract, and returns what came back.
    async fn go(&mut self, s: Scenario) -> Value {
        let (method, template, status, body) = check_body(&self.p.api, &self.spec, &s).await;
        self.exercised.insert((method, template, status));
        body
    }
}

#[tokio::test]
async fn the_operations_of_feature_011_conform_to_the_contract_on_their_normal_and_error_paths() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let (_, workspaces) = p.rest(Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"]
        .as_str()
        .expect("workspace")
        .to_owned();
    let staging = namespace(&p, &workspace, "billing-staging", "staging").await;
    let prod = namespace(&p, &workspace, "billing-prod", "prod").await;
    let dev = p.namespace_id().await;
    let mut r = Run {
        p: &p,
        spec: OpenApi::load(),
        exercised: BTreeSet::new(),
    };
    let alice = || As::User("alice", "acme");
    let dev1 = || As::User("dev1", "acme");
    let dev2 = || As::User("dev2", "acme");
    let olga = || As::User("olga", "acme");
    let vera = || As::User("vera", "acme");
    let bob = || As::User("bob", "globex");

    // ---------------------------------------------------------------- the policy
    let policy = "/namespaces/{namespaceId}/publication-policy";
    let prod_policy = format!("/namespaces/{prod}/publication-policy");
    let seen = r
        .go(scenario("GET", policy, &prod_policy, vera(), None, 200))
        .await;
    assert_eq!(
        (seen["requiredApprovals"].as_u64(), seen["source"].as_str()),
        (Some(1), Some("default"))
    );
    r.go(scenario(
        "GET",
        policy,
        &format!("/namespaces/{dev}/publication-policy"),
        vera(),
        None,
        200,
    ))
    .await;
    r.go(scenario("GET", policy, &prod_policy, bob(), None, 404))
        .await;
    r.go(scenario(
        "GET",
        policy,
        &prod_policy,
        As::Anonymous,
        None,
        401,
    ))
    .await;
    r.go(scenario(
        "PUT",
        policy,
        &prod_policy,
        dev1(),
        Some(json!({"requiredApprovals": 0, "reason": "i would rather not wait"})),
        403,
    ))
    .await;
    r.go(scenario(
        "PUT",
        policy,
        &prod_policy,
        alice(),
        Some(json!({"requiredApprovals": 4, "reason": "everybody should look"})),
        422,
    ))
    .await;
    r.go(scenario(
        "PUT",
        policy,
        &prod_policy,
        alice(),
        Some(json!({"requiredApprovals": 1, "why": "unknown field"})),
        422,
    ))
    .await;
    r.go(scenario(
        "PUT",
        policy,
        &prod_policy,
        bob(),
        Some(json!({"requiredApprovals": 1, "reason": "another tenant's namespace"})),
        404,
    ))
    .await;
    let set = r
        .go(scenario(
            "PUT",
            policy,
            &prod_policy,
            alice(),
            Some(json!({"requiredApprovals": 1, "reason": "one pair of eyes for production"})),
            200,
        ))
        .await;
    assert_eq!(set["source"], "set");

    // ---------------------------------------------------------------- a request to publish
    let created = r
        .go(in_namespace(
            scenario(
                "POST",
                "/workflows",
                "/workflows",
                dev1(),
                Some(json!({"name": "payments-flow", "ir": flow("payments-flow", "v1")})),
                201,
            ),
            "billing-prod",
        ))
        .await;
    let workflow = created["id"].as_str().expect("workflow").to_owned();
    let direct = format!("/workflows/{workflow}/publish");
    let refused = r
        .go(in_namespace(
            scenario(
                "POST",
                "/workflows/{workflowId}/publish",
                &direct,
                dev1(),
                Some(json!({"draftRevision": 1})),
                409,
            ),
            "billing-prod",
        ))
        .await;
    assert_eq!(refused["details"]["reason"], "APPROVAL_REQUIRED");
    assert!(
        refused["remediation"]
            .as_str()
            .expect("remediation")
            .contains("publication request")
    );

    let requests = "/workflows/{workflowId}/publication-requests";
    let requests_path = format!("/workflows/{workflow}/publication-requests");
    r.go(scenario(
        "POST",
        requests,
        &requests_path,
        vera(),
        Some(json!({"draftRevision": 1})),
        403,
    ))
    .await;
    r.go(scenario(
        "POST",
        requests,
        &requests_path,
        bob(),
        Some(json!({"draftRevision": 1})),
        404,
    ))
    .await;
    r.go(scenario(
        "POST",
        requests,
        &requests_path,
        dev1(),
        Some(json!({"draftRevision": 1, "why": "unknown"})),
        422,
    ))
    .await;
    r.go(scenario(
        "POST",
        requests,
        &requests_path,
        dev1(),
        Some(json!({"draftRevision": 9})),
        409,
    ))
    .await;
    let request = r
        .go(scenario(
            "POST",
            requests,
            &requests_path,
            dev1(),
            Some(json!({"draftRevision": 1, "releaseNotes": "first release"})),
            201,
        ))
        .await;
    assert_eq!(request["state"], "PENDING");
    let id = request["id"].as_str().expect("request").to_owned();
    let open = r
        .go(scenario(
            "POST",
            requests,
            &requests_path,
            dev1(),
            Some(json!({"draftRevision": 1})),
            409,
        ))
        .await;
    assert_eq!(open["details"]["reason"], "REQUEST_OPEN");
    r.go(scenario(
        "GET",
        requests,
        &format!("{requests_path}?state=PENDING"),
        vera(),
        None,
        200,
    ))
    .await;
    r.go(scenario(
        "GET",
        requests,
        &format!("{requests_path}?state=DONE"),
        vera(),
        None,
        422,
    ))
    .await;
    r.go(scenario("GET", requests, &requests_path, bob(), None, 404))
        .await;

    let one = "/publication-requests/{requestId}";
    let one_path = format!("/publication-requests/{id}");
    r.go(scenario("GET", one, &one_path, vera(), None, 200))
        .await;
    r.go(scenario(
        "GET",
        one,
        &format!("/publication-requests/{}", Uuid::now_v7()),
        vera(),
        None,
        404,
    ))
    .await;
    r.go(scenario("GET", one, &one_path, bob(), None, 404))
        .await;

    // ---------------------------------------------------------------- deciding
    let approve = "/publication-requests/{requestId}/approve";
    let approve_path = format!("{one_path}/approve");
    let reason = |text: &str| Some(json!({"reason": text}));
    let author = r
        .go(scenario(
            "POST",
            approve,
            &approve_path,
            dev1(),
            reason("it is fine, i wrote it"),
            403,
        ))
        .await;
    assert_eq!(author["details"]["reason"], "AUTHOR_CANNOT_DECIDE");
    r.go(scenario(
        "POST",
        approve,
        &approve_path,
        vera(),
        reason("a viewer may not approve"),
        403,
    ))
    .await;
    r.go(scenario(
        "POST",
        approve,
        &approve_path,
        dev2(),
        reason("no"),
        422,
    ))
    .await;
    r.go(scenario(
        "POST",
        approve,
        &approve_path,
        bob(),
        reason("another tenant approves"),
        404,
    ))
    .await;
    let publish = "/publication-requests/{requestId}/publish";
    let publish_path = format!("{one_path}/publish");
    let early = r
        .go(scenario("POST", publish, &publish_path, dev1(), None, 409)
            .with_header("idempotency-key", key()))
        .await;
    assert_eq!(early["details"]["reason"], "REQUEST_STATE");
    let approved = r
        .go(scenario(
            "POST",
            approve,
            &approve_path,
            dev2(),
            reason("reads well and the tests pass"),
            200,
        ))
        .await;
    assert_eq!(approved["state"], "APPROVED");
    let again = r
        .go(scenario(
            "POST",
            approve,
            &approve_path,
            dev2(),
            reason("approving it a second time"),
            409,
        ))
        .await;
    assert_eq!(again["details"]["reason"], "ALREADY_DECIDED");

    r.go(scenario("POST", publish, &publish_path, vera(), None, 403)
        .with_header("idempotency-key", key()))
        .await;
    r.go(scenario("POST", publish, &publish_path, dev1(), None, 400))
        .await;
    let published = r
        .go(scenario("POST", publish, &publish_path, dev1(), None, 201)
            .with_header("idempotency-key", key()))
        .await;
    assert_eq!(published["versionNumber"], 1);
    r.go(scenario("POST", publish, &publish_path, dev1(), None, 409)
        .with_header("idempotency-key", key()))
        .await;

    // ---------------------------------------------------------------- reject and withdraw
    let second = r
        .go(in_namespace(
            scenario(
                "POST",
                "/workflows",
                "/workflows",
                dev1(),
                Some(json!({"name": "refunds-flow", "ir": flow("refunds-flow", "v1")})),
                201,
            ),
            "billing-prod",
        ))
        .await;
    let second = second["id"].as_str().expect("workflow").to_owned();
    let to_reject = r
        .go(scenario(
            "POST",
            requests,
            &format!("/workflows/{second}/publication-requests"),
            dev1(),
            Some(json!({"draftRevision": 1})),
            201,
        ))
        .await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let reject = "/publication-requests/{requestId}/reject";
    let rejected = r
        .go(scenario(
            "POST",
            reject,
            &format!("/publication-requests/{to_reject}/reject"),
            dev2(),
            reason("breaks the contract with billing"),
            200,
        ))
        .await;
    assert_eq!(rejected["state"], "REJECTED");
    let withdraw = "/publication-requests/{requestId}/withdraw";
    let to_withdraw = r
        .go(scenario(
            "POST",
            requests,
            &format!("/workflows/{second}/publication-requests"),
            dev1(),
            Some(json!({"draftRevision": 1})),
            201,
        ))
        .await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    r.go(scenario(
        "POST",
        withdraw,
        &format!("/publication-requests/{to_withdraw}/withdraw"),
        dev2(),
        None,
        403,
    ))
    .await;
    let withdrawn = r
        .go(scenario(
            "POST",
            withdraw,
            &format!("/publication-requests/{to_withdraw}/withdraw"),
            dev1(),
            None,
            200,
        ))
        .await;
    assert_eq!(withdrawn["state"], "WITHDRAWN");
    r.go(scenario(
        "POST",
        withdraw,
        &format!("/publication-requests/{to_withdraw}/withdraw"),
        dev1(),
        None,
        409,
    ))
    .await;

    // ---------------------------------------------------------------- the active version
    let dev_flow = p.publish("checkout", &flow("checkout", "v1")).await;
    let (status, draft) = p
        .rest(Method::GET, &format!("/workflows/{dev_flow}"), None)
        .await;
    assert_eq!(status, 200, "{draft}");
    let revision = draft["draftRevision"].as_u64().expect("revision");
    let (status, updated) = p
        .rest(
            Method::PUT,
            &format!("/workflows/{dev_flow}"),
            Some(&json!({"draftRevision": revision, "ir": flow("checkout", "v2")})),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    let (status, v2) = p
        .rest(
            Method::POST,
            &format!("/workflows/{dev_flow}/publish"),
            Some(&json!({"draftRevision": revision + 1})),
        )
        .await;
    assert_eq!(status, 201, "{v2}");

    let activate = "/workflows/{workflowId}/active-version";
    let activate_path = format!("/workflows/{dev_flow}/active-version");
    r.go(scenario(
        "PUT",
        activate,
        &activate_path,
        dev1(),
        Some(json!({"version": 1, "reason": "a developer may not roll back"})),
        403,
    ))
    .await;
    r.go(scenario(
        "PUT",
        activate,
        &activate_path,
        olga(),
        Some(json!({"version": 1, "reason": "no"})),
        422,
    ))
    .await;
    r.go(scenario(
        "PUT",
        activate,
        &activate_path,
        olga(),
        Some(json!({"version": 9, "reason": "a version that is not there"})),
        404,
    ))
    .await;
    r.go(scenario(
        "PUT",
        activate,
        &activate_path,
        bob(),
        Some(json!({"version": 1, "reason": "another tenant rolls back"})),
        404,
    ))
    .await;
    let stale = r
        .go(scenario(
            "PUT",
            activate,
            &activate_path,
            olga(),
            Some(
                json!({"version": 1, "reason": "the operator saw version 1", "expectedVersion": 1}),
            ),
            409,
        ))
        .await;
    assert_eq!(stale["details"]["reason"], "ACTIVE_VERSION_CHANGED");
    let rolled = r
        .go(scenario("PUT", activate, &activate_path, olga(), Some(json!({"version": 1, "reason": "version 2 shows a regression", "expectedVersion": 2})), 200))
        .await;
    assert_eq!(
        (rolled["kind"].as_str(), rolled["changed"].as_bool()),
        (Some("ROLLBACK"), Some(true))
    );
    let noop = r
        .go(scenario(
            "PUT",
            activate,
            &activate_path,
            olga(),
            Some(json!({"version": 1, "reason": "it is version 1 already"})),
            200,
        ))
        .await;
    assert_eq!(noop["changed"], false);
    let history_template = "/workflows/{workflowId}/activations";
    let history_path = format!("/workflows/{dev_flow}/activations");
    let history = r
        .go(scenario(
            "GET",
            history_template,
            &history_path,
            vera(),
            None,
            200,
        ))
        .await;
    assert_eq!(history["items"][0]["kind"], "ROLLBACK");
    assert_eq!(history["items"].as_array().expect("items").len(), 3);
    r.go(scenario(
        "GET",
        history_template,
        &format!("{history_path}?limit=1"),
        vera(),
        None,
        200,
    ))
    .await;
    r.go(scenario(
        "GET",
        history_template,
        &format!("{history_path}?cursor=nope"),
        vera(),
        None,
        422,
    ))
    .await;
    r.go(scenario(
        "GET",
        history_template,
        &history_path,
        bob(),
        None,
        404,
    ))
    .await;
    r.go(scenario(
        "GET",
        history_template,
        &history_path,
        As::Anonymous,
        None,
        401,
    ))
    .await;

    // ---------------------------------------------------------------- promotion
    let promote = "/workflows/{workflowId}/versions/{versionNumber}/promote";
    let promote_path = format!("/workflows/{dev_flow}/versions/1/promote");
    let to = |namespace: &str| {
        Some(json!({"namespaceId": namespace, "reason": "tried in the environment before"}))
    };
    r.go(
        scenario("POST", promote, &promote_path, olga(), to(&staging), 403)
            .with_header("idempotency-key", key()),
    )
    .await;
    r.go(
        scenario("POST", promote, &promote_path, dev1(), to(&dev), 422)
            .with_header("idempotency-key", key()),
    )
    .await;
    r.go(scenario(
        "POST",
        promote,
        &promote_path,
        dev1(),
        Some(json!({"namespaceId": staging})),
        422,
    )
    .with_header("idempotency-key", key()))
        .await;
    r.go(
        scenario("POST", promote, &promote_path, bob(), to(&staging), 404)
            .with_header("idempotency-key", key()),
    )
    .await;
    r.go(scenario(
        "POST",
        promote,
        &format!("/workflows/{dev_flow}/versions/9/promote"),
        dev1(),
        to(&staging),
        404,
    )
    .with_header("idempotency-key", key()))
        .await;
    let promoted = r
        .go(
            scenario("POST", promote, &promote_path, dev1(), to(&staging), 201)
                .with_header("idempotency-key", key()),
        )
        .await;
    assert_eq!(promoted["outcome"], "PUBLISHED");
    let same = r
        .go(
            scenario("POST", promote, &promote_path, dev1(), to(&staging), 200)
                .with_header("idempotency-key", key()),
        )
        .await;
    assert_eq!(same["outcome"], "ALREADY_ACTIVE");
    // The workflow the promotion made in staging.
    let listed: Value = p
        .api
        .as_user(Method::GET, "/workflows", "dev1", "acme")
        .header("x-octopus-namespace", "billing-staging")
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("json");
    let staging_flow = listed["items"][0]["id"]
        .as_str()
        .expect("staging workflow")
        .to_owned();
    let requested = r
        .go(scenario(
            "POST",
            promote,
            &format!("/workflows/{staging_flow}/versions/1/promote"),
            dev1(),
            to(&prod),
            202,
        )
        .with_header("idempotency-key", key()))
        .await;
    assert_eq!(requested["outcome"], "REQUESTED");
    assert_eq!(requested["request"]["kind"], "PROMOTE");

    // Everything feature 011 added succeeds somewhere.
    let added = r.spec.operations_marked("x-implemented", "011");
    assert_eq!(added.len(), 12, "{added:?}");
    for (method, template) in &added {
        assert!(
            r.exercised
                .iter()
                .any(|(m, t, st)| m == method && t == template && matches!(*st, 200..=202)),
            "{method} {template} has no successful conformance scenario"
        );
    }
}
