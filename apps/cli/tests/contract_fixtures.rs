//! Executable examples of the onboarding diagnostic contract (T012-002, US04).
#![allow(clippy::expect_used)]

use sagat_cli::diagnostics::of_document;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Case {
    name: String,
    document: Value,
    codes: Vec<String>,
}

#[test]
fn onboarding_fixtures_use_the_platform_validation_and_explain_each_error() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/contract/developer-experience/fixtures/diagnostics.json"
    ))
    .expect("diagnostic fixture corpus");
    for case in cases {
        let bytes = serde_json::to_vec(&case.document).expect("document");
        let found = of_document(&bytes);
        let codes: Vec<_> = found.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, case.codes, "{}", case.name);
        for diagnostic in found {
            assert!(diagnostic.at.is_some(), "{}: no location", case.name);
            assert!(diagnostic.help.is_some(), "{}: no remediation", case.name);
            assert!(
                diagnostic
                    .lines(None)
                    .iter()
                    .any(|line| line.starts_with("  docs: "))
            );
        }
    }
}
