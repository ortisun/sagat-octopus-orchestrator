//! Design 043 (FR-043-003): the problem a refused run input is answered with: `422 VALIDATION_FAILED`, not retryable, with
//! `details.reason`, `details.field` and `details.rule` and the remediation of its reason; no value of the input.
#![allow(clippy::expect_used, clippy::panic)]

use axum::http::StatusCode;
use sagat_api_http::error::{ApiError, ErrorCode};
use sagat_application::error::AppError;
use sagat_application::run_input::{InputRefusal, Rule};

#[test]
fn a_refused_run_input_is_a_validation_failure_with_its_field_and_its_rule() {
    let error = ApiError::from(AppError::from(InputRefusal::Invalid {
        field: "/orderId".to_owned(),
        rule: Rule::Type,
    }));
    let parts = error.parts();
    assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parts.code, ErrorCode::ValidationFailed);
    assert!(!parts.retryable);
    let details = parts.details.as_ref().expect("details");
    assert_eq!(details["reason"], "RUN_INPUT_INVALID");
    assert_eq!(details["field"], "/orderId");
    assert_eq!(details["rule"], "type");
    assert!(
        ErrorCode::ValidationFailed
            .remediation(Some("RUN_INPUT_INVALID"))
            .contains("details.field")
    );
}

#[test]
fn an_unusable_schema_has_no_field_and_no_rule_and_no_room_is_retryable() {
    let error = ApiError::from(AppError::from(InputRefusal::SchemaUnusable));
    let parts = error.parts();
    assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
    let details = parts.details.as_ref().expect("details");
    assert_eq!(details["reason"], "RUN_INPUT_SCHEMA_UNUSABLE");
    assert!(
        details.get("field").is_none() && details.get("rule").is_none(),
        "{details}"
    );
    assert!(
        ErrorCode::ValidationFailed
            .remediation(Some("RUN_INPUT_SCHEMA_UNUSABLE"))
            .contains("input-schema-inventory")
    );

    let busy = ApiError::from(AppError::from(InputRefusal::Unavailable));
    assert_eq!(busy.parts().status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(busy.parts().retryable);
}

#[test]
fn every_rule_is_answered_by_its_closed_name() {
    for rule in Rule::ALL {
        let error = ApiError::from(AppError::from(InputRefusal::Invalid {
            field: "/".to_owned(),
            rule,
        }));
        assert_eq!(
            error.parts().details.as_ref().expect("details")["rule"],
            rule.as_str()
        );
    }
}

/// D043-177: a document whose `inputSchema` pre-check found no permit of the validator within its wait is not judged; it is
/// answered as a start's `Busy` is, `503`, retryable, never `422`.
#[test]
fn a_document_whose_pre_check_found_no_permit_is_unavailable_and_retryable() {
    let error = ApiError::from(AppError::InvalidDefinition(Box::new(
        sagat_application::definitions::IrError::ValidatorBusy,
    )));
    let parts = error.parts();
    assert_eq!(parts.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(parts.code, ErrorCode::Unavailable);
    assert!(parts.retryable);
}
