//! The cross-tenant battery (SC-009-001, US01): every implemented operation, attacked with a valid
//! token of *another tenant* that holds the administrator role there, against the resources of the
//! victim.
//!
//! Three things must hold for each operation:
//!
//! 1. A resource the attacker addresses (in the path or in the body) answers `404`, with exactly
//!    the body a resource that does not exist would get: its existence is not discoverable.
//! 2. Operations that need no foreign identifier (a listing, an operation on the attacker's own
//!    tenant) never show the victim's identifiers.
//! 3. After the whole battery, the victim's data is byte-for-byte what it was.
//!
//! The battery is driven by the OpenAPI document: an implemented operation without an entry fails
//! the test, so a new endpoint cannot ship without being attacked.

use std::collections::{BTreeMap, BTreeSet};

use reqwest::Method;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::{Options, TestApi, start_with};
use crate::support::contract::OpenApi;
use crate::support::{now, subject};

/// What the attacker sends for one operation: a body that is valid, so that validation cannot be
/// what refuses it.
fn body_of(operation: &str) -> Option<Value> {
    Some(match operation {
        "createWorkspace" => json!({"slug": "attacker-ws"}),
        "createNamespace" => json!({"slug": "attacker-ns", "environment": "dev"}),
        "createWorkflowDraft" => json!({"name": "attacker-flow", "ir": flow("attacker-flow")}),
        "updateWorkflowDraft" => json!({"draftRevision": 1, "ir": flow("victim-flow")}),
        "publishWorkflow" => json!({"draftRevision": 1}),
        // Feature 007: a dry run of the victim's draft with a document of the attacker's.
        "validateWorkflowDraft" => json!({"ir": flow("attacker-flow")}),
        "startRun" => json!({"workflowId": "{workflowId}", "input": {}}),
        "operateRun" => json!({"action": "CANCEL", "reason": "cross-tenant battery probe"}),
        "inspectRunPayload" => json!({"source": "input"}),
        "signalRun" => json!({"paid": true}),
        "setTenantQuota" | "setNamespaceQuota" | "setActivityQuota" => json!({"limit": 1}),
        "bindWorker" => json!({"activities": ["*"], "pools": ["attacker-pool"]}),
        "setSecretPools" => json!({"pools": ["attacker-pool"]}),
        "putSecret" => json!({"value": "attacker-supplied-value-0001"}),
        "setEgressPolicy" => json!({"rules": ["attacker.example.net"]}),
        // Feature 004: what an attacker would send to take over the victim's event plumbing.
        "publishEvent" => json!({"type": "order.created", "data": {"attacker": true}}),
        "createEventEndpoint" => json!({"slug": "attacker-endpoint", "secretName": "stripe-key"}),
        "updateEventEndpoint" => json!({"secretName": "stripe-key"}),
        "createWebhookSubscription" => json!({
            "name": "attacker-hook", "url": "https://attacker.example.net/in",
            "secretName": "stripe-key", "events": ["run.finished"]
        }),
        "updateWebhookSubscription" => json!({
            "url": "https://attacker.example.net/in", "secretName": "stripe-key",
            "events": ["run.finished"]
        }),
        "redriveWebhookDeliveries" | "redriveBrokerDeliveries" => json!({}),
        "createBrokerSubscription" => json!({
            "name": "attacker-broker", "kind": "NATS",
            "settings": {"servers": ["tls://attacker.example.net:4222"], "subject": "events"},
            "events": ["run.finished"]
        }),
        "updateBrokerSubscription" => json!({
            "kind": "NATS", "settings": {"servers": ["tls://attacker.example.net:4222"], "subject": "events"},
            "events": ["run.finished"]
        }),
        "createConnector" => json!({
            "slug": "attacker-connector", "kind": "KAFKA",
            "settings": {"servers": ["attacker.example.net:9092"], "topic": "orders", "startFrom": "earliest"}
        }),
        "updateConnector" => json!({
            "kind": "KAFKA",
            "settings": {"servers": ["attacker.example.net:9092"], "topic": "orders", "startFrom": "earliest"}
        }),
        "receiveSignedWebhook" => json!({"attacker": true}),
        // Feature 005: what an attacker would send to write, try, build or report on the victim's
        // scripts. (The body is read before the handler looks at the path, so an operation that
        // takes one needs a valid one for the answer to be the 404 it is meant to be.)
        "createScript" => {
            json!({"name": "attacker-script", "language": "typescript", "source": "export default () => 1"})
        }
        "updateScript" => json!({"revision": 1, "source": "export default () => 'attacker'"}),
        "createScriptTestRun" => json!({"input": {"attacker": true}}),
        "claimScriptBuild" => json!({"toolchains": {"typescript": "node@sha256:aa;shim=1"}}),
        "claimScriptTestRun" => json!({"languages": ["typescript"]}),
        "heartbeatScriptBuild" | "heartbeatScriptTestRun" => json!({"leaseToken": "0".repeat(64)}),
        "completeScriptBuild" => {
            json!({"leaseToken": "0".repeat(64), "outcome": "failed", "kind": "rejected", "message": "attacker"})
        }
        "completeScriptTestRun" => {
            json!({"leaseToken": "0".repeat(64), "result": {"attacker": true}})
        }
        "recordScriptExecution" => {
            json!({"namespaceId": "{namespaceId}", "digest": format!("sha256:{}", "ab".repeat(32)), "status": "FAILED", "denied": ["attacker"]})
        }
        "recordScriptRunnerIncident" => json!({
            "target": {
                "kind": "production", "namespaceId": "{namespaceId}",
                "runId": "{runId}", "stepId": Uuid::now_v7().to_string(),
                "attempt": 1, "generation": 1, "leaseToken": "0".repeat(64)
            },
            "category": "NOT_ALLOWLISTED"
        }),
        // Feature 011: an administrator of another tenant changing what the victim publishes.
        "setPublicationPolicy" => {
            json!({"requiredApprovals": 0, "reason": "cross-tenant battery probe"})
        }
        "openPublicationRequest" => json!({"draftRevision": 1}),
        "approvePublicationRequest" | "rejectPublicationRequest" => {
            json!({"reason": "cross-tenant battery probe"})
        }
        "activateWorkflowVersion" => {
            json!({"version": 1, "reason": "cross-tenant battery probe", "acceptBreaking": true})
        }
        "promoteWorkflowVersion" => {
            json!({"namespaceId": "{namespaceId}", "reason": "cross-tenant battery probe", "acceptBreaking": true})
        }
        // Feature 008: an administrator of another tenant draining the victim's worker.
        "drainWorker" | "undrainWorker" => {
            json!({"reason": "cross-tenant battery probe"})
        }
        _ => return None,
    })
}

