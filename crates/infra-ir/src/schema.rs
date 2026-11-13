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

/// Which grammar a document is validated against (ADR-0080): the **public** one, which is the contract, or the **private** one, which is
/// the same contract plus the node types only the platform may write. The private grammar is derived from the public one when it is
/// first used and is never kept as a second copy; a test holds that the two differ by exactly the private node types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Public,
    Private,
}

/// The node types only the platform may write. None of them is in `workflow.schema.json`.
const PRIVATE_NODE_TYPES: &[&str] = &["icebergProbe"];

/// The probe of an Iceberg connection (D035-150): the connection to test, where to go next and how long the attempt may take, and
/// nothing else (no table, mapping or records).
fn probe_node_schema() -> Value {
    json!({
        "description": "The probe of an Iceberg connection. Private: only the platform writes it (ADR-0080), and it is not part of the public contract.",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "type", "connectionRef"],
        "properties": {
            "id": {"type": "string"},
            "type": {"const": "icebergProbe"},
            "connectionRef": {"$ref": "#/$defs/lakeConnectionRef"},
            "next": {"type": "string"},
            "timeoutMs": {"type": "integer", "minimum": 1}
        }
    })
}

/// The schema of a mode: the embedded public one, and for the private mode that one with the private node types added, one definition
/// each and one member of the root `node` list. The bodies of a structured workflow are untouched: a private node is root-only.
fn schema_of(mode: Mode) -> Option<Value> {
    let mut schema: Value = serde_json::from_str(SCHEMA).ok()?;
    if mode == Mode::Private {
        let definitions = schema.get_mut("$defs")?.as_object_mut()?;
        for kind in PRIVATE_NODE_TYPES {
            definitions.insert((*kind).to_owned(), probe_node_schema());
        }
        let members = definitions
            .get_mut("node")?
            .get_mut("oneOf")?
            .as_array_mut()?;
        for kind in PRIVATE_NODE_TYPES {
            members.push(json!({"$ref": format!("#/$defs/{kind}")}));
        }
    }
    Some(schema)
}

fn validator(mode: Mode) -> Option<&'static Validator> {
    static PUBLIC: OnceLock<Option<Validator>> = OnceLock::new();
    static PRIVATE: OnceLock<Option<Validator>> = OnceLock::new();
    let cell = match mode {
        Mode::Public => &PUBLIC,
        Mode::Private => &PRIVATE,
    };
    cell.get_or_init(|| {
        let schema = schema_of(mode)?;
        jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .ok()
    })
    .as_ref()
}

