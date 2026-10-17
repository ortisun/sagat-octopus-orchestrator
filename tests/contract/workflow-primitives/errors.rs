use sagat_api_http::error::ErrorCode;

const CODES: [ErrorCode; 10] = [
    ErrorCode::BadRequest,
    ErrorCode::Unauthenticated,
    ErrorCode::Forbidden,
    ErrorCode::NotFound,
    ErrorCode::Conflict,
    ErrorCode::ValidationFailed,
    ErrorCode::RateLimited,
    ErrorCode::QuotaExceeded,
    ErrorCode::Unavailable,
    ErrorCode::Internal,
];

fn code_named(name: &str) -> ErrorCode {
    CODES
        .into_iter()
        .find(|code| {
            serde_json::to_value(code)
                .ok()
                .as_ref()
                .and_then(|v| v.as_str())
                == Some(name)
        })
        .unwrap_or_else(|| panic!("{name} is not an Error.code"))
}

/// The reasons the feature's error table introduces, with the API code each refines: every row whose
/// reason cell names backticked reasons that are not marked "(existing)".
fn introduced_reasons() -> Vec<(ErrorCode, String)> {
    let path = super::repository_root().join("specs/014-workflow-primitives/contracts/README.md");
    let table = std::fs::read_to_string(path).expect("contract README");
    let mut reasons = Vec::new();
    for line in table.lines().filter(|line| line.starts_with("| ")) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 6 {
            continue;
        }
        let (api, reason_cell) = (cells[3], cells[4]);
        let Some(code) = api
            .split('`')
            .nth(1)
            .filter(|name| name.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        else {
            continue;
        };
        if code == "Error" {
            continue;
        }
        // A cell can mix reasons that exist elsewhere ("`X` (existing); `Y`"): only the parts not
        // marked "(existing)" are introduced here.
        for part in reason_cell.split(';').flat_map(|part| part.split(" or ")) {
            if part.contains("(existing)") {
                continue;
            }
            for reason in part.split('`').skip(1).step_by(2) {
                reasons.push((code_named(code), reason.to_owned()));
            }
        }
    }
    reasons
}

#[test]
fn every_reason_the_table_introduces_has_a_remediation_and_is_in_the_openapi() {
    let openapi_path =
        super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    let openapi = std::fs::read_to_string(openapi_path).expect("OpenAPI");
    let reasons = introduced_reasons();
    for expected in [
        "INVALID_SCOPE",
        "UPDATE_REJECTED",
        "INVALID_ARGUMENTS",
        "INVALID_UPDATE_KEY",
        "INVALID_DECISION_KEY",
        "INVALID_REASON_CODE",
        "RUN_CONTINUED",
        "CHAIN_HEAD_CHANGED",
        "INVALID_OPERATION_STATE",
        "INITIATOR_CANNOT_DECIDE",
    ] {
        assert!(
            reasons.iter().any(|(_, reason)| reason == expected),
            "the table no longer introduces {expected}: {reasons:?}"
        );
    }
    for (code, reason) in &reasons {
        let specific = code.remediation(Some(reason));
        assert_ne!(
            specific,
            code.remediation(None),
            "{reason} has no specific remediation row"
        );
        assert!(
            !specific.contains('{') && !specific.contains('}'),
            "{reason}"
        );
        assert!(
            openapi.contains(&format!("`{reason}`")),
            "{reason} is not named in the OpenAPI Error.details description"
        );
    }
}
