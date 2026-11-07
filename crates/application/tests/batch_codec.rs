//! The selector and the parameters of a batch job as the store keeps them (`T018-045`): what is
//! written is read back, and a document that is not one of ours is refused instead of guessed at.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::batch::{JobParams, selector_from_json, selector_to_json};
use sagat_domain::batch::{Filter, Selector};
use sagat_domain::execution::RunStatus;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

#[test]
fn a_list_of_run_ids_is_read_back_as_written() {
    let ids = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let selector = Selector::run_ids(ids.clone()).expect("selector");

    let stored = selector_to_json(&selector);

    assert_eq!(stored["runIds"].as_array().map(Vec::len), Some(3));
    assert_eq!(selector_from_json(&stored).expect("read"), selector);
}

#[test]
fn a_filter_with_every_field_is_read_back_as_written() {
    let selector = Selector::filter(Filter {
        workflow_id: Some(Uuid::new_v4()),
        version_id: Some(Uuid::new_v4()),
        statuses: vec![RunStatus::Failed, RunStatus::TimedOut],
        origin: Some("AD_HOC".to_owned()),
        schedule_id: Some(Uuid::new_v4()),
        created_after: Some(at(1_700_000_000)),
        created_before: Some(at(1_800_000_000)),
        business_id_prefix: Some("order-".to_owned()),
    })
    .expect("selector");

    let stored = selector_to_json(&selector);

    assert_eq!(selector_from_json(&stored).expect("read"), selector);
    assert!(stored["filter"].get("status").is_some(), "{stored}");
}

#[test]
fn an_empty_filter_is_written_as_an_empty_object_and_selects_everything() {
    let selector = Selector::filter(Filter::default()).expect("selector");
    let stored = selector_to_json(&selector);
    assert_eq!(stored, json!({"filter": {}}));
    assert_eq!(selector_from_json(&stored).expect("read"), selector);
}

#[test]
fn a_document_that_is_not_a_selector_is_refused() {
    let id = Uuid::new_v4().to_string();
    let cases: Vec<Value> = vec![
        json!({}),
        json!([]),
        json!({"runIds": []}),
        json!({"runIds": [id, id]}),
        json!({"runIds": ["not-a-uuid"]}),
        json!({"runIds": [id], "filter": {}}),
        json!({"filter": {"status": ["NOT_A_STATUS"]}}),
        json!({"filter": {"workflowId": "not-a-uuid"}}),
        json!({"filter": {"createdAfter": "yesterday"}}),
        json!({"filter": {"status": "FAILED"}}),
        json!({"filter": {"createdAfter": "2026-01-02T00:00:00Z", "createdBefore": "2026-01-01T00:00:00Z"}}),
        json!({"query": "status = FAILED"}),
    ];
    for case in cases {
        assert!(selector_from_json(&case).is_err(), "{case}");
    }
}

#[test]
fn the_parameters_of_an_action_are_read_back_as_written_and_nothing_else_is_accepted() {
    let params = JobParams {
        step_id: Some("charge".to_owned()),
        instance_path: vec!["lines[1]".to_owned()],
        name: Some("approve".to_owned()),
        drop_business_id: true,
    };
    assert_eq!(
        JobParams::from_json(&params.to_json()).expect("read"),
        params
    );
    assert_eq!(JobParams::default().to_json(), json!({}));
    assert_eq!(
        JobParams::from_json(&json!({})).expect("read"),
        JobParams::default()
    );
    for case in [
        json!([]),
        json!({"stepId": 3}),
        json!({"instancePath": "lines"}),
        json!({"instancePath": [1]}),
        json!({"unknown": true}),
    ] {
        assert!(JobParams::from_json(&case).is_err(), "{case}");
    }
}
