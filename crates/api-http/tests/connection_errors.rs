//! The refusals of a connection that is not acceptable (feature 035), as the API words them: a `422` with the code
//! `VALIDATION_FAILED` and a `reason` a client can branch on. The reason is a closed name; it never carries a value of
//! the request, so a credential that was sent in a URI is not repeated back.
#![allow(clippy::expect_used, clippy::panic)]

use axum::http::StatusCode;
use sagat_api_http::error::{ApiError, ErrorCode};
use sagat_application::error::AppError;
use sagat_domain::database::LakeConfigError;

const REASONS: [LakeConfigError; 14] = [
    LakeConfigError::InvalidUri,
    LakeConfigError::CredentialInUri,
    LakeConfigError::PlainHttpOutsideLoopback,
    LakeConfigError::InvalidCatalog,
    LakeConfigError::InvalidStorage,
    LakeConfigError::VendedNeedsDelegation,
    LakeConfigError::StorageSecretWithVended,
    LakeConfigError::StorageSecretRequired,
    LakeConfigError::CatalogSecretRequired,
    LakeConfigError::CatalogSecretForbidden,
    LakeConfigError::InvalidLocations,
    LakeConfigError::LocationSchemeMismatch,
    LakeConfigError::InvalidModes,
    LakeConfigError::InvalidLimits,
];

#[test]
fn a_refused_connection_is_a_validation_failure_that_names_its_reason() {
    for reason in REASONS {
        let error = ApiError::from(AppError::InvalidConnection {
            reason: reason.reason(),
        });
        let parts = error.parts();

        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY, "{reason:?}");
        assert_eq!(parts.code, ErrorCode::ValidationFailed, "{reason:?}");
        assert!(!parts.retryable, "{reason:?}");
        let details = parts.details.as_ref().expect("details");
        assert_eq!(details["reason"], reason.reason());
    }
}

#[test]
fn the_message_of_a_refusal_repeats_no_value() {
    let error = ApiError::from(AppError::InvalidConnection {
        reason: LakeConfigError::CredentialInUri.reason(),
    });

    assert_eq!(
        error.parts().message,
        "the connection settings are not acceptable: CREDENTIAL_IN_URI"
    );
}

#[test]
fn a_credential_in_a_uri_tells_the_author_where_a_credential_goes() {
    let advice = ErrorCode::ValidationFailed.remediation(Some("CREDENTIAL_IN_URI"));

    assert!(advice.contains("secret"), "{advice}");
    assert!(
        advice.contains("user") || advice.contains("query"),
        "{advice}"
    );
}

#[test]
fn every_lake_reason_has_a_remediation_of_its_own() {
    let generic = ErrorCode::ValidationFailed.remediation(None);
    for reason in REASONS {
        let advice = ErrorCode::ValidationFailed.remediation(Some(reason.reason()));
        assert_ne!(
            advice, generic,
            "{reason:?} falls back to the generic advice"
        );
    }
}
