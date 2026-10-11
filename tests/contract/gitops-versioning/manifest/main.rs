//! The manifest as a contract (T011-002, FR-011-002): the JSON Schema published for editors and
//! pipelines and the reader inside `sagat` agree on a corpus of manifests. A manifest the tool
//! accepts is valid by the schema (the schema is never stricter than the tool); one the schema
//! refuses is refused by the tool (the schema never lets through what the tool would not read).
//! What only the tool can check (a name or file listed twice) is in `invalid/cross-*` and is
//! refused by the tool alone.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use sagat_cli::manifest::Manifest;
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(
        root().join("specs/011-gitops-versioning/contracts/manifest.schema.json"),
    )
    .expect("the schema");
    let schema: Value = serde_json::from_str(&text).expect("a JSON document");
    jsonschema::options()
        .build(&schema)
        .expect("a valid schema")
}

fn corpus(kind: &str) -> Vec<(String, String)> {
    let dir = root()
        .join("tests/contract/gitops-versioning/fixtures/manifests")
        .join(kind);
    let mut found: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures")
        .map(|e| e.expect("entry").path())
        .map(|p| {
            (
                p.file_name().expect("name").to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).expect("read"),
            )
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no fixtures in {kind}");
    found
}

/// What the tool says about a manifest: nothing when it takes it, the first problem otherwise.
fn tool(text: &str) -> Result<(), String> {
    let manifest = Manifest::parse(text, true).map_err(|p| p.to_string())?;
    match manifest.problems().first() {
        Some(problem) => Err(problem.to_string()),
        None => Ok(()),
    }
}

fn as_json(text: &str) -> Value {
    serde_yaml_ng::from_str(text).expect("a YAML document")
}

#[test]
fn every_valid_manifest_is_accepted_by_the_schema_and_by_the_tool() {
    let schema = schema();
    for (name, text) in corpus("valid") {
        let instance = as_json(&text);
        let errors: Vec<_> = schema
            .iter_errors(&instance)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{name} by the schema: {errors:?}");
        assert_eq!(tool(&text), Ok(()), "{name} by the tool");
    }
}

#[test]
fn every_invalid_manifest_is_refused_by_the_tool_and_the_structural_ones_by_the_schema() {
    let schema = schema();
    for (name, text) in corpus("invalid") {
        assert!(tool(&text).is_err(), "{name} must be refused by the tool");
        let accepted = schema.is_valid(&as_json(&text));
        if name.starts_with("schema-") {
            assert!(!accepted, "{name} must be refused by the schema too");
        } else {
            assert!(
                name.starts_with("cross-"),
                "{name}: a fixture is either schema-* or cross-*"
            );
            assert!(
                accepted,
                "{name} is a rule the schema cannot state, and it does not claim to"
            );
        }
    }
}

#[test]
fn the_limits_are_the_same_on_both_sides() {
    let schema = schema();
    let manifest = |entries: Vec<Value>, namespace: &str| {
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Manifest",
            "namespace": namespace, "workflows": entries
        })
    };
    let entry = |i: usize| json!({"name": format!("flow-{i}"), "file": format!("f{i}.json")});
    let both = |instance: &Value| {
        let text = serde_yaml_ng::to_string(instance).expect("yaml");
        (schema.is_valid(instance), tool(&text).is_ok())
    };

    let at_the_limit = manifest((0..500).map(entry).collect(), "billing-dev");
    assert_eq!(both(&at_the_limit), (true, true));
    let over = manifest((0..501).map(entry).collect(), "billing-dev");
    assert_eq!(both(&over), (false, false), "501 workflows");

    let long = "a".repeat(63);
    assert_eq!(
        both(&manifest(vec![], &long)),
        (true, true),
        "63 characters"
    );
    let too_long = "a".repeat(64);
    assert_eq!(
        both(&manifest(vec![], &too_long)),
        (false, false),
        "64 characters"
    );

    let notes = |n: usize| {
        manifest(
            vec![json!({"name": "checkout", "file": "a.json", "releaseNotes": "x".repeat(n)})],
            "billing-dev",
        )
    };
    assert_eq!(both(&notes(2048)), (true, true));
    assert_eq!(both(&notes(2049)), (false, false));
}
