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
//!
//! Feature 022 (tasks T022-058 and T022-090, release 0.3.0 slice): the victim also holds flow-control keys (a run-level
//! concurrency permit and rate-limit slot, a step-level slot, with a holder and a waiter each) whose raw value is a
//! canary, so the two key listings and the run fields `Run.concurrency`, `Run.rateLimit` and a step's `waitingOn` are
//! attacked over real rows; [`flow_control_keys_stay_in_their_tenant_and_namespace_and_never_show_their_value`] holds
//! the rest of what SC-022-010, SC-022-023 and NFR-022-002 ask of them.

use std::collections::{BTreeMap, BTreeSet};

use reqwest::Method;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::canary::{Canary, found};
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
        // Feature 018: the attacker signals-or-starts under the victim's workflow.
        "signalWithStart" => json!({
            "workflowId": "{workflowId}", "businessId": "battery-probe", "input": {},
            "signal": {"name": "battery.probe", "payload": {}}
        }),
        // Feature 018: the attacker asks for the values of the input of the victim's schedule.
        "discloseScheduleInput" => json!({}),
        "updateWithStart" => json!({
            "workflowId": "{workflowId}", "businessId": "battery-probe", "input": {},
            "update": {"name": "battery.probe", "updateKey": "battery-probe-0001", "args": {}}
        }),
        "operateRun" => json!({"action": "CANCEL", "reason": "cross-tenant battery probe"}),
        // Feature 018: the attacker asks what a reset of the victim's run would do, and resets it.
        "previewReset" => json!({"sequence": 1}),
        "resetRun" => json!({
            "sequence": 1, "reason": "cross-tenant battery probe", "acknowledgeReexecution": true
        }),
        "inspectRunPayload" | "discloseRunPayload" => json!({"source": "input"}),
        "signalRun" => json!({"paid": true}),
        // Feature 014: a chain is addressed by its id; the head the attacker claims to have seen
        // is made up, so the only thing that can refuse the call is that the chain is not theirs.
        "queryRun" => json!({"args": {}}),
        "updateRunState" => json!({"updateKey": "battery-probe-0001", "args": {}}),
        "decideWorkflowApproval" => {
            json!({"decisionKey": "battery-probe-0001", "decision": "APPROVE"})
        }
        "queryExecutionChain" => json!({"args": {}}),
        "updateExecutionChain" => json!({"updateKey": "battery-probe-0001", "args": {}}),
        "signalExecutionChain" => json!({
            "expectedRunId": "01900000-0000-7000-8000-000000000000", "payload": {"paid": true}
        }),
        "cancelExecutionChain" => json!({
            "expectedRunId": "01900000-0000-7000-8000-000000000000",
            "reason": "cross-tenant battery probe"
        }),
        "setTenantQuota" | "setNamespaceQuota" | "setActivityQuota" => json!({"limit": 1}),
        "bindWorker" => json!({"activities": ["*"], "pools": ["attacker-pool"]}),
        "setSecretPools" => json!({"pools": ["attacker-pool"]}),
        "putDatabaseConnection" => {
            json!({"requestId":Uuid::now_v7().to_string(),"expectedRevision":0,"status":"PAUSED","config":{"driver":"postgresql","host":"db.example.com","port":5432,"database":"orders","tls":{"mode":"verify-full"},"secretName":"stripe-key"}})
        }
        "deleteDatabaseConnection" => {
            json!({"requestId":Uuid::now_v7().to_string(),"expectedRevision":1})
        }
        "probeDatabaseConnection" => json!({}),
        "startDatabaseExecution" => json!({
            "mode": "step", "input": {},
            "definition": {
                "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                "metadata": {"name": "attacker-query"}, "entrypoint": "a",
                "nodes": [{"id": "a", "type": "database", "connectionRef": {"name": "orders-db"},
                    "operation": "query", "statement": "SELECT 1"}]
            }
        }),
        "putSecret" => json!({"value": "attacker-supplied-value-0001"}),
        "migrateSecretToOpenBao" => {
            json!({"requestId": Uuid::now_v7().to_string(), "expectedVersion": 1})
        }
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
        // Feature 018: an administrator of another tenant writing or pausing the victim's schedules.
        "createSchedule" => json!({
            "name": "attacker-schedule", "spec": {"intervals": [{"everyMs": 3_600_000}]},
            "action": {"workflowId": "{workflowId}", "input": {"attacker": true}}
        }),
        "updateSchedule" => json!({
            "name": "victim-schedule", "spec": {"intervals": [{"everyMs": 3_600_000}]},
            "action": {"workflowId": "{workflowId}", "input": {"attacker": true}},
            "expectedRevision": 1
        }),
        "pauseSchedule" | "resumeSchedule" => json!({"note": "cross-tenant battery probe"}),
        "triggerSchedule" => json!({"overlap": "ALLOW_ALL"}),
        "createScheduleBackfill" => {
            json!({"start": "2026-10-01T00:00:00Z", "end": "2026-10-02T00:00:00Z"})
        }
        // Feature 018: an administrator of another tenant batching, previewing or cancelling over
        // the victim's runs.
        "createBatchOperation" => json!({
            "selector": {"filter": {}}, "action": "CANCEL",
            "reason": "cross-tenant battery probe"
        }),
        "previewBatchOperation" => json!({"selector": {"filter": {}}}),
        "cancelBatchOperation" => json!({"reason": "cross-tenant battery probe"}),
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

