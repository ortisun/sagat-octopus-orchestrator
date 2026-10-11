//! What a person is told when a document is wrong: where, what to do and where it is explained
//! (US04), and that the tool, the codec and the documentation agree on the codes.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use sagat_cli::diagnostics::{CATALOG, DOCS_FILE, docs_link, help, of_document, render};
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn document(nodes: &Value, entry: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "demo"}, "entrypoint": entry, "nodes": nodes
    }))
    .expect("json")
}

/// The codes the codec and the validation can return, read out of the source that returns them.
fn codes_in_the_platform() -> BTreeSet<String> {
    let source = std::fs::read_to_string(root().join("crates/application/src/definitions.rs"))
        .expect("the source of the codec's problems");
    let from = source.find("pub fn problems_of").expect("problems_of");
    let to = source
        .find("/// One side of a comparison")
        .expect("the end of warning_problem");
    let region = &source[from..to];
    let mut found = BTreeSet::new();
    let mut rest = region;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let end = after.find('"').expect("a closing quote");
        let literal = &after[..end];
        if literal.len() >= 3
            && literal.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && !literal.starts_with('_')
            && !literal.ends_with('_')
        {
            found.insert(literal.to_owned());
        }
        rest = &after[end + 1..];
    }
    found
}

#[test]
fn every_code_the_platform_returns_has_help_and_the_help_names_no_code_the_platform_does_not() {
    let platform = codes_in_the_platform();
    assert!(
        platform.len() >= 30,
        "the codes were not read out of the source: {platform:?}"
    );
    // Found by the platform's validation of the draft against a namespace, not by the codec.
    let server_only: BTreeSet<String> = ["SECRET_UNAVAILABLE".to_owned()].into();
    let known: BTreeSet<String> = CATALOG.iter().map(|(code, _)| (*code).to_owned()).collect();
    assert_eq!(known.len(), CATALOG.len(), "a code is in the catalog twice");
    let without_help: Vec<_> = platform.difference(&known).collect();
    assert!(
        without_help.is_empty(),
        "codes with no help: {without_help:?}"
    );
    let stale: Vec<_> = known
        .difference(&platform)
        .filter(|c| !server_only.contains(*c))
        .collect();
    assert!(
        stale.is_empty(),
        "help for codes the platform no longer returns: {stale:?}"
    );
}

#[test]
fn every_code_is_explained_in_the_documentation_with_the_sentence_the_tool_prints() {
    let page = std::fs::read_to_string(root().join(DOCS_FILE)).expect("the page of errors");
    for (code, sentence) in CATALOG {
        let heading = format!("### {code}\n");
        let at = page
            .find(&heading)
            .unwrap_or_else(|| panic!("{DOCS_FILE} has no section for {code}"));
        let section_end = page[at + heading.len()..]
            .find("\n### ")
            .map_or(page.len(), |n| at + heading.len() + n);
        assert!(
            page[at..section_end].contains(sentence),
            "the section of {code} does not hold the help the tool prints:\n{sentence}"
        );
    }
    // And no section for a code the tool does not know.
    for line in page.lines().filter(|l| l.starts_with("### ")) {
        let code = line.trim_start_matches("### ");
        assert!(
            help(code).is_some(),
            "{DOCS_FILE} explains {code}, which the tool has no help for"
        );
    }
}

