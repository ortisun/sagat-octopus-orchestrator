//! Validation against the published JSON Schema (`specs/001-platform-foundation/contracts`).

use std::sync::OnceLock;

use jsonschema::Validator;
use sagat_application::definitions::IrIssue;
use serde_json::Value;

const SCHEMA: &str =
    include_str!("../../../specs/001-platform-foundation/contracts/workflow.schema.json");
/// Most issues reported at once; a document with thousands of problems gets the first few.
const MAX_ISSUES: usize = 20;
const MAX_MESSAGE: usize = 200;

fn validator() -> Option<&'static Validator> {
    static VALIDATOR: OnceLock<Option<Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .ok()
        })
        .as_ref()
}

fn clip(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(MAX_MESSAGE).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// The schema problems of `document`, empty when it conforms.
pub(crate) fn issues(document: &Value) -> Vec<IrIssue> {
    let Some(validator) = validator() else {
        // The embedded schema is part of the build; failing to compile it is a build defect, and
        // refusing every document is the safe response.
        return vec![IrIssue {
            path: String::new(),
            message: "the workflow schema could not be loaded".into(),
        }];
    };
    validator
        .iter_errors(document)
        .take(MAX_ISSUES)
        .map(|error| IrIssue {
            path: error.instance_path().to_string(),
            message: clip(&error.to_string()),
        })
        .collect()
}
