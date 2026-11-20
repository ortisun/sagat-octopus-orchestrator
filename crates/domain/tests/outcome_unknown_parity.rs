//! The set of failure codes that leave an effect's outcome unknown is decided by the domain
//! (`FailureCode::leaves_outcome_unknown`). Two other layers keep a literal copy: the chain-progress
//! store labels a blocker `unknown_effect` with a SQL `IN (...)` list, and the Studio offers or
//! withholds a retry with its own set. A code added to one place only would mislabel the blocker
//! or offer a retry the server refuses, so this test reads both copies and compares.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;

use sagat_domain::execution::FailureCode;

fn domain() -> BTreeSet<String> {
    FailureCode::outcome_unknown_codes()
        .iter()
        .map(|code| code.as_str().to_owned())
        .collect()
}

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The quoted upper-case codes between `start` and the first `end` after it.
fn quoted_codes(text: &str, start: &str, end: &str) -> BTreeSet<String> {
    let from = text
        .find(start)
        .unwrap_or_else(|| panic!("marker {start:?} not found"));
    let rest = &text[from + start.len()..];
    let to = rest
        .find(end)
        .unwrap_or_else(|| panic!("end {end:?} not found"));
    rest[..to]
        .split(['\'', '"'])
        .filter(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_chain_progress_sql_lists_the_domain_codes() {
    let sql = read("../infra-postgres/src/chain_progress.rs");
    assert_eq!(
        quoted_codes(
            &sql,
            "waiting_kind='OPERATOR' AND failure_code IN",
            "THEN 'unknown_effect'"
        ),
        domain()
    );
}

#[test]
fn the_studio_guard_lists_the_domain_codes() {
    let ops = read("../../frontend/apps/studio/src/features/runs/ops.ts");
    assert_eq!(
        quoted_codes(&ops, "const OUTCOME_UNKNOWN = new Set([", "]);"),
        domain()
    );
}

#[test]
fn the_parser_sees_a_missing_code() {
    let mut codes = quoted_codes(
        "x IN ('LEASE_EXPIRED','ATTEMPT_TIMEOUT') THEN 'unknown_effect'",
        "x IN",
        "THEN 'unknown_effect'",
    );
    codes.insert("OUTCOME_UNKNOWN".into());
    assert_ne!(codes, domain(), "a partial list must not compare equal");
}