/// The root node types a mode knows, in the order of its schema.
fn root_node_types(mode: Mode) -> &'static [&'static str] {
    static PRIVATE: OnceLock<Vec<&'static str>> = OnceLock::new();
    match mode {
        Mode::Public => ROOT_NODE_TYPES,
        Mode::Private => PRIVATE.get_or_init(|| {
            ROOT_NODE_TYPES
                .iter()
                .chain(PRIVATE_NODE_TYPES)
                .copied()
                .collect()
        }),
    }
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
    "database",
    "icebergWrite",
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
fn node_validators(mode: Mode) -> &'static HashMap<&'static str, Validator> {
    static PUBLIC: OnceLock<HashMap<&'static str, Validator>> = OnceLock::new();
    static PRIVATE: OnceLock<HashMap<&'static str, Validator>> = OnceLock::new();
    let cell = match mode {
        Mode::Public => &PUBLIC,
        Mode::Private => &PRIVATE,
    };
    cell.get_or_init(|| {
        let Some(schema) = schema_of(mode) else {
            return HashMap::new();
        };
        let definitions = schema.get("$defs").cloned().unwrap_or(Value::Null);
        root_node_types(mode)
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
fn node_types(path: &str, mode: Mode) -> Option<&'static [&'static str]> {
    if path
        .strip_prefix("/nodes/")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(root_node_types(mode));
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
fn node_issues(node: &Value, path: &str, expected: &[&str], mode: Mode) -> Vec<IrIssue> {
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
        .then(|| node_validators(mode).get(kind))
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
            // JSON Schema diagnostics can quote the entire invalid instance (including SQL and
            // values), or user-authored unknown keys. Only a safe node path leaves this boundary.
            if kind == "database" {
                return issue(
                    path.to_owned(),
                    "invalid database node fields or limits".into(),
                );
            }
            if kind == "icebergWrite" {
                return issue(
                    path.to_owned(),
                    "invalid icebergWrite node fields or limits".into(),
                );
            }
            if PRIVATE_NODE_TYPES.contains(&kind) {
                return issue(path.to_owned(), format!("invalid {kind} node fields"));
            }
            issue(
                format!("{path}{}", error.instance_path()),
                error.to_string(),
            )
        })
        .collect()
}

/// The schema problems of `document`, empty when it conforms.
pub(crate) fn issues(document: &Value, mode: Mode) -> Vec<IrIssue> {
    let Some(validator) = validator(mode) else {
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
            && let Some(expected) = node_types(&path, mode)
            && let Some(node) = document.pointer(&path)
        {
            let precise = node_issues(node, &path, expected, mode);
            if !precise.is_empty() {
                found.extend(precise);
                continue;
            }
        }
        found.push(IrIssue {
            path,
            message: if document
                .get("nodes")
                .and_then(Value::as_array)
                .is_some_and(|nodes| {
                    nodes
                        .iter()
                        .any(|n| n.get("type").and_then(Value::as_str) == Some("database"))
                }) {
                "invalid workflow document containing a database operation".into()
            } else {
                clip(&error.to_string())
            },
        });
    }
    found.truncate(MAX_ISSUES);
    found
}

#[cfg(test)]
mod embedded_contract_tests {
    #[test]
    #[allow(clippy::expect_used)]
    fn embedded_workflow_schema_matches_the_canonical_contract_bytes() {
        let canonical = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../specs/001-platform-foundation/contracts/workflow.schema.json"),
        )
        .expect("canonical workflow schema");
        assert_eq!(super::SCHEMA.as_bytes(), canonical);
    }

    /// ADR-0080: the private grammar is the public one and nothing else but the private node types. Take the additions away and what is
    /// left is the public schema, byte for byte; and the additions are exactly one definition and one member of the root list for each
    /// private node type, none of which the public schema has.
    #[test]
    #[allow(clippy::expect_used)]
    fn the_private_schema_is_the_public_schema_plus_the_private_node_types_and_nothing_else() {
        use super::{Mode, PRIVATE_NODE_TYPES, schema_of};
        let public = schema_of(Mode::Public).expect("the public schema");
        let mut private = schema_of(Mode::Private).expect("the private schema");
        assert_ne!(public, private, "the private grammar adds something");
        let definitions = private["$defs"].as_object_mut().expect("definitions");
        for kind in PRIVATE_NODE_TYPES {
            assert!(
                public["$defs"].get(*kind).is_none(),
                "{kind} is not in the public schema"
            );
            assert!(definitions.remove(*kind).is_some(), "{kind} is defined");
        }
        let members = definitions["node"]["oneOf"]
            .as_array_mut()
            .expect("the node list");
        let added = members.split_off(members.len() - PRIVATE_NODE_TYPES.len());
        let expected: Vec<serde_json::Value> = PRIVATE_NODE_TYPES
            .iter()
            .map(|kind| serde_json::json!({"$ref": format!("#/$defs/{kind}")}))
            .collect();
        assert_eq!(added, expected, "one member of the root list for each");
        assert_eq!(private, public, "and nothing else");
    }

    #[test]
    fn no_private_node_type_is_in_the_text_of_the_public_contract() {
        for kind in super::PRIVATE_NODE_TYPES {
            assert!(
                !super::SCHEMA.contains(kind),
                "{kind} must not be in workflow.schema.json"
            );
        }
    }
}
