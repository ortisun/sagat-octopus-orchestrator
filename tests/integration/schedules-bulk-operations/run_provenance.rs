//! What a run says about the schedule that started it and about the reset that made it (`G2057`, `FR-018-016`,
//! `FR-018-037`, `FR-018-046`): the run resource carries an optional `schedule {scheduleId, nominalTime, kind, revision}`
//! and an optional `resetFrom {runId, sequence}`, read from the columns the run was created with, in the query that loads
//! the run (the list included), and ABSENT, not `null`, for every run that has none. A real API, a real PostgreSQL, a real
//! firing of the schedule pass's own service, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_application::schedule_firing::Fired;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::reset_rest::{REASON, ended, points, reset_with, sequences};
use crate::support::schedules::fire_at;
use crate::support::worker::{HUMAN, Platform, TENANT, chain_ir, platform};

/// The instant a JSON string holds, parsed: an instant that went through PostgreSQL is whole microseconds and a clock reads
/// nanoseconds on Linux, so the strings of two instants are never compared, their values are.
fn instant(text: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(text.as_str().expect("a string"), &Rfc3339).expect("RFC 3339")
}

fn keys(object: &Value) -> Vec<&str> {
    let mut names: Vec<&str> = object
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    names
}

struct Rig {
    p: Platform,
    namespace: String,
    workflow: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let namespace = p.namespace_id().await;
    let workflow = p.publish("reports", &chain_ir("reports")).await;
    Rig {
        p,
        namespace,
        workflow,
    }
}

impl Rig {
    fn schedules(&self) -> String {
        format!("/namespaces/{}/schedules", self.namespace)
    }

    fn document(&self, name: &str) -> Value {
        json!({
            "name": name,
            "spec": {"intervals": [{"everyMs": 3_600_000}]},
            "action": {"workflowId": self.workflow, "input": {"region": "eu"}}
        })
    }

    async fn create(&self, name: &str) -> String {
        let (status, created) = self
            .p
            .rest(Method::POST, &self.schedules(), Some(&self.document(name)))
            .await;
        assert_eq!(status, 201, "{created}");
        created["scheduleId"].as_str().expect("id").to_owned()
    }

    /// Trigger-now under a key of the test's choosing and `ALLOW_ALL`, so a second one is not skipped: the action it made.
    async fn trigger(&self, schedule: &str, key: &str) -> Value {
        let response = self
            .p
            .api
            .as_user(
                Method::POST,
                &format!("{}/{schedule}/trigger", self.schedules()),
                HUMAN,
                TENANT,
            )
            .header("idempotency-key", key)
            .json(&json!({"overlap": "ALLOW_ALL"}))
            .send()
            .await
            .expect("send");
        let status = response.status().as_u16();
        let text = response.text().await.expect("body");
        assert_eq!(status, 202, "{text}");
        serde_json::from_str(&text).expect("the action")
    }

    /// The newest action of the schedule, as the actions route lists it.
    async fn last_action(&self, schedule: &str) -> Value {
        let (status, page) = self
            .p
            .rest(
                Method::GET,
                &format!("{}/{schedule}/actions", self.schedules()),
                None,
            )
            .await;
        assert_eq!(status, 200, "{page}");
        page["items"][0].clone()
    }

    async fn runs_of(&self, schedule: &str) -> Value {
        let (status, page) = self
            .p
            .rest(Method::GET, &format!("/runs?scheduleId={schedule}"), None)
            .await;
        assert_eq!(status, 200, "{page}");
        page
    }
}

#[tokio::test]
async fn a_run_a_schedule_started_says_which_schedule_what_time_what_kind_and_what_revision() {
    let r = rig().await;
    let id = r.create("provenance").await;

    // trigger-now: the nominal time is the instant of the action, which the actions route also gives
    let first = r.trigger(&id, "provenance-key-0001").await;
    assert_eq!(first["outcome"], "STARTED");
    let run = first["runId"].as_str().expect("a run").to_owned();
    let one = r.p.run(&run).await;
    assert_eq!(one["startedBy"], format!("system:schedule:{id}"));
    let schedule = &one["schedule"];
    assert_eq!(
        keys(schedule),
        ["kind", "nominalTime", "revision", "scheduleId"],
        "{one}"
    );
    assert_eq!(schedule["scheduleId"], id.as_str());
    assert_eq!(schedule["kind"], "TRIGGERED");
    assert_eq!(schedule["revision"], 1);
    assert_eq!(
        instant(&schedule["nominalTime"]),
        instant(&first["nominalTime"]),
        "the nominal time of the run is the nominal time of its action, to the microsecond"
    );
    assert!(one.get("resetFrom").is_none(), "{one}");

    // the list says the same, from the same query: the same object for the same run
    let page = r.runs_of(&id).await;
    assert_eq!(page["items"].as_array().expect("items").len(), 1, "{page}");
    assert_eq!(page["items"][0]["schedule"], one["schedule"]);
    assert!(page["items"][0].get("resetFrom").is_none());

    // a replacement is revision 2, and the next action says so: the run keeps what it was started under
    let mut replacement = r.document("provenance");
    replacement["expectedRevision"] = json!(1);
    replacement["spec"] = json!({"intervals": [{"everyMs": 7_200_000}]});
    let (status, updated) =
        r.p.rest(
            Method::PUT,
            &format!("{}/{id}", r.schedules()),
            Some(&replacement),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    let second = r.trigger(&id, "provenance-key-0002").await;
    let run_two = second["runId"].as_str().expect("a run").to_owned();
    assert_ne!(run_two, run);
    let two = r.p.run(&run_two).await;
    assert_eq!(two["schedule"]["revision"], 2, "{two}");
    assert_eq!(two["schedule"]["kind"], "TRIGGERED");
    assert_eq!(
        r.p.run(&run).await["schedule"]["revision"],
        1,
        "the first run still says the revision it was started under"
    );
}

#[tokio::test]
async fn a_run_nobody_scheduled_has_neither_a_schedule_nor_a_reset_field() {
    let r = rig().await;
    let id = r.create("other").await;
    r.trigger(&id, "plain-key-0001").await;
    let plain = r.p.start(&r.workflow, &json!({})).await;

    for (what, body) in [
        ("the run read by id", r.p.run(&plain).await),
        (
            "the answer of the start",
            r.p.rest(
                Method::POST,
                "/runs",
                Some(&json!({"workflowId": r.workflow, "input": {}})),
            )
            .await
            .1,
        ),
    ] {
        assert!(body.get("schedule").is_none(), "{what}: {body}");
        assert!(body.get("resetFrom").is_none(), "{what}: {body}");
    }
    // in the list a plain run and a scheduled one sit side by side, and only the scheduled one has the object
    let (status, page) = r.p.rest(Method::GET, "/runs", None).await;
    assert_eq!(status, 200, "{page}");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 3, "{page}");
    let scheduled: Vec<_> = items
        .iter()
        .filter(|item| item.get("schedule").is_some())
        .collect();
    assert_eq!(scheduled.len(), 1, "{page}");
    assert!(items.iter().all(|item| item.get("resetFrom").is_none()));
    assert!(
        items
            .iter()
            .all(|item| item.get("schedule") != Some(&Value::Null)),
        "absent, not null"
    );
}

