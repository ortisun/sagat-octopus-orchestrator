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

/// Only errors belong in `issues` of a refused document: a warning (`CLASSIFICATION_INHERITED_OPEN`)
/// is reported by `validate`, and listing it as a reason for the refusal misleads the author.
#[test]
fn a_refused_classification_lists_errors_only() {
    use sagat_application::classification::{
        ClassificationFinding, ClassificationFindingCode as Code, ClassificationPointer,
    };
    use sagat_application::definitions::IrError;
    let finding = |code, path: &str| ClassificationFinding {
        code,
        path: ClassificationPointer::parse(path).expect("pointer"),
    };
    let error = ApiError::from(AppError::InvalidDefinition(Box::new(
        IrError::Classification(vec![
            finding(Code::InheritedOpen, "/inputSchema/properties/id"),
            finding(Code::Loosened, "/inputSchema/properties/open"),
        ]),
    )));
    let parts = error.parts();
    let details = parts.details.as_ref().expect("details");
    assert_eq!(details["reason"], "CLASSIFICATION");
    let issues = details["issues"].as_array().expect("issues");
    let codes: Vec<_> = issues.iter().map(|i| i["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["CLASSIFICATION_LOOSENED"], "{details}");
}
