//! The input of a schedule comes out only through an audited, classified read (`T018-033`,
//! `FR-018-042`): the values are projected under the classification of the input schema of the version
//! the schedule would start now, for the audiences of feature 016, and nothing leaves before the record
//! of the read exists. Real PostgreSQL, the real service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::classification::{ClassifiedNode, ScalarValue};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::payload_disclosure::{
    PayloadDisclosureError, PayloadDisclosureResult, StructureOnlyReason,
    UnavailableDisclosureReason,
};
use sagat_application::schedule_disclosure::{
    ScheduleInputDisclosure, ScheduleInputDisclosureService, ScheduleInputDisclosureUseCase,
};
use sagat_application::schedules::ScheduleRecord;
use sagat_domain::identity::{NamespaceId, Principal, Role};
use sagat_infra_ir::JsonPayloadProjector;
use sagat_infra_postgres::PgScheduleStore;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{ctx, document, end_node};
use crate::support::schedules::{Fixture, fixture, hourly};

type Service =
    ScheduleInputDisclosureService<PgScheduleStore, SystemClock, Classifier, JsonPayloadProjector>;

fn service(f: &Fixture) -> Service {
    ScheduleInputDisclosureService::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SystemClock,
        Classifier::default(),
        JsonPayloadProjector,
    )
}

async fn disclose_in(
    f: &Fixture,
    who: &Principal,
    namespace: NamespaceId,
    schedule: &ScheduleRecord,
    reveal: bool,
) -> Result<ScheduleInputDisclosure, PayloadDisclosureError> {
    service(f)
        .disclose_input(who, namespace, schedule.id, reveal, &ctx())
        .await
}

async fn disclose(
    f: &Fixture,
    who: &Principal,
    schedule: &ScheduleRecord,
    reveal: bool,
) -> Result<ScheduleInputDisclosure, PayloadDisclosureError> {
    disclose_in(f, who, f.w.namespace.id(), schedule, reveal).await
}

const CANARIES: [&str; 3] = ["staff", "pii-canary", "secret-canary"];

/// A schedule whose target classifies each field of its input at another level.
async fn classified_schedule(f: &Fixture) -> ScheduleRecord {
    let mut definition = document(
        &format!("classified-input-{}", Uuid::now_v7()),
        "done",
        vec![end_node("done")],
    );
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
    let (workflow, _) = f.w.publish(definition).await;
    let mut write = f.write_of(
        "classified",
        &hourly(),
        &json!({}),
        Some(json!({"workflowId": workflow.to_string()})),
    );
    write.input = json!({
        "open": "shown", "inside": "staff", "personal": "pii-canary", "sealed": "secret-canary"
    });
    f.try_create(write).await.expect("create the schedule")
}

fn shown<'a>(disclosure: &'a ScheduleInputDisclosure, name: &str) -> Option<&'a str> {
    let PayloadDisclosureResult::Disclosed { projection, .. } = &disclosure.result else {
        panic!("a projection was expected: {:?}", disclosure.result)
    };
    let ClassifiedNode::Container { children, .. } = &projection.tree else {
        return None;
    };
    children.iter().find_map(|child| {
        if child.name.as_deref() != Some(name) {
            return None;
        }
        match &*child.node {
            ClassifiedNode::Value {
                value: ScalarValue::String(value),
                ..
            } => Some(value.as_str()),
            _ => None,
        }
    })
}

/// The audit records of `action` about the schedule by `actor`.
async fn audits(f: &Fixture, schedule: &ScheduleRecord, actor: &Principal, action: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND actor = $2 \
         AND resource_id = $3 AND action = $4",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(actor.subject().as_str())
    .bind(schedule.id.to_string())
    .bind(action)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit count")
}

