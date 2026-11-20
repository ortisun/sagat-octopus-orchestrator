//! The refusals of flow control as the API words them (feature 022, T022-012, `contracts/errors.md` "REST refusals"):
//! one test per row that names concurrency or a rate limit, each an existing `Error.code` with its own `details.reason`,
//! and none naming a key's value. Of the rows that 018's reset owns, `CONCURRENCY_PERMIT_UNAVAILABLE` landed with the reset
//! case (T022-064); `RATE_LIMIT_SLOT_UNAVAILABLE` lands with T022-086 (G1181).
#![allow(clippy::expect_used, clippy::panic)]

use axum::http::StatusCode;
use sagat_api_http::error::{ApiError, ErrorCode};
use sagat_application::definitions::IrError;
use sagat_application::error::AppError;
use sagat_application::flow_control::{FlowControlCapability, FlowControlRefusal};
use serde_json::json;

fn refused(refusal: FlowControlRefusal) -> sagat_api_http::error::ErrorParts {
    ApiError::from(AppError::FlowControl(refusal))
        .parts()
        .clone()
}

#[test]
fn a_full_concurrency_queue_is_quota_exceeded_and_not_retryable() {
    let parts = refused(FlowControlRefusal::ConcurrencyQueueFull);
    assert_eq!(parts.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(parts.code, ErrorCode::QuotaExceeded);
    assert!(!parts.retryable);
    assert_eq!(
        parts.details,
        Some(json!({"reason": "CONCURRENCY_QUEUE_FULL"}))
    );
}

/// 018's contract (`specs/018-schedules-bulk-operations/contracts/errors.md`): a reset of a run whose start takes a permit,
/// with no free permit, is a conflict the caller resolves by retrying once a holder has finished; a reset never queues.
#[test]
fn a_reset_without_a_free_permit_is_a_conflict_with_its_reason_and_remedy() {
    let parts = refused(FlowControlRefusal::PermitUnavailable);
    assert_eq!(parts.status, StatusCode::CONFLICT);
    assert_eq!(parts.code, ErrorCode::Conflict);
    assert!(!parts.retryable);
    assert_eq!(
        parts.details,
        Some(json!({"reason": "CONCURRENCY_PERMIT_UNAVAILABLE"}))
    );
    assert_eq!(
        ErrorCode::Conflict.remediation(Some("CONCURRENCY_PERMIT_UNAVAILABLE")),
        "Retry the reset when a holder of the key has finished."
    );
}

#[test]
fn a_full_rate_limit_buffer_is_quota_exceeded_and_not_retryable() {
    let parts = refused(FlowControlRefusal::RateLimitQueueFull);
    assert_eq!(parts.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(parts.code, ErrorCode::QuotaExceeded);
    assert!(!parts.retryable);
    assert_eq!(
        parts.details,
        Some(json!({"reason": "RATE_LIMIT_QUEUE_FULL"}))
    );
}

#[test]
fn a_namespace_ceiling_of_flow_control_names_the_ceiling() {
    for limit in ["live semaphore keys", "queued runs", "live rate-limit keys"] {
        let parts = refused(FlowControlRefusal::Limit { limit });
        assert_eq!(parts.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(parts.code, ErrorCode::QuotaExceeded);
        assert!(!parts.retryable);
        assert_eq!(
            parts.details,
            Some(json!({"reason": "FLOW_CONTROL_LIMIT", "limit": limit}))
        );
    }
}

#[test]
fn a_start_before_the_activation_is_a_validation_failure_with_its_reason() {
    for capability in FlowControlCapability::ALL {
        let parts = refused(FlowControlRefusal::ActivationPending { capability });
        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(parts.code, ErrorCode::ValidationFailed);
        assert!(!parts.retryable);
        assert_eq!(
            parts.details,
            Some(
                json!({"reason": "FLOW_CONTROL_ACTIVATION_PENDING", "capability": capability.as_str()})
            )
        );
    }
}

#[test]
fn a_publication_before_the_activation_is_refused_with_the_same_reason() {
    let api = ApiError::from(AppError::InvalidDefinition(Box::new(
        IrError::FlowControlActivationPending {
            capability: "concurrencyKeys",
        },
    )));
    let parts = api.parts();
    assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parts.code, ErrorCode::ValidationFailed);
    assert_eq!(
        parts.details,
        Some(json!({"reason": "FLOW_CONTROL_ACTIVATION_PENDING", "capability": "concurrencyKeys"}))
    );
}

#[test]
fn a_process_without_the_gate_answers_unavailable_and_retryable() {
    let parts = refused(FlowControlRefusal::BinaryCapabilityMissing);
    assert_eq!(parts.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(parts.code, ErrorCode::Unavailable);
    assert!(parts.retryable, "another process may run the gate");
    assert_eq!(
        parts.details,
        Some(json!({"reason": "BINARY_CAPABILITY_MISSING"}))
    );
}

#[test]
fn a_start_of_a_version_whose_capability_was_switched_off_is_capability_disabled() {
    let parts = refused(FlowControlRefusal::CapabilityDisabled {
        capability: FlowControlCapability::ConcurrencyKeys,
    });
    assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parts.code, ErrorCode::ValidationFailed);
    assert_eq!(
        parts.details,
        Some(json!({"reason": "CAPABILITY_DISABLED", "capability": "concurrencyKeys"}))
    );
}

#[test]
fn a_gate_on_a_disabled_capability_or_in_a_body_is_an_unsupported_node() {
    for (error, expected) in [
        (
            IrError::Unsupported {
                node: "fetch".to_owned(),
                kind: "rateLimit".to_owned(),
                reason: "this workflow capability is not enabled by the runtime",
            },
            json!({"reason": "UNSUPPORTED_NODE", "node": "fetch", "kind": "rateLimit"}),
        ),
        (
            IrError::UnsupportedDeclaration {
                field: "concurrency",
                reason: "this workflow capability is not enabled by the runtime",
            },
            json!({"reason": "UNSUPPORTED_NODE", "path": "/concurrency"}),
        ),
        (
            IrError::Pending {
                node: "charge".to_owned(),
                kind: "concurrency".to_owned(),
                dependency: "SCOPED_CONCURRENCY",
                reason: "x",
            },
            json!({"reason": "UNSUPPORTED_NODE", "node": "charge", "kind": "concurrency", "dependency": "SCOPED_CONCURRENCY"}),
        ),
    ] {
        let api = ApiError::from(AppError::InvalidDefinition(Box::new(error)));
        assert_eq!(api.parts().code, ErrorCode::ValidationFailed);
        assert_eq!(api.parts().details, Some(expected));
    }
}

/// No refusal of flow control adds an `Error.code`: the closed catalog of the SDKs stays as it is (FR-022-034, D1192).
#[test]
fn the_error_catalog_of_the_sdks_is_unchanged() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../../../sdks/error-catalog.json"))
            .expect("the catalog");
    let text = catalog.to_string();
    for reason in [
        "CONCURRENCY_QUEUE_FULL",
        "RATE_LIMIT_QUEUE_FULL",
        "FLOW_CONTROL_LIMIT",
        "FLOW_CONTROL_ACTIVATION_PENDING",
    ] {
        assert!(
            !text.contains(&format!("\"code\":\"{reason}\"")),
            "{reason} is a reason, not a code"
        );
    }
    for code in [
        ErrorCode::QuotaExceeded,
        ErrorCode::ValidationFailed,
        ErrorCode::Unavailable,
    ] {
        let wire = serde_json::to_value(code).expect("wire");
        assert!(
            text.contains(wire.as_str().expect("a string")),
            "{wire} is in the catalog"
        );
    }
}
