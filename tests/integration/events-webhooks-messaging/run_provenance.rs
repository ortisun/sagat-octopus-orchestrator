//! `FR-018-046`, `G2057`: the webhook of a finished run says which schedule started it and for which nominal time, and the webhook
//! of every other run is what it was (the same properties, nothing added). The business identifier of a run is caller text and is
//! in no event (ADR-0027): not in the body a subscriber is sent, not in the hint of the outbox, **also for a run a schedule
//! started**, which is the branch that adds properties to the body (a schedule's action cannot name an identifier yet, G1955, so the
//! run that carries one in this test is stamped as a scheduled one: the leak this test names can only happen on that branch).

use sagat_application::runs::{BusinessStart, StartOptions, StartRun};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::outbound::subscribe;
use crate::support::engine::{World, activity, ctx, document};
use crate::support::receiver::Receiver;
use crate::support::schedules::{fixture, hourly, mark_as_scheduled, t0};
use crate::{SECRET, SECRET_NAME};

/// The worker of the world completes the one task of a one-activity run, which then ends.
async fn work_done(w: &World) {
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
}

/// A run started under the business identifier `canary`, which is checked to be in the run: a search for it anywhere else means
/// something only if it is somewhere.
async fn started_under(w: &World, workflow: WorkflowId, canary: &str) -> RunId {
    let run = w
        .runs
        .start_with_options(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key: &format!("provenance-{}", uuid::Uuid::now_v7()),
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(canary).expect("an identifier"),
                    conflict: ConflictPolicy::Fail,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                ..StartOptions::default()
            },
            &ctx(),
        )
        .await
        .expect("the start")
        .run
        .info
        .run_id;
    let kept: Option<String> = sqlx::query_scalar("SELECT business_id FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the business id of the run");
    assert_eq!(
        kept.as_deref(),
        Some(canary),
        "the canary is in the run, so a search for it elsewhere means something"
    );
    run
}

#[tokio::test]
async fn the_webhook_of_a_run_a_schedule_started_names_the_schedule_and_the_nominal_time_and_never_the_business_id()
 {
    let f = fixture().await;
    let w = &f.w;
    w.put_secret(SECRET_NAME, SECRET).await;
    let receiver = Receiver::start().await;
    subscribe(w, "provenance", &receiver).await;
    let (workflow, _) = w
        .publish(document("provenance", "work", vec![activity("work")]))
        .await;
    let schedule = f
        .create(
            "hourly",
            &hourly(),
            &json!({}),
            Some(json!({"workflowId": workflow.to_string()})),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(3));
    f.drain(&schedule).await;
    let scheduled = f.runs(&schedule).await;
    assert_eq!(scheduled.len(), 1);
    work_done(w).await;

    // a run with a business identifier that no schedule started: customer text that must be in no event
    let canary = format!("canary-order-{}", uuid::Uuid::now_v7().simple());
    let plain = started_under(w, workflow, &canary).await;
    work_done(w).await;

    // and a run with a business identifier that a schedule started (stamped: the identifier a schedule's action cannot yet name),
    // so that the branch that adds `scheduleId` and `nominalTime` to the body is the one the canary goes through
    let scheduled_canary = format!("canary-scheduled-{}", uuid::Uuid::now_v7().simple());
    let marked = started_under(w, workflow, &scheduled_canary).await;
    let marked_by = uuid::Uuid::now_v7();
    mark_as_scheduled(
        &w.db.pool,
        marked.as_uuid(),
        marked_by,
        t0() + Duration::hours(2),
    )
    .await;
    work_done(w).await;
    w.deliveries.tick().await;

    let bodies: Vec<Value> = receiver
        .seen()
        .iter()
        .map(|seen| serde_json::from_slice(&seen.body).expect("a JSON body"))
        .collect();
    assert_eq!(bodies.len(), 3, "{bodies:?}");
    let body_of = |run: String| {
        bodies
            .iter()
            .find(|body| body["data"]["runId"] == run.as_str())
            .unwrap_or_else(|| panic!("no webhook for {run}: {bodies:?}"))
    };
    let tenant = w.admin.tenant_id().to_string();
    let namespace = w.namespace.id().to_string();

    let told = body_of(scheduled[0].to_string());
    assert_eq!(
        told["data"],
        json!({
            "tenantId": tenant, "namespaceId": namespace, "runId": scheduled[0].to_string(),
            "workflow": "provenance", "status": "SUCCEEDED", "traceId": told["data"]["traceId"],
            "scheduleId": schedule.id.to_string(),
            "nominalTime": "2026-10-28T13:00:00Z",
        }),
        "{told}"
    );
    assert_eq!(
        t0() + Duration::hours(1),
        OffsetDateTime::parse(
            told["data"]["nominalTime"].as_str().expect("text"),
            &Rfc3339
        )
        .expect("RFC 3339"),
    );

    // the run that has both: the schedule and the identifier. The body says the first and not the second, key for key.
    let both = body_of(marked.to_string());
    assert_eq!(
        both["data"],
        json!({
            "tenantId": tenant, "namespaceId": namespace, "runId": marked.to_string(),
            "workflow": "provenance", "status": "SUCCEEDED", "traceId": both["data"]["traceId"],
            "scheduleId": marked_by.to_string(),
            "nominalTime": "2026-10-28T14:00:00Z",
        }),
        "a run that a schedule started and that has a business identifier says the schedule and not the identifier: {both}"
    );

    let other = body_of(plain.to_string());
    assert_eq!(
        other["data"],
        json!({
            "tenantId": tenant, "namespaceId": namespace, "runId": plain.to_string(),
            "workflow": "provenance", "status": "SUCCEEDED", "traceId": other["data"]["traceId"],
        }),
        "the webhook of a run nobody scheduled is what it was: no `scheduleId`, no `nominalTime`, not even null: {other}"
    );

    // the customer text of a run is in no body and in no hint, whichever branch the run went through
    for seen in receiver.seen() {
        let text = String::from_utf8(seen.body).expect("UTF-8");
        for canary in [&canary, &scheduled_canary] {
            assert!(
                !text.contains(canary.as_str()),
                "the business identifier is in a webhook: {text}"
            );
        }
        assert!(!text.contains("businessId"), "{text}");
    }
    let hints: Vec<String> = sqlx::query_scalar(
        "SELECT payload::text FROM event_outbox WHERE event_type = 'run.finished' ORDER BY id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("the hints");
    assert_eq!(hints.len(), 3);
    for hint in hints {
        for canary in [&canary, &scheduled_canary] {
            assert!(!hint.contains(canary.as_str()), "{hint}");
        }
        assert!(
            !hint.contains("scheduleId") && !hint.contains("nominalTime"),
            "the hint on the broker is unchanged: {hint}"
        );
    }
}