async fn grant_reveal(f: &Fixture, who: &Principal) {
    sqlx::query(
        "INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) \
         VALUES ($1,$2,$3,$4,clock_timestamp())",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(who.subject().as_str())
    .bind(f.w.admin.subject().as_str())
    .execute(&f.w.db.pool)
    .await
    .expect("grant");
}

#[tokio::test]
async fn each_level_is_shown_to_the_audience_that_may_see_it_and_no_other() {
    let f = fixture().await;
    let schedule = classified_schedule(&f).await;
    let viewer = f.w.member("input-viewer", Role::Viewer).await;
    let developer = f.w.member("input-developer", Role::Developer).await;

    // a reader sees what is public, and nothing of the rest, not even in a debug print of the answer
    let read = disclose(&f, &viewer, &schedule, false)
        .await
        .expect("a viewer reads");
    assert_eq!(shown(&read, "open"), Some("shown"));
    for hidden in ["inside", "personal", "sealed"] {
        assert_eq!(shown(&read, hidden), None, "{hidden}");
    }
    assert_eq!(read.revision, 1);
    let printed = format!("{read:?}");
    for canary in CANARIES {
        assert!(!printed.contains(canary), "{canary} leaked: {printed}");
    }

    // an author who may read internal values sees them, and personal data and secrets stay out
    let read = disclose(&f, &developer, &schedule, false)
        .await
        .expect("a developer reads");
    assert_eq!(shown(&read, "open"), Some("shown"));
    assert_eq!(shown(&read, "inside"), Some("staff"));
    for hidden in ["personal", "sealed"] {
        assert_eq!(shown(&read, hidden), None, "{hidden}");
    }
    let printed = format!("{read:?}");
    for canary in &CANARIES[1..] {
        assert!(!printed.contains(canary), "{canary} leaked");
    }
}

#[tokio::test]
async fn the_version_the_schedule_would_start_now_decides_the_classification() {
    let f = fixture().await;
    let viewer = f.w.member("version-viewer", Role::Viewer).await;
    let with_note = |level: &str| {
        let mut definition = document("versioned-input", "done", vec![end_node("done")]);
        definition["inputSchema"] = json!({
            "type": "object", "additionalProperties": false,
            "x-sagat-classification": "public",
            "properties": {"note": {"type": "string", "x-sagat-classification": level}}
        });
        definition
    };
    // the first version keeps the note to the staff, the second (the active one) opens it
    let (workflow, first) = f.w.publish(with_note("internal")).await;
    f.w.publish_new_version(workflow, with_note("public")).await;
    let schedule = |name: &str, target: Value| {
        let mut write = f.write_of(name, &hourly(), &json!({}), Some(target));
        write.input = json!({"note": "for the staff"});
        write
    };
    let following = f
        .try_create(schedule(
            "follows-the-active-version",
            json!({"workflowId": workflow.to_string()}),
        ))
        .await
        .expect("create");
    let pinned = f
        .try_create(schedule(
            "pinned-to-the-first",
            json!({
                "workflowId": workflow.to_string(),
                "versionPolicy": "PINNED",
                "versionId": first.to_string()
            }),
        ))
        .await
        .expect("create");

    let read = disclose(&f, &viewer, &following, false)
        .await
        .expect("active");
    assert_eq!(shown(&read, "note"), Some("for the staff"), "public now");
    let read = disclose(&f, &viewer, &pinned, false).await.expect("pinned");
    assert_eq!(
        shown(&read, "note"),
        None,
        "internal in the version it pins"
    );
    assert!(!format!("{read:?}").contains("for the staff"));
}

#[tokio::test]
async fn personal_data_needs_the_reveal_and_the_grant_and_a_secret_never_comes_out() {
    let f = fixture().await;
    let schedule = classified_schedule(&f).await;
    let developer = f.w.member("pii-developer", Role::Developer).await;

    // asked for without a grant: refused, and the refusal is on record
    let refused = disclose(&f, &developer, &schedule, true).await;
    assert_eq!(refused, Err(PayloadDisclosureError::PiiRevealNotGranted));
    assert_eq!(
        audits(&f, &schedule, &developer, "payload.reveal_denied").await,
        1
    );

    grant_reveal(&f, &developer).await;
    for round in 1..=2 {
        let read = disclose(&f, &developer, &schedule, true)
            .await
            .expect("granted reveal");
        assert_eq!(shown(&read, "personal"), Some("pii-canary"));
        assert_eq!(shown(&read, "inside"), Some("staff"));
        assert_eq!(shown(&read, "sealed"), None, "a secret never comes out");
        assert!(!format!("{read:?}").contains("secret-canary"));
        // each reveal is its own record: repeating one is not the same as having done it once
        assert_eq!(
            audits(&f, &schedule, &developer, "payload.pii_revealed").await,
            round,
            "reveal {round}"
        );
    }
}

#[tokio::test]
async fn an_internal_read_is_audited_as_a_disclosure_of_the_schedule_and_a_public_one_is_not() {
    let f = fixture().await;
    let schedule = classified_schedule(&f).await;
    let viewer = f.w.member("quiet-viewer", Role::Viewer).await;
    let developer = f.w.member("audited-developer", Role::Developer).await;

    disclose(&f, &viewer, &schedule, false)
        .await
        .expect("viewer");
    assert_eq!(
        audits(&f, &schedule, &viewer, "schedule.input_disclosed").await,
        0,
        "nothing but public values left: nothing to record"
    );

    disclose(&f, &developer, &schedule, false)
        .await
        .expect("developer");
    disclose(&f, &developer, &schedule, false)
        .await
        .expect("developer again");
    assert_eq!(
        audits(&f, &schedule, &developer, "schedule.input_disclosed").await,
        1,
        "the same read, made again at once, is one record"
    );
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE resource_id = $1 AND action = 'schedule.input_disclosed'",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("reason");
    assert_eq!(reason.as_deref(), Some("classified-v1:schedule-input"));
}

#[tokio::test]
async fn a_schedule_of_another_tenant_or_namespace_or_one_that_is_gone_is_not_found() {
    let f = fixture().await;
    let schedule = classified_schedule(&f).await;
    let elsewhere = f.w.add_namespace("elsewhere").await.id();

    // another tenant, whatever the namespace it names
    let foreign = disclose(&f, &f.w.other_admin, &schedule, false).await;
    assert_eq!(
        foreign.err(),
        Some(PayloadDisclosureError::Application(AppError::NotFound))
    );
    // another namespace of the same tenant
    let wrong = disclose_in(&f, &f.w.admin, elsewhere, &schedule, false).await;
    assert_eq!(
        wrong.err(),
        Some(PayloadDisclosureError::Application(AppError::NotFound))
    );
    // a schedule that never existed
    let mut unknown = schedule.clone();
    unknown.id = sagat_domain::schedules::ScheduleId::generate();
    assert_eq!(
        disclose(&f, &f.w.admin, &unknown, false).await.err(),
        Some(PayloadDisclosureError::Application(AppError::NotFound))
    );
    // and one that was deleted
    f.service
        .delete(&f.w.admin, schedule.id, None, &ctx())
        .await
        .expect("delete");
    assert_eq!(
        disclose(&f, &f.w.admin, &schedule, false).await.err(),
        Some(PayloadDisclosureError::Application(AppError::NotFound))
    );
}

#[tokio::test]
async fn an_input_with_no_schema_to_classify_by_is_structure_for_an_inspector_and_nothing_for_a_reader()
 {
    let f = fixture().await;
    // the workflow of the fixture declares no classification of its input
    let schedule = f.create("plain", &hourly(), &json!({}), None).await;
    let viewer = f.w.member("plain-viewer", Role::Viewer).await;
    let operator = f.w.member("plain-operator", Role::Operator).await;

    let read = disclose(&f, &viewer, &schedule, false)
        .await
        .expect("viewer");
    assert_eq!(
        read.result,
        PayloadDisclosureResult::Unavailable {
            reason: UnavailableDisclosureReason::NotDisclosable,
            size_bytes: None
        },
        "a reader learns neither that there is an input nor its size"
    );

    let read = disclose(&f, &operator, &schedule, false)
        .await
        .expect("operator");
    let PayloadDisclosureResult::StructureOnly { reason, .. } = read.result else {
        panic!("the structure was expected: {:?}", read.result)
    };
    assert_eq!(reason, StructureOnlyReason::NoSchema);
    assert_eq!(
        audits(&f, &schedule, &operator, "schedule.input_disclosed").await,
        1,
        "the structure of an input is recorded as inspected, under the same action"
    );
}
