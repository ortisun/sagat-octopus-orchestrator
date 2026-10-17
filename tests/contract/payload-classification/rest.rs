//! Closed REST request and audience table for payload disclosure (T016-024).

use std::collections::BTreeSet;
use std::fs;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Value,
    Omitted,
    WithheldLevel,
    WithheldReveal,
    WithheldSecret,
    Forbidden,
}

fn request_validator() -> jsonschema::Validator {
    let openapi: Value = serde_saphyr::from_str(
        &fs::read_to_string(
            super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml"),
        )
        .expect("canonical OpenAPI exists"),
    )
    .expect("canonical OpenAPI is YAML");
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/components/schemas/PayloadDisclosureRequest",
        "components": openapi["components"].clone()
    });
    jsonschema::options()
        .should_validate_formats(true)
        .build(&root)
        .expect("PayloadDisclosureRequest compiles")
}

#[test]
fn request_envelope_is_closed_at_one_selector_and_one_explicit_pii_reveal() {
    let validator = request_validator();
    for valid in [
        json!({"source":"input"}),
        json!({"source":"result"}),
        json!({"source":"journal", "sequence":1}),
        json!({"source":"journal", "sequence":i64::MAX, "reveal":["pii"]}),
    ] {
        assert!(validator.is_valid(&valid), "rejected valid request {valid}");
    }
    for invalid in [
        json!({"source":"unknown"}),
        json!({"source":"journal"}),
        json!({"source":"journal", "sequence":0}),
        json!({"source":"journal", "sequence":-1}),
        json!({"source":"journal", "sequence":1.5}),
        json!({"source":"input", "sequence":1}),
        json!({"source":"input", "reveal":["secret"]}),
        json!({"source":"input", "reveal":["pii", "pii"]}),
        json!({"source":"input", "unknown":true}),
    ] {
        assert!(
            !validator.is_valid(&invalid),
            "accepted invalid request {invalid}"
        );
    }
}

#[test]
fn audience_table_has_every_role_level_reveal_and_grant_cell() {
    let roles = [
        "viewer",
        "developer",
        "operator",
        "admin",
        "worker",
        "runner",
    ];
    let levels = ["public", "internal", "pii", "secret"];
    let mut rows = BTreeSet::new();
    for role in roles {
        for level in levels {
            for reveal in [false, true] {
                for grant in [false, true] {
                    rows.insert((
                        role,
                        level,
                        reveal,
                        grant,
                        outcome(role, level, reveal, grant),
                    ));
                }
            }
        }
    }
    assert_eq!(rows.len(), 6 * 4 * 2 * 2);
    assert_eq!(outcome("viewer", "public", false, false), Outcome::Value);
    assert_eq!(
        outcome("developer", "internal", false, false),
        Outcome::Value
    );
    assert_eq!(
        outcome("operator", "pii", false, false),
        Outcome::WithheldLevel
    );
    assert_eq!(
        outcome("operator", "pii", false, true),
        Outcome::WithheldReveal
    );
    assert_eq!(outcome("admin", "pii", true, true), Outcome::Value);
    assert_eq!(
        outcome("admin", "secret", true, true),
        Outcome::WithheldSecret
    );
    assert_eq!(
        outcome("worker", "public", false, false),
        Outcome::Forbidden
    );
    assert_eq!(
        outcome("runner", "public", false, false),
        Outcome::Forbidden
    );
}

fn outcome(role: &str, level: &str, reveal: bool, grant: bool) -> Outcome {
    if matches!(role, "worker" | "runner") || (reveal && !(grant && has_internal(role))) {
        return Outcome::Forbidden;
    }
    match level {
        "public" => Outcome::Value,
        "internal" if has_internal(role) => Outcome::Value,
        "internal" => Outcome::Omitted,
        "pii" if !has_internal(role) => Outcome::Omitted,
        "pii" if reveal => Outcome::Value,
        "pii" if matches!(role, "operator" | "admin") && grant => Outcome::WithheldReveal,
        "pii" if matches!(role, "operator" | "admin") => Outcome::WithheldLevel,
        "pii" => Outcome::Omitted,
        "secret" if matches!(role, "operator" | "admin") => Outcome::WithheldSecret,
        "secret" => Outcome::Omitted,
        _ => unreachable!("closed level table"),
    }
}

fn has_internal(role: &str) -> bool {
    matches!(role, "developer" | "operator" | "admin")
}
