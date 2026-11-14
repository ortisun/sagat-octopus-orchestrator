//! Live conformance of what a run says about the schedule that started it (`G2057`, `FR-018-016`, `FR-018-046`): `getRun` and
//! `listRuns` answer, for a run a schedule started, with the `schedule` object the `Run` schema declares, and the values are
//! the ones the action recorded. A scheduled time is acted on for real (the service the schedule pass calls, over the stores of
//! the API), a trigger-now through the API, and the answers are held to the document.

use reqwest::Method;
use sagat_application::schedule_firing::Fired;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::support::api::start;
use crate::support::contract::{As, OpenApi, Scenario, check_body, scenario};
use crate::support::schedules::fire_at;

fn keyed(s: Scenario, n: u32) -> Scenario {
    s.with_header("idempotency-key", format!("provenance-key-{n:04}"))
        .with_header("x-octopus-namespace", "payments")
}

fn namespaced(s: Scenario) -> Scenario {
    s.with_header("x-octopus-namespace", "payments")
}

fn instant(text: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(text.as_str().expect("a string"), &Rfc3339).expect("RFC 3339")
}

#[tokio::test]
async fn the_runs_of_a_schedule_conform_to_the_run_schema_with_the_provenance_the_action_recorded()
{
    let spec = OpenApi::load();
    let api = start().await;
    let tenant = api.tenant("acme", "alice").await;
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
        .header("idempotency-key", "provenance-setup-0001")
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
        .header("idempotency-key", "provenance-setup-0002")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);

    let alice = As::User("alice", "acme");
    let list = format!("/namespaces/{ns}/schedules");
    let (_, _, _, schedule) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                "/namespaces/{namespaceId}/schedules",
                &list,
                alice.clone(),
                Some(json!({
                    "name": "hourly",
                    "spec": {"intervals": [{"everyMs": 3_600_000}]},
                    "action": {"workflowId": workflow, "input": {"region": "eu"}}
                })),
                201,
            ),
            1,
        ),
    )
    .await;
    let id = schedule["scheduleId"].as_str().expect("id").to_owned();
    let one = format!("{list}/{id}");

    // a scheduled time, acted on for real under a fixed clock: one period before the first time the schedule calculated, which is
    // passed through to the run (what `fire_at` proves), and the run and its action agree to the microsecond
    let next = instant(&schedule["nextActionAt"]);
    let nominal = next - Duration::hours(1);
    assert_eq!(fire_at(&api, tenant, &id, nominal).await, Fired::Acted);
    let (_, _, _, actions) = check_body(
        &api,
        &spec,
        &namespaced(scenario(
            "GET",
            "/namespaces/{namespaceId}/schedules/{scheduleId}/actions",
            &format!("{one}/actions"),
            alice.clone(),
            None,
            200,
        )),
    )
    .await;
    let action = &actions["items"][0];
    assert_eq!(action["outcome"], "STARTED");
    let run = action["runId"].as_str().expect("a run").to_owned();
    let (_, _, _, read) = check_body(
        &api,
        &spec,
        &namespaced(scenario(
            "GET",
            "/runs/{runId}",
            &format!("/runs/{run}"),
            alice.clone(),
            None,
            200,
        )),
    )
    .await;
    assert_eq!(
        read["schedule"],
        json!({
            "scheduleId": id,
            "nominalTime": action["nominalTime"],
            "kind": "SCHEDULED",
            "revision": 1
        }),
        "{read}"
    );
    assert_eq!(
        instant(&read["schedule"]["nominalTime"]),
        nominal,
        "the nominal time of the action, passed through to the run, exact"
    );
    assert_eq!(read["startedBy"], format!("system:schedule:{id}"));

    // trigger-now through the API: another kind, the same shape; the run of the list is the run of the read
    let (_, _, _, triggered) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                "/namespaces/{namespaceId}/schedules/{scheduleId}/trigger",
                &format!("{one}/trigger"),
                alice.clone(),
                Some(json!({"overlap": "ALLOW_ALL"})),
                202,
            ),
            2,
        ),
    )
    .await;
    let triggered_run = triggered["runId"].as_str().expect("a run").to_owned();
    let (_, _, _, page) = check_body(
        &api,
        &spec,
        &namespaced(scenario(
            "GET",
            "/runs",
            &format!("/runs?scheduleId={id}"),
            alice.clone(),
            None,
            200,
        )),
    )
    .await;
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "{page}");
    let of = |run: &str| {
        items
            .iter()
            .find(|item| item["runId"] == run)
            .unwrap_or_else(|| panic!("{run} is not in {page}"))
    };
    assert_eq!(of(&run)["schedule"], read["schedule"]);
    assert_eq!(of(&triggered_run)["schedule"]["kind"], "TRIGGERED");
    assert_eq!(
        instant(&of(&triggered_run)["schedule"]["nominalTime"]),
        instant(&triggered["nominalTime"]),
        "trigger-now: the nominal time of the run is that of its action"
    );
    assert_eq!(of(&triggered_run)["schedule"]["revision"], 1);

    // a run nobody scheduled has no such object
    let (_, _, _, plain) = check_body(
        &api,
        &spec,
        &keyed(
            scenario(
                "POST",
                "/runs",
                "/runs",
                alice.clone(),
                Some(json!({"workflowId": workflow, "input": {}})),
                202,
            ),
            3,
        ),
    )
    .await;
    assert!(
        plain.get("schedule").is_none() && plain.get("resetFrom").is_none(),
        "{plain}"
    );
}
