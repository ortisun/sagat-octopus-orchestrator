//! Validation against the published JSON Schema (`specs/001-platform-foundation/contracts`).

use std::collections::HashMap;
use std::sync::OnceLock;

use jsonschema::Validator;
use sagat_application::definitions::IrIssue;
use serde_json::{Value, json};

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

/// Root node types in the order used by the canonical schema.
const ROOT_NODE_TYPES: &[&str] = &[
    "activity",
    "timer",
    "waitEvent",
    "condition",
    "parallel",
    "child",
    "saga",
    "script",
    "foreach",
    "humanApproval",
    "end",
    "forEach",
    "while",
    "repeatUntil",
    "approval",
    "switch",
    "continueAsNew",
];

/// Closed-body node types in the order used by the canonical schema.
const BODY_NODE_TYPES: &[&str] = &[
    "activity",
    "timer",
    "waitEvent",
    "condition",
    "parallel",
    "child",
    "saga",
    "script",
    "forEach",
    "while",
    "repeatUntil",
    "approval",
    "switch",
    "return",
];

/// One validator per node type, to say what is wrong with a node against the type it claims.
fn node_validators() -> &'static HashMap<&'static str, Validator> {
    static VALIDATORS: OnceLock<HashMap<&'static str, Validator>> = OnceLock::new();
    VALIDATORS.get_or_init(|| {
        let Ok(schema) = serde_json::from_str::<Value>(SCHEMA) else {
            return HashMap::new();
        };
        let definitions = schema.get("$defs").cloned().unwrap_or(Value::Null);
        ROOT_NODE_TYPES
            .iter()
            .chain(BODY_NODE_TYPES.iter())
            .filter_map(|kind| {
                let own = json!({"$ref": format!("#/$defs/{kind}"), "$defs": definitions});
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&own)
                    .ok()
                    .map(|validator| (*kind, validator))
            })
            .collect()
    })
}

/// The allowed node kinds at a failed `oneOf` path.
fn node_types(path: &str) -> Option<&'static [&'static str]> {
    if path
        .strip_prefix("/nodes/")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(ROOT_NODE_TYPES);
    }
    let mut parts = path.split('/');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(""), Some("bodies"), Some(body), Some("nodes"), Some(index), None)
            if !body.is_empty()
                && !index.is_empty()
                && index.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(BODY_NODE_TYPES)
        }
        _ => None,
    }
}

/// What is wrong with `node`, said against the type it claims to be (the generic answer of a
/// `oneOf`, "is not valid under any of the schemas", tells an author nothing).
fn node_issues(node: &Value, path: &str, expected: &[&str]) -> Vec<IrIssue> {
    let issue = |path: String, message: String| IrIssue {
        path,
        message: clip(&message),
    };
    let Some(kind) = node.get("type").and_then(Value::as_str) else {
        return vec![issue(
            format!("{path}/type"),
            format!("a node needs a `type`, one of {}", expected.join(", ")),
        )];
    };
    let Some(validator) = expected
        .contains(&kind)
        .then(|| node_validators().get(kind))
        .flatten()
    else {
        return vec![issue(
            format!("{path}/type"),
            format!(
                "unknown node type '{}'; expected one of {}",
                kind.chars().take(32).collect::<String>(),
                expected.join(", ")
            ),
        )];
    };
    validator
        .iter_errors(node)
        .take(MAX_ISSUES)
        .map(|error| {
            issue(
                format!("{path}{}", error.instance_path()),
                error.to_string(),
            )
        })
        .collect()
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
    let mut found = Vec::new();
    for error in validator.iter_errors(document).take(MAX_ISSUES) {
        let path = error.instance_path().to_string();
        let failed_one_of = format!("{:?}", error.kind()).starts_with("OneOf");
        if failed_one_of
            && let Some(expected) = node_types(&path)
            && let Some(node) = document.pointer(&path)
        {
            let precise = node_issues(node, &path, expected);
            if !precise.is_empty() {
                found.extend(precise);
                continue;
            }
        }
        found.push(IrIssue {
            path,
            message: clip(&error.to_string()),
        });
    }
    found.truncate(MAX_ISSUES);
    found
}
