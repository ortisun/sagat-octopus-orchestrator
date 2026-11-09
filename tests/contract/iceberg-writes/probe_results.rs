//! The result of a probe, as a corpus (feature 035, D035-150 slice S4a): `fixtures/probe-results/{accepted,refused}`.
//!
//! The server never stores what a runner sent. It reads the report into typed fields that accept only what the contract names, checks it
//! against the probe (the connection and the revision it was leased with), and stores the report built again from those fields. This corpus
//! is what that codec is held to, and what the Python host (slice S5) is held to as well: each accepted file is a report that must be taken,
//! and each refused file is one that must be refused **for the reason its name begins with**.
//!
//! Every refused file carries a canary (`CANARY`, text in a member where a closed word or a number belongs, or a member the report does not
//! have), and the test asserts that no refusal and no stored result ever contains it.
use sagat_application::database::{ExpectedProbe, InvalidLakeProbe, LakeProbeCodec};
use sagat_infra_ir::JsonLakeProbeCodec;
use serde_json::Value;

fn files(sub: &str) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(super::fixtures().join("probe-results").join(sub))
        .expect("the corpus")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    found.sort();
    found
}

fn expected() -> ExpectedProbe<'static> {
    ExpectedProbe {
        connection: "lake-main",
        revision: 3,
    }
}

fn reason(prefix: &str) -> InvalidLakeProbe {
    match prefix {
        "malformed" => InvalidLakeProbe::Malformed,
        "kind" => InvalidLakeProbe::Kind,
        "mismatch" => InvalidLakeProbe::Mismatch,
        "inconsistent" => InvalidLakeProbe::Inconsistent,
        "large" => InvalidLakeProbe::TooLarge,
        other => panic!("{other} is not a refusal of the codec"),
    }
}

#[test]
fn every_accepted_report_is_taken_and_stored_as_the_server_wrote_it_again() {
    let accepted = files("accepted");
    assert!(accepted.len() >= 9, "the corpus has not shrunk");
    for path in accepted
        .iter()
        .filter(|p| !p.to_string_lossy().ends_with(".canonical.json"))
    {
        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let sent = std::fs::read(path).expect("the report");
        let rebuilt = JsonLakeProbeCodec
            .rebuild(&sent, &expected())
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        // the golden is the `.canonical.json` beside it when there is one, and the report itself otherwise (it is already canonical)
        let golden = path.with_extension("canonical.json");
        let golden = if golden.exists() {
            std::fs::read(&golden).expect("the golden")
        } else {
            sent.clone()
        };
        assert_eq!(
            String::from_utf8(rebuilt.canonical.clone()).expect("UTF-8"),
            String::from_utf8(golden).expect("UTF-8").trim_end(),
            "{name}: what is stored is the server's own writing of the typed fields"
        );
        // and it is stable: the canonical form, read again, is itself
        let again = JsonLakeProbeCodec
            .rebuild(&rebuilt.canonical, &expected())
            .unwrap_or_else(|error| panic!("{name} again: {error:?}"));
        assert_eq!(again.canonical, rebuilt.canonical, "{name}");
    }
}

#[test]
fn every_refused_report_is_refused_for_the_reason_its_name_gives_and_says_nothing_of_what_it_held()
{
    let refused = files("refused");
    assert!(refused.len() >= 25, "the corpus has not shrunk");
    for path in refused {
        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let prefix = name.split('-').next().expect("a prefix");
        let sent = std::fs::read(&path).expect("the report");
        let outcome = JsonLakeProbeCodec.rebuild(&sent, &expected());
        let error = outcome.expect_err(&name);
        assert_eq!(error, reason(prefix), "{name}");
        for text in [format!("{error:?}"), error.to_string()] {
            assert!(
                !text.contains("CANARY")
                    && !text.contains("10.0.0.7")
                    && !text.contains("catalog.example"),
                "{name}: a refusal does not repeat what it was handed: {text}"
            );
        }
    }
}

#[test]
fn what_is_stored_holds_only_closed_words_and_numbers() {
    // The stored result is built from typed fields: the only members are these, and the only text in them is the connection's own name
    // (which the server supplied) and the words of the vocabularies.
    for path in files("accepted") {
        let sent = std::fs::read(&path).expect("the report");
        if let Ok(rebuilt) = JsonLakeProbeCodec.rebuild(&sent, &expected()) {
            let value: Value = serde_json::from_slice(&rebuilt.canonical).expect("JSON");
            let mut members: Vec<&str> = value
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            members.sort_unstable();
            assert_eq!(members, ["checks", "connection", "kind"], "{path:?}");
            for check in value["checks"].as_array().expect("checks") {
                let mut members: Vec<&str> = check
                    .as_object()
                    .expect("an object")
                    .keys()
                    .map(String::as_str)
                    .collect();
                members.sort_unstable();
                assert!(
                    members == ["check", "outcome"] || members == ["check", "code", "outcome"],
                    "{path:?}: {members:?}"
                );
            }
        }
    }
}
