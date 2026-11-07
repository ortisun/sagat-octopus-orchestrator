//! The reasons feature 018 adds to the closed `Error.code` set (`T018-044`, `FR-018-043`): every reason an
//! operation raises today has the remediation sentence the contract's table says and is named in the
//! OpenAPI; every reason the table lists and nothing raises yet is on a list that has to shrink as the
//! tasks that raise them land, so that the table, the API's closed table of sentences and the OpenAPI
//! cannot drift apart without a test saying so.

use std::path::PathBuf;

use sagat_api_http::error::ErrorCode;

/// The reasons that operations raise today.
const RAISED: &[&str] = &[
    "SCHEDULE_SPEC_INVALID",
    "SCHEDULE_NEVER_FIRES",
    "SCHEDULE_TOO_FREQUENT",
    "SCHEDULE_ZONE_UNKNOWN",
    "SCHEDULE_INPUT_INVALID",
    "BACKFILL_RANGE_INVALID",
    "BACKFILL_LIMIT",
    "SCHEDULE_TRIGGER_OWNED",
    "SCHEDULE_TRIGGER_HANDOVER_PENDING",
    "SCHEDULE_LIMIT",
    "BACKFILL_CONCURRENCY",
    "BATCH_LIMIT",
    "PENDING_RESET_LIMIT",
    "BUSINESS_ID_INVALID",
    "BUSINESS_ID_IN_USE",
    "BUSINESS_ID_REUSE_REJECTED",
    "START_DELAY_INVALID",
    "START_DELAY_WITH_SIGNAL",
    "NOT_WAITING",
    "SIGNAL_NOT_ACCEPTED_AT_START",
    "BATCH_SELECTOR_INVALID",
    "QUERY_UNSUPPORTED_OPERATOR",
    "BATCH_ACTION_UNAVAILABLE",
    "BATCH_LIMITS_INVALID",
    "BATCH_REASON_INVALID",
    "BATCH_PAYLOAD_INVALID",
    "BATCH_PAYLOAD_TOO_LARGE",
    "BATCH_SELECTOR_UNSUPPORTED",
    "BATCH_ACTION_PARAMETERS",
    "BATCH_NOT_CANCELLABLE",
    "RESET_NOT_SUPPORTED_FOR_SOURCE",
    "RESET_POINT_NOT_ELIGIBLE",
    "RESET_PREFIX_TOO_LONG",
    "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
    "RESET_SOURCE_NOT_TERMINAL",
    "RESET_RULE_MISSING",
    "RESET_REAPPLY_NOT_AVAILABLE",
    "RESET_REAPPLY_TOO_MANY",
    "RESET_CANCEL_SOURCE_NOT_AVAILABLE",
];

/// The reasons the table lists that no operation raises yet, and why.
const NOT_YET: &[(&str, &str)] = &[
    (
        "NOMINAL_TIME_FIELD_INVALID",
        "a bad nominalTimeField is reported as SCHEDULE_SPEC_INVALID with its field pointer (G1975)",
    ),
    (
        "SCHEDULE_DELETED",
        "a deleted schedule is NOT_FOUND, as one that never existed (G1975)",
    ),
    ("BINARY_CAPABILITY_MISSING", "features 029 and 031"),
    ("CONCURRENCY_QUEUE_FULL", "feature 022"),
    ("CONCURRENCY_PERMIT_UNAVAILABLE", "feature 022 and US06"),
    ("RUN_PURGED", "feature 020"),
    ("RESET_SOURCE_TEST_RUN", "feature 022"),
];

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

/// One row of the table: the API code, the backticked reasons of its reason cell, and its remediation.
struct Row {
    code: ErrorCode,
    reasons: Vec<String>,
    /// "(existing reason)" or "reused" in the reason cell: another feature owns the reason.
    owned_elsewhere: bool,
    remediation: String,
}

fn rows() -> Vec<Row> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../specs/018-schedules-bulk-operations/contracts/errors.md");
    let table = std::fs::read_to_string(path).expect("the error contract");
    table
        .lines()
        .filter(|line| line.starts_with("| `") && !line.starts_with("| `code`"))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            Row {
                code: code_named(cells[1].trim_matches('`')),
                reasons: cells[3]
                    .split('`')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_owned)
                    .collect(),
                owned_elsewhere: cells[3].contains("existing") || cells[3].contains("reused"),
                remediation: cells[6].to_owned(),
            }
        })
        .collect()
}

fn introduced(rows: &[Row]) -> Vec<(&Row, &str)> {
    rows.iter()
        .filter(|row| !row.owned_elsewhere)
        .flat_map(|row| row.reasons.iter().map(move |reason| (row, reason.as_str())))
        .collect()
}

#[test]
fn every_reason_the_table_introduces_is_raised_or_waits_on_a_named_task() {
    let rows = rows();
    for (_, reason) in introduced(&rows) {
        let raised = RAISED.contains(&reason);
        let waiting = NOT_YET.iter().any(|(name, _)| *name == reason);
        // QUERY_UNSUPPORTED_OPERATOR is feature 020's reason, raised here by the batch selector
        let reused_and_raised = reason == "QUERY_UNSUPPORTED_OPERATOR";
        assert!(
            raised ^ waiting || reused_and_raised,
            "{reason} must be exactly one of raised and waiting (update the lists of this test)"
        );
    }
    for reason in RAISED {
        assert!(
            rows.iter()
                .any(|row| row.reasons.iter().any(|r| r == reason)),
            "{reason} is raised but the table does not list it"
        );
    }
}

#[test]
fn a_raised_reason_has_the_sentence_the_table_says_and_the_openapi_names_it() {
    let rows = rows();
    let openapi = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../specs/001-platform-foundation/contracts/openapi.yaml"),
    )
    .expect("the OpenAPI");
    for reason in RAISED {
        let row = rows
            .iter()
            .find(|row| row.reasons.iter().any(|r| r == reason))
            .unwrap_or_else(|| panic!("{reason} is not in the table"));
        let sentence = row.code.remediation(Some(reason));
        assert_eq!(
            sentence, row.remediation,
            "{reason}: the API's sentence is not the table's"
        );
        assert_ne!(
            sentence,
            row.code.remediation(None),
            "{reason} has no row of its own"
        );
        assert!(
            openapi.contains(&format!("`{reason}`")),
            "{reason} is not named in the OpenAPI Error.details description"
        );
    }
}

#[test]
fn a_reason_nothing_raises_yet_has_no_remediation_row_of_its_own() {
    let rows = rows();
    for (reason, _) in NOT_YET {
        let row = rows
            .iter()
            .find(|row| row.reasons.iter().any(|r| r == reason))
            .unwrap_or_else(|| panic!("{reason} is no longer in the table"));
        assert_eq!(
            row.code.remediation(Some(reason)),
            row.code.remediation(None),
            "{reason} has a row: move it from the list of reasons nothing raises to the list of those that are raised"
        );
    }
}
