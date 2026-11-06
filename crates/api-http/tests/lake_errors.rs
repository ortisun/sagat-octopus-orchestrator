//! The refusals of an Apache Iceberg write at publication, as the API words them (feature 035): each has a `reason` a
//! client can branch on, the one the contract names (`contracts/errors.md`), and the node that caused it when there is
//! one to name.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_api_http::error::ApiError;
use sagat_application::definitions::IrError;
use sagat_application::error::AppError;
use sagat_domain::classification::Level;
use sagat_domain::database::{EgressRefusal, LakeRefusal, WriteMode};
use sagat_domain::workflow::WorkflowError;
use serde_json::json;

fn details_of(error: IrError) -> serde_json::Value {
    let api = ApiError::from(AppError::InvalidDefinition(Box::new(error)));
    api.parts()
        .details
        .clone()
        .expect("a refused definition says why")
}

fn refused(refusal: LakeRefusal) -> serde_json::Value {
    details_of(IrError::LakeWriteRefused {
        node: "save".to_owned(),
        refusal,
    })
}

#[test]
fn a_lake_that_is_off_names_its_reason_and_the_node_that_needs_it() {
    let details = details_of(IrError::LakeDisabled {
        node: "save".to_owned(),
    });
    assert_eq!(details["reason"], "LAKE_DISABLED", "{details}");
    assert_eq!(details["node"], "save", "{details}");
}

#[test]
fn an_iceberg_write_outside_the_root_graph_names_its_reason() {
    let details = details_of(IrError::Invalid(WorkflowError::IcebergNodeNotRoot));
    assert_eq!(details["reason"], "ICEBERG_NODE_NOT_ROOT", "{details}");
}

#[test]
fn a_malformed_iceberg_write_names_its_reason() {
    let details = details_of(IrError::Invalid(WorkflowError::InvalidIcebergNode));
    assert_eq!(details["reason"], "INVALID_ICEBERG_NODE", "{details}");
}

#[test]
fn a_mode_the_connection_does_not_allow_names_the_node_and_the_mode() {
    let details = refused(LakeRefusal::ModeNotAllowed(WriteMode::Append));
    assert_eq!(
        details,
        json!({"reason": "MODE_NOT_ALLOWED", "node": "save", "mode": "append"})
    );
}

#[test]
fn fields_above_the_ceiling_are_listed_by_path_and_class_and_never_by_value() {
    let details = refused(LakeRefusal::Egress(vec![
        EgressRefusal::Field {
            path: "/email".to_owned(),
            level: Level::Pii,
        },
        EgressRefusal::Field {
            path: "/token".to_owned(),
            level: Level::Secret,
        },
    ]));
    assert_eq!(
        details,
        json!({"reason": "CLASSIFICATION_EGRESS_REFUSED", "node": "save", "fields": [
            {"path": "/email", "level": "pii"},
            {"path": "/token", "level": "secret"}
        ]})
    );
}

#[test]
fn a_step_nobody_classified_says_so_instead_of_listing_a_field() {
    let details = refused(LakeRefusal::Egress(vec![EgressRefusal::NotComputed]));
    assert_eq!(
        details,
        json!({"reason": "CLASSIFICATION_EGRESS_REFUSED", "node": "save", "fields": [],
            "unclassified": true})
    );
}

#[test]
fn a_long_list_of_refused_fields_is_cut_and_says_how_many_were_left_out() {
    let many = (0..30)
        .map(|n| EgressRefusal::Field {
            path: format!("/f{n}"),
            level: Level::Pii,
        })
        .collect();
    let details = refused(LakeRefusal::Egress(many));
    assert_eq!(details["fields"].as_array().expect("fields").len(), 20);
    assert_eq!(details["omitted"], 10, "{details}");
}