fn flow(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
    })
}

/// What an error looks like once the parts that legitimately differ are dropped.
fn shape(body: &Value) -> Value {
    json!({
        "code": body["code"], "message": body["message"], "retryable": body["retryable"],
        "details": body["details"],
    })
}

struct Victim {
    ids: BTreeMap<&'static str, String>,
}

impl Victim {
    /// `template` with each path parameter replaced: by the victim's real value, or (for the
    /// baseline) by one that exists nowhere.
    fn fill(&self, template: &str, real: bool) -> String {
        let mut path = template.to_owned();
        for (name, value) in &self.ids {
            let placeholder = format!("{{{name}}}");
            if path.contains(&placeholder) {
                let replacement = if real || !is_resource_id(name) {
                    value.clone()
                } else {
                    Uuid::now_v7().to_string()
                };
                path = path.replace(&placeholder, &replacement);
            }
        }
        path
    }
}

/// The parameters that name a resource of a tenant (the others are names and enumerations).
fn is_resource_id(name: &str) -> bool {
    matches!(
        name,
        "workspaceId"
            | "namespaceId"
            | "workflowId"
            | "runId"
            | "scriptId"
            | "executionId"
            | "requestId"
    )
}

async fn send(
    api: &TestApi,
    method: &str,
    path: &str,
    who: (&str, &str),
    body: Option<&Value>,
) -> (u16, Value) {
    let mut request = api
        .as_user(
            Method::from_bytes(method.as_bytes()).expect("method"),
            path,
            who.0,
            who.1,
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", format!("key-{}", Uuid::now_v7()));
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Everything the victim can read about its own data, to compare before and after.
async fn snapshot(api: &TestApi, victim: &Victim) -> Value {
    let ns = &victim.ids["namespaceId"];
    let wf = &victim.ids["workflowId"];
    let run = &victim.ids["runId"];
    let mut out = BTreeMap::new();
    for path in [
        "/workspaces".to_owned(),
        "/namespaces".to_owned(),
        "/workflows".to_owned(),
        format!("/workflows/{wf}"),
        format!("/workflows/{wf}/versions"),
        "/runs".to_owned(),
        format!("/runs/{run}"),
        format!("/runs/{run}/events"),
        "/quotas".to_owned(),
        format!("/namespaces/{ns}/quotas"),
        format!("/namespaces/{ns}/activity-quotas"),
        format!("/namespaces/{ns}/worker-bindings"),
        format!("/namespaces/{ns}/secrets"),
        format!("/namespaces/{ns}/secrets/stripe-key/access"),
        format!("/namespaces/{ns}/secrets/stripe-key/pools"),
        format!("/namespaces/{ns}/egress-policy"),
        format!("/namespaces/{ns}/scripts"),
        format!(
            "/namespaces/{ns}/scripts/{}/versions",
            victim.ids["scriptId"]
        ),
        format!(
            "/namespaces/{ns}/scripts/{}/test-runs",
            victim.ids["scriptId"]
        ),
    ] {
        let (status, body) = send(api, "GET", &path, ("alice", "acme"), None).await;
        assert_eq!(status, 200, "{path}: {body}");
        out.insert(path, body);
    }
    json!(out)
}

#[tokio::test]
async fn every_implemented_operation_resists_an_administrator_of_another_tenant() {
    let spec = OpenApi::load();
    let api = start_with(Options {
        blobs: Some(sagat_application::blobs::BlobLimits::default()),
        ..Options::default()
    })
    .await;
    let acme = api.tenant("acme", "alice").await;
    let globex = api.tenant("globex", "bob").await;
    api.owner_store
        .insert_membership(&Membership::restore(
            acme,
            subject("dave"),
            Role::Developer,
            now(),
        ))
        .await
        .expect("grant");
    api.owner_store
        .insert_membership(&Membership::restore(
            globex,
            subject("runner-bob"),
            Role::Runner,
            now(),
        ))
        .await
        .expect("grant runner");

    // The victim: a tenant with data of every kind the API can hold.
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    // The attacker has a namespace with the same slug, so the header resolves for it too.
    let globex_ws = api.create_workspace("bob", "globex", "platform").await;
    api.create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;

    let alice = ("alice", "acme");
    let (status, created) = send(
        &api,
        "POST",
        "/workflows",
        alice,
        Some(&json!({"name": "victim-flow", "ir": flow("victim-flow")})),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let wf = created["id"].as_str().expect("id").to_owned();
    let (status, published) = send(
        &api,
        "POST",
        &format!("/workflows/{wf}/publish"),
        alice,
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{published}");
    let (status, run) = send(
        &api,
        "POST",
        "/runs",
        alice,
        Some(&json!({"workflowId": wf, "input": {"victim": true}})),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let run = run["runId"].as_str().expect("run id").to_owned();
    // A request to publish of the victim's (the namespace asks for no approval: born approved).
    let (status, request) = send(
        &api,
        "POST",
        &format!("/workflows/{wf}/publication-requests"),
        alice,
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{request}");
    let request = request["id"].as_str().expect("request id").to_owned();
    for (method, path, body) in [
        (
            "PUT",
            format!("/namespaces/{ns}/secrets/stripe-key"),
            Some(json!({"value": "victim-secret-value-0001"})),
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/secrets/stripe-key/access/dave"),
            None,
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/secrets/stripe-key/pools"),
            Some(json!({"pools": ["victim-pool"]})),
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/worker-bindings/worker-7"),
            Some(json!({"activities": ["payments.*"], "pools": ["victim-pool"]})),
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/egress-policy"),
            Some(json!({"rules": ["partner.example.com"]})),
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/quotas/active_runs"),
            Some(json!({"limit": 77})),
        ),
        (
            "PUT",
            format!("/namespaces/{ns}/activity-quotas/payment.capture"),
            Some(json!({"limit": 3})),
        ),
        (
            "PUT",
            "/quotas/workflows".to_owned(),
            Some(json!({"limit": 88})),
        ),
    ] {
        let (status, body) = send(&api, method, &path, alice, body.as_ref()).await;
        assert!(
            status == 200 || status == 204,
            "{method} {path}: {status} {body}"
        );
    }

    // Scripts of every kind the victim can hold: a draft, a version being built, a test run.
    let (status, script) = send(
        &api,
        "POST",
        &format!("/namespaces/{ns}/scripts"),
        alice,
        Some(&json!({"name": "victim-script", "language": "typescript", "source": "export default () => 'victim-source'"})),
    )
    .await;
    assert_eq!(status, 201, "{script}");
    let script = script["id"].as_str().expect("script id").to_owned();
    let (status, version) = send(
        &api,
        "POST",
        &format!("/namespaces/{ns}/scripts/{script}/versions"),
        alice,
        None,
    )
    .await;
    assert_eq!(status, 202, "{version}");
    let (status, test_run) = send(
        &api,
        "POST",
        &format!("/namespaces/{ns}/scripts/{script}/test-runs"),
        alice,
        Some(&json!({"input": {"victim": true}})),
    )
    .await;
    assert_eq!(status, 202, "{test_run}");
    let test_run = test_run["id"].as_str().expect("test run id").to_owned();

    let victim = Victim {
        ids: BTreeMap::from([
            ("scriptId", script.clone()),
            ("testRunId", test_run.clone()),
            ("executionId", Uuid::now_v7().to_string()),
            ("jobId", Uuid::now_v7().to_string()),
            ("digest", format!("sha256:{}", "ab".repeat(32))),
            ("workspaceId", ws.clone()),
            ("namespaceId", ns.clone()),
            ("workflowId", wf.clone()),
            ("runId", run.clone()),
            ("requestId", request.clone()),
            ("versionNumber", "1".to_owned()),
            ("signalName", "payment.confirmed".to_owned()),
            ("resource", "workflows".to_owned()),
            ("activity", "payment.capture".to_owned()),
            ("subject", "worker-7".to_owned()),
            ("secretName", "stripe-key".to_owned()),
        ]),
    };
    let before = snapshot(&api, &victim).await;
    let victim_ids: Vec<&String> = vec![&ws, &ns, &wf, &run, &script, &test_run, &request];

    // Operations nobody has built yet are listed by name, so they cannot be forgotten later.
    let not_built: BTreeSet<&str> = ["publishEvent", "receiveSignedWebhook"]
        .into_iter()
        .collect();
    let bob = ("bob", "globex");
    let mut attacked = BTreeSet::new();
    let mut addressed = 0;

    for (method, template, operation, implemented) in spec.all_operations() {
        if !implemented {
            assert!(
                not_built.contains(operation.as_str()),
                "{operation} is neither implemented nor on the list of operations not built yet"
            );
            continue;
        }
        if matches!(operation.as_str(), "getHealth" | "getReadiness") {
            continue; // probes: no tenant, no data
        }
        let incident = operation == "recordScriptRunnerIncident";
        let names_a_victim_resource = template
            .split('/')
            .filter_map(|part| part.strip_prefix('{').and_then(|p| p.strip_suffix('}')))
            .any(is_resource_id)
            || matches!(
                operation.as_str(),
                "startRun" | "recordScriptRunnerIncident"
            );
        let body = body_of(&operation).map(|b| {
            // A body that names the victim's workflow (startRun) or namespace (a report from a
            // runner) names it by its real id.
            serde_json::from_str(
                &b.to_string()
                    .replace("{workflowId}", &wf)
                    .replace("{namespaceId}", &ns)
                    .replace("{runId}", &run),
            )
            .expect("json")
        });

        let real = victim.fill(&template, true);
        let attacker = if incident {
            ("runner-bob", "globex")
        } else {
            bob
        };
        let (status, answer) = send(&api, &method, &real, attacker, body.as_ref()).await;

        if names_a_victim_resource {
            addressed += 1;
            let absent_status = if incident { 409 } else { 404 };
            assert_eq!(
                status, absent_status,
                "{operation} {method} {template}: {answer}"
            );
            // The same call against ids that exist nowhere gets the same answer.
            let fake = victim.fill(&template, false);
            let fake_body = body.as_ref().map(|b| {
                serde_json::from_str(
                    &b.to_string()
                        .replace(wf.as_str(), &Uuid::now_v7().to_string())
                        .replace(ns.as_str(), &Uuid::now_v7().to_string())
                        .replace(run.as_str(), &Uuid::now_v7().to_string()),
                )
                .expect("json")
            });
            let (fake_status, fake_answer) =
                send(&api, &method, &fake, attacker, fake_body.as_ref()).await;
            assert_eq!(fake_status, absent_status, "{operation} (baseline)");
            assert_eq!(
                shape(&answer),
                shape(&fake_answer),
                "{operation}: a resource of another tenant must look exactly like a missing one"
            );
        } else {
            // Not addressed at a foreign resource: it acts on the attacker's own tenant, or lists
            // it. Whatever it answers, it must not mention the victim.
            assert!(
                status < 500,
                "{operation} {method} {template} failed: {status} {answer}"
            );
            let text = answer.to_string();
            for id in &victim_ids {
                assert!(
                    !text.contains(id.as_str()),
                    "{operation} leaked an identifier of the victim: {text}"
                );
            }
            for needle in [
                "victim-flow",
                "victim-secret-value",
                "partner.example.com",
                "worker-7",
                "victim-pool",
                "victim-script",
                "victim-source",
            ] {
                assert!(
                    !text.contains(needle),
                    "{operation} leaked {needle}: {text}"
                );
            }
        }
        attacked.insert(operation);
    }
    assert!(
        addressed >= 25,
        "the battery attacked only {addressed} addressed operations"
    );
    let expected = spec
        .all_operations()
        .iter()
        .filter(|(_, _, id, implemented)| {
            *implemented && !matches!(id.as_str(), "getHealth" | "getReadiness")
        })
        .count();
    assert_eq!(
        attacked.len(),
        expected,
        "every implemented operation was attacked"
    );

    // The victim's data did not move.
    assert_eq!(snapshot(&api, &victim).await, before);

    // The attempts are on record against the attacker's tenant and never against the victim's.
    let globex: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
         WHERE t.slug = 'globex' AND a.action = 'access.denied.cross_tenant'",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("count");
    assert!(globex >= 1, "no cross-tenant attempt was recorded");
    let acme_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
         WHERE t.slug = 'acme' AND a.action = 'access.denied.cross_tenant'",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("count");
    assert_eq!(acme_denials, 0);
}

#[tokio::test]
async fn without_a_valid_token_no_operation_answers_anything_but_401() {
    let spec = OpenApi::load();
    let api = start_with(Options {
        blobs: Some(sagat_application::blobs::BlobLimits::default()),
        ..Options::default()
    })
    .await;
    for (method, template, operation, implemented) in spec.all_operations() {
        if !implemented || matches!(operation.as_str(), "getHealth" | "getReadiness") {
            continue;
        }
        let path = template
            .replace("{workspaceId}", "x")
            .replace("{namespaceId}", &Uuid::now_v7().to_string())
            .replace("{workflowId}", "x")
            .replace("{runId}", "x")
            .replace("{versionNumber}", "1")
            .replace("{signalName}", "s")
            .replace("{resource}", "workflows")
            .replace("{subject}", "s")
            .replace("{secretName}", "k");
        for (what, token) in [("no token", None), ("garbage", Some("not.a.jwt"))] {
            let mut request = api.request(
                Method::from_bytes(method.as_bytes()).expect("method"),
                &path,
            );
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.expect("send");
            assert_eq!(
                response.status().as_u16(),
                401,
                "{operation} with {what} must be refused before anything else"
            );
        }
    }
}
