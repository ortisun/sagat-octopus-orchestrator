use sagat_api_http::error::ErrorCode;

#[test]
fn primitive_reasons_have_specific_safe_remediation() {
    for (code, reason) in [
        (ErrorCode::ValidationFailed, "INVALID_SCOPE"),
        (ErrorCode::ValidationFailed, "INVALID_OPERATION_STATE"),
        (ErrorCode::Forbidden, "NOT_ELIGIBLE"),
        (ErrorCode::Forbidden, "INITIATOR_CANNOT_DECIDE"),
        (ErrorCode::Conflict, "CHAIN_HEAD_CHANGED"),
        (ErrorCode::Conflict, "RUN_NOT_MUTABLE"),
        (ErrorCode::Conflict, "RUN_CONTINUED"),
        (ErrorCode::ValidationFailed, "UPDATE_REJECTED"),
    ] {
        let specific = code.remediation(Some(reason));
        assert_ne!(specific, code.remediation(None), "{reason}");
        assert!(!specific.is_empty(), "{reason}");
        assert!(
            !specific.contains('{') && !specific.contains('}'),
            "{reason}"
        );
    }
}