/// Held for their whole run by the two tests of this file that drive the flow-control gates, so that they never run
/// together. `tracing` caches, per call site, whether any subscriber wants it: a call site of the gates that one test
/// reaches while the other is installing its capture can be cached as wanted by nobody, and the capture then never sees
/// it (`tests/support/logs.rs`; seen once as the runtime role, the `workflow.rate_limit.admit` span missing, decision
/// D022-103). Held, a gate's call site is reached only by the test that holds the lock.
static GATES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
            | "scheduleId"
            | "businessId"
            | "batchId"
            | "resetId"
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
    send_in(api, method, path, who, "payments", body).await
}

/// [`send`] with the namespace header naming `namespace` (a slug of the caller's tenant).
async fn send_in(
    api: &TestApi,
    method: &str,
    path: &str,
    who: (&str, &str),
    namespace: &str,
    body: Option<&Value>,
) -> (u16, Value) {
    let mut request = api
        .as_user(
            Method::from_bytes(method.as_bytes()).expect("method"),
            path,
            who.0,
            who.1,
        )
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", format!("key-{}", Uuid::now_v7()));
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Everything the victim can read about its own data, to compare before and after; `extra` adds paths of its own.
async fn snapshot(api: &TestApi, victim: &Victim, extra: &[String]) -> Value {
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
        format!("/namespaces/{ns}/schedules"),
        format!("/namespaces/{ns}/batch-operations"),
        format!(
            "/namespaces/{ns}/batch-operations/{}",
            victim.ids["batchId"]
        ),
        format!(
            "/namespaces/{ns}/batch-operations/{}/items",
            victim.ids["batchId"]
        ),
        format!(
            "/namespaces/{ns}/scripts/{}/versions",
            victim.ids["scriptId"]
        ),
        format!(
            "/namespaces/{ns}/scripts/{}/test-runs",
            victim.ids["scriptId"]
        ),
    ]
    .into_iter()
    .chain(extra.iter().cloned())
    {
        let (status, body) = send(api, "GET", &path, ("alice", "acme"), None).await;
        assert_eq!(status, 200, "{path}: {body}");
        out.insert(path, body);
    }
    json!(out)
}

