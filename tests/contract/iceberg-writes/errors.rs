//! The typed failures of feature 035 (contracts/errors.md, D035-042).
//!
//! - Every code of the table is in `sdks/error-catalog.json`, with the retry flag the table gives it.
//! - Every reason of a publication or API refusal is written in the OpenAPI `Error` schema, where the reasons of the
//!   existing codes are (a reason refines a code; it is not a new `Error.code`).
//!
//! The lists are read from errors.md, so a code added to the document without the catalog or the OpenAPI text fails
//! here, and a code that the document no longer names is not kept alive by this test.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{fragments, read_json, read_text, repository_root};

/// (code, retryable as the table says: `Some(true)`, `Some(false)`, or `None` for the em dash).
fn table_codes() -> Vec<(String, Option<bool>)> {
    let text = read_text(&fragments().join("errors.md"));
    let codes: Vec<(String, Option<bool>)> = text
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let code = cells[1].trim_matches('`').to_string();
            let retryable = match cells[2] {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
            (code, retryable)
        })
        .collect();
    assert!(
        codes.len() >= 24,
        "errors.md lists the failure codes of the feature, found {}",
        codes.len()
    );
    codes
}

/// The reasons errors.md and connections.md give to refusals, with the paragraph each comes from.
fn refusal_reasons() -> Vec<String> {
    let errors = read_text(&fragments().join("errors.md"));
    let paragraph = errors
        .split("with reasons")
        .nth(1)
        .and_then(|rest| rest.split("An older engine").next())
        .expect("errors.md lists the reasons of publication refusals");
    let mut reasons: Vec<String> = paragraph
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| token.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        .map(str::to_string)
        .collect();
    assert!(
        errors.contains("`UNSUPPORTED_NODE`"),
        "errors.md names the refusal of an older engine"
    );
    reasons.push("UNSUPPORTED_NODE".to_string());
    let connections = read_text(&fragments().join("connections.md"));
    assert!(
        connections.contains("reason: CREDENTIAL_IN_URI"),
        "connections.md names the refusal of a URI with credentials"
    );
    reasons.push("CREDENTIAL_IN_URI".to_string());
    assert!(reasons.len() >= 10, "{reasons:?}");
    reasons
}

/// Every code of the table is an entry of the SDK error catalog, with the retry flag of the table and the shape of a
/// step failure: of the worker origin, no HTTP status, a gRPC class the entries use, and a remediation that is a
/// sentence without a value in it. A code in the catalog also needs a typed constant in each of the six SDKs, whose own
/// tests require one (T035-024, D035-073).
#[test]
fn every_failure_code_is_in_the_sdk_error_catalog_with_the_retry_flag_of_the_table() {
    const GRPC: [&str; 9] = [
        "NOT_FOUND",
        "FAILED_PRECONDITION",
        "PERMISSION_DENIED",
        "INVALID_ARGUMENT",
        "ABORTED",
        "RESOURCE_EXHAUSTED",
        "UNAVAILABLE",
        "ALREADY_EXISTS",
        "INTERNAL",
    ];
    let catalog = read_json(&repository_root().join("sdks/error-catalog.json"));
    let entries: BTreeMap<&str, &Value> = catalog["errors"]
        .as_array()
        .expect("the catalog lists errors")
        .iter()
        .map(|entry| (entry["code"].as_str().expect("a code"), entry))
        .collect();

    let mut problems = Vec::new();
    for (code, retryable) in table_codes() {
        let Some(entry) = entries.get(code.as_str()) else {
            problems.push(format!("{code}: not in the catalog"));
            continue;
        };
        if let Some(expected) = retryable
            && entry["retryable"] != expected
        {
            problems.push(format!("{code}: retryable should be {expected}"));
        }
        if entry["origin"] != "worker" || entry["http"] != serde_json::json!([]) {
            problems.push(format!(
                "{code}: a step failure is of the worker origin and has no HTTP status"
            ));
        }
        let grpc = entry["grpc"].as_str().unwrap_or_default();
        if code != "OUTCOME_UNKNOWN" && !GRPC.contains(&grpc) {
            problems.push(format!(
                "{code}: gRPC class {grpc} is not one the entries use"
            ));
        }
        let remediation = entry["remediation"].as_str().unwrap_or_default();
        if remediation.len() <= 20 || remediation.chars().any(|c| c.is_ascii_digit()) {
            problems.push(format!(
                "{code}: a remediation is a sentence with no value in it"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn every_refusal_reason_is_written_in_the_openapi_error_schema() {
    let path = repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    let openapi: Value = serde_saphyr::from_str(&read_text(&path)).expect("OpenAPI is YAML");
    let error = serde_json::to_string(&openapi["components"]["schemas"]["Error"]).expect("JSON");

    let missing: Vec<String> = refusal_reasons()
        .into_iter()
        .filter(|reason| !error.contains(reason.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "reasons missing from the Error schema: {missing:?}"
    );
}

#[test]
fn the_table_has_no_code_twice() {
    let mut seen = std::collections::BTreeSet::new();
    for (code, _) in table_codes() {
        assert!(seen.insert(code.clone()), "{code} is listed twice");
    }
}
