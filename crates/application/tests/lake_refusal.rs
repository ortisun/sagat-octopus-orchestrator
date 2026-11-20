//! How a refusal of an Apache Iceberg write by the settings of its connection reaches the author (feature 035: T035-054;
//! FR-035-005, FR-035-025).
//!
//! The store finds the connection and compares; the application turns what it says into an invalid definition, with a
//! problem for each thing to fix. A message names a node, a mode, a field path and a class, never a value.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::{
    definitions::{IrError, problems_of},
    error::{AppError, StoreError},
};
use sagat_domain::{
    classification::Level,
    database::{EgressRefusal, LakeRefusal, WriteMode},
};

fn field(path: &str, level: Level) -> EgressRefusal {
    EgressRefusal::Field {
        path: path.to_owned(),
        level,
    }
}

fn refused(refusal: LakeRefusal) -> IrError {
    IrError::LakeWriteRefused {
        node: "save".to_owned(),
        refusal,
    }
}

#[test]
fn a_refusal_by_the_store_reaches_the_api_as_an_invalid_definition_naming_the_node() {
    let refusal = LakeRefusal::ModeNotAllowed(WriteMode::Append);
    let error = AppError::from(StoreError::LakeRefused {
        node: "save".to_owned(),
        refusal: refusal.clone(),
    });
    assert_eq!(
        error,
        AppError::InvalidDefinition(Box::new(refused(refusal)))
    );
}

#[test]
fn a_mode_the_connection_does_not_allow_is_one_problem_at_the_node_naming_the_mode() {
    let problems = problems_of(&refused(LakeRefusal::ModeNotAllowed(WriteMode::Overwrite)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, "MODE_NOT_ALLOWED");
    assert_eq!(problems[0].node.as_deref(), Some("save"));
    assert!(
        problems[0].message.contains("overwrite"),
        "{:?}",
        problems[0]
    );
}

#[test]
fn each_field_above_the_ceiling_is_a_problem_with_its_path_and_class() {
    let problems = problems_of(&refused(LakeRefusal::Egress(vec![
        field("/email", Level::Pii),
        field("/token", Level::Secret),
    ])));
    let codes: Vec<_> = problems.iter().map(|problem| problem.code).collect();
    assert_eq!(
        codes,
        [
            "CLASSIFICATION_EGRESS_REFUSED",
            "CLASSIFICATION_EGRESS_REFUSED"
        ]
    );
    assert!(problems.iter().all(|p| p.node.as_deref() == Some("save")));
    assert!(problems[0].message.contains("/email") && problems[0].message.contains("pii"));
    assert!(problems[1].message.contains("/token") && problems[1].message.contains("secret"));
}

#[test]
fn a_step_nobody_classified_is_one_problem_that_says_so() {
    let problems = problems_of(&refused(LakeRefusal::Egress(vec![
        EgressRefusal::NotComputed,
    ])));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, "CLASSIFICATION_EGRESS_REFUSED");
    assert!(problems[0].message.contains("classif"), "{:?}", problems[0]);
}

#[test]
fn the_problems_of_a_step_with_many_refused_fields_are_bounded() {
    let many = (0..60)
        .map(|n| field(&format!("/f{n}"), Level::Pii))
        .collect();
    let problems = problems_of(&refused(LakeRefusal::Egress(many)));
    assert_eq!(problems.len(), 20, "the report of one step stays short");
}

#[test]
fn the_message_of_the_error_names_the_node_but_never_a_field() {
    let mode = refused(LakeRefusal::ModeNotAllowed(WriteMode::Upsert)).to_string();
    assert!(mode.contains("save") && mode.contains("upsert"), "{mode}");
    let egress = refused(LakeRefusal::Egress(vec![field("/email", Level::Pii)])).to_string();
    assert!(egress.contains("save"), "{egress}");
    assert!(
        !egress.contains("/email"),
        "paths travel in the details and the problems, not in the message: {egress}"
    );
}
