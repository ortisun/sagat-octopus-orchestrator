//! The refusals of the publishing rules, as the API words them: each has a `reason` a client can
//! branch on, and the one found inside the transaction is worded like the ones found before it.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_api_http::error::ApiError;
use sagat_application::error::{AppError, ConflictKind};

#[test]
fn a_request_that_stopped_being_approved_while_it_was_published_says_so() {
    let error = ApiError::from(AppError::Conflict(ConflictKind::RequestNotApproved));
    let parts = error.parts();
    let details = parts.details.as_ref().expect("details");
    assert_eq!(details["reason"], "REQUEST_NOT_APPROVED");
    let advice = parts.code.remediation(details["reason"].as_str());
    assert!(advice.contains("read it again"), "{advice}");
}