#[tokio::test]
async fn every_implemented_operation_resists_an_administrator_of_another_tenant() {
    let _gates = GATES.lock().await;
    let spec = OpenApi::load();
    // flow control on (feature 022), so that the victim's keys are real rows the attacker reads past
    let api = start_with(Options {
        blobs: Some(sagat_application::blobs::BlobLimits::default()),
        flow_control: true,
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
        Some(&json!({
            "workflowId": wf, "input": {"victim": true},
            "businessId": "victim-order-4471"
        })),
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

    let (status, schedule) = send(
        &api,
        "POST",
        &format!("/namespaces/{ns}/schedules"),
        alice,
        Some(&json!({
            "name": "victim-schedule", "spec": {"intervals": [{"everyMs": 3_600_000}]},
            "action": {"workflowId": wf, "input": {"victim": true}}
        })),
    )
    .await;
    assert_eq!(status, 201, "{schedule}");
    let schedule = schedule["scheduleId"]
        .as_str()
        .expect("schedule id")
        .to_owned();

    let (status, batch) = send(
        &api,
        "POST",
        &format!("/namespaces/{ns}/batch-operations"),
        alice,
        Some(&json!({
            "selector": {"filter": {}}, "action": "CANCEL", "reason": "the victim's own batch"
        })),
    )
    .await;
    assert_eq!(status, 202, "{batch}");
    let batch = batch["batchId"].as_str().expect("batch id").to_owned();

    // Feature 018: a reset the victim made from a run of their own that ended, so the register has an
    // entry to be asked for.
    let (status, ended) = send(
        &api,
        "POST",
        &format!("/runs/{run}/operations"),
        alice,
        Some(&json!({"action": "CANCEL", "reason": "the victim ends its own run"})),
    )
    .await;
    assert!(status == 200 || status == 202, "{status} {ended}");
    let (status, reset) = send(
        &api,
        "POST",
        &format!("/runs/{run}/resets"),
        alice,
        Some(&json!({
            "sequence": 1, "reason": "the victim resets its own run", "acknowledgeReexecution": true
        })),
    )
    .await;
    assert_eq!(status, 202, "{reset}");
    let reset = reset["resetId"].as_str().expect("reset id").to_owned();

    // Feature 022: keys whose raw value is a canary, each with a holder and a waiter, at run and at step level.
    let raw_key = Canary::unique("battery", "raw-key");
    let gated = publish_in(
        &api,
        alice,
        "payments",
        "victim-gated",
        gated_flow("victim-gated"),
    )
    .await;
    let gated_step = publish_in(
        &api,
        alice,
        "payments",
        "victim-gated-step",
        gated_step_flow("victim-gated-step"),
    )
    .await;
    let mut gated_runs = Vec::new();
    for workflow in [&gated, &gated, &gated_step, &gated_step] {
        gated_runs.push(start_in(&api, alice, "payments", workflow, raw_key.value()).await);
    }
    let victim_labels = labels_of(&api, &ns).await;
    // a key's digest is its gate's, its namespace's and its value's: the step's slot shares the run's label
    assert_eq!(
        victim_labels.len(),
        2,
        "one key of each gate: {victim_labels:?}"
    );

    let victim = Victim {
        ids: BTreeMap::from([
            ("resetId", reset.clone()),
            ("scheduleId", schedule.clone()),
            ("businessId", "victim-order-4471".to_owned()),
            ("batchId", batch.clone()),
            ("backfillId", Uuid::now_v7().to_string()),
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
            ("connectionName", "orders-db".to_owned()),
            ("nodeId", "a".to_owned()),
        ]),
    };
    // the key listings, and the step waiting for its slot (the waiting runs' own reads are left out: a place read at
    // read time may be left out when it cannot be read in time, D022-068 item 2, and would make the comparison flaky)
    let flow_control_reads = [
        format!("/namespaces/{ns}/concurrency-keys"),
        format!("/namespaces/{ns}/rate-limit-keys"),
        format!("/runs/{}", gated_runs[0]),
        format!("/runs/{}/graph", gated_runs[3]),
    ];
    let before = snapshot(&api, &victim, &flow_control_reads).await;
    let (_, waiting_step) = send(&api, "GET", &flow_control_reads[3], alice, None).await;
    assert!(
        waiting_step["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|n| n["waitingOn"] == "rate_limit")),
        "the victim's step waits for its slot: {waiting_step}"
    );
    let mut victim_ids: Vec<&String> =
        vec![&ws, &ns, &wf, &run, &script, &test_run, &request, &schedule];
    victim_ids.extend([&gated, &gated_step]);
    victim_ids.extend(gated_runs.iter());

    // Operations nobody has built yet are listed by name, so they cannot be forgotten later.
    let not_built: BTreeSet<&str> = [
        "publishEvent",
        "receiveSignedWebhook",
        // Feature 035 merged its contract before its routes (`x-planned: '035'`); each leaves this list when its route
        // lands, and the battery then attacks it like any other (so it also needs its `body_of` entry).
        "startCatalogBrowse",
        "getCatalogBrowse",
        // Feature 041 merged its contract before its routes (`x-planned: '041'`, T041-3); each leaves this list in the task
        // that serves it (T041-11, T041-12, T041-30).
        "listRunLogs",
        "listRunLogStreams",
        "streamRunLogs",
        "deleteRunLogs",
    ]
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
                "startRun" | "signalWithStart" | "updateWithStart" | "recordScriptRunnerIncident"
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

        // Deleting a schedule names the revision it expects, in the query.
        let query = if operation == "deleteSchedule" {
            "?expectedRevision=1"
        } else {
            ""
        };
        let real = format!("{}{query}", victim.fill(&template, true));
        let attacker = if incident {
            ("runner-bob", "globex")
        } else {
            bob
        };
        let (status, answer) = send(&api, &method, &real, attacker, body.as_ref()).await;
        // feature 022: no answer, addressed or not, carries the raw value of the victim's key
        if let Some(class) = found(answer.to_string(), &[&raw_key]).first() {
            panic!("{operation} leaked the {class} canary");
        }

        if names_a_victim_resource {
            addressed += 1;
            let absent_status = if incident { 409 } else { 404 };
            assert_eq!(
                status, absent_status,
                "{operation} {method} {template}: {answer}"
            );
            // The same call against ids that exist nowhere gets the same answer.
            let fake = format!("{}{query}", victim.fill(&template, false));
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
                "victim-gated",
                "victim-secret-value",
                "partner.example.com",
                "worker-7",
                "victim-pool",
                "victim-script",
                "victim-source",
                "victim-order-4471",
            ] {
                assert!(
                    !text.contains(needle),
                    "{operation} leaked {needle}: {text}"
                );
            }
            // feature 022: a key's label does not cross to the attacker either
            for label in &victim_labels {
                assert!(
                    !text.contains(label.as_str()),
                    "{operation} leaked the victim's key label {label}: {text}"
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
    assert_eq!(snapshot(&api, &victim, &flow_control_reads).await, before);

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

// Feature 022, flow control (tasks T022-058 and T022-090, release 0.3.0 slice).

/// A document whose start takes a concurrency permit and a rate-limit slot of `input.accountId`, one of each per key (an
/// hour's window, buffered for two hours: nothing ends while the test reads).
fn gated_flow(name: &str) -> Value {
    let mut doc = flow(name);
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc["rateLimit"] = json!({
        "group": "accounts", "key": "input.accountId", "limit": 1, "per": "hour",
        "queueTimeoutMs": 7_200_000
    });
    doc
}

/// A document whose one step takes a rate-limit slot of `input.accountId`, one per key and hour.
fn gated_step_flow(name: &str) -> Value {
    let mut doc = flow(name);
    doc["nodes"][0]["rateLimit"] =
        json!({"group": "provider", "key": "input.accountId", "limit": 1, "per": "hour"});
    doc
}

/// Creates and publishes `document` as `name` in the namespace `namespace` (a slug) of `who`'s tenant; its id.
async fn publish_in(
    api: &TestApi,
    who: (&str, &str),
    namespace: &str,
    name: &str,
    document: Value,
) -> String {
    let body = json!({"name": name, "ir": document});
    let (status, created) = send_in(api, "POST", "/workflows", who, namespace, Some(&body)).await;
    assert_eq!(status, 201, "{created}");
    let workflow = created["id"].as_str().expect("id").to_owned();
    let (status, published) = send_in(
        api,
        "POST",
        &format!("/workflows/{workflow}/publish"),
        who,
        namespace,
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{published}");
    workflow
}

/// Starts `workflow` in `namespace` (a slug) with `accountId` as its key's raw value; the run's id.
async fn start_in(
    api: &TestApi,
    who: (&str, &str),
    namespace: &str,
    workflow: &str,
    account: &str,
) -> String {
    let body = json!({"workflowId": workflow, "input": {"accountId": account}});
    let (status, started) = send_in(api, "POST", "/runs", who, namespace, Some(&body)).await;
    assert_eq!(status, 202, "{started}");
    started["runId"].as_str().expect("run id").to_owned()
}

/// The labels of the concurrency and rate-limit keys `namespace` files, from their rows' digests (the first six bytes,
/// as `KeyLabel::of_digest` shows them).
async fn labels_of(api: &TestApi, namespace: &str) -> BTreeSet<String> {
    labels_in(api, namespace, "concurrency_semaphores")
        .await
        .union(&labels_in(api, namespace, "rate_limit_windows").await)
        .cloned()
        .collect()
}

/// The labels of the key rows of `table` (`concurrency_semaphores` or `rate_limit_windows`) in `namespace`.
async fn labels_in(api: &TestApi, namespace: &str, table: &'static str) -> BTreeSet<String> {
    let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT encode(substr(key_digest, 1, 6), 'hex') FROM {table} WHERE namespace_id = $1::uuid"
    )))
    .bind(namespace)
    .fetch_all(&api.db.pool)
    .await
    .expect("key rows");
    rows.into_iter().collect()
}

/// What a surface carried that it must not, collected so that one run of the test names every surface at fault (a hand
/// mutation that plants a key in several places turns each of them red at once). A canary is named by its class only,
/// never by its text.
#[derive(Default)]
struct Leaks(Vec<String>);

impl Leaks {
    fn raw(&mut self, surface: &str, bytes: impl AsRef<[u8]>, raw: &[&Canary]) {
        for class in found(bytes, raw) {
            self.0.push(format!("{surface}: the {class} canary"));
        }
    }

    fn labels(&mut self, surface: &str, text: &str, labels: &BTreeSet<String>) {
        for label in labels {
            if text.contains(label.as_str()) {
                self.0.push(format!("{surface}: the key label {label}"));
            }
        }
    }
}

/// Reads `path` as `who` with the namespace header `namespace`; the body's raw-key canaries go to `leaks`.
async fn read_checked(
    api: &TestApi,
    leaks: &mut Leaks,
    raw: &[&Canary],
    path: &str,
    who: (&str, &str),
    namespace: &str,
) -> (u16, Value) {
    let (status, body) = send_in(api, "GET", path, who, namespace, None).await;
    leaks.raw(
        &format!("body of GET {path} as {}", who.0),
        body.to_string(),
        raw,
    );
    (status, body)
}

/// The labels a key-listing page shows in `items` (its `view: KEYS` rows).
fn listed_labels(page: &Value) -> BTreeSet<String> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["keyLabel"].as_str().expect("a label").to_owned())
        .collect()
}

/// Holds `body` to the canonical contract's `200` schema of `GET template`.
fn conforms(spec: &OpenApi, template: &str, body: &Value) {
    spec.validate_body("GET", template, 200, body)
        .expect("a documented JSON body")
        .unwrap_or_else(|errors| panic!("GET {template} against the contract: {errors}\n{body}"));
}

/// Feature 022 (FR-022-022, FR-022-048, FR-022-057; SC-022-010, SC-022-023; NFR-022-002; decisions D022-059, D022-061,
/// D022-086): the two key listings and the run fields of flow control, attacked from every side the battery knows.
///
/// - Another tenant: both listings of the victim's namespace answer `404` exactly as a namespace that does not exist,
///   with a group filter or a page size too; the victim's runs, their graph included, are `404`.
/// - Another namespace of the same tenant: a listing shows its own keys and queued runs only, and the same raw key in
///   two namespaces, or two tenants, files keys whose labels are unrelated.
/// - A caller without `ops:read` (a worker, a runner) is `403`.
/// - A caller who may not reveal personal data in the namespace (a viewer; a developer without the grant) reads `view:
///   GROUPS` with no key row, `keyLabel: null` in `Run.concurrency` and `Run.rateLimit` of both run reads, and no
///   estimate of another run's grant (`estimatedGrantAt: null`).
/// - A cursor is sealed for its namespace of its tenant and for its listing: one of another namespace, of another
///   tenant (in both directions) or of the other listing is a `400`, and a cursor of the victim's sent by the attacker
///   to the victim's namespace answers what it answers for a namespace that does not exist.
/// - The raw value of a key (a canary) is in no body any caller reads, no log line, no span, no metric series, no audit
///   record and no journal event of the gates; a key's label is in no log line, span or series either. The capture is
///   proven to see the gates' path (their span names, their series): a canary that cannot fail proves nothing.
///
/// The grant pass of the scheduler is not driven here: it reads keys by their digests and never holds a raw value (it is
/// evaluated at admission only, which every start below goes through).
#[tokio::test]
async fn flow_control_keys_stay_in_their_tenant_and_namespace_and_never_show_their_value() {
    let _gates = GATES.lock().await;
    // the process's recorder, installed before the gates record a decision
    let telemetry = sagat_telemetry::Metrics::global();
    let spec = OpenApi::load();
    let api = start_with(Options {
        flow_control: true,
        ..Options::default()
    })
    .await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    for (who, role) in [
        ("vera", Role::Viewer),
        ("dan", Role::Developer),
        ("wally", Role::Worker),
        ("rita", Role::Runner),
    ] {
        api.owner_store
            .insert_membership(&Membership::restore(acme, subject(who), role, now()))
            .await
            .expect("membership");
    }
    let (alice, bob) = (("alice", "acme"), ("bob", "globex"));
    let (vera, dan) = (("vera", "acme"), ("dan", "acme"));
    let workspace = api.create_workspace("alice", "acme", "platform").await;
    let ns_a = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let ns_b = api
        .create_namespace("alice", "acme", &workspace, "refunds", "dev")
        .await;
    let globex_workspace = api.create_workspace("bob", "globex", "platform").await;
    let ns_g = api
        .create_namespace("bob", "globex", &globex_workspace, "payments", "dev")
        .await;
    // each administrator may reveal personal data in their namespaces (feature 009's grant); nobody else may
    for (namespace, who, slug) in [
        (&ns_a, alice, "payments"),
        (&ns_b, alice, "refunds"),
        (&ns_g, bob, "payments"),
    ] {
        let (status, body) = send_in(
            &api,
            "PUT",
            &format!("/namespaces/{namespace}/payload-reveal-grants/{}", who.0),
            who,
            slug,
            None,
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }
    let gated_a = publish_in(&api, alice, "payments", "gated", gated_flow("gated")).await;
    let step_a = publish_in(
        &api,
        alice,
        "payments",
        "gated-step",
        gated_step_flow("gated-step"),
    )
    .await;
    let gated_b = publish_in(&api, alice, "refunds", "gated", gated_flow("gated")).await;
    let gated_g = publish_in(&api, bob, "payments", "gated", gated_flow("gated")).await;

    let key = Canary::unique("flow-control", "raw-key");
    let other_key = Canary::unique("flow-control", "second-raw-key");
    let raw = [&key, &other_key];
    let mut leaks = Leaks::default();

    // from here on every log line and span is captured: the starts evaluate and file the keys
    let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    let holder_a = start_in(&api, alice, "payments", &gated_a, key.value()).await;
    let waiter_a = start_in(&api, alice, "payments", &gated_a, key.value()).await;
    start_in(&api, alice, "payments", &gated_a, other_key.value()).await;
    start_in(&api, alice, "payments", &step_a, key.value()).await;
    let step_waiter_a = start_in(&api, alice, "payments", &step_a, key.value()).await;
    for _ in 0..2 {
        start_in(&api, alice, "refunds", &gated_b, key.value()).await;
        start_in(&api, bob, "payments", &gated_g, key.value()).await;
    }
    start_in(&api, bob, "payments", &gated_g, other_key.value()).await;
    let runs_a = [&holder_a, &waiter_a, &step_waiter_a];

    let (labels_a, labels_b, labels_g) = (
        labels_of(&api, &ns_a).await,
        labels_of(&api, &ns_b).await,
        labels_of(&api, &ns_g).await,
    );
    // two keys of each gate in `payments` of acme (a key's digest is its gate's, namespace's and value's, not its
    // group's or level's: the step's slot shares the run's label) and of globex, one of each in `refunds`
    assert_eq!((labels_a.len(), labels_b.len(), labels_g.len()), (4, 2, 4));
    // the same raw value files unrelated keys in another namespace and in another tenant
    assert!(
        labels_a.is_disjoint(&labels_b)
            && labels_a.is_disjoint(&labels_g)
            && labels_b.is_disjoint(&labels_g),
        "{labels_a:?} {labels_b:?} {labels_g:?}"
    );
    let all_labels: BTreeSet<String> = labels_a
        .iter()
        .chain(&labels_b)
        .chain(&labels_g)
        .cloned()
        .collect();
    let listings = ["concurrency-keys", "rate-limit-keys"];

    // Another tenant: the victim's namespace is a namespace that does not exist, whatever the query.
    for listing in listings {
        for query in ["", "?limit=1", "?group=accounts"] {
            let real = format!("/namespaces/{ns_a}/{listing}{query}");
            let (status, answer) =
                read_checked(&api, &mut leaks, &raw, &real, bob, "payments").await;
            let fake = format!("/namespaces/{}/{listing}{query}", Uuid::now_v7());
            let (fake_status, fake_answer) =
                read_checked(&api, &mut leaks, &raw, &fake, bob, "payments").await;
            assert_eq!((status, fake_status), (404, 404), "{real}: {answer}");
            assert_eq!(shape(&answer), shape(&fake_answer), "{real}");
        }
    }
    for run in runs_a {
        for path in [format!("/runs/{run}"), format!("/runs/{run}/graph")] {
            let (status, answer) =
                read_checked(&api, &mut leaks, &raw, &path, bob, "payments").await;
            assert_eq!(status, 404, "{path}: {answer}");
        }
    }
    // what the attacker reads of its own tenant names nothing of the victim's
    let mut own = Vec::new();
    for path in [
        "/runs".to_owned(),
        format!("/namespaces/{ns_g}/concurrency-keys"),
        format!("/namespaces/{ns_g}/rate-limit-keys"),
    ] {
        let (status, body) = read_checked(&api, &mut leaks, &raw, &path, bob, "payments").await;
        assert_eq!(status, 200, "{path}: {body}");
        let text = body.to_string();
        for id in runs_a.iter().copied().chain([&ns_a, &gated_a]) {
            assert!(!text.contains(id.as_str()), "{path} names {id}: {text}");
        }
        leaks.labels(&format!("body of GET {path} as bob"), &text, &labels_a);
        own.push(body);
    }
    assert_eq!(
        listed_labels(&own[1])
            .union(&listed_labels(&own[2]))
            .cloned()
            .collect::<BTreeSet<_>>(),
        labels_g,
        "the attacker's administrator lists its own keys"
    );

    // A caller without `ops:read`.
    for who in [("wally", "acme"), ("rita", "acme")] {
        for listing in listings {
            let path = format!("/namespaces/{ns_a}/{listing}");
            let (status, answer) =
                read_checked(&api, &mut leaks, &raw, &path, who, "payments").await;
            assert_eq!(status, 403, "{} {path}: {answer}", who.0);
        }
    }

    // The administrator, who may reveal personal data: the keys by label, each listing its own gate's keys.
    let (status, keys_a) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/namespaces/{ns_a}/concurrency-keys"),
        alice,
        "payments",
    )
    .await;
    assert_eq!(status, 200, "{keys_a}");
    conforms(&spec, "/namespaces/{namespaceId}/concurrency-keys", &keys_a);
    assert_eq!(keys_a["view"], "KEYS", "{keys_a}");
    assert_eq!(
        listed_labels(&keys_a),
        labels_in(&api, &ns_a, "concurrency_semaphores").await
    );
    let (status, rate_a) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/namespaces/{ns_a}/rate-limit-keys"),
        alice,
        "payments",
    )
    .await;
    assert_eq!(status, 200, "{rate_a}");
    conforms(&spec, "/namespaces/{namespaceId}/rate-limit-keys", &rate_a);
    assert_eq!(
        listed_labels(&rate_a),
        labels_in(&api, &ns_a, "rate_limit_windows").await
    );
    // another namespace of the tenant: its own keys and its own queued runs only
    let (status, keys_b) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/namespaces/{ns_b}/concurrency-keys"),
        alice,
        "refunds",
    )
    .await;
    assert_eq!(status, 200, "{keys_b}");
    assert_eq!(
        listed_labels(&keys_b),
        labels_in(&api, &ns_b, "concurrency_semaphores").await
    );
    assert_eq!(
        (keys_a["queuedRuns"].as_u64(), keys_b["queuedRuns"].as_u64()),
        (Some(1), Some(1)),
        "a run queued at its start in each: {keys_a} {keys_b}"
    );
    let (status, run) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/runs/{holder_a}"),
        alice,
        "payments",
    )
    .await;
    assert_eq!(status, 200, "{run}");
    conforms(&spec, "/runs/{runId}", &run);
    for field in ["concurrency", "rateLimit"] {
        let label = run[field]["keyLabel"].as_str().unwrap_or_default();
        assert!(labels_a.contains(label), "{field}: {run}");
    }
    let (status, graph) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/runs/{step_waiter_a}/graph"),
        alice,
        "payments",
    )
    .await;
    assert_eq!(status, 200, "{graph}");
    assert!(
        graph["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|n| n["waitingOn"] == "rate_limit")),
        "the step waits for its slot: {graph}"
    );

    // A caller who may not reveal personal data: groups, null labels, no estimate. What would show the key is a leak,
    // collected with the others; the shape of what they read is held after the report.
    let mut hidden = Vec::new();
    for who in [vera, dan] {
        let mut reads = BTreeMap::new();
        for path in [
            format!("/namespaces/{ns_a}/concurrency-keys"),
            format!("/namespaces/{ns_a}/rate-limit-keys"),
            format!("/runs/{holder_a}"),
            format!("/runs/{waiter_a}"),
            format!("/runs/{step_waiter_a}/graph"),
            "/runs".to_owned(),
        ] {
            let (status, body) = read_checked(&api, &mut leaks, &raw, &path, who, "payments").await;
            assert_eq!(status, 200, "{} {path}: {body}", who.0);
            leaks.labels(
                &format!("body of GET {path} as {}", who.0),
                &body.to_string(),
                &all_labels,
            );
            reads.insert(path, body);
        }
        hidden.push((who.0, reads));
    }

    // Cursors: sealed for one namespace of one tenant and for one listing.
    let first_page =
        |listing: &str, namespace: &str| format!("/namespaces/{namespace}/{listing}?limit=1");
    let cursor_of = |body: &Value| body["nextCursor"].as_str().expect("a next page").to_owned();
    let (_, page) = send(
        &api,
        "GET",
        &first_page("concurrency-keys", &ns_a),
        alice,
        None,
    )
    .await;
    let victim_cursor = cursor_of(&page);
    let (_, page) = send(
        &api,
        "GET",
        &first_page("rate-limit-keys", &ns_a),
        alice,
        None,
    )
    .await;
    let victim_rate_cursor = cursor_of(&page);
    let (_, page) = send(
        &api,
        "GET",
        &first_page("concurrency-keys", &ns_g),
        bob,
        None,
    )
    .await;
    let attacker_cursor = cursor_of(&page);
    let (status, next) = read_checked(
        &api,
        &mut leaks,
        &raw,
        &format!("/namespaces/{ns_a}/concurrency-keys?limit=1&cursor={victim_cursor}"),
        alice,
        "payments",
    )
    .await;
    assert_eq!(status, 200, "the cursor reads the next page: {next}");
    assert!(listed_labels(&next).is_subset(&labels_a), "{next}");
    let refused = [
        // another namespace of the same tenant
        (
            alice,
            ns_b.clone(),
            "concurrency-keys",
            victim_cursor.clone(),
            "refunds",
        ),
        (
            alice,
            ns_b.clone(),
            "rate-limit-keys",
            victim_rate_cursor.clone(),
            "refunds",
        ),
        // the other listing
        (
            alice,
            ns_a.clone(),
            "rate-limit-keys",
            victim_cursor.clone(),
            "payments",
        ),
        // another tenant's, in both directions, each to its own namespace
        (
            bob,
            ns_g.clone(),
            "concurrency-keys",
            victim_cursor.clone(),
            "payments",
        ),
        (
            bob,
            ns_g.clone(),
            "rate-limit-keys",
            victim_rate_cursor.clone(),
            "payments",
        ),
        (
            alice,
            ns_a.clone(),
            "concurrency-keys",
            attacker_cursor.clone(),
            "payments",
        ),
    ];
    for (who, namespace, listing, cursor, slug) in refused {
        let path = format!("/namespaces/{namespace}/{listing}?cursor={cursor}");
        let (status, answer) = read_checked(&api, &mut leaks, &raw, &path, who, slug).await;
        assert_eq!(status, 400, "{} {listing} of {namespace}: {answer}", who.0);
    }
    // another tenant's cursor sent to the namespace it was sealed for: what a namespace that does not exist answers
    for (who, namespace, cursor) in [
        (bob, &ns_a, &victim_cursor),
        (alice, &ns_g, &attacker_cursor),
    ] {
        let real = format!("/namespaces/{namespace}/concurrency-keys?cursor={cursor}");
        let fake = format!(
            "/namespaces/{}/concurrency-keys?cursor={cursor}",
            Uuid::now_v7()
        );
        let (status, answer) = read_checked(&api, &mut leaks, &raw, &real, who, "payments").await;
        let (fake_status, fake_answer) =
            read_checked(&api, &mut leaks, &raw, &fake, who, "payments").await;
        assert_eq!(
            (status, fake_status),
            (400, 400),
            "{} {real}: {answer}",
            who.0
        );
        assert_eq!(shape(&answer), shape(&fake_answer), "{} {real}", who.0);
    }

    // a waiter's cancellation dequeues it under the capture too
    let (status, cancelled) = send(
        &api,
        "POST",
        &format!("/runs/{waiter_a}/operations"),
        alice,
        Some(&json!({"action": "CANCEL", "reason": "the battery dequeues a waiter"})),
    )
    .await;
    assert!(status == 200 || status == 202, "{status} {cancelled}");
    drop(guard);

    // Logs and spans: a span's line is its opening or its closing; every other line is a log event.
    let text = captured.text();
    for span in ["workflow.concurrency.acquire", "workflow.rate_limit.admit"] {
        assert!(
            text.contains(span),
            "the capture saw no {span} span: the canary below would prove nothing"
        );
    }
    let (spans, events): (Vec<&str>, Vec<&str>) = text
        .lines()
        .partition(|line| line.ends_with(": new") || line.contains(": close time."));
    for (surface, lines) in [("spans", spans), ("logs", events)] {
        let joined = lines.join("\n");
        leaks.raw(surface, &joined, &raw);
        leaks.labels(surface, &joined, &all_labels);
    }
    // Metrics: the process's page, the gates' series on it.
    let page = telemetry.render();
    for series in [
        "concurrency_acquisitions_total",
        "rate_limit_decisions_total",
    ] {
        assert!(
            page.contains(series),
            "{series} is not on the page: the canary below would prove nothing"
        );
    }
    leaks.raw("metrics", &page, &raw);
    leaks.labels("metrics", &page, &all_labels);
    // The audit and the gates' journal events.
    let audit: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(row_to_json(a)::text, E'\\n'), '') FROM audit_records a",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("audit");
    leaks.raw("audit", &audit, &raw);
    let (gate_events, journal): (i64, String) = sqlx::query_as(
        "SELECT count(*), coalesce(string_agg(detail::text, E'\\n'), '') FROM run_events \
          WHERE event_type LIKE 'concurrency.%' OR event_type LIKE 'rate_limit.%'",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("journal");
    assert!(
        gate_events >= 10,
        "the gates wrote their events: {gate_events}"
    );
    leaks.raw("journal", &journal, &raw);

    assert!(
        leaks.0.is_empty(),
        "raw keys or labels where they must not be:\n{}",
        leaks.0.join("\n")
    );

    // What a caller without the reveal right read: the groups, never a key, and no estimate of another run's grant.
    for (who, reads) in &hidden {
        for listing in listings {
            let page = &reads[&format!("/namespaces/{ns_a}/{listing}")];
            assert_eq!(page["view"], "GROUPS", "{who}: {page}");
            assert_eq!(page["items"], json!([]), "{who}: {page}");
            assert!(
                page["groups"]
                    .as_array()
                    .is_some_and(|groups| !groups.is_empty()
                        && groups
                            .iter()
                            .all(|g| g.get("keyLabel").is_none() && g.get("keys").is_none())),
                "{who}: {page}"
            );
        }
        for run in [&holder_a, &waiter_a] {
            let body = &reads[&format!("/runs/{run}")];
            conforms(&spec, "/runs/{runId}", body);
            for field in ["concurrency", "rateLimit"] {
                assert!(body[field].is_object(), "{who} {field}: {body}");
                assert_eq!(
                    body[field]["keyLabel"],
                    Value::Null,
                    "{who} {field}: {body}"
                );
            }
        }
        let waiting = &reads[&format!("/runs/{waiter_a}")];
        assert_eq!(waiting["rateLimit"]["state"], "WAITING", "{who}: {waiting}");
        assert_eq!(
            waiting["rateLimit"]["estimatedGrantAt"],
            Value::Null,
            "{who}: {waiting}"
        );
        let listed = reads["/runs"]["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter(|item| runs_a.iter().any(|run| item["runId"] == run.as_str()))
            .flat_map(|item| [item["concurrency"].clone(), item["rateLimit"].clone()])
            .filter(Value::is_object)
            .collect::<Vec<_>>();
        assert!(
            !listed.is_empty() && listed.iter().all(|gate| gate["keyLabel"].is_null()),
            "{who}: {listed:?}"
        );
    }
}