/// A real firing, at a nominal time the test chooses: one period before the first time the schedule calculated (`nextActionAt`), so
/// that it lies on the schedule's own grid and is already past. The schedule is made due at it (the times it keeps are moved there:
/// what the pass finds) and the service the pass calls acts under a clock fixed five seconds after it, so that no other time of the
/// schedule is due while the test runs. What this proves is that the nominal time is **passed through** to the run, to the
/// microsecond, with the kind and the revision. What the schedule calculates is shown by the two times it gives: the first before
/// the firing (on the hour, after its creation) and the next one after it (the one it calculated before).
#[tokio::test]
async fn a_scheduled_firing_passes_its_nominal_time_kind_and_revision_to_the_run_and_the_schedule_calculates_the_next_time()
 {
    let r = rig().await;
    let (status, created) =
        r.p.rest(Method::POST, &r.schedules(), Some(&r.document("hourly")))
            .await;
    assert_eq!(status, 201, "{created}");
    let id = created["scheduleId"].as_str().expect("id").to_owned();
    // the schedule's own calculation of its first time: on the hour, after it was created
    let next = instant(&created["nextActionAt"]);
    assert_eq!(next.unix_timestamp() % 3_600, 0, "{created}");
    assert!(next > instant(&created["createdAt"]), "{created}");
    let nominal = next - Duration::hours(1);
    assert_eq!(
        fire_at(&r.p.api, r.p.tenant, &id, nominal).await,
        Fired::Acted
    );

    let action = r.last_action(&id).await;
    assert_eq!(action["outcome"], "STARTED");
    assert_eq!(action["kind"], "SCHEDULED");
    assert_eq!(instant(&action["nominalTime"]), nominal);
    let run = action["runId"].as_str().expect("a run");
    let one = r.p.run(run).await;
    let schedule = &one["schedule"];
    assert_eq!(schedule["scheduleId"], id.as_str(), "{one}");
    assert_eq!(schedule["kind"], "SCHEDULED");
    assert_eq!(schedule["revision"], 1);
    assert_eq!(
        instant(&schedule["nominalTime"]),
        nominal,
        "the nominal time of the action, passed through to the run, not the time the firing woke at"
    );
    let (_, after) =
        r.p.rest(Method::GET, &format!("{}/{id}", r.schedules()), None)
            .await;
    assert_eq!(
        instant(&after["nextActionAt"]),
        next,
        "the schedule's own calculation of the time after the one acted on"
    );
    assert_eq!(
        r.runs_of(&id).await["items"][0]["schedule"],
        one["schedule"]
    );
}

#[tokio::test]
async fn a_run_a_reset_made_says_the_run_and_the_sequence_it_came_from_and_a_plain_rerun_does_not()
{
    let p = platform().await;
    let run = ended(&p).await;
    let (_, page) = points(&p, &run, "").await;
    let after_a = sequences(&page)[1];
    let (status, made) = reset_with(
        &p,
        &run,
        "provenance-reset-0001",
        &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 202, "{made}");
    let new_run = made["newRunId"].as_str().expect("the new run").to_owned();

    let created = p.run(&new_run).await;
    assert_eq!(created["rerunOf"], run.as_str(), "{created}");
    assert_eq!(
        created["resetFrom"],
        json!({"runId": run, "sequence": after_a}),
        "{created}"
    );
    assert!(created.get("schedule").is_none(), "{created}");
    // the same in the list
    let (_, listed) = p.rest(Method::GET, "/runs", None).await;
    let item = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["runId"] == new_run.as_str())
        .expect("the run is listed");
    assert_eq!(item["resetFrom"], created["resetFrom"]);
    // the source is not a reset run
    assert!(p.run(&run).await.get("resetFrom").is_none());

    // a rerun as new names its source in `rerunOf` and nothing more: it is not a reset
    let (status, rerun) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "RERUN_AS_NEW", "reason": "start it again after the fix"})),
        )
        .await;
    assert_eq!(status, 202, "{rerun}");
    let again = p.run(rerun["newRunId"].as_str().expect("a run")).await;
    assert_eq!(again["rerunOf"], run.as_str());
    assert!(again.get("resetFrom").is_none(), "{again}");
}
