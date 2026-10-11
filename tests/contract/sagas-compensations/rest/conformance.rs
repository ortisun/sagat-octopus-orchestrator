//! Live conformance of what feature 003 added: `getCompensationPlans`, the new decisions of
//! `operateRun`, the new run statuses and the compensation steps of the runtime graph.

use std::collections::BTreeSet;

use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::contract::{As, OpenApi, Scenario, check, scenario};
use crate::support::worker::platform;

fn saga_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "tx",
        "nodes": [
            {"id": "tx", "type": "saga", "begin": "reserve"},
            {"id": "reserve", "type": "activity", "activity": "limit.reserve",
             "compensateWith": "limit.release", "next": "submit",
             "compensationRetry": {"maxAttempts": 1, "strategy": "fixed",
                                   "initialDelayMs": 100, "maxDelayMs": 100, "jitter": false}},
            {"id": "submit", "type": "activity", "activity": "payment.submit",
             "compensateWith": "payment.return", "reconcileWith": "payment.lookup"}
        ]
    })
}

fn key() -> String {
    format!("key-{}", Uuid::now_v7())
}

#[tokio::test]
async fn the_saga_operations_conform_to_the_contract_on_their_normal_and_error_paths() {
    let spec = OpenApi::load();
    let p = platform().await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    let wf = p.publish("settlement", &saga_ir("settlement")).await;
    let plain = p
        .publish(
            "plain",
            &json!({
                "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                "metadata": {"name": "plain"}, "entrypoint": "a",
                "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
            }),
        )
        .await;

    // A run whose compensation ran out of attempts and waits for a person, reached through the
    // worker API the way a real worker would.
    let run_id = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&[
        "limit.reserve",
        "payment.submit",
        "limit.release",
        "payment.return",
        "payment.lookup",
    ])
    .await;
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task().await;
    w.fail(&submit, false).await.expect("declined");
    let release = w.one_task().await;
    w.fail_with(&release, "PROVIDER_DOWN", true)
        .await
        .expect("down");
    assert_eq!(p.status(&run_id).await, "MANUAL_REVIEW");

    let missing = "019b5d6e-0000-7000-8000-000000000000";
    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let olga = As::User("olga", "acme");
    let bob = As::User("bob", "globex");
    let ns = |s: Scenario| s.with_header("x-octopus-namespace", "payments");
    let mutating = |s: Scenario| ns(s).with_header("idempotency-key", key());
    let plans_t = "/workflows/{workflowId}/versions/{versionNumber}/compensation-plans";
    let ops_t = "/runs/{runId}/operations";
    let run_t = "/runs/{runId}";
    let graph_t = "/runs/{runId}/graph";
    let ops_path = format!("/runs/{run_id}/operations");
    let plans = |id: &str, n: u32| format!("/workflows/{id}/versions/{n}/compensation-plans");

    let scenarios = vec![
        // ---- the plans a version was published with
        scenario("GET", plans_t, &plans(&wf, 1), alice.clone(), None, 200),
        scenario("GET", plans_t, &plans(&wf, 1), vera.clone(), None, 200),
        scenario("GET", plans_t, &plans(&plain, 1), alice.clone(), None, 200),
        scenario("GET", plans_t, &plans(&wf, 9), alice.clone(), None, 404),
        scenario("GET", plans_t, &plans(missing, 1), alice.clone(), None, 404),
        scenario("GET", plans_t, &plans(&wf, 1), bob.clone(), None, 404),
        scenario("GET", plans_t, &plans(&wf, 1), As::Anonymous, None, 401),
        // ---- the run and its graph show what the saga is doing
        ns(scenario(
            "GET",
            run_t,
            &format!("/runs/{run_id}"),
            alice.clone(),
            None,
            200,
        )),
        scenario(
            "GET",
            graph_t,
            &format!("/runs/{run_id}/graph"),
            vera.clone(),
            None,
            200,
        ),
        // ---- decisions on a step that waits for a person
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            olga.clone(),
            Some(
                json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "operators may not give up"}),
            ),
            403,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            vera.clone(),
            Some(
                json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "viewers may not retry"}),
            ),
            403,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            alice.clone(),
            Some(
                json!({"action": "RETRY_COMPENSATION", "stepId": "reserve", "reason": "this is a forward step"}),
            ),
            422,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            alice.clone(),
            Some(
                json!({"action": "RESOLVE_STEP", "stepId": "reserve~c", "reason": "no outcome given here"}),
            ),
            422,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            alice.clone(),
            Some(
                json!({"action": "RETRY_STEP", "stepId": "ghost", "reason": "there is no such step"}),
            ),
            404,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            bob.clone(),
            Some(
                json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "not this tenant's run"}),
            ),
            404,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            olga.clone(),
            Some(
                json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "the provider is back"}),
            ),
            202,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &ops_path,
            olga.clone(),
            Some(
                json!({"action": "RETRY_COMPENSATION", "stepId": "reserve~c", "reason": "asking a second time"}),
            ),
            409,
        )),
    ];
    let mut exercised: BTreeSet<(String, String, u16)> = BTreeSet::new();
    for s in &scenarios {
        exercised.insert(check(&p.api, &spec, s).await);
    }

    // What the feature added is implemented and exercised successfully; what it extended says so.
    let added = spec.operations_marked("x-implemented", "003");
    assert_eq!(added.len(), 1, "{added:?}");
    for (method, template) in &added {
        assert!(
            exercised
                .iter()
                .any(|(m, t, st)| m == method && t == template && *st == 200),
            "{method} {template} has no successful conformance scenario"
        );
    }
    let extended = spec.operations_marked("x-extended", "003");
    assert_eq!(extended.len(), 1, "{extended:?}");
    for (method, template) in &extended {
        assert!(
            exercised
                .iter()
                .any(|(m, t, st)| m == method && t == template && *st == 202),
            "{method} {template} has no successful conformance scenario"
        );
    }
}

#[tokio::test]
async fn every_run_status_the_engine_can_produce_is_documented() {
    // The contract is the list clients switch on: a status the engine can produce and the contract
    // does not name would be an undocumented value in a documented enumeration.
    let spec = OpenApi::load();
    let documented = spec.run_statuses();
    for status in sagat_domain::execution::RunStatus::ALL {
        assert!(
            documented.contains(&status.as_str().to_owned()),
            "{status:?} is not in the contract"
        );
    }
}

#[test]
fn every_way_a_run_can_end_is_in_the_run_finished_event_contract() {
    // `run.finished` is published for every terminal status; a status the event contract does not
    // name would be an undocumented value in a documented enumeration, for every consumer.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specs/001-platform-foundation/contracts/events.asyncapi.yaml"
    );
    let doc: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(path).expect("asyncapi"))
        .expect("valid YAML");
    let documented: Vec<&str> =
        doc["components"]["messages"]["RunFinished"]["payload"]["properties"]["status"]["enum"]
            .as_array()
            .expect("the status is an enumeration")
            .iter()
            .filter_map(Value::as_str)
            .collect();
    for status in sagat_domain::execution::RunStatus::ALL {
        if status.is_terminal() {
            assert!(
                documented.contains(&status.as_str()),
                "{status:?} is not in the event contract"
            );
        }
    }
    for name in &documented {
        assert!(
            sagat_domain::execution::RunStatus::ALL
                .iter()
                .any(|s| s.as_str() == *name && s.is_terminal()),
            "{name} is documented but is not a terminal status"
        );
    }
}