#[test]
fn a_problem_says_where_what_to_do_and_where_it_is_explained() {
    let cases = [
        (
            document(
                &json!([{"id":"charge","type":"activity","next":"ship"},{"id":"ship","type":"activity","activity":"a.b"}]),
                "charge",
            ),
            "SCHEMA",
            "at /nodes/0",
        ),
        (
            document(
                &json!([{"id":"a","type":"activity","activity":"x.a","next":"b"},{"id":"b","type":"activity","activity":"x.b","next":"a"}]),
                "a",
            ),
            "CYCLE",
            "node 'a'",
        ),
        (
            document(
                &json!([{"id":"validate","type":"activity","activity":"x.a","next":"chrage"}]),
                "validate",
            ),
            "UNKNOWN_TARGET",
            "node 'validate'",
        ),
        (
            document(
                &json!([{"id":"a","type":"activity","activity":"x.a"},{"id":"b","type":"activity","activity":"x.b"}]),
                "a",
            ),
            "UNREACHABLE",
            "node 'b'",
        ),
        (
            document(
                &json!([{"id":"c","type":"condition","expression":"input.amount >","ifTrue":"a","ifFalse":"a"},{"id":"a","type":"activity","activity":"x.a"}]),
                "c",
            ),
            "EXPRESSION",
            "node 'c'",
        ),
        (
            document(
                &json!([{"id":"a","type":"activity","activity":"x.a"}]),
                "zzz",
            ),
            "UNKNOWN_ENTRYPOINT",
            "node 'zzz'",
        ),
        (b"{}".to_vec(), "SCHEMA", "at the document"),
        (b"[1]".to_vec(), "SCHEMA", "at the document"),
    ];
    for (bytes, code, place) in cases {
        let found = of_document(&bytes);
        let first = found
            .first()
            .unwrap_or_else(|| panic!("{code}: nothing was said"));
        assert_eq!(first.code, code, "{first:?}");
        assert_eq!(first.at.as_deref(), Some(place), "{code}");
        assert!(first.help.is_some(), "{code} has no help");
        let lines = first.lines(None);
        assert!(
            lines[0].starts_with(&format!("error[{code}]: ")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.starts_with("  --> ")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("  help: ")), "{lines:?}");
        assert_eq!(
            lines.last().expect("a last line"),
            &format!("  docs: {DOCS_FILE}#{}", code.to_ascii_lowercase())
        );
    }
}

#[test]
fn a_valid_document_says_nothing_and_a_warning_does_not_stop_it() {
    let fine = document(&json!([{"id":"a","type":"activity","activity":"x.a"}]), "a");
    assert!(of_document(&fine).is_empty());
    // A compensation outside a saga never runs: a warning.
    let warned = document(
        &json!([{"id":"a","type":"activity","activity":"x.a","compensateWith":"x.undo"}]),
        "a",
    );
    let found = of_document(&warned);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].warning);
    assert_eq!(found[0].code, "COMPENSATION_OUTSIDE_SAGA");
    assert!(found[0].lines(None)[0].starts_with("warning[COMPENSATION_OUTSIDE_SAGA]"));
}

#[test]
fn the_help_and_the_link_are_told_once_for_each_code() {
    let nothing = of_document(b"{}");
    assert!(nothing.len() >= 4);
    let lines = render(&nothing, None);
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("  help:")).count(),
        1,
        "{lines:#?}"
    );
    assert_eq!(lines.iter().filter(|l| l.starts_with("  docs:")).count(), 1);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("error[SCHEMA]"))
            .count(),
        nothing.len()
    );
}

#[test]
fn the_link_leads_to_the_published_documentation_when_there_is_an_address() {
    assert_eq!(docs_link("CYCLE", None), "docs/quickstarts/errors.md#cycle");
    assert_eq!(
        docs_link("INVALID_NODE_ID", Some("https://docs.example.com/")),
        "https://docs.example.com/docs/quickstarts/errors.md#invalid_node_id"
    );
}

#[test]
fn what_a_document_says_is_shown_on_one_line() {
    // A name that holds a new line cannot forge a line of the output.
    let bytes = document(
        &json!([{"id":"a","type":"activity","activity":"x.a","next":"b\nerror[FORGED]: all is well"}]),
        "a",
    );
    let found = of_document(&bytes);
    let lines = render(&found, None);
    assert!(lines.iter().all(|l| !l.contains('\n')), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("error[FORGED]")),
        "{lines:?}"
    );
}
