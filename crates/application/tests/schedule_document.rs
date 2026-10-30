//! The document a caller sends to create or replace a schedule (`T018-018`): what it is decoded to,
//! and where a refusal points.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::schedules::{ScheduleDocument, decode_document};
use sagat_domain::schedules::Ceilings;
use serde_json::{Value, json};

fn document() -> ScheduleDocument {
    ScheduleDocument {
        name: "nightly-report".to_owned(),
        spec: json!({"intervals": [{"everyMs": 3_600_000}]}),
        policies: None,
        action: json!({
            "workflowId": "0194f5e2-7b1c-7000-8000-000000000001",
            "input": {"region": "eu"}
        }),
        paused: false,
        note: None,
        remaining_actions: None,
    }
}

fn refused(document: ScheduleDocument) -> (&'static str, String) {
    match decode_document(document, &Ceilings::default()) {
        Err(AppError::InvalidSchedule { reason, field }) => (reason, field.unwrap_or_default()),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_whole_document_is_decoded_and_the_input_leaves_the_action() {
    let mut doc = document();
    doc.policies = Some(json!({"overlap": "BUFFER_ONE"}));
    doc.note = Some("created by the nightly job".to_owned());
    doc.remaining_actions = Some(5);
    doc.paused = true;
    let write = decode_document(doc, &Ceilings::default()).expect("decoded");
    assert_eq!(write.name.as_str(), "nightly-report");
    assert_eq!(write.input, json!({"region": "eu"}));
    assert!(write.paused);
    assert_eq!(write.remaining_actions.map(|n| n.get()), Some(5));
    assert_eq!(
        write.note.as_ref().map(|n| n.as_str()),
        Some("created by the nightly job")
    );
}

#[test]
fn policies_are_optional() {
    let write = decode_document(document(), &Ceilings::default()).expect("decoded");
    assert_eq!(
        sagat_application::schedule_codec::policies_to_json(&write.policies),
        sagat_application::schedule_codec::policies_to_json(
            &sagat_application::schedule_codec::policies_from_json(
                &json!({}),
                &Ceilings::default()
            )
            .expect("defaults")
        )
    );
}

#[test]
fn a_refusal_points_at_the_field_with_a_json_pointer() {
    let mut doc = document();
    doc.spec = json!({"calendars": [{"hour": [{"start": 99}]}]});
    let (reason, field) = refused(doc);
    assert_eq!(reason, "SCHEDULE_SPEC_INVALID");
    assert!(field.starts_with("/spec/calendars/0"), "{field}");

    let mut doc = document();
    doc.spec = json!({"intervals": [{"everyMs": 3_600_000}], "nonsense": true});
    assert_eq!(
        refused(doc),
        ("SCHEDULE_SPEC_INVALID", "/spec/nonsense".to_owned())
    );

    let mut doc = document();
    doc.policies = Some(json!({"overlap": "SOMETIMES"}));
    assert_eq!(
        refused(doc),
        ("SCHEDULE_SPEC_INVALID", "/policies/overlap".to_owned())
    );

    let mut doc = document();
    doc.action = json!({"input": {}});
    assert_eq!(
        refused(doc),
        ("SCHEDULE_SPEC_INVALID", "/action/workflowId".to_owned())
    );
}

#[test]
fn the_name_the_note_the_count_and_the_input_are_checked() {
    for (mutate, expected) in [
        (
            (|d: &mut ScheduleDocument| d.name = "Not Valid".to_owned())
                as fn(&mut ScheduleDocument),
            ("SCHEDULE_SPEC_INVALID", "/name"),
        ),
        (
            |d| d.note = Some("x".repeat(513)),
            ("SCHEDULE_SPEC_INVALID", "/note"),
        ),
        (
            |d| d.remaining_actions = Some(0),
            ("SCHEDULE_SPEC_INVALID", "/remainingActions"),
        ),
        (
            |d| d.action = json!({"workflowId": "0194f5e2-7b1c-7000-8000-000000000001"}),
            ("SCHEDULE_INPUT_INVALID", "/action/input"),
        ),
        (
            |d| d.action = Value::String("nope".to_owned()),
            ("SCHEDULE_SPEC_INVALID", "/action"),
        ),
    ] {
        let mut doc = document();
        mutate(&mut doc);
        let (reason, field) = refused(doc);
        assert_eq!((reason, field.as_str()), expected);
    }
}
