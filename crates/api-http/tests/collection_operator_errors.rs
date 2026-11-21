//! The refusals of a `forEach` operator chain at publication, as the API words them (design 042, T042-6 and T042-47):
//! `VALIDATION_FAILED` with a `reason` a client can branch on and the node, stage and operator that caused it.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_api_http::error::ApiError;
use sagat_application::definitions::{IrError, OperatorRefusal};
use sagat_application::error::AppError;
use serde_json::json;

fn details_of(error: IrError) -> serde_json::Value {
    let api = ApiError::from(AppError::InvalidDefinition(Box::new(error)));
    api.parts()
        .details
        .clone()
        .expect("a refused definition says why")
}

#[test]
fn switched_off_collection_operators_name_their_reason_and_the_node() {
    let details = details_of(IrError::CollectionOperatorsDisabled {
        node: "lines".to_owned(),
    });
    assert_eq!(
        details,
        json!({"reason": "COLLECTION_OPERATORS_DISABLED", "node": "lines"})
    );
}

#[test]
fn a_refused_stage_names_the_node_the_stage_the_operator_and_the_refusal() {
    let details = details_of(IrError::Operator {
        node: "lines".to_owned(),
        stage: 2,
        operator: "debounce".to_owned(),
        refusal: OperatorRefusal::Surface,
        message: "only a trigger has this operator".to_owned(),
    });
    assert_eq!(
        details,
        json!({
            "reason": "INVALID_OPERATOR",
            "node": "lines",
            "stage": 2,
            "operator": "debounce",
            "refusal": "SURFACE"
        })
    );
}
